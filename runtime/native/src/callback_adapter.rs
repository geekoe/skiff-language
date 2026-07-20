use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock, RwLock},
};

use skiff_artifact_model::{
    BoundaryCallbackOperation, ContractSchemaType, ContractTypeId, ContractTypeRef,
};
use skiff_runtime_model::{
    addr::ExecutableAddr,
    request_heap::{deep_clone_runtime_value_between_heaps, RequestHeap},
    runtime_value::{
        InterfaceCarrier, InterfaceMethodTarget, InterfaceReceiverCallAbi, InterfaceValue,
        RuntimeValue,
    },
};

const EXPLICIT_NATIVE_ADAPTER_PREFIX: &str = "native-callback-adapter:";

static EXPLICIT_NATIVE_ADAPTERS: OnceLock<
    RwLock<BTreeMap<String, ExplicitNativeCallbackAdapterDescriptor>>,
> = OnceLock::new();

pub fn explicit_native_callback_adapter_concrete_type(adapter_identity: &str) -> String {
    format!("{EXPLICIT_NATIVE_ADAPTER_PREFIX}{adapter_identity}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitNativeCallbackAdapterDescriptor {
    adapter_identity: String,
    boundary_type: ContractTypeRef,
    adapter_contract: String,
    operations: BTreeMap<String, BoundaryCallbackOperation>,
}

impl ExplicitNativeCallbackAdapterDescriptor {
    pub fn new(
        adapter_identity: impl Into<String>,
        boundary_type: ContractTypeRef,
        adapter_contract: impl Into<String>,
        operations: BTreeMap<String, BoundaryCallbackOperation>,
    ) -> Result<Self, CallbackAdapterError> {
        let descriptor = Self {
            adapter_identity: adapter_identity.into(),
            boundary_type,
            adapter_contract: adapter_contract.into(),
            operations,
        };
        if descriptor.adapter_identity.is_empty() {
            return Err(CallbackAdapterError::MissingAdapterIdentity);
        }
        if descriptor.adapter_contract.is_empty() {
            return Err(CallbackAdapterError::MissingContract);
        }
        Ok(descriptor)
    }

    pub fn adapter_identity(&self) -> &str {
        &self.adapter_identity
    }

    pub fn boundary_type(&self) -> &ContractTypeRef {
        &self.boundary_type
    }

    pub fn adapter_contract(&self) -> &str {
        &self.adapter_contract
    }

    pub fn operations(&self) -> &BTreeMap<String, BoundaryCallbackOperation> {
        &self.operations
    }
}

pub fn register_explicit_native_callback_adapter(
    descriptor: ExplicitNativeCallbackAdapterDescriptor,
) -> Result<(), CallbackAdapterError> {
    let mut adapters = EXPLICIT_NATIVE_ADAPTERS
        .get_or_init(|| RwLock::new(BTreeMap::new()))
        .write()
        .map_err(|_| CallbackAdapterError::AdapterRegistryUnavailable)?;
    if let Some(existing) = adapters.get(descriptor.adapter_identity()) {
        return if existing == &descriptor {
            Ok(())
        } else {
            Err(CallbackAdapterError::DuplicateAdapterIdentity {
                adapter_identity: descriptor.adapter_identity,
            })
        };
    }
    adapters.insert(descriptor.adapter_identity.clone(), descriptor);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InProcessCallbackAdapterKind {
    LocalInterface,
    ExplicitNative { adapter_identity: String },
}

#[derive(Debug, Clone)]
pub struct InProcessCallbackOperation {
    contract_operation: String,
    slot: u32,
    method_abi_id: String,
    executable: ExecutableAddr,
    receiver_call_abi: InterfaceReceiverCallAbi,
    parameters: Vec<ContractTypeRef>,
    return_type: ContractTypeRef,
    may_suspend: bool,
}

impl InProcessCallbackOperation {
    pub fn contract_operation(&self) -> &str {
        &self.contract_operation
    }

    pub const fn slot(&self) -> u32 {
        self.slot
    }

    pub fn method_abi_id(&self) -> &str {
        &self.method_abi_id
    }

    pub fn executable(&self) -> &ExecutableAddr {
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

    pub const fn may_suspend(&self) -> bool {
        self.may_suspend
    }
}

#[derive(Debug, Clone)]
pub struct InProcessCallbackAdapter {
    contract: String,
    source_interface: String,
    kind: InProcessCallbackAdapterKind,
    operations: Vec<InProcessCallbackOperation>,
    receiver: RuntimeValue,
    boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
    owner_heap: Arc<tokio::sync::Mutex<RequestHeap>>,
}

impl InProcessCallbackAdapter {
    pub fn from_local_interface(
        contract: impl Into<String>,
        interface: &InterfaceValue,
        contract_operations: &BTreeMap<String, BoundaryCallbackOperation>,
        boundary_schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
        source_heap: &RequestHeap,
    ) -> Result<Self, CallbackAdapterError> {
        Self::from_interface(
            contract.into(),
            interface,
            contract_operations,
            boundary_schema,
            source_heap,
            false,
        )
    }

    pub fn from_registered_explicit_native_interface(
        boundary_type: &ContractTypeRef,
        interface: &InterfaceValue,
        boundary_schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
        source_heap: &RequestHeap,
    ) -> Result<Self, CallbackAdapterError> {
        let adapter_identity = explicit_native_adapter_identity(interface)?;
        let adapters = EXPLICIT_NATIVE_ADAPTERS
            .get_or_init(|| RwLock::new(BTreeMap::new()))
            .read()
            .map_err(|_| CallbackAdapterError::AdapterRegistryUnavailable)?;
        let descriptor = adapters.get(adapter_identity).ok_or_else(|| {
            CallbackAdapterError::UnregisteredExplicitNativeAdapter {
                adapter_identity: adapter_identity.to_string(),
            }
        })?;
        if descriptor.boundary_type() != boundary_type {
            return Err(CallbackAdapterError::BoundaryTypeMismatch);
        }
        Self::from_interface(
            descriptor.adapter_contract().to_string(),
            interface,
            descriptor.operations(),
            boundary_schema,
            source_heap,
            true,
        )
    }

    fn from_interface(
        contract: String,
        interface: &InterfaceValue,
        contract_operations: &BTreeMap<String, BoundaryCallbackOperation>,
        boundary_schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
        source_heap: &RequestHeap,
        require_native_adapter: bool,
    ) -> Result<Self, CallbackAdapterError> {
        if contract.is_empty() {
            return Err(CallbackAdapterError::MissingContract);
        }
        let InterfaceCarrier::Local {
            concrete_type: _,
            method_table,
            payload,
        } = interface.carrier()
        else {
            return Err(CallbackAdapterError::SourceMustBeLocal);
        };
        if method_table.interface_abi_id() != interface.interface() {
            return Err(CallbackAdapterError::InterfaceIdentityMismatch);
        }
        if interface.interface() != contract {
            return Err(CallbackAdapterError::InterfaceIdentityMismatch);
        }
        let kind = if require_native_adapter {
            let adapter_identity = explicit_native_adapter_identity(interface)?;
            InProcessCallbackAdapterKind::ExplicitNative {
                adapter_identity: adapter_identity.to_string(),
            }
        } else {
            InProcessCallbackAdapterKind::LocalInterface
        };
        if method_table.slots().len() != contract_operations.len() {
            return Err(CallbackAdapterError::OperationCountMismatch {
                contract: contract_operations.len(),
                implementation: method_table.slots().len(),
            });
        }
        let operations = contract_operations
            .iter()
            .zip(method_table.slots())
            .enumerate()
            .map(|(index, ((contract_operation, descriptor), method))| {
                let expected_slot = u32::try_from(index)
                    .map_err(|_| CallbackAdapterError::OperationSlotOverflow)?;
                if method.slot() != expected_slot || method.method_abi_id().is_empty() {
                    return Err(CallbackAdapterError::MethodTableMismatch {
                        contract_operation: contract_operation.clone(),
                        expected_slot,
                    });
                }
                let InterfaceMethodTarget::LocalExecutable {
                    executable,
                    receiver_call_abi,
                } = method.target();
                Ok(InProcessCallbackOperation {
                    contract_operation: contract_operation.clone(),
                    slot: method.slot(),
                    method_abi_id: method.method_abi_id().to_string(),
                    executable: executable.clone(),
                    receiver_call_abi: *receiver_call_abi,
                    parameters: descriptor.parameters.clone(),
                    return_type: descriptor.return_type.clone(),
                    may_suspend: descriptor.may_suspend,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut owner_heap = RequestHeap::new(source_heap.limits().clone());
        let receiver =
            deep_clone_runtime_value_between_heaps(source_heap, &mut owner_heap, payload).map_err(
                |error| CallbackAdapterError::OwnerStateMaterialization {
                    message: error.to_string(),
                },
            )?;
        Ok(Self {
            contract,
            source_interface: interface.interface().to_string(),
            kind,
            operations,
            receiver,
            boundary_schema: boundary_schema.clone(),
            owner_heap: Arc::new(tokio::sync::Mutex::new(owner_heap)),
        })
    }

    pub fn contract(&self) -> &str {
        &self.contract
    }

    pub fn source_interface(&self) -> &str {
        &self.source_interface
    }

    pub fn kind(&self) -> &InProcessCallbackAdapterKind {
        &self.kind
    }

    pub fn receiver(&self) -> &RuntimeValue {
        &self.receiver
    }

    pub fn boundary_schema(&self) -> &BTreeMap<ContractTypeId, ContractSchemaType> {
        &self.boundary_schema
    }

    pub fn operations(&self) -> &[InProcessCallbackOperation] {
        &self.operations
    }

    pub fn owner_heap(&self) -> &tokio::sync::Mutex<RequestHeap> {
        &self.owner_heap
    }

    pub fn operation(
        &self,
        slot: u32,
        method_abi_id: &str,
    ) -> Result<&InProcessCallbackOperation, CallbackAdapterError> {
        let index =
            usize::try_from(slot).map_err(|_| CallbackAdapterError::OperationUnavailable {
                slot,
                method_abi_id: method_abi_id.to_string(),
            })?;
        let operation = self
            .operations
            .get(index)
            .filter(|operation| operation.slot == slot && operation.method_abi_id == method_abi_id)
            .ok_or_else(|| CallbackAdapterError::OperationUnavailable {
                slot,
                method_abi_id: method_abi_id.to_string(),
            })?;
        Ok(operation)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallbackAdapterError {
    #[error("callback adapter identity must be non-empty")]
    MissingAdapterIdentity,
    #[error("callback adapter contract must be non-empty")]
    MissingContract,
    #[error("callback adapter source must be an owner-local interface")]
    SourceMustBeLocal,
    #[error("callback adapter interface identity does not match its method table or contract")]
    InterfaceIdentityMismatch,
    #[error("native callback adapter does not declare the requested boundary type")]
    BoundaryTypeMismatch,
    #[error("native value has no explicit callback adapter")]
    MissingExplicitNativeAdapter,
    #[error("native callback adapter {adapter_identity} is not registered")]
    UnregisteredExplicitNativeAdapter { adapter_identity: String },
    #[error(
        "native callback adapter identity {adapter_identity} is already registered differently"
    )]
    DuplicateAdapterIdentity { adapter_identity: String },
    #[error("native callback adapter registry is unavailable")]
    AdapterRegistryUnavailable,
    #[error("callback adapter owner state cannot be materialized: {message}")]
    OwnerStateMaterialization { message: String },
    #[error(
        "callback adapter operation count mismatch: contract declares {contract}, implementation has {implementation}"
    )]
    OperationCountMismatch {
        contract: usize,
        implementation: usize,
    },
    #[error("callback adapter operation slot does not fit u32")]
    OperationSlotOverflow,
    #[error(
        "callback adapter method table does not implement contract operation {contract_operation} at slot {expected_slot}"
    )]
    MethodTableMismatch {
        contract_operation: String,
        expected_slot: u32,
    },
    #[error("callback operation {method_abi_id} at slot {slot} is unavailable")]
    OperationUnavailable { slot: u32, method_abi_id: String },
}

fn explicit_native_adapter_identity(
    interface: &InterfaceValue,
) -> Result<&str, CallbackAdapterError> {
    let InterfaceCarrier::Local { concrete_type, .. } = interface.carrier() else {
        return Err(CallbackAdapterError::SourceMustBeLocal);
    };
    concrete_type
        .strip_prefix(EXPLICIT_NATIVE_ADAPTER_PREFIX)
        .filter(|identity| !identity.is_empty())
        .ok_or(CallbackAdapterError::MissingExplicitNativeAdapter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_runtime_model::runtime_value::{
        InterfaceMethodSlot, InterfaceMethodTable, InterfaceValue,
    };

    const CONTRACT: &str = "contract:observer";
    const METHOD: &str = "method:observer:observe";

    fn boundary_type() -> ContractTypeRef {
        ContractTypeRef::builtin("native-handle")
    }

    fn operations() -> BTreeMap<String, BoundaryCallbackOperation> {
        BTreeMap::from([(
            "observe".to_string(),
            BoundaryCallbackOperation {
                parameters: vec![ContractTypeRef::builtin("string")],
                return_type: ContractTypeRef::builtin("bool"),
                may_suspend: false,
            },
        )])
    }

    fn interface(concrete_type: String) -> InterfaceValue {
        InterfaceValue::new(
            CONTRACT.to_string(),
            InterfaceCarrier::Local {
                concrete_type,
                method_table: InterfaceMethodTable::new(
                    "table:observer".to_string(),
                    CONTRACT.to_string(),
                    vec![InterfaceMethodSlot::new(
                        0,
                        METHOD.to_string(),
                        InterfaceMethodTarget::LocalExecutable {
                            executable: ExecutableAddr::service(0, 7),
                            receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
                        },
                    )],
                ),
                payload: RuntimeValue::String("owner-state".to_string()),
            },
        )
    }

    #[test]
    fn callback_adapter_accepts_explicit_native_preimage_and_bounds_operations() {
        register_explicit_native_callback_adapter(
            ExplicitNativeCallbackAdapterDescriptor::new(
                "builtin:test",
                boundary_type(),
                CONTRACT,
                operations(),
            )
            .unwrap(),
        )
        .unwrap();
        let adapter = InProcessCallbackAdapter::from_registered_explicit_native_interface(
            &boundary_type(),
            &interface(explicit_native_callback_adapter_concrete_type(
                "builtin:test",
            )),
            &BTreeMap::new(),
            &RequestHeap::default(),
        )
        .expect("explicit native adapter should project");
        assert_eq!(adapter.contract(), CONTRACT);
        assert!(matches!(
            adapter.kind(),
            InProcessCallbackAdapterKind::ExplicitNative { adapter_identity }
                if adapter_identity == "builtin:test"
        ));
        assert_eq!(
            adapter.operation(0, METHOD).unwrap().contract_operation(),
            "observe"
        );
        assert!(matches!(
            adapter.operation(1, METHOD),
            Err(CallbackAdapterError::OperationUnavailable { .. })
        ));
        assert!(matches!(
            adapter.operation(0, "method:undeclared"),
            Err(CallbackAdapterError::OperationUnavailable { .. })
        ));
        assert!(matches!(
            InProcessCallbackAdapter::from_registered_explicit_native_interface(
                &ContractTypeRef::builtin("different-native-handle"),
                &interface(explicit_native_callback_adapter_concrete_type(
                    "builtin:test"
                )),
                &BTreeMap::new(),
                &RequestHeap::default(),
            ),
            Err(CallbackAdapterError::BoundaryTypeMismatch)
        ));
    }

    #[test]
    fn callback_adapter_rejects_native_without_explicit_adapter_marker() {
        assert!(matches!(
            InProcessCallbackAdapter::from_registered_explicit_native_interface(
                &boundary_type(),
                &interface("native-handle:secret".to_string()),
                &BTreeMap::new(),
                &RequestHeap::default(),
            ),
            Err(CallbackAdapterError::MissingExplicitNativeAdapter)
        ));

        let identity = "builtin:unregistered";
        assert!(matches!(
            InProcessCallbackAdapter::from_registered_explicit_native_interface(
                &boundary_type(),
                &interface(explicit_native_callback_adapter_concrete_type(identity)),
                &BTreeMap::new(),
                &RequestHeap::default(),
            ),
            Err(CallbackAdapterError::UnregisteredExplicitNativeAdapter {
                adapter_identity
            }) if adapter_identity == identity
        ));
    }
}
