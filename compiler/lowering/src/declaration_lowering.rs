use std::collections::{BTreeMap, BTreeSet};

use crate::file_ir::{
    FileIrUnit, FunctionTypeParamIr, InterfaceDeclIr, InterfaceOperationIr, TypeDeclIr,
    TypeDeclarationIr, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_source::{
    LocalDbObjectIndex, PublicationDbMetadataIndex, PublicationTypeSymbolIndex,
};
use skiff_syntax::{
    ast::{AliasDecl, InterfaceDecl, InterfaceOperation, TypeDecl},
    error::Result,
};

use super::{
    function_lowering::LocalTypeFieldIndex,
    source_unit_lowering::{
        push_source_span, source_span_ref, symbol, type_index, type_param_scope,
    },
    type_lowering::{lower_type_ref, TypeLoweringContext},
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
    type_indices: &BTreeMap<String, u32>,
    module_path: &str,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    source_alias_targets: &BTreeMap<String, String>,
    unit: &mut FileIrUnit,
    next_span_id: &mut u64,
) -> Result<()> {
    for ty in types {
        let type_index = type_index(type_indices, &ty.name)?;
        let source_span = source_span_ref(ty.span);
        let type_params = type_param_scope(std::iter::empty::<&String>(), ty.type_params.iter());
        unit.type_table.push(TypeDeclIr {
            name: ty.name.clone(),
            descriptor: lower_type_decl_descriptor(
                ty,
                &type_params,
                type_indices,
                local_db_objects,
                publication_db_metadata,
                package_aliases,
                external_type_symbols,
                source_alias_targets,
            )?,
            type_params: ty.type_params.clone(),
            discriminator: ty.discriminator.clone(),
            implements: ty
                .implements
                .iter()
                .map(|implemented| {
                    lower_type_ref(
                        implemented,
                        type_indices,
                        local_db_objects,
                        publication_db_metadata,
                        package_aliases,
                        external_type_symbols,
                        source_alias_targets,
                        TypeLoweringContext::value_with_type_params(&type_params),
                    )
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
        // `LoweredPublication::lower`).
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
                    type_indices,
                    local_db_objects,
                    publication_db_metadata,
                    package_aliases,
                    external_type_symbols,
                    source_alias_targets,
                    TypeLoweringContext::value(),
                )?,
            },
            type_params: Vec::new(),
            discriminator: None,
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
        // link_targets recomputed post-lowering (see `LoweredPublication::lower`).
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
            descriptor: TypeDescriptorIr::Native {
                symbol: symbol(module_path, &interface.name),
            },
            type_params: interface.type_params.clone(),
            discriminator: None,
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
            lower_interface_declaration(
                interface,
                type_indices,
                local_db_objects,
                publication_db_metadata,
                package_aliases,
                external_type_symbols,
                source_alias_targets,
            )?,
        );
        // link_targets recomputed post-lowering (see `LoweredPublication::lower`).
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
    type_indices: &BTreeMap<String, u32>,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    source_alias_targets: &BTreeMap<String, String>,
) -> Result<TypeDescriptorIr> {
    if let Some(alias) = &ty.alias {
        let lowered = lower_type_ref(
            alias,
            type_indices,
            local_db_objects,
            publication_db_metadata,
            package_aliases,
            external_type_symbols,
            source_alias_targets,
            TypeLoweringContext::value_with_type_params(type_param_scope),
        )?;
        return Ok(match lowered {
            TypeRefIr::Union { items } => TypeDescriptorIr::Union { variants: items },
            other => TypeDescriptorIr::Alias { target: other },
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
                    type_indices,
                    local_db_objects,
                    publication_db_metadata,
                    package_aliases,
                    external_type_symbols,
                    source_alias_targets,
                    TypeLoweringContext::value_with_type_params(type_param_scope),
                )?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(TypeDescriptorIr::Record { fields })
}

fn lower_interface_declaration(
    interface: &InterfaceDecl,
    type_indices: &BTreeMap<String, u32>,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    source_alias_targets: &BTreeMap<String, String>,
) -> Result<InterfaceDeclIr> {
    let interface_type_params =
        type_param_scope(std::iter::empty::<&String>(), interface.type_params.iter());
    Ok(InterfaceDeclIr {
        name: interface.name.clone(),
        type_params: interface.type_params.clone(),
        operations: interface
            .operations
            .iter()
            .map(|operation| {
                lower_interface_operation(
                    operation,
                    &interface_type_params,
                    type_indices,
                    local_db_objects,
                    publication_db_metadata,
                    package_aliases,
                    external_type_symbols,
                    source_alias_targets,
                )
            })
            .collect::<Result<Vec<_>>>()?,
        source_span: Some(source_span_ref(interface.span)),
    })
}

fn lower_interface_operation(
    operation: &InterfaceOperation,
    interface_type_params: &BTreeSet<String>,
    type_indices: &BTreeMap<String, u32>,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    source_alias_targets: &BTreeMap<String, String>,
) -> Result<InterfaceOperationIr> {
    let type_params = type_param_scope(interface_type_params.iter(), operation.type_params.iter());
    let context = TypeLoweringContext::value_with_type_params(&type_params);
    Ok(InterfaceOperationIr {
        name: operation.name.clone(),
        type_params: operation.type_params.clone(),
        params: operation
            .params
            .iter()
            .map(|param| {
                Ok(FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: lower_type_ref(
                        &param.ty,
                        type_indices,
                        local_db_objects,
                        publication_db_metadata,
                        package_aliases,
                        external_type_symbols,
                        source_alias_targets,
                        context,
                    )?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        return_type: lower_type_ref(
            &operation.return_type,
            type_indices,
            local_db_objects,
            publication_db_metadata,
            package_aliases,
            external_type_symbols,
            source_alias_targets,
            context,
        )?,
        is_native: operation.is_native,
        is_provider: operation.is_provider,
        is_static: operation.is_static,
        implicit_self: operation
            .implicit_self
            .as_ref()
            .map(|ty| {
                lower_type_ref(
                    ty,
                    type_indices,
                    local_db_objects,
                    publication_db_metadata,
                    package_aliases,
                    external_type_symbols,
                    source_alias_targets,
                    context,
                )
            })
            .transpose()?,
    })
}
