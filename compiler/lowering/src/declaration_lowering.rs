use std::collections::{BTreeMap, BTreeSet};

use crate::file_ir::{
    FileIrUnit, LiteralIr, TypeDeclIr, TypeDeclarationIr, TypeDescriptorIr, TypeRefIr,
};
use skiff_artifact_identity::interface_instantiation_ref;
use skiff_artifact_model::NamedUnionBranchIr;
use skiff_compiler_source::{
    LocalDbObjectIndex, PublicationDbMetadataIndex, PublicationTypeSymbolIndex,
    SourceInterfaceSignatureFacts, TypeResolutionContext, TypeResolutionModel,
};
use skiff_syntax::{
    ast::{AliasDecl, InterfaceDecl, TypeDecl, TypeRef},
    error::{CompileError, Result},
    type_syntax::split_top_level,
};

use super::{
    function_lowering::LocalTypeFieldIndex,
    interface_declaration_lowering::lower_interface_declaration,
    source_unit_lowering::{
        push_source_span, source_span_ref, symbol, type_index, type_param_scope,
    },
    type_lowering::{lower_type_ref, TypeLoweringContext, TypeLoweringEnvironment},
};

pub(super) fn local_type_field_index(unit: &FileIrUnit) -> LocalTypeFieldIndex {
    unit.type_table
        .iter()
        .enumerate()
        .filter_map(|(type_index, declaration)| {
            let TypeDescriptorIr::Record { fields } = &declaration.descriptor else {
                return None;
            };
            Some((type_index as u32, fields.clone()))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn lower_type_declarations(
    types: &[TypeDecl],
    aliases: &[AliasDecl],
    interfaces: &[InterfaceDecl],
    interface_signatures: Option<&SourceInterfaceSignatureFacts>,
    type_indices: &BTreeMap<String, u32>,
    module_path: &str,
    type_resolution: &TypeResolutionModel,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    source_alias_targets: &BTreeMap<String, String>,
    unit: &mut FileIrUnit,
    next_span_id: &mut u64,
) -> Result<()> {
    let type_environment = TypeLoweringEnvironment::new(
        type_indices,
        local_db_objects,
        publication_db_metadata,
        package_aliases,
        external_type_symbols,
        source_alias_targets,
    );
    for ty in types {
        let type_index = type_index(type_indices, &ty.name)?;
        let source_span = source_span_ref(ty.span);
        let type_params = type_param_scope(std::iter::empty::<&String>(), ty.type_params.iter());
        unit.type_table.push(TypeDeclIr {
            name: ty.name.clone(),
            descriptor: lower_type_decl_descriptor(
                ty,
                &type_params,
                type_environment,
                type_resolution,
                module_path,
            )?,
            type_params: ty.type_params.clone(),
            implements: ty
                .implements
                .iter()
                .map(|implemented| {
                    let context =
                        TypeResolutionContext::with_type_params(module_path, type_params.clone());
                    type_resolution
                        .resolve_canonical_interface_selector_type_ref(implemented, &context)
                        .and_then(|selector| {
                            let identity = match selector.identity {
                                TypeRefIr::ServiceSymbol { symbol }
                                    if symbol
                                        .module_path
                                        .strip_prefix("root.")
                                        .unwrap_or(&symbol.module_path)
                                        == module_path =>
                                {
                                    TypeRefIr::LocalType {
                                        type_index: *type_indices
                                            .get(&symbol.symbol)
                                            .ok_or_else(|| {
                                                format!(
                                                    "local interface `{}` has no File IR type index",
                                                    symbol.symbol
                                                )
                                            })?,
                                    }
                                }
                                identity => identity,
                            };
                            Ok(TypeRefIr::AnyInterface {
                                interface: interface_instantiation_ref(identity, selector.args),
                            })
                        })
                        .map_err(|error| {
                            CompileError::Semantic(format!(
                                "type `{}` implements invalid interface selector `{}`: {error}",
                                ty.name, implemented.name
                            ))
                        })
                })
                .collect::<Result<Vec<_>>>()?,
            source_span: Some(source_span.clone()),
        });
        unit.declarations.types.insert(
            ty.name.clone(),
            TypeDeclarationIr {
                type_index,
                symbol: symbol(module_path, &ty.name),
                source_span: Some(source_span.clone()),
            },
        );
        // link_targets are no longer derived from the per-declaration `exported`
        // modifier; they are recomputed in a post-lowering pass from the
        // re-export set plus the ABI/schema closure (see
        // `LoweredPackage::lower`).
        push_source_span(
            &mut unit.source_map.spans,
            next_span_id,
            "type",
            &ty.name,
            ty.span,
        );
    }

    for alias in aliases {
        let type_index = type_index(type_indices, &alias.name)?;
        let source_span = source_span_ref(alias.span);
        unit.type_table.push(TypeDeclIr {
            name: alias.name.clone(),
            descriptor: TypeDescriptorIr::Alias {
                target: lower_type_ref(
                    &alias.target_type,
                    type_environment,
                    TypeLoweringContext::value(),
                )?,
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: Some(source_span.clone()),
        });
        unit.declarations.types.insert(
            alias.name.clone(),
            TypeDeclarationIr {
                type_index,
                symbol: symbol(module_path, &alias.name),
                source_span: Some(source_span.clone()),
            },
        );
        // link_targets recomputed post-lowering (see `LoweredPackage::lower`).
        push_source_span(
            &mut unit.source_map.spans,
            next_span_id,
            "alias",
            &alias.name,
            alias.span,
        );
    }

    for interface in interfaces {
        let type_index = type_index(type_indices, &interface.name)?;
        let source_span = source_span_ref(interface.span);
        unit.type_table.push(TypeDeclIr {
            name: interface.name.clone(),
            descriptor: TypeDescriptorIr::Interface,
            type_params: interface.type_params.clone(),
            implements: Vec::new(),
            source_span: Some(source_span.clone()),
        });
        unit.declarations.types.insert(
            interface.name.clone(),
            TypeDeclarationIr {
                type_index,
                symbol: symbol(module_path, &interface.name),
                source_span: Some(source_span.clone()),
            },
        );
        unit.declarations.interfaces.insert(
            interface.name.clone(),
            lower_interface_declaration(interface, interface_signatures, module_path)?,
        );
        // link_targets recomputed post-lowering (see `LoweredPackage::lower`).
        push_source_span(
            &mut unit.source_map.spans,
            next_span_id,
            "interface",
            &interface.name,
            interface.span,
        );
    }
    Ok(())
}

fn lower_type_decl_descriptor(
    ty: &TypeDecl,
    type_param_scope: &BTreeSet<String>,
    type_environment: TypeLoweringEnvironment<'_>,
    type_resolution: &TypeResolutionModel,
    module_path: &str,
) -> Result<TypeDescriptorIr> {
    if let Some(alias) = &ty.alias {
        let branches = split_top_level(&alias.name, '|');
        if branches.len() > 1 {
            let context =
                TypeResolutionContext::with_type_params(module_path, type_param_scope.clone());
            return Ok(TypeDescriptorIr::Union {
                branches: branches
                    .into_iter()
                    .map(|branch| lower_named_union_branch(ty, branch, &context, type_resolution))
                    .collect::<Result<Vec<_>>>()?,
            });
        }
        return Ok(TypeDescriptorIr::Representation {
            representation: lower_type_ref(
                alias,
                type_environment,
                TypeLoweringContext::value_with_type_params(type_param_scope),
            )?,
        });
    }

    let fields = ty
        .fields
        .iter()
        .map(|field| {
            Ok((
                field.name.clone(),
                lower_type_ref(
                    &field.ty,
                    type_environment,
                    TypeLoweringContext::value_with_type_params(type_param_scope),
                )?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(TypeDescriptorIr::Record { fields })
}

fn lower_named_union_branch(
    owner: &TypeDecl,
    branch: &str,
    type_context: &TypeResolutionContext<'_>,
    type_resolution: &TypeResolutionModel,
) -> Result<NamedUnionBranchIr> {
    let branch_ref = TypeRef {
        name: branch.to_string(),
    };
    let lowered = type_resolution
        .resolve_type_ref(&branch_ref, type_context)
        .map(|resolved| resolved.ir)
        .map_err(|error| {
            CompileError::Semantic(format!(
                "named union `{}` branch `{branch}` has invalid nominal type: {error}",
                owner.name
            ))
        });

    if branch.trim().starts_with('{') {
        let payload_type = lowered?;
        let TypeRefIr::Record { fields } = &payload_type else {
            return Err(invalid_named_union_branch(owner, branch));
        };
        let discriminator_field = owner
            .discriminator
            .as_deref()
            .ok_or_else(|| invalid_named_union_branch(owner, branch))?;
        let Some(TypeRefIr::Literal {
            value: LiteralIr::String {
                value: discriminator_value,
            },
        }) = fields.get(discriminator_field)
        else {
            return Err(invalid_named_union_branch(owner, branch));
        };
        let discriminator_value = discriminator_value.clone();
        return Ok(NamedUnionBranchIr::SyntheticDiscriminator {
            payload_type,
            discriminator_field: discriminator_field.to_string(),
            discriminator_value,
        });
    }

    if branch.trim().starts_with('"') {
        let TypeRefIr::Literal { value } = lowered? else {
            return Err(invalid_named_union_branch(owner, branch));
        };
        return Ok(NamedUnionBranchIr::Literal { value });
    }

    let nominal_type = lowered?;
    if !matches!(
        nominal_type,
        TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::PackageSchema { .. }
            | TypeRefIr::AppliedNominal { .. }
    ) {
        return Err(invalid_named_union_branch(owner, branch));
    }
    Ok(NamedUnionBranchIr::ConcreteNominal { nominal_type })
}

fn invalid_named_union_branch(owner: &TypeDecl, branch: &str) -> CompileError {
    CompileError::Semantic(format!(
        "named union `{}` branch `{branch}` must be a concrete nominal type, anonymous discriminator record, or literal",
        owner.name
    ))
}
