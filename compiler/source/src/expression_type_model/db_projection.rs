use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{FunctionTypeParamIr, TypeRefIr};
use skiff_compiler_core::db_projection::project_db_read_type;

use crate::{
    shared::type_expr::TypeExpr, PublicationDbMetadata, PublicationDbMetadataIndex,
    ResolvedTypeRef, TypeResolutionContext, TypeResolutionModel,
};

pub(super) struct DbProjectionTypeResolver<'a> {
    module_path: &'a str,
    type_resolution: &'a TypeResolutionModel,
    metadata: &'a PublicationDbMetadataIndex,
}

impl<'a> DbProjectionTypeResolver<'a> {
    pub(super) fn new(
        module_path: &'a str,
        type_resolution: &'a TypeResolutionModel,
        metadata: &'a PublicationDbMetadataIndex,
    ) -> Self {
        Self {
            module_path,
            type_resolution,
            metadata,
        }
    }

    pub(super) fn project_read_type(
        &self,
        target_name: &str,
        full_target: TypeRefIr,
        projection_paths: &[Vec<String>],
    ) -> Result<TypeRefIr, String> {
        let metadata = self.resolve_metadata(target_name)?.ok_or_else(|| {
            format!("db read projection target `{target_name}` has no DB metadata")
        })?;
        let field_types = self.projection_field_types(metadata)?;
        project_db_read_type(
            &metadata.type_name,
            &metadata.key.name,
            full_target,
            &field_types,
            Some(projection_paths),
        )
    }

    fn resolve_metadata(
        &self,
        target_name: &str,
    ) -> Result<Option<&PublicationDbMetadata>, String> {
        if target_name.contains('.') {
            return Ok(self.metadata.resolve_qualified(target_name));
        }
        let local_name = format!("{}.{}", self.module_path, target_name);
        if let Some(metadata) = self.metadata.resolve_qualified(&local_name) {
            return Ok(Some(metadata));
        }
        self.metadata
            .resolve_bare(target_name)
            .map_err(|error| error.to_string())
    }

    fn projection_field_types(
        &self,
        metadata: &PublicationDbMetadata,
    ) -> Result<BTreeMap<String, TypeRefIr>, String> {
        let context = TypeResolutionContext::source(&metadata.module_path);
        let mut seen = BTreeSet::new();
        metadata
            .field_types
            .iter()
            .map(|(name, ty)| {
                Ok((
                    name.clone(),
                    self.resolve_structural_type(&ty.name, &context, &mut seen)?,
                ))
            })
            .collect()
    }

    fn resolve_structural_type(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
        seen: &mut BTreeSet<String>,
    ) -> Result<TypeRefIr, String> {
        let resolved = self.type_resolution.resolve_type_text(raw, context)?;
        self.expand_structural_type(&resolved, context, seen)
    }

    fn expand_structural_type(
        &self,
        resolved: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
        seen: &mut BTreeSet<String>,
    ) -> Result<TypeRefIr, String> {
        let expression = TypeExpr::parse(&resolved.source_text);
        match expression {
            TypeExpr::EmptyRecord => Ok(TypeRefIr::Record {
                fields: BTreeMap::new(),
            }),
            TypeExpr::Nullable(inner) => Ok(TypeRefIr::Nullable {
                inner: Box::new(self.resolve_structural_type(
                    &inner.to_type_string(),
                    context,
                    seen,
                )?),
            }),
            TypeExpr::Union(items) => Ok(TypeRefIr::Union {
                items: items
                    .iter()
                    .map(|item| self.resolve_structural_type(&item.to_type_string(), context, seen))
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            TypeExpr::Record(fields) => Ok(TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|field| {
                        Ok((
                            field.name.clone(),
                            self.resolve_structural_type(
                                &field.ty.to_type_string(),
                                context,
                                seen,
                            )?,
                        ))
                    })
                    .collect::<Result<BTreeMap<_, _>, String>>()?,
            }),
            TypeExpr::Named { name: _, args } => {
                let marker = format!("{}::{}", context.module_path, resolved.source_text);
                if matches!(
                    &resolved.ir,
                    TypeRefIr::LocalType { .. }
                        | TypeRefIr::PublicationType { .. }
                        | TypeRefIr::ServiceSymbol { .. }
                        | TypeRefIr::PackageSymbol { .. }
                        | TypeRefIr::DbObjectSymbol { .. }
                ) && seen.insert(marker.clone())
                {
                    if let Ok(target) = self
                        .type_resolution
                        .resolve_constructor_target_text(&resolved.source_text, context)
                    {
                        let fields = target
                            .fields
                            .into_iter()
                            .map(|(field, ty)| {
                                Ok((field, self.expand_structural_type(&ty, context, seen)?))
                            })
                            .collect::<Result<BTreeMap<_, _>, String>>();
                        seen.remove(&marker);
                        return Ok(TypeRefIr::Record { fields: fields? });
                    }
                    seen.remove(&marker);
                }

                match &resolved.ir {
                    TypeRefIr::Native {
                        name: resolved_name,
                        args: resolved_args,
                    } if args.len() == resolved_args.len() => Ok(TypeRefIr::Native {
                        name: resolved_name.clone(),
                        args: args
                            .iter()
                            .map(|arg| {
                                self.resolve_structural_type(&arg.to_type_string(), context, seen)
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    }),
                    _ => Ok(resolved.ir.clone()),
                }
            }
            TypeExpr::Function {
                params,
                return_type,
            } => Ok(TypeRefIr::Function {
                params: params
                    .iter()
                    .map(|param| {
                        Ok(FunctionTypeParamIr {
                            name: param.name.clone(),
                            ty: self.resolve_structural_type(
                                &param.ty.to_type_string(),
                                context,
                                seen,
                            )?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?,
                return_type: Box::new(self.resolve_structural_type(
                    &return_type.to_type_string(),
                    context,
                    seen,
                )?),
            }),
            TypeExpr::StringLiteral(_) | TypeExpr::AnyInterface { .. } => Ok(resolved.ir.clone()),
        }
    }
}
