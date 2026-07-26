use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    BoundaryCallbackOperation, ContractLiteral, ContractTypeRef, PackageSchemaTypeRef,
};

use crate::runtime_value::{
    InterfaceCarrier, InterfaceMethodLiteral, InterfaceMethodTarget, InterfaceMethodType,
    InterfaceReceiverCallAbi, InterfaceValue,
};

/// Validated bridge between a canonical callback contract and one admitted
/// local interface method table. Contract, interface and method identities are
/// deliberately stored in separate fields and are never compared across
/// domains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallbackContractProjection {
    canonical_package_schema_type: PackageSchemaTypeRef,
    local_interface_abi_id: String,
    operations: Vec<CallbackContractOperationProjection>,
}

impl CallbackContractProjection {
    pub fn build(
        canonical_package_schema_type: PackageSchemaTypeRef,
        contract_operations: &BTreeMap<String, BoundaryCallbackOperation>,
        interface: &InterfaceValue,
    ) -> Result<Self, CallbackContractProjectionError> {
        let InterfaceCarrier::Local { method_table, .. } = interface.carrier() else {
            return Err(CallbackContractProjectionError::SourceMustBeLocal);
        };
        if interface.interface().is_empty() {
            return Err(CallbackContractProjectionError::MissingLocalInterfaceIdentity);
        }
        if method_table.interface_abi_id() != interface.interface() {
            return Err(CallbackContractProjectionError::LocalInterfaceIdentityMismatch);
        }
        if contract_operations.len() != method_table.slots().len() {
            return Err(CallbackContractProjectionError::OperationSetMismatch {
                contract: contract_operations.len(),
                implementation: method_table.slots().len(),
            });
        }

        let mut methods_by_name = BTreeMap::new();
        let mut method_abis = BTreeSet::new();
        for (expected_slot, method) in method_table.slots().iter().enumerate() {
            let expected_slot = u32::try_from(expected_slot)
                .map_err(|_| CallbackContractProjectionError::OperationSlotOverflow)?;
            if method.slot() != expected_slot {
                return Err(CallbackContractProjectionError::NonContiguousSlot {
                    expected: expected_slot,
                    actual: method.slot(),
                });
            }
            let method_name = method.method_name().ok_or(
                CallbackContractProjectionError::MissingAdmittedMethodMetadata {
                    slot: method.slot(),
                },
            )?;
            if method_name.is_empty() || method.method_abi_id().is_empty() {
                return Err(
                    CallbackContractProjectionError::MissingAdmittedMethodMetadata {
                        slot: method.slot(),
                    },
                );
            }
            if methods_by_name.insert(method_name, method).is_some() {
                return Err(CallbackContractProjectionError::DuplicateLocalMethodName {
                    method_name: method_name.to_string(),
                });
            }
            if !method_abis.insert(method.method_abi_id()) {
                return Err(CallbackContractProjectionError::DuplicateMethodAbi {
                    method_abi_id: method.method_abi_id().to_string(),
                });
            }
        }

        let mut operations = Vec::with_capacity(contract_operations.len());
        for (operation_name, contract_operation) in contract_operations {
            let method = methods_by_name
                .get(operation_name.as_str())
                .ok_or_else(|| CallbackContractProjectionError::MissingLocalMethod {
                    contract_operation: operation_name.clone(),
                })?;
            let signature = method.signature().ok_or(
                CallbackContractProjectionError::MissingAdmittedMethodMetadata {
                    slot: method.slot(),
                },
            )?;
            let InterfaceMethodTarget::LocalExecutable {
                executable,
                receiver_call_abi,
            } = method.target();
            let local_parameters = match receiver_call_abi {
                InterfaceReceiverCallAbi::ExplicitSelfFirst => {
                    signature.parameters().get(1..).ok_or_else(|| {
                        CallbackContractProjectionError::MissingReceiverParameter {
                            contract_operation: operation_name.clone(),
                        }
                    })?
                }
            };
            if local_parameters.len() != contract_operation.parameters.len()
                || !contract_operation
                    .parameters
                    .iter()
                    .zip(local_parameters)
                    .all(|(contract, local)| contract_type_matches_local(contract, local))
                || !contract_type_matches_local(
                    &contract_operation.return_type,
                    signature.return_type(),
                )
            {
                return Err(CallbackContractProjectionError::SignatureMismatch {
                    contract_operation: operation_name.clone(),
                });
            }
            operations.push(CallbackContractOperationProjection {
                contract_operation: operation_name.clone(),
                local_method_name: operation_name.clone(),
                slot: method.slot(),
                method_abi_id: method.method_abi_id().to_string(),
                executable: executable.clone(),
                receiver_call_abi: *receiver_call_abi,
                parameters: contract_operation.parameters.clone(),
                return_type: contract_operation.return_type.clone(),
            });
        }
        operations.sort_by_key(CallbackContractOperationProjection::slot);

        Ok(Self {
            canonical_package_schema_type,
            local_interface_abi_id: interface.interface().to_string(),
            operations,
        })
    }

    pub fn canonical_package_schema_type(&self) -> &PackageSchemaTypeRef {
        &self.canonical_package_schema_type
    }

    pub fn local_interface_abi_id(&self) -> &str {
        &self.local_interface_abi_id
    }

    pub fn operations(&self) -> &[CallbackContractOperationProjection] {
        &self.operations
    }

    pub fn operation(
        &self,
        slot: u32,
        method_abi_id: &str,
    ) -> Option<&CallbackContractOperationProjection> {
        usize::try_from(slot)
            .ok()
            .and_then(|index| self.operations.get(index))
            .filter(|operation| operation.slot == slot && operation.method_abi_id == method_abi_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallbackContractOperationProjection {
    contract_operation: String,
    local_method_name: String,
    slot: u32,
    method_abi_id: String,
    executable: crate::addr::ExecutableAddr,
    receiver_call_abi: InterfaceReceiverCallAbi,
    parameters: Vec<ContractTypeRef>,
    return_type: ContractTypeRef,
}

impl CallbackContractOperationProjection {
    pub fn contract_operation(&self) -> &str {
        &self.contract_operation
    }

    pub fn local_method_name(&self) -> &str {
        &self.local_method_name
    }

    pub const fn slot(&self) -> u32 {
        self.slot
    }

    pub fn method_abi_id(&self) -> &str {
        &self.method_abi_id
    }

    pub fn executable(&self) -> &crate::addr::ExecutableAddr {
        &self.executable
    }

    pub const fn receiver_call_abi(&self) -> InterfaceReceiverCallAbi {
        self.receiver_call_abi
    }

    pub fn parameters(&self) -> &[ContractTypeRef] {
        &self.parameters
    }

    pub fn return_type(&self) -> &ContractTypeRef {
        &self.return_type
    }
}

fn contract_type_matches_local(contract: &ContractTypeRef, local: &InterfaceMethodType) -> bool {
    match (contract, local) {
        (
            ContractTypeRef::Builtin { name, arguments },
            InterfaceMethodType::Builtin {
                name: local_name,
                arguments: local_arguments,
            },
        ) => {
            name == local_name
                && arguments.len() == local_arguments.len()
                && arguments
                    .iter()
                    .zip(local_arguments)
                    .all(|(contract, local)| contract_type_matches_local(contract, local))
        }
        (ContractTypeRef::Record { fields }, InterfaceMethodType::Record(local_fields)) => {
            fields.len() == local_fields.len()
                && fields.iter().all(|(name, contract)| {
                    local_fields
                        .get(name)
                        .is_some_and(|local| contract_type_matches_local(contract, local))
                })
        }
        (
            ContractTypeRef::StructuralUnion { variants },
            InterfaceMethodType::Union(local_variants),
        ) => {
            variants.len() == local_variants.len()
                && variants
                    .iter()
                    .zip(local_variants)
                    .all(|(contract, local)| contract_type_matches_local(contract, local))
        }
        (ContractTypeRef::Nullable { inner }, InterfaceMethodType::Nullable(local_inner)) => {
            contract_type_matches_local(inner, local_inner)
        }
        (
            ContractTypeRef::Literal {
                value: ContractLiteral::String { value },
            },
            InterfaceMethodType::Literal(InterfaceMethodLiteral::String(local_value)),
        ) => value == local_value,
        // A Package-owned nominal type can only match an execution type carrying
        // the same owner, stable schema key and type id. InterfaceMethodType does
        // not yet retain that mapping, so accepting any local shape here would
        // erase nominal identity. Fail closed until the exact mapping exists.
        (ContractTypeRef::PackageSchema { .. }, _) => false,
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CallbackContractProjectionError {
    #[error("callback projection source must be an owner-local interface")]
    SourceMustBeLocal,
    #[error("callback projection local interface ABI does not match its method table")]
    LocalInterfaceIdentityMismatch,
    #[error("callback projection local interface ABI must be non-empty")]
    MissingLocalInterfaceIdentity,
    #[error(
        "callback projection operation set mismatch: contract declares {contract}, implementation has {implementation}"
    )]
    OperationSetMismatch {
        contract: usize,
        implementation: usize,
    },
    #[error("callback projection operation slot does not fit u32")]
    OperationSlotOverflow,
    #[error("callback projection slots are not contiguous: expected {expected}, found {actual}")]
    NonContiguousSlot { expected: u32, actual: u32 },
    #[error("callback projection slot {slot} lacks admitted method name, ABI, or signature")]
    MissingAdmittedMethodMetadata { slot: u32 },
    #[error("callback projection local method name {method_name} is duplicated")]
    DuplicateLocalMethodName { method_name: String },
    #[error("callback projection method ABI {method_abi_id} is duplicated")]
    DuplicateMethodAbi { method_abi_id: String },
    #[error("callback contract operation {contract_operation} has no same-name local method")]
    MissingLocalMethod { contract_operation: String },
    #[error("callback contract operation {contract_operation} local method has no receiver")]
    MissingReceiverParameter { contract_operation: String },
    #[error(
        "callback contract operation {contract_operation} signature does not match local method"
    )]
    SignatureMismatch { contract_operation: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        addr::ExecutableAddr,
        runtime_value::{
            InterfaceMethodSignature, InterfaceMethodSlot, InterfaceMethodTable,
            InterfaceMethodTarget, RuntimeValue,
        },
    };
    use skiff_artifact_model::PackageSchemaTypeId;

    fn local_interface(method_name: &str, method_abi_id: &str) -> InterfaceValue {
        InterfaceValue::new(
            "interface-abi:observer".to_string(),
            InterfaceCarrier::Local {
                concrete_type: "local:Observer".to_string(),
                method_table: InterfaceMethodTable::new(
                    "table:observer".to_string(),
                    "interface-abi:observer".to_string(),
                    vec![InterfaceMethodSlot::from_admitted_metadata(
                        0,
                        method_name.to_string(),
                        method_abi_id.to_string(),
                        InterfaceMethodSignature::new(
                            vec![InterfaceMethodType::builtin("Self")],
                            InterfaceMethodType::builtin("bool"),
                        ),
                        InterfaceMethodTarget::LocalExecutable {
                            executable: ExecutableAddr::service(0, 1),
                            receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
                        },
                    )],
                ),
                payload: RuntimeValue::Bool(true),
            },
        )
    }

    fn operations() -> BTreeMap<String, BoundaryCallbackOperation> {
        BTreeMap::from([(
            "invoke".to_string(),
            BoundaryCallbackOperation {
                parameters: Vec::new(),
                return_type: ContractTypeRef::builtin("bool"),
            },
        )])
    }

    fn package_schema_type(
        package_id: &str,
        stable_schema_key: &str,
        type_id: &str,
    ) -> PackageSchemaTypeRef {
        PackageSchemaTypeRef {
            package_id: package_id.to_string(),
            stable_schema_key: stable_schema_key.to_string(),
            package_schema_type_id: PackageSchemaTypeId::new(type_id),
        }
    }

    #[test]
    fn callback_contract_projection_separates_identity_domains_and_maps_by_name() {
        let projection = CallbackContractProjection::build(
            package_schema_type(
                "skiff.run/observer",
                "api.Observer",
                "package-schema-type:observer",
            ),
            &operations(),
            &local_interface("invoke", "method-abi:callback-probe:invoke"),
        )
        .expect("typed admitted method metadata should project");

        assert_eq!(
            projection.canonical_package_schema_type(),
            &package_schema_type(
                "skiff.run/observer",
                "api.Observer",
                "package-schema-type:observer",
            )
        );
        assert_eq!(
            projection.local_interface_abi_id(),
            "interface-abi:observer"
        );
        assert_eq!(
            projection.operations()[0].method_abi_id(),
            "method-abi:callback-probe:invoke"
        );
    }

    #[test]
    fn callback_contract_projection_rejects_order_or_string_identity_shortcuts() {
        assert!(matches!(
            CallbackContractProjection::build(
                package_schema_type(
                    "skiff.run/observer",
                    "api.Observer",
                    "package-schema-type:observer",
                ),
                &operations(),
                &local_interface("different", "method-abi:callback-probe:invoke"),
            ),
            Err(CallbackContractProjectionError::MissingLocalMethod { .. })
        ));
        assert!(matches!(
            CallbackContractProjection::build(
                package_schema_type(
                    "skiff.run/observer",
                    "api.Observer",
                    "package-schema-type:observer",
                ),
                &operations(),
                &local_interface("invoke", ""),
            ),
            Err(CallbackContractProjectionError::MissingAdmittedMethodMetadata { .. })
        ));
    }

    #[test]
    fn callback_contract_projection_rejects_package_nominal_without_exact_execution_identity() {
        let callback_type = package_schema_type(
            "skiff.run/observer",
            "api.Observer",
            "package-schema-type:observer",
        );
        let interface = InterfaceValue::new(
            "interface-abi:observer".to_string(),
            InterfaceCarrier::Local {
                concrete_type: "local:Observer".to_string(),
                method_table: InterfaceMethodTable::new(
                    "table:observer".to_string(),
                    "interface-abi:observer".to_string(),
                    vec![InterfaceMethodSlot::from_admitted_metadata(
                        0,
                        "invoke".to_string(),
                        "method-abi:invoke".to_string(),
                        InterfaceMethodSignature::new(
                            vec![
                                InterfaceMethodType::builtin("Self"),
                                InterfaceMethodType::builtin("unknown"),
                            ],
                            InterfaceMethodType::builtin("unknown"),
                        ),
                        InterfaceMethodTarget::LocalExecutable {
                            executable: ExecutableAddr::service(0, 1),
                            receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
                        },
                    )],
                ),
                payload: RuntimeValue::Bool(true),
            },
        );
        for payload_type in [
            ContractTypeRef::package_schema(
                "skiff.run/types",
                "api.Payload",
                PackageSchemaTypeId::new("package-schema-type:payload"),
            ),
            ContractTypeRef::package_schema(
                "skiff.run/other-types",
                "api.Payload",
                PackageSchemaTypeId::new("package-schema-type:payload"),
            ),
            ContractTypeRef::package_schema(
                "skiff.run/types",
                "api.OtherPayload",
                PackageSchemaTypeId::new("package-schema-type:payload"),
            ),
            ContractTypeRef::package_schema(
                "skiff.run/types",
                "api.Payload",
                PackageSchemaTypeId::new("package-schema-type:other-payload"),
            ),
        ] {
            let operations = BTreeMap::from([(
                "invoke".to_string(),
                BoundaryCallbackOperation {
                    parameters: vec![payload_type.clone()],
                    return_type: payload_type,
                },
            )]);
            assert!(matches!(
                CallbackContractProjection::build(callback_type.clone(), &operations, &interface),
                Err(CallbackContractProjectionError::SignatureMismatch { .. })
            ));
        }
    }
}
