use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{FunctionTypeParamIr, TypeRefIr};
use skiff_compiler_core::db_projection::project_db_read_type;

use crate::{
    PublicationDbMetadata, PublicationDbMetadataIndex, ResolvedTypeRef, TypeResolutionContext,
    TypeResolutionModel,
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
        if let Some(key_type) = &metadata.canonical_key_type {
            let mut fields = metadata.canonical_field_types.clone();
            fields.insert(metadata.key.name.clone(), key_type.clone());
            return Ok(fields);
        }
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
        let recurse = |ty: &TypeRefIr, seen: &mut BTreeSet<String>| {
            self.expand_structural_type(
                &ResolvedTypeRef::with_text(ty.clone(), String::new()),
                context,
                seen,
            )
        };
        match &resolved.ir {
            TypeRefIr::Record { fields } => Ok(TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| Ok((name.clone(), recurse(ty, seen)?)))
                    .collect::<Result<_, String>>()?,
            }),
            TypeRefIr::Nullable { inner } => Ok(TypeRefIr::Nullable {
                inner: Box::new(recurse(inner, seen)?),
            }),
            TypeRefIr::Union { items } => Ok(TypeRefIr::Union {
                items: items
                    .iter()
                    .map(|item| recurse(item, seen))
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            TypeRefIr::Function {
                params,
                return_type,
            } => Ok(TypeRefIr::Function {
                params: params
                    .iter()
                    .map(|param| {
                        Ok(FunctionTypeParamIr {
                            name: param.name.clone(),
                            ty: recurse(&param.ty, seen)?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?,
                return_type: Box::new(recurse(return_type, seen)?),
            }),
            TypeRefIr::Builtin { name, args } => Ok(TypeRefIr::Builtin {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| recurse(arg, seen))
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::AppliedNominal { .. } => {
                let marker = serde_json::to_string(&resolved.ir)
                    .map_err(|error| format!("DB projection type marker failed: {error}"))?;
                if matches!(
                    &resolved.ir,
                    TypeRefIr::LocalType { .. }
                        | TypeRefIr::PublicationType { .. }
                        | TypeRefIr::ServiceSymbol { .. }
                        | TypeRefIr::PackageSymbol { .. }
                        | TypeRefIr::AppliedNominal { .. }
                ) && seen.insert(marker.clone())
                {
                    if let Ok(target) = self
                        .type_resolution
                        .resolve_constructor_target_resolved(resolved, context)
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
                Ok(resolved.ir.clone())
            }
            _ => Ok(resolved.ir.clone()),
        }
    }
}
