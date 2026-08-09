use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{PackageArtifact, PackageCallableId, PackageLocalAbiSymbol};

use crate::{ContractDefinitionError, Result};

/// Canonical typed interpretation of `service.yml.serviceCalls`.
///
/// The manifest roots are retained in sorted order for identity inputs, while
/// `operations` is the exact deployment-facing map after public instances have
/// expanded to all listed-interface methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServiceCallSelection {
    pub(crate) roots: Vec<String>,
    pub(crate) operations: BTreeMap<String, PackageCallableId>,
    pub(crate) public_instances: Vec<String>,
}

pub(crate) fn select_service_calls(
    package: &PackageArtifact,
    selection_paths: &[String],
) -> Result<ServiceCallSelection> {
    let roots = canonical_roots(selection_paths)?;
    let methods = public_instance_methods(package);
    let method_paths_by_callable = method_paths_by_callable(&methods);
    let mut operations = BTreeMap::new();
    let mut callable_paths = BTreeMap::new();
    let mut public_instances = Vec::new();

    for root in &roots {
        if let Some((public_instance, _)) = methods.get(root) {
            return Err(ContractDefinitionError::PublicInstanceMethodSelection {
                path: root.clone(),
                public_instance: public_instance.clone(),
            });
        }

        match package.package_local_abi.public_symbols.get(root) {
            Some(PackageLocalAbiSymbol::Callable { callable_id, .. }) => {
                if let Some(method_paths) = method_paths_by_callable.get(callable_id) {
                    return Err(ContractDefinitionError::PublicInstanceMethodAlias {
                        path: root.clone(),
                        callable_id: callable_id.to_string(),
                        method_paths: method_paths.clone(),
                    });
                }
                insert_operation(
                    &mut operations,
                    &mut callable_paths,
                    root.clone(),
                    callable_id.clone(),
                )?;
            }
            Some(PackageLocalAbiSymbol::PublicInstance { methods, .. }) => {
                public_instances.push(root.clone());
                for (method, callable_id) in methods {
                    let method_path = format!("{root}.{method}");
                    let Some(PackageLocalAbiSymbol::Callable {
                        callable_id: public_callable_id,
                        ..
                    }) = package.package_local_abi.public_symbols.get(&method_path)
                    else {
                        return Err(ContractDefinitionError::InvalidPublicInstanceMethod {
                            public_instance: root.clone(),
                            method_path,
                            callable_id: callable_id.to_string(),
                        });
                    };
                    if public_callable_id != callable_id {
                        return Err(ContractDefinitionError::InvalidPublicInstanceMethod {
                            public_instance: root.clone(),
                            method_path,
                            callable_id: callable_id.to_string(),
                        });
                    }
                    insert_operation(
                        &mut operations,
                        &mut callable_paths,
                        method_path,
                        callable_id.clone(),
                    )?;
                }
            }
            Some(symbol) => {
                return Err(ContractDefinitionError::NonCallableServiceCallPath {
                    path: root.clone(),
                    kind: symbol_kind(symbol),
                });
            }
            None => {
                return Err(ContractDefinitionError::UnknownServiceCallPath { path: root.clone() });
            }
        }
    }

    Ok(ServiceCallSelection {
        roots,
        operations,
        public_instances,
    })
}

fn canonical_roots(selection_paths: &[String]) -> Result<Vec<String>> {
    let mut seen = BTreeSet::new();
    for path in selection_paths {
        if !seen.insert(path.clone()) {
            return Err(ContractDefinitionError::DuplicateServiceCallPath { path: path.clone() });
        }
    }
    Ok(seen.into_iter().collect())
}

fn public_instance_methods(
    package: &PackageArtifact,
) -> BTreeMap<String, (String, PackageCallableId)> {
    package
        .package_local_abi
        .public_symbols
        .iter()
        .filter_map(|(public_instance, symbol)| {
            let PackageLocalAbiSymbol::PublicInstance { methods, .. } = symbol else {
                return None;
            };
            Some(methods.iter().map(move |(method, callable_id)| {
                (
                    format!("{public_instance}.{method}"),
                    (public_instance.clone(), callable_id.clone()),
                )
            }))
        })
        .flatten()
        .collect()
}

fn method_paths_by_callable(
    methods: &BTreeMap<String, (String, PackageCallableId)>,
) -> BTreeMap<PackageCallableId, Vec<String>> {
    let mut paths = BTreeMap::<PackageCallableId, Vec<String>>::new();
    for (path, (_, callable_id)) in methods {
        paths
            .entry(callable_id.clone())
            .or_default()
            .push(path.clone());
    }
    paths
}

fn insert_operation(
    operations: &mut BTreeMap<String, PackageCallableId>,
    callable_paths: &mut BTreeMap<PackageCallableId, String>,
    operation_path: String,
    callable_id: PackageCallableId,
) -> Result<()> {
    if let Some(first) = callable_paths.insert(callable_id.clone(), operation_path.clone()) {
        return Err(ContractDefinitionError::DuplicatePublicCallable {
            callable_id: callable_id.to_string(),
            first,
            second: operation_path,
        });
    }
    operations.insert(operation_path, callable_id);
    Ok(())
}

fn symbol_kind(symbol: &PackageLocalAbiSymbol) -> &'static str {
    match symbol {
        PackageLocalAbiSymbol::Type { .. } => "type",
        PackageLocalAbiSymbol::Callable { .. } => "callable",
        PackageLocalAbiSymbol::Constant { .. } => "constant",
        PackageLocalAbiSymbol::PublicInstance { .. } => "public instance",
    }
}
