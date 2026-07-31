use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock, RwLock},
};

use skiff_artifact_model::{
    BoundaryCallbackOperation, ContractTypeDescriptor, ContractTypeRef, PackageSchemaTypeRef,
};
use skiff_runtime_boundary::package_schema_records::PackageSchemaRecords;
use skiff_runtime_boundary::service_linkable::{
    ServiceLinkableContractPlan, ServiceLinkableMaterializationError,
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
    package_schema_records: PackageSchemaRecords,
    owner_heap: Arc<tokio::sync::Mutex<RequestHeap>>,
}

impl InProcessCallbackAdapter {
    pub fn from_local_interface(
        canonical_package_schema_type: PackageSchemaTypeRef,
        interface: &InterfaceValue,
        contract_operations: &BTreeMap<String, BoundaryCallbackOperation>,
        package_schema_records: &PackageSchemaRecords,
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
        package_schema_records: &PackageSchemaRecords,
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
        package_schema_records: &PackageSchemaRecords,
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

    pub fn package_schema_records(&self) -> &PackageSchemaRecords {
        &self.package_schema_records
    }

    pub fn projection(&self) -> &CallbackContractProjection {
        &self.projection
    }

    pub fn operations(&self) -> &[CallbackContractOperationProjection] {
        self.projection.operations()
    }

    /// Acquires the callback owner's heap without borrowing the adapter.
    ///
    /// Callback execution uses the owned guard as its one invocation-scoped
    /// authority. The adapter deliberately exposes no independent heap
    /// mutation methods, and reentrant invocation fails immediately instead
    /// of waiting on its own owner lock.
    pub fn try_lock_owner_heap_owned(
        &self,
    ) -> Result<tokio::sync::OwnedMutexGuard<RequestHeap>, CallbackAdapterError> {
        Arc::clone(&self.owner_heap)
            .try_lock_owned()
            .map_err(|_| CallbackAdapterError::OwnerStateUnavailable)
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
    #[error("callback adapter owner state is already executing")]
    OwnerStateUnavailable,
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
    records: &PackageSchemaRecords,
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
mod tests;
