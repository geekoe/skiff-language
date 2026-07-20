use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock, RwLock},
};

use skiff_artifact_model::{
    BoundaryCallbackOperation, ContractSchemaType, ContractTypeId, ContractTypeRef,
};
use skiff_runtime_model::{
    callback_projection::{
        CallbackContractOperationProjection, CallbackContractProjection,
        CallbackContractProjectionError,
    },
    request_heap::{deep_clone_runtime_value_between_heaps, RequestHeap},
    runtime_value::{InterfaceCarrier, InterfaceValue, RuntimeValue},
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
    canonical_contract_type_id: ContractTypeId,
    operations: BTreeMap<String, ExplicitNativeCallbackOperation>,
}

/// Native adapters must declare the same explicit stable-name/method-ABI
/// mapping used by local callback projection. A native marker alone is never a
/// license to expose every method in the local table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitNativeCallbackOperation {
    contract: BoundaryCallbackOperation,
    local_method_name: String,
    method_abi_id: String,
}

impl ExplicitNativeCallbackOperation {
    pub fn new(
        contract: BoundaryCallbackOperation,
        local_method_name: impl Into<String>,
        method_abi_id: impl Into<String>,
    ) -> Result<Self, CallbackAdapterError> {
        let operation = Self {
            contract,
            local_method_name: local_method_name.into(),
            method_abi_id: method_abi_id.into(),
        };
        if operation.local_method_name.is_empty() || operation.method_abi_id.is_empty() {
            return Err(CallbackAdapterError::MissingNativeOperationMapping);
        }
        Ok(operation)
    }

    pub fn contract(&self) -> &BoundaryCallbackOperation {
        &self.contract
    }

    pub fn local_method_name(&self) -> &str {
        &self.local_method_name
    }

    pub fn method_abi_id(&self) -> &str {
        &self.method_abi_id
    }
}

impl ExplicitNativeCallbackAdapterDescriptor {
    pub fn new(
        adapter_identity: impl Into<String>,
        boundary_type: ContractTypeRef,
        canonical_contract_type_id: ContractTypeId,
        operations: BTreeMap<String, ExplicitNativeCallbackOperation>,
    ) -> Result<Self, CallbackAdapterError> {
        let descriptor = Self {
            adapter_identity: adapter_identity.into(),
            boundary_type,
            canonical_contract_type_id,
            operations,
        };
        if descriptor.adapter_identity.is_empty() {
            return Err(CallbackAdapterError::MissingAdapterIdentity);
        }
        if descriptor.canonical_contract_type_id.as_str().is_empty() {
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

    pub fn canonical_contract_type_id(&self) -> &ContractTypeId {
        &self.canonical_contract_type_id
    }

    pub fn operations(&self) -> &BTreeMap<String, ExplicitNativeCallbackOperation> {
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
pub struct InProcessCallbackAdapter {
    projection: CallbackContractProjection,
    kind: InProcessCallbackAdapterKind,
    receiver: RuntimeValue,
    boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
    owner_heap: Arc<tokio::sync::Mutex<RequestHeap>>,
}

impl InProcessCallbackAdapter {
    pub fn from_local_interface(
        canonical_contract_type_id: ContractTypeId,
        interface: &InterfaceValue,
        contract_operations: &BTreeMap<String, BoundaryCallbackOperation>,
        boundary_schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
        source_heap: &RequestHeap,
    ) -> Result<Self, CallbackAdapterError> {
        Self::from_interface(
            canonical_contract_type_id,
            interface,
            contract_operations,
            None,
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
        let contract_operations = descriptor
            .operations()
            .iter()
            .map(|(name, operation)| (name.clone(), operation.contract().clone()))
            .collect();
        Self::from_interface(
            descriptor.canonical_contract_type_id().clone(),
            interface,
            &contract_operations,
            Some(descriptor.operations()),
            boundary_schema,
            source_heap,
            true,
        )
    }

    fn from_interface(
        canonical_contract_type_id: ContractTypeId,
        interface: &InterfaceValue,
        contract_operations: &BTreeMap<String, BoundaryCallbackOperation>,
        native_mappings: Option<&BTreeMap<String, ExplicitNativeCallbackOperation>>,
        boundary_schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
        source_heap: &RequestHeap,
        require_native_adapter: bool,
    ) -> Result<Self, CallbackAdapterError> {
        if canonical_contract_type_id.as_str().is_empty() {
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
        let kind = if require_native_adapter {
            let adapter_identity = explicit_native_adapter_identity(interface)?;
            InProcessCallbackAdapterKind::ExplicitNative {
                adapter_identity: adapter_identity.to_string(),
            }
        } else {
            InProcessCallbackAdapterKind::LocalInterface
        };
        let projection = CallbackContractProjection::build(
            canonical_contract_type_id,
            contract_operations,
            interface,
        )?;
        if let Some(native_mappings) = native_mappings {
            if native_mappings.len() != projection.operations().len()
                || projection.operations().iter().any(|operation| {
                    native_mappings
                        .get(operation.contract_operation())
                        .is_none_or(|mapping| {
                            mapping.local_method_name() != operation.local_method_name()
                                || mapping.method_abi_id() != operation.method_abi_id()
                        })
                })
            {
                return Err(CallbackAdapterError::NativeOperationMappingMismatch);
            }
        }
        let mut owner_heap = RequestHeap::new(source_heap.limits().clone());
        let receiver =
            deep_clone_runtime_value_between_heaps(source_heap, &mut owner_heap, payload).map_err(
                |error| CallbackAdapterError::OwnerStateMaterialization {
                    message: error.to_string(),
                },
            )?;
        Ok(Self {
            projection,
            kind,
            receiver,
            boundary_schema: boundary_schema.clone(),
            owner_heap: Arc::new(tokio::sync::Mutex::new(owner_heap)),
        })
    }

    pub fn canonical_contract_type_id(&self) -> &ContractTypeId {
        self.projection.canonical_contract_type_id()
    }

    pub fn source_interface(&self) -> &str {
        self.projection.local_interface_abi_id()
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

    pub fn projection(&self) -> &CallbackContractProjection {
        &self.projection
    }

    pub fn operations(&self) -> &[CallbackContractOperationProjection] {
        self.projection.operations()
    }

    pub fn owner_heap(&self) -> &tokio::sync::Mutex<RequestHeap> {
        &self.owner_heap
    }

    pub fn operation(
        &self,
        slot: u32,
        method_abi_id: &str,
    ) -> Result<&CallbackContractOperationProjection, CallbackAdapterError> {
        let operation = self
            .projection
            .operation(slot, method_abi_id)
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
    #[error("callback adapter local interface ABI does not match its method table")]
    InterfaceIdentityMismatch,
    #[error(
        "native callback adapter operation mapping must declare local method name and method ABI"
    )]
    MissingNativeOperationMapping,
    #[error("native callback adapter operation mapping does not match admitted local metadata")]
    NativeOperationMappingMismatch,
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
    #[error("callback operation {method_abi_id} at slot {slot} is unavailable")]
    OperationUnavailable { slot: u32, method_abi_id: String },
    #[error(transparent)]
    CanonicalProjection(#[from] CallbackContractProjectionError),
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
    use skiff_runtime_model::addr::ExecutableAddr;
    use skiff_runtime_model::runtime_value::{
        InterfaceMethodSignature, InterfaceMethodSlot, InterfaceMethodTable, InterfaceMethodTarget,
        InterfaceMethodType, InterfaceReceiverCallAbi, InterfaceValue,
    };

    const CONTRACT: &str = "contract:observer";
    const INTERFACE: &str = "interface-abi:observer";
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

    fn native_operations() -> BTreeMap<String, ExplicitNativeCallbackOperation> {
        BTreeMap::from([(
            "observe".to_string(),
            ExplicitNativeCallbackOperation::new(
                operations().remove("observe").unwrap(),
                "observe",
                METHOD,
            )
            .unwrap(),
        )])
    }

    fn interface(concrete_type: String) -> InterfaceValue {
        InterfaceValue::new(
            INTERFACE.to_string(),
            InterfaceCarrier::Local {
                concrete_type,
                method_table: InterfaceMethodTable::new(
                    "table:observer".to_string(),
                    INTERFACE.to_string(),
                    vec![InterfaceMethodSlot::from_admitted_metadata(
                        0,
                        "observe".to_string(),
                        METHOD.to_string(),
                        InterfaceMethodSignature::new(
                            vec![
                                InterfaceMethodType::builtin("Self"),
                                InterfaceMethodType::builtin("string"),
                            ],
                            InterfaceMethodType::builtin("bool"),
                        ),
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
                ContractTypeId::new(CONTRACT),
                native_operations(),
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
        assert_eq!(adapter.canonical_contract_type_id().as_str(), CONTRACT);
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
    fn callback_adapter_projects_distinct_contract_and_interface_identities_by_name() {
        let adapter = InProcessCallbackAdapter::from_local_interface(
            ContractTypeId::new(CONTRACT),
            &interface("local:observer".to_string()),
            &operations(),
            &BTreeMap::new(),
            &RequestHeap::default(),
        )
        .expect("contract identity must not be compared to local interface ABI");
        assert_eq!(adapter.canonical_contract_type_id().as_str(), CONTRACT);
        assert_eq!(adapter.source_interface(), INTERFACE);
        assert_eq!(
            adapter.operation(0, METHOD).unwrap().local_method_name(),
            "observe"
        );
    }

    #[test]
    fn callback_adapter_rejects_explicit_native_mapping_that_disagrees_with_admitted_abi() {
        let identity = "builtin:wrong-mapping";
        let wrong_mapping = BTreeMap::from([(
            "observe".to_string(),
            ExplicitNativeCallbackOperation::new(
                operations().remove("observe").unwrap(),
                "observe",
                "method:wrong",
            )
            .unwrap(),
        )]);
        register_explicit_native_callback_adapter(
            ExplicitNativeCallbackAdapterDescriptor::new(
                identity,
                boundary_type(),
                ContractTypeId::new(CONTRACT),
                wrong_mapping,
            )
            .unwrap(),
        )
        .unwrap();

        assert!(matches!(
            InProcessCallbackAdapter::from_registered_explicit_native_interface(
                &boundary_type(),
                &interface(explicit_native_callback_adapter_concrete_type(identity)),
                &BTreeMap::new(),
                &RequestHeap::default(),
            ),
            Err(CallbackAdapterError::NativeOperationMappingMismatch)
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
