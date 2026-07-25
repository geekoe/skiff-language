use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    FileIrUnit, PackageRefIr, PackageSymbolRef, ServiceSymbolRef, TypeDescriptorIr, TypeRefIr,
};

use crate::package_artifact::{
    api_exports::PackageExportPublicInstanceInterface, model::PackageExportLinkProjectionInput,
};

use super::{public_instance_error, PackagePublicInstanceInterface, PackagePublicInstanceReceiver};
use crate::error::ProjectionError;

pub(super) fn resolve_receiver(
    package: &PackageExportLinkProjectionInput<'_>,
    file_units_by_module: &BTreeMap<&str, &FileIrUnit>,
    public_path: &str,
    const_module: &str,
    ty: &TypeRefIr,
    expected_module: &str,
    expected_symbol: &str,
) -> Result<PackagePublicInstanceReceiver, ProjectionError> {
    let symbol =
        nominal_service_symbol(file_units_by_module, const_module, ty).ok_or_else(|| {
            public_instance_error(
                package,
                public_path,
                "const must have an explicit nominal receiver type",
            )
        })?;
    if symbol.module_path != expected_module || symbol.symbol != expected_symbol {
        return Err(public_instance_error(
            package,
            public_path,
            format!(
                "const receiver {}.{} does not match source-validated receiver {}.{}",
                symbol.module_path, symbol.symbol, expected_module, expected_symbol
            ),
        ));
    }
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
    Ok(PackagePublicInstanceReceiver { symbol })
}

pub(super) fn resolve_interfaces(
    package: &PackageExportLinkProjectionInput<'_>,
    file_units_by_module: &BTreeMap<&str, &FileIrUnit>,
    public_path: &str,
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
        if !seen.insert((interface.module.clone(), interface.symbol.clone())) {
            return Err(public_instance_error(
                package,
                public_path,
                format!(
                    "duplicate interface selector {}.{}",
                    interface.module, interface.symbol
                ),
            ));
        }
        let mut method_names = BTreeSet::new();
        for method in &interface.methods {
            if !method_names.insert(method.name.clone()) {
                return Err(public_instance_error(
                    package,
                    public_path,
                    format!(
                        "interface {}.{} contains duplicate validated method {}",
                        interface.module, interface.symbol, method.name
                    ),
                ));
            }
        }
        let declared_method_names = interface_decl
            .operations
            .iter()
            .map(|method| method.name.clone())
            .collect::<BTreeSet<_>>();
        if method_names != declared_method_names {
            let missing = declared_method_names
                .difference(&method_names)
                .cloned()
                .collect::<Vec<_>>();
            let extra = method_names
                .difference(&declared_method_names)
                .cloned()
                .collect::<Vec<_>>();
            return Err(public_instance_error(
                package,
                public_path,
                format!(
                    "source-validated methods do not match File IR interface {}.{}; missing={missing:?}, extra={extra:?}",
                    interface.module, interface.symbol
                ),
            ));
        }
        let interface_ty = public_interface_type_ref(package, &interface.module, &interface.symbol);
        projected.push(PackagePublicInstanceInterface {
            ty: interface_ty,
            methods: interface.methods.clone(),
        });
    }
    Ok(projected)
}

fn public_interface_type_ref(
    package: &PackageExportLinkProjectionInput<'_>,
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
        TypeRefIr::Builtin { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
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
