use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::canonical_interface_instantiation_key;
use skiff_artifact_model::{
    ContractOperationId, ContractPublicInstance, ContractPublicInstanceInterface,
    ContractPublicInstanceMethod, InterfaceInstantiationRef, TypeRefIr,
};

use crate::{selection::ServiceCallSelection, ContractDefinitionError, Result};

/// Exact, provider-free public-instance operation facts accepted by contract
/// projection. No executable, callable, build, signature, method name, or
/// effect fact can cross this seam.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ServicePublicInstanceOperationFacts {
    interfaces: Vec<ServicePublicInstanceInterfaceOperations>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServicePublicInstanceInterfaceOperations {
    public_root: String,
    interface: InterfaceInstantiationRef,
    slots: Vec<ServicePublicInstanceOperationSlot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServicePublicInstanceOperationSlot {
    method_abi_id: String,
    operation_stable_key: String,
}

impl ServicePublicInstanceOperationSlot {
    pub fn try_new(
        method_abi_id: impl Into<String>,
        operation_stable_key: impl Into<String>,
    ) -> Result<Self> {
        let method_abi_id = method_abi_id.into();
        if method_abi_id.is_empty() {
            return Err(ContractDefinitionError::EmptyPublicInstanceMethodAbi);
        }
        let operation_stable_key = operation_stable_key.into();
        if operation_stable_key.is_empty() {
            return Err(ContractDefinitionError::EmptyPublicInstanceOperationStableKey);
        }
        Ok(Self {
            method_abi_id,
            operation_stable_key,
        })
    }

    pub fn method_abi_id(&self) -> &str {
        &self.method_abi_id
    }

    pub fn operation_stable_key(&self) -> &str {
        &self.operation_stable_key
    }
}

impl ServicePublicInstanceInterfaceOperations {
    pub fn try_new(
        public_root: impl Into<String>,
        interface: InterfaceInstantiationRef,
        slots: Vec<ServicePublicInstanceOperationSlot>,
    ) -> Result<Self> {
        let row = Self {
            public_root: public_root.into(),
            interface,
            slots,
        };
        row.validate()?;
        Ok(row)
    }

    pub fn public_root(&self) -> &str {
        &self.public_root
    }

    pub fn interface(&self) -> &InterfaceInstantiationRef {
        &self.interface
    }

    pub fn slots(&self) -> &[ServicePublicInstanceOperationSlot] {
        &self.slots
    }

    fn validate(&self) -> Result<()> {
        if self.public_root.is_empty() || self.public_root.split('.').any(str::is_empty) {
            return Err(ContractDefinitionError::InvalidPublicInstanceRoot {
                public_instance: self.public_root.clone(),
            });
        }
        if self.interface.interface_abi_id.is_empty() {
            return Err(ContractDefinitionError::EmptyPublicInstanceInterfaceAbi {
                public_instance: self.public_root.clone(),
            });
        }
        let canonical_interface = canonical_interface_instantiation_key(&self.interface);
        if self
            .interface
            .canonical_type_args
            .iter()
            .any(contains_type_parameter)
        {
            return Err(ContractDefinitionError::OpenPublicInstanceInterface {
                public_instance: self.public_root.clone(),
                canonical_interface,
            });
        }
        let mut method_abi_ids = BTreeSet::new();
        let mut operation_stable_keys = BTreeSet::new();
        for slot in &self.slots {
            if slot.method_abi_id.is_empty() {
                return Err(ContractDefinitionError::EmptyPublicInstanceMethodAbi);
            }
            if slot.operation_stable_key.is_empty() {
                return Err(ContractDefinitionError::EmptyPublicInstanceOperationStableKey);
            }
            if !method_abi_ids.insert(slot.method_abi_id.clone()) {
                return Err(
                    ContractDefinitionError::DuplicateOrEmptyPublicInstanceMethodAbi {
                        public_instance: self.public_root.clone(),
                        canonical_interface,
                        method_abi_id: slot.method_abi_id.clone(),
                    },
                );
            }
            if !operation_stable_keys.insert(slot.operation_stable_key.clone()) {
                return Err(ContractDefinitionError::DuplicatePublicInstanceOperation {
                    operation_stable_key: slot.operation_stable_key.clone(),
                });
            }
        }
        Ok(())
    }
}

fn contains_type_parameter(ty: &TypeRefIr) -> bool {
    match ty {
        TypeRefIr::Builtin { args, .. } => args.iter().any(contains_type_parameter),
        TypeRefIr::AppliedNominal { arguments, .. } => {
            arguments.iter().any(contains_type_parameter)
        }
        TypeRefIr::Record { fields } => fields.values().any(contains_type_parameter),
        TypeRefIr::Union { items } => items.iter().any(contains_type_parameter),
        TypeRefIr::Nullable { inner } => contains_type_parameter(inner),
        TypeRefIr::TypeParam { .. } => true,
        TypeRefIr::AnyInterface { interface } => interface
            .canonical_type_args
            .iter()
            .any(contains_type_parameter),
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            params
                .iter()
                .any(|parameter| contains_type_parameter(&parameter.ty))
                || contains_type_parameter(return_type)
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. } => false,
    }
}

impl ServicePublicInstanceOperationFacts {
    pub fn try_from_interfaces(
        interfaces: impl IntoIterator<Item = ServicePublicInstanceInterfaceOperations>,
    ) -> Result<Self> {
        let mut keyed = Vec::new();
        let mut operation_stable_keys = BTreeSet::new();
        for row in interfaces {
            row.validate()?;
            for slot in row.slots() {
                if !operation_stable_keys.insert(slot.operation_stable_key.clone()) {
                    return Err(ContractDefinitionError::DuplicatePublicInstanceOperation {
                        operation_stable_key: slot.operation_stable_key.clone(),
                    });
                }
            }
            let key = (
                row.public_root.clone(),
                canonical_interface_instantiation_key(&row.interface),
            );
            keyed.push((key, row));
        }
        keyed.sort_by(|left, right| left.0.cmp(&right.0));
        if let Some(duplicate) = keyed.windows(2).find(|rows| rows[0].0 == rows[1].0) {
            let ((public_instance, canonical_interface), _) = &duplicate[0];
            return Err(ContractDefinitionError::DuplicatePublicInstanceInterface {
                public_instance: public_instance.clone(),
                canonical_interface: canonical_interface.clone(),
            });
        }
        Ok(Self {
            interfaces: keyed.into_iter().map(|(_, row)| row).collect(),
        })
    }

    pub fn interfaces(&self) -> &[ServicePublicInstanceInterfaceOperations] {
        &self.interfaces
    }

    pub(crate) fn interfaces_for_root<'a>(
        &'a self,
        public_root: &'a str,
    ) -> impl Iterator<Item = &'a ServicePublicInstanceInterfaceOperations> + 'a {
        self.interfaces
            .iter()
            .filter(move |row| row.public_root == public_root)
    }

    pub(crate) fn public_root_for_operation(&self, operation_stable_key: &str) -> Option<&str> {
        self.interfaces.iter().find_map(|row| {
            row.slots
                .iter()
                .any(|slot| slot.operation_stable_key == operation_stable_key)
                .then_some(row.public_root.as_str())
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ProjectedPublicInstances {
    instances: BTreeMap<String, ProjectedPublicInstance>,
}

#[derive(Debug, Clone, PartialEq)]
struct ProjectedPublicInstance {
    interfaces: Vec<ProjectedPublicInstanceInterface>,
}

#[derive(Debug, Clone, PartialEq)]
struct ProjectedPublicInstanceInterface {
    interface: InterfaceInstantiationRef,
    methods: Vec<ProjectedPublicInstanceMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectedPublicInstanceMethod {
    method_abi_id: String,
    operation_stable_key: String,
}

pub(crate) fn project_public_instances(
    selection: &ServiceCallSelection,
    facts: &ServicePublicInstanceOperationFacts,
) -> Result<ProjectedPublicInstances> {
    let mut instances = BTreeMap::new();
    let mut selected_operation_keys = BTreeSet::new();

    for (public_root, expected_operations) in &selection.public_instances {
        let rows = facts.interfaces_for_root(public_root).collect::<Vec<_>>();
        if rows.is_empty() {
            return Err(
                ContractDefinitionError::MissingSelectedPublicInstanceOperationFacts {
                    public_instance: public_root.clone(),
                },
            );
        }

        let mut remaining = expected_operations.clone();
        let mut exact_interfaces = BTreeSet::new();
        let mut projected_rows = Vec::new();
        for row in rows {
            let canonical_key = canonical_interface_instantiation_key(&row.interface);
            if !exact_interfaces.insert(canonical_key.clone()) {
                return Err(ContractDefinitionError::DuplicatePublicInstanceInterface {
                    public_instance: public_root.clone(),
                    canonical_interface: canonical_key,
                });
            }

            let mut method_abi_ids = BTreeSet::new();
            let mut methods = Vec::with_capacity(row.slots.len());
            for slot in &row.slots {
                if slot.method_abi_id.is_empty()
                    || !method_abi_ids.insert(slot.method_abi_id.clone())
                {
                    return Err(
                        ContractDefinitionError::DuplicateOrEmptyPublicInstanceMethodAbi {
                            public_instance: public_root.clone(),
                            canonical_interface: canonical_key.clone(),
                            method_abi_id: slot.method_abi_id.clone(),
                        },
                    );
                }
                if !remaining.remove(&slot.operation_stable_key) {
                    return Err(ContractDefinitionError::UnexpectedPublicInstanceOperation {
                        public_instance: public_root.clone(),
                        operation_stable_key: slot.operation_stable_key.clone(),
                    });
                }
                if !selected_operation_keys.insert(slot.operation_stable_key.clone()) {
                    return Err(ContractDefinitionError::DuplicatePublicInstanceOperation {
                        operation_stable_key: slot.operation_stable_key.clone(),
                    });
                }
                methods.push(ProjectedPublicInstanceMethod {
                    method_abi_id: slot.method_abi_id.clone(),
                    operation_stable_key: slot.operation_stable_key.clone(),
                });
            }
            projected_rows.push((
                canonical_key,
                ProjectedPublicInstanceInterface {
                    interface: row.interface.clone(),
                    methods,
                },
            ));
        }

        if !remaining.is_empty() {
            return Err(ContractDefinitionError::MissingPublicInstanceOperations {
                public_instance: public_root.clone(),
                operation_stable_keys: remaining.into_iter().collect(),
            });
        }

        projected_rows.sort_by(|left, right| left.0.cmp(&right.0));
        instances.insert(
            public_root.clone(),
            ProjectedPublicInstance {
                interfaces: projected_rows.into_iter().map(|(_, row)| row).collect(),
            },
        );
    }

    Ok(ProjectedPublicInstances { instances })
}

pub(crate) fn bind_contract_operation_ids(
    projected: ProjectedPublicInstances,
    operation_ids: &BTreeMap<String, ContractOperationId>,
) -> Result<BTreeMap<String, ContractPublicInstance>> {
    let mut bound_operation_ids = BTreeSet::new();
    projected
        .instances
        .into_iter()
        .map(|(public_root, instance)| {
            let interfaces = instance
                .interfaces
                .into_iter()
                .map(|interface| {
                    let methods = interface
                        .methods
                        .into_iter()
                        .map(|method| {
                            let contract_operation_id = operation_ids
                                .get(&method.operation_stable_key)
                                .cloned()
                                .ok_or_else(|| {
                                    ContractDefinitionError::UnknownPublicInstanceOperation {
                                        public_instance: public_root.clone(),
                                        operation_stable_key: method.operation_stable_key.clone(),
                                    }
                                })?;
                            if !bound_operation_ids.insert(contract_operation_id.clone()) {
                                return Err(
                                    ContractDefinitionError::DuplicatePublicInstanceOperationId {
                                        contract_operation_id: contract_operation_id.to_string(),
                                    },
                                );
                            }
                            Ok(ContractPublicInstanceMethod {
                                method_abi_id: method.method_abi_id,
                                contract_operation_id,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Ok(ContractPublicInstanceInterface {
                        interface: interface.interface,
                        methods,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((public_root, ContractPublicInstance { interfaces }))
        })
        .collect()
}

#[cfg(test)]
mod tests;
