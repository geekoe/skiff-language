use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{PackageArtifact, PackageCallableId, PackageLocalAbiSymbol};

use crate::{
    public_instances::ServicePublicInstanceOperationFacts, ContractDefinitionError, Result,
};

/// Canonical typed interpretation of `service.yml.serviceCalls`.
///
/// The manifest roots are retained in sorted order for identity inputs, while
/// `operations` is the exact deployment-facing map after public instances have
/// expanded to all listed-interface methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServiceCallSelection {
    pub(crate) roots: Vec<String>,
    pub(crate) operations: BTreeMap<String, PackageCallableId>,
    pub(crate) public_instances: BTreeMap<String, BTreeSet<String>>,
}

pub(crate) fn select_service_calls(
    package: &PackageArtifact,
    selection_paths: &[String],
    public_instance_facts: Option<&ServicePublicInstanceOperationFacts>,
) -> Result<ServiceCallSelection> {
    let roots = canonical_roots(selection_paths)?;
    if public_instance_facts.is_none() {
        let public_instances = roots
            .iter()
            .filter(|root| {
                matches!(
                    package.package_local_abi.public_symbols.get(*root),
                    Some(PackageLocalAbiSymbol::PublicInstance { .. })
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        if !public_instances.is_empty() {
            return Err(
                ContractDefinitionError::MissingPublicInstanceContractFacts { public_instances },
            );
        }
    }
    let public_instance_roots_by_callable = public_instance_roots_by_callable(package);
    let mut operations = BTreeMap::new();
    let mut callable_paths = BTreeMap::new();
    let mut public_instances = BTreeMap::new();

    for root in &roots {
        if let Some(public_instance) =
            public_instance_facts.and_then(|facts| facts.public_root_for_operation(root))
        {
            return Err(ContractDefinitionError::PublicInstanceMethodSelection {
                path: root.clone(),
                public_instance: public_instance.to_string(),
            });
        }

        match package.package_local_abi.public_symbols.get(root) {
            Some(PackageLocalAbiSymbol::Callable { callable_id, .. }) => {
                if public_instance_roots_by_callable.contains_key(callable_id) {
                    let method_paths = public_instance_facts
                        .map(|facts| {
                            exact_operation_paths_for_callable(package, facts, callable_id)
                        })
                        .unwrap_or_default();
                    return Err(ContractDefinitionError::PublicInstanceMethodAlias {
                        path: root.clone(),
                        callable_id: callable_id.to_string(),
                        method_paths,
                    });
                }
                insert_operation(
                    &mut operations,
                    &mut callable_paths,
                    root.clone(),
                    callable_id.clone(),
                )?;
            }
            Some(PackageLocalAbiSymbol::PublicInstance {
                instance_id,
                methods,
                ..
            }) => {
                if instance_id != root {
                    return Err(
                        ContractDefinitionError::PublicInstanceRootIdentityMismatch {
                            public_path: root.clone(),
                            instance_id: instance_id.clone(),
                        },
                    );
                }
                let Some(facts) = public_instance_facts else {
                    return Err(
                        ContractDefinitionError::MissingPublicInstanceContractFacts {
                            public_instances: vec![root.clone()],
                        },
                    );
                };
                let rows = facts.interfaces_for_root(root).collect::<Vec<_>>();
                if rows.is_empty() {
                    return Err(
                        ContractDefinitionError::MissingSelectedPublicInstanceOperationFacts {
                            public_instance: root.clone(),
                        },
                    );
                }

                let package_method_ids = methods.values().cloned().collect::<BTreeSet<_>>();
                if package_method_ids.len() != methods.len() {
                    return Err(ContractDefinitionError::PublicInstanceOperationCoverage {
                        public_instance: root.clone(),
                    });
                }
                let mut fact_method_ids = BTreeSet::new();
                let mut operation_stable_keys = BTreeSet::new();
                for row in rows {
                    for slot in row.slots() {
                        let operation_stable_key = slot.operation_stable_key();
                        if !operation_stable_keys.insert(operation_stable_key.to_string()) {
                            return Err(
                                ContractDefinitionError::DuplicatePublicInstanceOperation {
                                    operation_stable_key: operation_stable_key.to_string(),
                                },
                            );
                        }
                        let Some(PackageLocalAbiSymbol::Callable { callable_id, .. }) = package
                            .package_local_abi
                            .public_symbols
                            .get(operation_stable_key)
                        else {
                            return Err(
                                ContractDefinitionError::InvalidPublicInstanceOperationFact {
                                    public_instance: root.clone(),
                                    operation_stable_key: operation_stable_key.to_string(),
                                },
                            );
                        };
                        fact_method_ids.insert(callable_id.clone());
                        insert_operation(
                            &mut operations,
                            &mut callable_paths,
                            operation_stable_key.to_string(),
                            callable_id.clone(),
                        )?;
                    }
                }
                if fact_method_ids != package_method_ids {
                    return Err(ContractDefinitionError::PublicInstanceOperationCoverage {
                        public_instance: root.clone(),
                    });
                }
                if public_instances
                    .insert(root.clone(), operation_stable_keys)
                    .is_some()
                {
                    return Err(ContractDefinitionError::DuplicateServiceCallPath {
                        path: root.clone(),
                    });
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

fn public_instance_roots_by_callable(
    package: &PackageArtifact,
) -> BTreeMap<PackageCallableId, Vec<String>> {
    let mut roots = BTreeMap::<PackageCallableId, Vec<String>>::new();
    for (public_instance, symbol) in &package.package_local_abi.public_symbols {
        let PackageLocalAbiSymbol::PublicInstance { methods, .. } = symbol else {
            continue;
        };
        for callable_id in methods.values() {
            roots
                .entry(callable_id.clone())
                .or_default()
                .push(public_instance.clone());
        }
    }
    roots
}

fn exact_operation_paths_for_callable(
    package: &PackageArtifact,
    facts: &ServicePublicInstanceOperationFacts,
    expected_callable_id: &PackageCallableId,
) -> Vec<String> {
    facts
        .interfaces()
        .iter()
        .flat_map(|row| row.slots())
        .filter_map(|slot| {
            let PackageLocalAbiSymbol::Callable { callable_id, .. } = package
                .package_local_abi
                .public_symbols
                .get(slot.operation_stable_key())?
            else {
                return None;
            };
            (callable_id == expected_callable_id).then(|| slot.operation_stable_key().to_string())
        })
        .collect()
}

fn insert_operation(
    operations: &mut BTreeMap<String, PackageCallableId>,
    callable_paths: &mut BTreeMap<PackageCallableId, String>,
    operation_path: String,
    callable_id: PackageCallableId,
) -> Result<()> {
    if operations.contains_key(&operation_path) {
        return Err(
            ContractDefinitionError::DuplicateServiceOperationStableKey {
                operation_stable_key: operation_path,
            },
        );
    }
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
