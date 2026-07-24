use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock, RwLock},
};

use skiff_artifact_model::{
    BoundaryCallbackOperation, ContractTypeDescriptor, ContractTypeRef, PackageSchemaTypeRef,
};
use skiff_runtime_boundary::service_linkable::{
    ServiceLinkableContractPlan, ServiceLinkableMaterializationError,
};
use skiff_runtime_boundary::service_schema_records::ServiceSchemaRecords;
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
    RwLock<BTreeMap<(String, PackageSchemaTypeRef), ExplicitNativeCallbackAdapterDescriptor>>,
> = OnceLock::new();

pub fn explicit_native_callback_adapter_concrete_type(adapter_identity: &str) -> String {
    format!("{EXPLICIT_NATIVE_ADAPTER_PREFIX}{adapter_identity}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitNativeCallbackAdapterDescriptor {
    adapter_identity: String,
    boundary_type: ContractTypeRef,
    canonical_package_schema_type: PackageSchemaTypeRef,
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
        canonical_package_schema_type: PackageSchemaTypeRef,
        operations: BTreeMap<String, ExplicitNativeCallbackOperation>,
    ) -> Result<Self, CallbackAdapterError> {
        let descriptor = Self {
            adapter_identity: adapter_identity.into(),
            boundary_type,
            canonical_package_schema_type,
            operations,
        };
        if descriptor.adapter_identity.is_empty() {
            return Err(CallbackAdapterError::MissingAdapterIdentity);
        }
        if descriptor
            .canonical_package_schema_type
            .package_id
            .is_empty()
            || descriptor
                .canonical_package_schema_type
                .stable_schema_key
                .is_empty()
            || descriptor
                .canonical_package_schema_type
                .package_schema_type_id
                .as_str()
                .is_empty()
        {
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

    pub fn canonical_package_schema_type(&self) -> &PackageSchemaTypeRef {
        &self.canonical_package_schema_type
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
    let key = (
        descriptor.adapter_identity().to_string(),
        descriptor.canonical_package_schema_type().clone(),
    );
    if let Some(existing) = adapters.get(&key) {
        return if existing == &descriptor {
            Ok(())
        } else {
            Err(CallbackAdapterError::DuplicateAdapterIdentity {
                adapter_identity: descriptor.adapter_identity,
            })
        };
    }
    adapters.insert(key, descriptor);
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
    package_schema_records: ServiceSchemaRecords,
    owner_heap: Arc<tokio::sync::Mutex<RequestHeap>>,
}

impl InProcessCallbackAdapter {
    pub fn from_local_interface(
        canonical_package_schema_type: PackageSchemaTypeRef,
        interface: &InterfaceValue,
        contract_operations: &BTreeMap<String, BoundaryCallbackOperation>,
        package_schema_records: &ServiceSchemaRecords,
        source_heap: &RequestHeap,
    ) -> Result<Self, CallbackAdapterError> {
        Self::from_interface(
            canonical_package_schema_type,
            interface,
            contract_operations,
            None,
            package_schema_records,
            source_heap,
            false,
        )
    }

    pub fn from_registered_explicit_native_interface(
        boundary_type: &ContractTypeRef,
        canonical_package_schema_type: PackageSchemaTypeRef,
        contract_operations: &BTreeMap<String, BoundaryCallbackOperation>,
        interface: &InterfaceValue,
        package_schema_records: &ServiceSchemaRecords,
        source_heap: &RequestHeap,
    ) -> Result<Self, CallbackAdapterError> {
        let adapter_identity = explicit_native_adapter_identity(interface)?;
        let adapters = EXPLICIT_NATIVE_ADAPTERS
            .get_or_init(|| RwLock::new(BTreeMap::new()))
            .read()
            .map_err(|_| CallbackAdapterError::AdapterRegistryUnavailable)?;
        let descriptor = adapters
            .get(&(
                adapter_identity.to_string(),
                canonical_package_schema_type.clone(),
            ))
            .ok_or_else(|| CallbackAdapterError::UnregisteredExplicitNativeAdapter {
                adapter_identity: adapter_identity.to_string(),
            })?;
        if descriptor.boundary_type() != boundary_type {
            return Err(CallbackAdapterError::BoundaryTypeMismatch);
        }
        if descriptor.operations().len() != contract_operations.len()
            || descriptor.operations().iter().any(|(name, operation)| {
                contract_operations.get(name) != Some(operation.contract())
            })
        {
            return Err(CallbackAdapterError::NativeOperationMappingMismatch);
        }
        Self::from_interface(
            canonical_package_schema_type,
            interface,
            contract_operations,
            Some(descriptor.operations()),
            package_schema_records,
            source_heap,
            true,
        )
    }

    fn from_interface(
        canonical_package_schema_type: PackageSchemaTypeRef,
        interface: &InterfaceValue,
        contract_operations: &BTreeMap<String, BoundaryCallbackOperation>,
        native_mappings: Option<&BTreeMap<String, ExplicitNativeCallbackOperation>>,
        package_schema_records: &ServiceSchemaRecords,
        source_heap: &RequestHeap,
        require_native_adapter: bool,
    ) -> Result<Self, CallbackAdapterError> {
        if canonical_package_schema_type.package_id.is_empty()
            || canonical_package_schema_type.stable_schema_key.is_empty()
            || canonical_package_schema_type
                .package_schema_type_id
                .as_str()
                .is_empty()
        {
            return Err(CallbackAdapterError::MissingContract);
        }
        validate_callback_schema(
            &canonical_package_schema_type,
            contract_operations,
            package_schema_records,
        )?;
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
            canonical_package_schema_type,
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
            package_schema_records: package_schema_records.clone(),
            owner_heap: Arc::new(tokio::sync::Mutex::new(owner_heap)),
        })
    }

    pub fn canonical_package_schema_type(&self) -> &PackageSchemaTypeRef {
        self.projection.canonical_package_schema_type()
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

    pub fn package_schema_records(&self) -> &ServiceSchemaRecords {
        &self.package_schema_records
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
    #[error("callback adapter package schema is invalid: {message}")]
    InvalidPackageSchema { message: String },
    #[error("callback operation {method_abi_id} at slot {slot} is unavailable")]
    OperationUnavailable { slot: u32, method_abi_id: String },
    #[error(transparent)]
    CanonicalProjection(#[from] CallbackContractProjectionError),
}

fn validate_callback_schema(
    canonical_type: &PackageSchemaTypeRef,
    operations: &BTreeMap<String, BoundaryCallbackOperation>,
    records: &ServiceSchemaRecords,
) -> Result<(), CallbackAdapterError> {
    let record = records
        .get(&canonical_type.package_schema_type_id)
        .ok_or_else(|| CallbackAdapterError::InvalidPackageSchema {
            message: format!(
                "missing package schema record {}",
                canonical_type.package_schema_type_id
            ),
        })?;
    if record.package_id != canonical_type.package_id
        || record.stable_schema_key != canonical_type.stable_schema_key
        || record.package_schema_type_id != canonical_type.package_schema_type_id
    {
        return Err(CallbackAdapterError::InvalidPackageSchema {
            message: "callback package owner, stable key, or type identity mismatch".to_string(),
        });
    }
    let ContractTypeDescriptor::CallbackInterface {
        operations: admitted,
    } = &record.canonical_descriptor.descriptor
    else {
        return Err(CallbackAdapterError::InvalidPackageSchema {
            message: "package schema type is not a callback interface".to_string(),
        });
    };
    if admitted != operations {
        return Err(CallbackAdapterError::InvalidPackageSchema {
            message: "callback operation set does not match admitted package schema".to_string(),
        });
    }
    let detached_plan = skiff_artifact_model::BoundaryValuePlan::Linkable {
        carrier: skiff_artifact_model::BoundaryValueCarrier::DetachedValueGraph,
        encoding: skiff_artifact_model::BoundaryValueEncoding::CanonicalValue,
        owner: skiff_artifact_model::BoundaryValueOwner::Caller,
        lifetime: skiff_artifact_model::BoundaryValueLifetime::Call,
    };
    for operation in operations.values() {
        for ty in operation
            .parameters
            .iter()
            .chain(std::iter::once(&operation.return_type))
        {
            ServiceLinkableContractPlan::new(ty, records, &detached_plan).map_err(
                |error: ServiceLinkableMaterializationError| {
                    CallbackAdapterError::InvalidPackageSchema {
                        message: error.to_string(),
                    }
                },
            )?;
        }
    }
    Ok(())
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
    use skiff_artifact_model::{
        PackageSchemaCanonicalDescriptor, PackageSchemaTypeId, PackageSchemaTypeRecord,
    };
    use skiff_runtime_model::addr::ExecutableAddr;
    use skiff_runtime_model::runtime_value::{
        InterfaceMethodSignature, InterfaceMethodSlot, InterfaceMethodTable, InterfaceMethodTarget,
        InterfaceMethodType, InterfaceReceiverCallAbi, InterfaceValue,
    };

    const CONTRACT: &str = "package-schema:observer";
    const PACKAGE: &str = "example.observer";
    const STABLE_KEY: &str = "api.Observer";
    const INTERFACE: &str = "interface-abi:observer";
    const METHOD: &str = "method:observer:observe";

    fn boundary_type() -> ContractTypeRef {
        let reference = package_schema_type();
        ContractTypeRef::package_schema(
            reference.package_id,
            reference.stable_schema_key,
            reference.package_schema_type_id,
        )
    }

    fn package_schema_type() -> PackageSchemaTypeRef {
        package_schema_type_for(PACKAGE, STABLE_KEY, CONTRACT)
    }

    fn package_schema_type_for(
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

    fn schema() -> ServiceSchemaRecords {
        let reference = package_schema_type();
        BTreeMap::from([(
            reference.package_schema_type_id.clone(),
            Arc::new(PackageSchemaTypeRecord {
                package_id: reference.package_id,
                stable_schema_key: reference.stable_schema_key,
                package_schema_type_id: reference.package_schema_type_id,
                canonical_descriptor: PackageSchemaCanonicalDescriptor {
                    type_params: Vec::new(),
                    descriptor: ContractTypeDescriptor::CallbackInterface {
                        operations: operations(),
                    },
                },
            }),
        )])
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
                package_schema_type(),
                native_operations(),
            )
            .unwrap(),
        )
        .unwrap();
        let adapter = InProcessCallbackAdapter::from_registered_explicit_native_interface(
            &boundary_type(),
            package_schema_type(),
            &operations(),
            &interface(explicit_native_callback_adapter_concrete_type(
                "builtin:test",
            )),
            &schema(),
            &RequestHeap::default(),
        )
        .expect("explicit native adapter should project");
        assert_eq!(
            adapter
                .canonical_package_schema_type()
                .package_schema_type_id
                .as_str(),
            CONTRACT
        );
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
                package_schema_type(),
                &operations(),
                &interface(explicit_native_callback_adapter_concrete_type(
                    "builtin:test"
                )),
                &schema(),
                &RequestHeap::default(),
            ),
            Err(CallbackAdapterError::BoundaryTypeMismatch)
        ));
    }

    #[test]
    fn callback_adapter_projects_distinct_contract_and_interface_identities_by_name() {
        let adapter = InProcessCallbackAdapter::from_local_interface(
            package_schema_type(),
            &interface("local:observer".to_string()),
            &operations(),
            &schema(),
            &RequestHeap::default(),
        )
        .expect("contract identity must not be compared to local interface ABI");
        assert_eq!(
            adapter.canonical_package_schema_type(),
            &package_schema_type()
        );
        assert_eq!(adapter.source_interface(), INTERFACE);
        assert_eq!(
            adapter.operation(0, METHOD).unwrap().local_method_name(),
            "observe"
        );
    }

    #[test]
    fn callback_adapter_retains_the_admitted_shared_record_without_payload_clone() {
        let records = schema();
        let id = package_schema_type().package_schema_type_id;
        let admitted = Arc::clone(
            records
                .get(&id)
                .expect("callback record should be admitted"),
        );
        let owners_before = Arc::strong_count(&admitted);

        let adapter = InProcessCallbackAdapter::from_local_interface(
            package_schema_type(),
            &interface("local:shared-record".to_string()),
            &operations(),
            &records,
            &RequestHeap::default(),
        )
        .expect("adapter should retain shared schema records");

        assert_eq!(Arc::strong_count(&admitted), owners_before + 1);
        assert!(Arc::ptr_eq(
            adapter
                .package_schema_records()
                .get(&id)
                .expect("adapter should retain the callback record"),
            &admitted,
        ));
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
                package_schema_type(),
                wrong_mapping,
            )
            .unwrap(),
        )
        .unwrap();

        assert!(matches!(
            InProcessCallbackAdapter::from_registered_explicit_native_interface(
                &boundary_type(),
                package_schema_type(),
                &operations(),
                &interface(explicit_native_callback_adapter_concrete_type(identity)),
                &schema(),
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
                package_schema_type(),
                &operations(),
                &interface("native-handle:secret".to_string()),
                &schema(),
                &RequestHeap::default(),
            ),
            Err(CallbackAdapterError::MissingExplicitNativeAdapter)
        ));

        let identity = "builtin:unregistered";
        assert!(matches!(
            InProcessCallbackAdapter::from_registered_explicit_native_interface(
                &boundary_type(),
                package_schema_type(),
                &operations(),
                &interface(explicit_native_callback_adapter_concrete_type(identity)),
                &schema(),
                &RequestHeap::default(),
            ),
            Err(CallbackAdapterError::UnregisteredExplicitNativeAdapter {
                adapter_identity
            }) if adapter_identity == identity
        ));
    }

    #[test]
    fn callback_adapter_rejects_package_identity_descriptor_and_closure_mismatches() {
        let canonical = package_schema_type();
        for mutate in [
            |record: &mut PackageSchemaTypeRecord| record.package_id.push_str(".wrong"),
            |record: &mut PackageSchemaTypeRecord| record.stable_schema_key.push_str(".wrong"),
            |record: &mut PackageSchemaTypeRecord| {
                record.package_schema_type_id = PackageSchemaTypeId::new("wrong")
            },
        ] as [fn(&mut PackageSchemaTypeRecord); 3]
        {
            let mut invalid = schema();
            mutate(Arc::make_mut(
                invalid.get_mut(&canonical.package_schema_type_id).unwrap(),
            ));
            assert!(matches!(
                InProcessCallbackAdapter::from_local_interface(
                    canonical.clone(),
                    &interface("local:invalid".to_string()),
                    &operations(),
                    &invalid,
                    &RequestHeap::default(),
                ),
                Err(CallbackAdapterError::InvalidPackageSchema { .. })
            ));
        }

        let mut non_callback = schema();
        Arc::make_mut(
            non_callback
                .get_mut(&canonical.package_schema_type_id)
                .unwrap(),
        )
        .canonical_descriptor
        .descriptor = ContractTypeDescriptor::Enumeration {
            variants: vec!["value".to_string()],
        };
        assert!(matches!(
            InProcessCallbackAdapter::from_local_interface(
                canonical.clone(),
                &interface("local:non-callback".to_string()),
                &operations(),
                &non_callback,
                &RequestHeap::default(),
            ),
            Err(CallbackAdapterError::InvalidPackageSchema { .. })
        ));

        let child = package_schema_type_for("example.payload", "api.Payload", "schema:payload");
        let mut nested_operations = operations();
        nested_operations.get_mut("observe").unwrap().parameters =
            vec![ContractTypeRef::package_schema(
                child.package_id,
                child.stable_schema_key,
                child.package_schema_type_id,
            )];
        let mut missing_closure = schema();
        let ContractTypeDescriptor::CallbackInterface {
            operations: admitted,
        } = &mut Arc::make_mut(
            missing_closure
                .get_mut(&canonical.package_schema_type_id)
                .unwrap(),
        )
        .canonical_descriptor
        .descriptor
        else {
            unreachable!()
        };
        *admitted = nested_operations.clone();
        assert!(matches!(
            InProcessCallbackAdapter::from_local_interface(
                canonical,
                &interface("local:missing-closure".to_string()),
                &nested_operations,
                &missing_closure,
                &RequestHeap::default(),
            ),
            Err(CallbackAdapterError::InvalidPackageSchema { .. })
        ));
    }

    #[test]
    fn native_adapter_registry_isolates_same_name_across_packages() {
        let adapter_identity = "builtin:cross-package";
        let first =
            package_schema_type_for("example.first", "api.Observer", "schema:first-observer");
        let second =
            package_schema_type_for("example.second", "api.Observer", "schema:second-observer");
        let first_boundary = ContractTypeRef::package_schema(
            first.package_id.clone(),
            first.stable_schema_key.clone(),
            first.package_schema_type_id.clone(),
        );
        let second_boundary = ContractTypeRef::package_schema(
            second.package_id.clone(),
            second.stable_schema_key.clone(),
            second.package_schema_type_id.clone(),
        );
        for (boundary, canonical) in [
            (first_boundary.clone(), first.clone()),
            (second_boundary.clone(), second.clone()),
        ] {
            register_explicit_native_callback_adapter(
                ExplicitNativeCallbackAdapterDescriptor::new(
                    adapter_identity,
                    boundary,
                    canonical,
                    native_operations(),
                )
                .unwrap(),
            )
            .unwrap();
        }
        let schema_for = |canonical: &PackageSchemaTypeRef| {
            BTreeMap::from([(
                canonical.package_schema_type_id.clone(),
                Arc::new(PackageSchemaTypeRecord {
                    package_id: canonical.package_id.clone(),
                    stable_schema_key: canonical.stable_schema_key.clone(),
                    package_schema_type_id: canonical.package_schema_type_id.clone(),
                    canonical_descriptor: PackageSchemaCanonicalDescriptor {
                        type_params: Vec::new(),
                        descriptor: ContractTypeDescriptor::CallbackInterface {
                            operations: operations(),
                        },
                    },
                }),
            )])
        };
        let first_adapter = InProcessCallbackAdapter::from_registered_explicit_native_interface(
            &first_boundary,
            first.clone(),
            &operations(),
            &interface(explicit_native_callback_adapter_concrete_type(
                adapter_identity,
            )),
            &schema_for(&first),
            &RequestHeap::default(),
        )
        .unwrap();
        let second_adapter = InProcessCallbackAdapter::from_registered_explicit_native_interface(
            &second_boundary,
            second.clone(),
            &operations(),
            &interface(explicit_native_callback_adapter_concrete_type(
                adapter_identity,
            )),
            &schema_for(&second),
            &RequestHeap::default(),
        )
        .unwrap();
        assert_ne!(
            first_adapter.canonical_package_schema_type(),
            second_adapter.canonical_package_schema_type()
        );
    }
}
