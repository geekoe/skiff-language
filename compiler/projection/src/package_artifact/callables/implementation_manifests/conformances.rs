use std::collections::BTreeSet;

use serde::Serialize;
use skiff_artifact_model::{
    FileIrUnit, InterfaceInstantiationRef, NominalTypeRefBaseIr, PackageLocalInterfaceConformance,
    PackageRefIr, PackageRequirement, PackageSymbolRef, ServiceSymbolRef, TypeRefIr,
};
use skiff_compiler_core::json_utils::canonical_json_bytes;
use skiff_compiler_projection_input::ProjectionLocalInterfaceConformanceFacts;

use crate::error::ProjectionError;

use super::{projection_error, ImplementationCallableIndex};
use crate::package_artifact::callables::normalization;

pub(super) fn project_local_interface_conformances(
    package_id: &str,
    units: &[FileIrUnit],
    facts: &ProjectionLocalInterfaceConformanceFacts,
    package_requirements: &[PackageRequirement],
    callables: &ImplementationCallableIndex,
) -> Result<Vec<PackageLocalInterfaceConformance>, ProjectionError> {
    let expected = expected_local_conformance_keys(package_id, units, package_requirements)?;
    let mut actual = BTreeSet::new();
    let mut rows = Vec::new();
    for fact in facts.conformances() {
        let receiver = normalize_receiver(
            package_id,
            fact.receiver().module_path(),
            fact.receiver().symbol(),
            units,
            package_requirements,
        )?;
        let interface = normalize_interface(
            package_id,
            fact.receiver().module_path(),
            fact.interface(),
            units,
            package_requirements,
        )?;
        let methods = fact
            .implementation_executables()
            .iter()
            .enumerate()
            .map(|(slot, executable)| {
                callables.conformance_method(
                    package_id,
                    executable,
                    &format!(
                        "local interface conformance {}.{} slot {slot}",
                        fact.receiver().module_path(),
                        fact.receiver().symbol()
                    ),
                )
            })
            .collect::<Result<Vec<_>, ProjectionError>>()?;
        let row = PackageLocalInterfaceConformance {
            type_parameters: fact.type_parameters().to_vec(),
            receiver,
            interface,
            methods,
        };
        let key = canonical_conformance_key(package_id, &row)?;
        if !actual.insert(key.clone()) {
            return Err(projection_error(
                package_id,
                format!(
                    "typed local interface conformances collapse to duplicate artifact key {}",
                    canonical_key_text(&key)
                ),
            ));
        }
        rows.push((key, row));
    }

    if expected != actual {
        let missing = expected
            .difference(&actual)
            .map(|key| canonical_key_text(key))
            .collect::<Vec<_>>();
        let extra = actual
            .difference(&expected)
            .map(|key| canonical_key_text(key))
            .collect::<Vec<_>>();
        return Err(projection_error(
            package_id,
            format!(
                "typed local interface conformance facts must exactly cover File IR implements declarations; missing={missing:?}, extra={extra:?}"
            ),
        ));
    }

    rows.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(rows.into_iter().map(|(_, row)| row).collect())
}

fn expected_local_conformance_keys(
    package_id: &str,
    units: &[FileIrUnit],
    package_requirements: &[PackageRequirement],
) -> Result<BTreeSet<Vec<u8>>, ProjectionError> {
    let mut expected = BTreeSet::new();
    for unit in units {
        for (type_index, ty) in unit.type_table.iter().enumerate() {
            if ty.implements.is_empty() {
                continue;
            }
            validate_type_parameters(package_id, &unit.module_path, &ty.name, &ty.type_params)?;
            let declared_name =
                exact_type_declaration_name(package_id, unit, type_index, &ty.name)?;
            let receiver = normalize_receiver(
                package_id,
                &unit.module_path,
                declared_name,
                units,
                package_requirements,
            )?;
            for implemented in &ty.implements {
                let TypeRefIr::AnyInterface { interface } = implemented else {
                    return Err(projection_error(
                        package_id,
                        format!(
                            "type {}.{} implements entry is not an exact any-interface identity",
                            unit.module_path, declared_name
                        ),
                    ));
                };
                let interface = normalize_interface(
                    package_id,
                    &unit.module_path,
                    interface,
                    units,
                    package_requirements,
                )?;
                let row = PackageLocalInterfaceConformance {
                    type_parameters: ty.type_params.clone(),
                    receiver: receiver.clone(),
                    interface,
                    methods: Vec::new(),
                };
                let key = canonical_conformance_key(package_id, &row)?;
                if !expected.insert(key.clone()) {
                    return Err(projection_error(
                        package_id,
                        format!(
                            "File IR implements declarations collapse to duplicate artifact key {}",
                            canonical_key_text(&key)
                        ),
                    ));
                }
            }
        }
    }
    Ok(expected)
}

fn exact_type_declaration_name<'a>(
    package_id: &str,
    unit: &'a FileIrUnit,
    type_index: usize,
    type_name: &str,
) -> Result<&'a str, ProjectionError> {
    let Ok(type_index) = u32::try_from(type_index) else {
        return Err(projection_error(
            package_id,
            format!(
                "type {}.{type_name} has an index that does not fit the File IR address space",
                unit.module_path
            ),
        ));
    };
    let mut declarations = unit
        .declarations
        .types
        .iter()
        .filter(|(_, declaration)| declaration.type_index == type_index);
    let (declared_name, _) = declarations.next().ok_or_else(|| {
        projection_error(
            package_id,
            format!(
                "type {}.{type_name} with implements declarations has no exact top-level declaration",
                unit.module_path
            ),
        )
    })?;
    if declarations.next().is_some() || declared_name.as_str() != type_name {
        return Err(projection_error(
            package_id,
            format!(
                "type {}.{type_name} with implements declarations has an ambiguous or mismatched top-level declaration",
                unit.module_path
            ),
        ));
    }
    Ok(declared_name.as_str())
}

fn validate_type_parameters(
    package_id: &str,
    module_path: &str,
    type_name: &str,
    type_parameters: &[String],
) -> Result<(), ProjectionError> {
    let mut names = BTreeSet::new();
    for name in type_parameters {
        if name.is_empty() || !names.insert(name) {
            return Err(projection_error(
                package_id,
                format!(
                    "type {module_path}.{type_name} has an empty or duplicate conformance type parameter {name:?}"
                ),
            ));
        }
    }
    Ok(())
}

fn normalize_artifact_type(
    package_id: &str,
    owner_module: &str,
    ty: &TypeRefIr,
    units: &[FileIrUnit],
    package_requirements: &[PackageRequirement],
) -> Result<TypeRefIr, String> {
    let mut resolved = ty.clone();
    resolve_dependency_refs(&mut resolved, package_requirements)?;
    normalization::normalize_implementation_type(package_id, owner_module, &resolved, units)
}

fn resolve_dependency_refs(
    ty: &mut TypeRefIr,
    package_requirements: &[PackageRequirement],
) -> Result<(), String> {
    match ty {
        TypeRefIr::Builtin { args, .. } => {
            for argument in args {
                resolve_dependency_refs(argument, package_requirements)?;
            }
        }
        TypeRefIr::PackageSymbol { symbol } => {
            resolve_dependency_symbol(symbol, package_requirements)?;
        }
        TypeRefIr::AppliedNominal { base, arguments } => {
            if let NominalTypeRefBaseIr::PackageSymbol { symbol } = base {
                resolve_dependency_symbol(symbol, package_requirements)?;
            }
            for argument in arguments {
                resolve_dependency_refs(argument, package_requirements)?;
            }
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values_mut() {
                resolve_dependency_refs(field, package_requirements)?;
            }
        }
        TypeRefIr::Union { items } => {
            for item in items {
                resolve_dependency_refs(item, package_requirements)?;
            }
        }
        TypeRefIr::Nullable { inner } => {
            resolve_dependency_refs(inner, package_requirements)?;
        }
        TypeRefIr::AnyInterface { interface } => {
            let mut identity = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
                .map_err(|error| {
                    format!("interface identity is not an exact TypeRefIr: {error}")
                })?;
            resolve_dependency_refs(&mut identity, package_requirements)?;
            interface.interface_abi_id = skiff_artifact_identity::type_ref_abi_key(&identity);
            for argument in &mut interface.canonical_type_args {
                resolve_dependency_refs(argument, package_requirements)?;
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for parameter in params {
                resolve_dependency_refs(&mut parameter.ty, package_requirements)?;
            }
            resolve_dependency_refs(return_type, package_requirements)?;
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => {}
    }
    Ok(())
}

fn resolve_dependency_symbol(
    symbol: &mut PackageSymbolRef,
    package_requirements: &[PackageRequirement],
) -> Result<(), String> {
    let PackageRefIr::Dependency { dependency_ref } = &symbol.package else {
        return Ok(());
    };
    let dependency_ref = dependency_ref.clone();
    let mut matches = package_requirements
        .iter()
        .filter(|requirement| requirement.alias.as_str() == dependency_ref.as_str());
    let requirement = matches.next().ok_or_else(|| {
        format!(
            "package symbol {} uses dependency alias {dependency_ref:?} without a package requirement",
            symbol.symbol_path
        )
    })?;
    if matches.next().is_some() {
        return Err(format!(
            "package symbol {} uses ambiguous dependency alias {dependency_ref:?}",
            symbol.symbol_path
        ));
    }
    let expected_abi = requirement.expected_local_abi.as_str();
    if symbol
        .abi_expectation
        .as_deref()
        .is_some_and(|actual| actual != expected_abi)
    {
        return Err(format!(
            "package symbol {} dependency alias {dependency_ref:?} ABI expectation {:?} does not match {}",
            symbol.symbol_path, symbol.abi_expectation, requirement.expected_local_abi
        ));
    }
    symbol.package = PackageRefIr::PackageId {
        package_id: requirement.package_id.clone(),
    };
    symbol.abi_expectation = Some(expected_abi.to_string());
    Ok(())
}

fn normalize_receiver(
    package_id: &str,
    module_path: &str,
    symbol: &str,
    units: &[FileIrUnit],
    package_requirements: &[PackageRequirement],
) -> Result<TypeRefIr, ProjectionError> {
    let receiver = normalize_artifact_type(
        package_id,
        module_path,
        &TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: module_path.to_string(),
                symbol: symbol.to_string(),
            },
        },
        units,
        package_requirements,
    )
    .map_err(|message| {
        projection_error(
            package_id,
            format!("local conformance receiver {module_path}.{symbol}: {message}"),
        )
    })?;
    if !matches!(
        &receiver,
        TypeRefIr::PackageSymbol { symbol }
            if matches!(
                &symbol.package,
                PackageRefIr::PackageId { package_id: owner } if owner.as_str() == package_id
            )
    ) {
        return Err(projection_error(
            package_id,
            format!(
                "local conformance receiver {module_path}.{symbol} is not an exact package-local type"
            ),
        ));
    }
    Ok(receiver)
}

fn normalize_interface(
    package_id: &str,
    owner_module: &str,
    interface: &InterfaceInstantiationRef,
    units: &[FileIrUnit],
    package_requirements: &[PackageRequirement],
) -> Result<InterfaceInstantiationRef, ProjectionError> {
    let normalized = normalize_artifact_type(
        package_id,
        owner_module,
        &TypeRefIr::AnyInterface {
            interface: interface.clone(),
        },
        units,
        package_requirements,
    )
    .map_err(|message| {
        projection_error(
            package_id,
            format!("local conformance interface owned by {owner_module}: {message}"),
        )
    })?;
    match normalized {
        TypeRefIr::AnyInterface { interface } => Ok(interface),
        other => Err(projection_error(
            package_id,
            format!("local conformance interface normalization returned unexpected type {other:?}"),
        )),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalConformanceKey<'a> {
    type_parameters: &'a [String],
    receiver: &'a TypeRefIr,
    interface: &'a InterfaceInstantiationRef,
}

fn canonical_conformance_key(
    package_id: &str,
    row: &PackageLocalInterfaceConformance,
) -> Result<Vec<u8>, ProjectionError> {
    canonical_json_bytes(&CanonicalConformanceKey {
        type_parameters: &row.type_parameters,
        receiver: &row.receiver,
        interface: &row.interface,
    })
    .map_err(|error| {
        projection_error(
            package_id,
            format!("local conformance artifact key could not be canonicalized: {error}"),
        )
    })
}

fn canonical_key_text(key: &[u8]) -> String {
    String::from_utf8_lossy(key).into_owned()
}
