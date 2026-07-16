use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{canonical_interface_instantiation_key, interface_instantiation_ref};
use skiff_artifact_model::{
    FileIrUnit, PackageRefIr, PackageSymbolRef, ServiceSymbolRef, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_core::package_interface_methods::instantiate_interface_method_signatures;

use crate::{
    package_exports::PackageExportPublicInstanceInterface,
    package_unit_artifacts::PackageIrProjectionSource,
    publication_visible_types::{
        projection_visible_interface_method_signature, projection_visible_type_ref,
        PublicationVisibleTypeNames,
    },
    typed_artifacts::interface_methods::{
        package_interface_method_signatures, PackageTypeSymbolIndex,
    },
};

use super::{public_instance_error, PackagePublicInstanceInterface, PackagePublicInstanceReceiver};
use crate::error::ProjectionError;

pub(super) fn resolve_receiver<'a>(
    package: &PackageIrProjectionSource<'_>,
    file_units_by_module: &'a BTreeMap<&str, &FileIrUnit>,
    public_path: &str,
    const_module: &str,
    ty: &TypeRefIr,
) -> Result<PackagePublicInstanceReceiver<'a>, ProjectionError> {
    let symbol =
        nominal_service_symbol(file_units_by_module, const_module, ty).ok_or_else(|| {
            public_instance_error(
                package,
                public_path,
                "const must have an explicit nominal receiver type",
            )
        })?;
    let (unit, _type_index, decl) =
        type_decl_by_module_local_name(file_units_by_module, &symbol.module_path, &symbol.symbol)
            .ok_or_else(|| {
            public_instance_error(
                package,
                public_path,
                format!(
                    "receiver type {}.{} does not resolve to a package type",
                    symbol.module_path, symbol.symbol
                ),
            )
        })?;
    if matches!(decl.descriptor, TypeDescriptorIr::Alias { .. })
        || unit.declarations.interfaces.contains_key(&symbol.symbol)
    {
        return Err(public_instance_error(
            package,
            public_path,
            format!(
                "receiver type {}.{} must be a concrete nominal type, not an alias or interface",
                symbol.module_path, symbol.symbol
            ),
        ));
    }
    Ok(PackagePublicInstanceReceiver { symbol, unit, decl })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_interfaces(
    package: &PackageIrProjectionSource<'_>,
    file_units_by_module: &BTreeMap<&str, &FileIrUnit>,
    type_symbols: &PackageTypeSymbolIndex,
    publication_type_names: &PublicationVisibleTypeNames,
    public_path: &str,
    receiver: &PackagePublicInstanceReceiver<'_>,
    interfaces: &[PackageExportPublicInstanceInterface],
) -> Result<Vec<PackagePublicInstanceInterface>, ProjectionError> {
    if interfaces.is_empty() {
        return Err(public_instance_error(
            package,
            public_path,
            "interfaces must not be empty",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut projected = Vec::new();
    for interface in interfaces {
        let (interface_unit, _type_index, _type_decl) = type_decl_by_module_local_name(
            file_units_by_module,
            &interface.module,
            &interface.symbol,
        )
        .ok_or_else(|| {
            public_instance_error(
                package,
                public_path,
                format!(
                    "interface selector points to missing type {}.{}",
                    interface.module, interface.symbol
                ),
            )
        })?;
        let interface_decl = interface_unit
            .declarations
            .interfaces
            .get(&interface.symbol)
            .ok_or_else(|| {
                public_instance_error(
                    package,
                    public_path,
                    format!(
                        "interface selector {}.{} must resolve to an interface",
                        interface.module, interface.symbol
                    ),
                )
            })?;
        if !receiver.decl.implements.iter().any(|implemented| {
            type_ref_matches_interface_selector(
                file_units_by_module,
                &receiver.symbol.module_path,
                implemented,
                &interface.module,
                &interface.symbol,
            )
        }) {
            return Err(public_instance_error(
                package,
                public_path,
                format!(
                    "receiver {}.{} does not explicitly implement listed interface {}.{}",
                    receiver.symbol.module_path,
                    receiver.symbol.symbol,
                    interface.module,
                    interface.symbol
                ),
            ));
        }
        let interface_ty = public_interface_type_ref(package, &interface.module, &interface.symbol);
        let canonical_type_args = interface
            .canonical_type_args
            .iter()
            .map(|arg| {
                projection_visible_type_ref(
                    &receiver.symbol.module_path,
                    arg,
                    publication_type_names,
                )
            })
            .collect();
        let interface_ref = interface_instantiation_ref(interface_ty.clone(), canonical_type_args);
        let interface_key = canonical_interface_instantiation_key(&interface_ref);
        if !seen.insert(interface_key) {
            return Err(public_instance_error(
                package,
                public_path,
                format!(
                    "duplicate interface selector {}.{}",
                    interface.module, interface.symbol
                ),
            ));
        }
        let methods = package_interface_method_signatures(
            package.package_id,
            type_symbols,
            &interface_unit.module_path,
            interface_decl,
        )
        .map_err(|message| public_instance_error(package, public_path, message))?;
        let methods = instantiate_interface_method_signatures(
            methods,
            &interface_decl.type_params,
            &interface_ref.canonical_type_args,
        )
        .map_err(|error| {
            public_instance_error(
                package,
                public_path,
                format!(
                    "interface {}.{} expects {} type arguments but got {}",
                    interface.module,
                    interface.symbol,
                    error.expected_type_args,
                    error.actual_type_args
                ),
            )
        })?
        .into_iter()
        .map(|method| {
            projection_visible_interface_method_signature(
                &interface_unit.module_path,
                &method,
                publication_type_names,
            )
        })
        .collect();
        projected.push(PackagePublicInstanceInterface {
            ty: interface_ty,
            instantiation: interface_ref,
            methods,
        });
    }
    Ok(projected)
}

fn public_interface_type_ref(
    package: &PackageIrProjectionSource<'_>,
    module: &str,
    symbol: &str,
) -> TypeRefIr {
    let symbol_path = package
        .exports
        .symbols
        .iter()
        .find_map(|(public_path, export)| {
            (export.module == module && export.symbol == symbol)
                .then(|| super::super::package_scoped_export_symbol(package, public_path))
        })
        .unwrap_or_else(|| format!("{module}.{symbol}"));
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: package.package_id.to_string(),
            },
            symbol_path,
            abi_expectation: None,
        },
    }
}

fn nominal_service_symbol(
    file_units_by_module: &BTreeMap<&str, &FileIrUnit>,
    module_path: &str,
    ty: &TypeRefIr,
) -> Option<ServiceSymbolRef> {
    match ty {
        TypeRefIr::LocalType { type_index } => {
            let unit = file_units_by_module.get(module_path).copied()?;
            let decl = unit.type_table.get(*type_index as usize)?;
            Some(ServiceSymbolRef {
                module_path: module_path.to_string(),
                symbol: decl.name.clone(),
            })
        }
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => {
            let unit = file_units_by_module.get(module_path.as_str()).copied()?;
            let decl = unit.type_table.get(*type_index as usize)?;
            Some(ServiceSymbolRef {
                module_path: module_path.clone(),
                symbol: decl.name.clone(),
            })
        }
        TypeRefIr::ServiceSymbol { symbol } => {
            type_decl_by_module_local_name(
                file_units_by_module,
                &symbol.module_path,
                &symbol.symbol,
            )?;
            Some(symbol.clone())
        }
        TypeRefIr::Native { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Record { .. }
        | TypeRefIr::Union { .. }
        | TypeRefIr::Nullable { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::Function { .. } => None,
    }
}

fn type_decl_by_module_local_name<'a>(
    file_units_by_module: &'a BTreeMap<&str, &FileIrUnit>,
    module_path: &str,
    name: &str,
) -> Option<(&'a FileIrUnit, u32, &'a skiff_artifact_model::TypeDeclIr)> {
    let unit = file_units_by_module.get(module_path).copied()?;
    let type_index = unit.declarations.types.get(name)?.type_index;
    let decl = unit.type_table.get(type_index as usize)?;
    Some((unit, type_index, decl))
}

fn type_ref_matches_interface_selector(
    file_units_by_module: &BTreeMap<&str, &FileIrUnit>,
    context_module: &str,
    ty: &TypeRefIr,
    interface_module: &str,
    interface_symbol: &str,
) -> bool {
    match ty {
        TypeRefIr::LocalType { type_index } => file_units_by_module
            .get(context_module)
            .and_then(|unit| unit.type_table.get(*type_index as usize))
            .is_some_and(|decl| {
                decl.name == interface_symbol && context_module == interface_module
            }),
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => file_units_by_module
            .get(module_path.as_str())
            .and_then(|unit| unit.type_table.get(*type_index as usize))
            .is_some_and(|decl| decl.name == interface_symbol && module_path == interface_module),
        TypeRefIr::ServiceSymbol { symbol } => {
            symbol.module_path == interface_module && symbol.symbol == interface_symbol
        }
        TypeRefIr::Native { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Record { .. }
        | TypeRefIr::Union { .. }
        | TypeRefIr::Nullable { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::Function { .. } => false,
    }
}
