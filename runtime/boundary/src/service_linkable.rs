use skiff_artifact_model::{
    BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner,
    BoundaryValuePlan, BoundaryValuePlanUnavailableReason, ContractTypeRef, PackageSchemaTypeId,
};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    value::{CallbackCapabilityCarrier, InterfaceCarrier, InterfaceValue, RuntimeValue},
};

use crate::{
    package_schema_records::PackageSchemaRecords,
    service_linkable_detached::{
        materialize_detached_graph, model_error, reject_detached_interface_graph,
    },
    service_linkable_schema::{contract_type_is_callback_interface, validate_schema_closure},
    service_value_plan::ServiceValuePlan,
};

/// Borrowed, canonical service contract plan used by the in-process boundary.
/// It retains no File IR or runtime-inferred type descriptor.
pub struct ServiceLinkableContractPlan<'a> {
    ty: &'a ContractTypeRef,
    package_schema_records: &'a PackageSchemaRecords,
    value_plan: &'a BoundaryValuePlan,
    detached_value_plan: Option<ServiceValuePlan<'a>>,
}

impl<'a> ServiceLinkableContractPlan<'a> {
    pub fn new(
        ty: &'a ContractTypeRef,
        package_schema_records: &'a PackageSchemaRecords,
        value_plan: &'a BoundaryValuePlan,
    ) -> Result<Self, ServiceLinkableMaterializationError> {
        validate_value_plan_shape(value_plan)?;
        let detached_value_plan = match value_plan {
            BoundaryValuePlan::Linkable {
                carrier: BoundaryValueCarrier::DetachedValueGraph,
                ..
            } => Some(ServiceValuePlan::compile(ty, package_schema_records)?),
            BoundaryValuePlan::Linkable {
                carrier: BoundaryValueCarrier::CallbackCapability,
                ..
            } => {
                validate_schema_closure(ty, package_schema_records)?;
                if !contract_type_is_callback_interface(ty, package_schema_records)? {
                    return Err(ServiceLinkableMaterializationError::InvalidContractPlan {
                        message: "callback capability requires an exact non-generic any interface contract".to_string(),
                    });
                }
                None
            }
            BoundaryValuePlan::Unsupported { .. } => {
                unreachable!("unsupported plans are rejected by validate_value_plan_shape")
            }
        };
        Ok(Self {
            ty,
            package_schema_records,
            value_plan,
            detached_value_plan,
        })
    }

    pub fn ty(&self) -> &ContractTypeRef {
        self.ty
    }

    pub fn value_plan(&self) -> &BoundaryValuePlan {
        self.value_plan
    }

    pub fn materialize(
        &self,
        value: &RuntimeValue,
        source_heap: &RequestHeap,
        destination_heap: &mut RequestHeap,
        scope: ServiceLinkableMaterializationScope,
        hooks: &dyn ServiceLinkableCapabilityHooks,
    ) -> Result<RuntimeValue, ServiceLinkableMaterializationError> {
        let BoundaryValuePlan::Linkable {
            carrier,
            encoding,
            owner,
            lifetime,
        } = self.value_plan
        else {
            let BoundaryValuePlan::Unsupported { reason } = self.value_plan else {
                unreachable!();
            };
            return Err(ServiceLinkableMaterializationError::UnsupportedPlan { reason: *reason });
        };
        if *owner != scope.owner {
            return Err(ServiceLinkableMaterializationError::OwnerMismatch {
                expected: *owner,
                actual: scope.owner,
            });
        }
        if *lifetime != scope.lifetime {
            return Err(ServiceLinkableMaterializationError::LifetimeMismatch {
                expected: *lifetime,
                actual: scope.lifetime,
            });
        }
        match (carrier, encoding) {
            (BoundaryValueCarrier::DetachedValueGraph, BoundaryValueEncoding::CanonicalValue) => {
                if scope.owner == BoundaryValueOwner::CapabilityOwner {
                    return Err(ServiceLinkableMaterializationError::InvalidPlan {
                        message: "detached value graph cannot be owned by capability owner",
                    });
                }
                reject_detached_interface_graph(value, source_heap)?;
                self.detached_value_plan
                    .as_ref()
                    .expect("detached carrier has a compiled service-value plan")
                    .validate_value(value, source_heap)?;
                let checkpoint = destination_heap.checkpoint();
                let result = materialize_detached_graph(value, source_heap, destination_heap);
                if result.is_err() {
                    destination_heap.rollback_to_checkpoint(checkpoint);
                }
                result
            }
            (BoundaryValueCarrier::CallbackCapability, BoundaryValueEncoding::OpaqueCapability) => {
                if scope.owner != BoundaryValueOwner::CapabilityOwner {
                    return Err(ServiceLinkableMaterializationError::InvalidPlan {
                        message: "callback capability must be owned by capability owner",
                    });
                }
                let request = ServiceLinkableCapabilityRequest {
                    value,
                    source_heap,
                    ty: self.ty,
                    package_schema_records: self.package_schema_records,
                    lifetime: *lifetime,
                };
                let projection = hooks.project_callback_capability(request)?;
                validate_projected_capability(projection.capability(), *lifetime)?;
                if projection.receiver_interface_abi_id().is_empty() {
                    return Err(ServiceLinkableMaterializationError::InvalidProjectedCapability);
                }
                let checkpoint = destination_heap.checkpoint();
                let handle = match destination_heap.alloc_interface(InterfaceValue::new(
                    projection.receiver_interface_abi_id().to_string(),
                    InterfaceCarrier::CallbackCapability(projection.capability().clone()),
                )) {
                    Ok(handle) => handle,
                    Err(error) => {
                        destination_heap.rollback_to_checkpoint(checkpoint);
                        drop(projection);
                        return Err(model_error(error));
                    }
                };
                projection.commit();
                Ok(RuntimeValue::Heap(handle))
            }
            _ => Err(ServiceLinkableMaterializationError::InvalidPlan {
                message: "carrier and encoding are not a canonical service-linkable pair",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceLinkableMaterializationScope {
    pub owner: BoundaryValueOwner,
    pub lifetime: BoundaryValueLifetime,
}

#[derive(Clone, Copy)]
pub struct ServiceLinkableCapabilityRequest<'a> {
    pub value: &'a RuntimeValue,
    pub source_heap: &'a RequestHeap,
    pub ty: &'a ContractTypeRef,
    pub package_schema_records: &'a PackageSchemaRecords,
    pub lifetime: BoundaryValueLifetime,
}

pub trait ServiceLinkableCapabilityHooks: Send + Sync {
    fn project_callback_capability(
        &self,
        request: ServiceLinkableCapabilityRequest<'_>,
    ) -> Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError>;

    fn project_native_adapter_capability(
        &self,
        request: ServiceLinkableCapabilityRequest<'_>,
    ) -> Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError>;
}

/// Owns the rollback obligation for a newly registered capability until the
/// destination carrier has been allocated successfully. Dropping an uncommitted
/// projection invokes the rollback exactly once.
pub struct ServiceLinkableCapabilityProjection {
    capability: CallbackCapabilityCarrier,
    /// Execution-facing local interface ABI used by the destination wrapper.
    /// This is deliberately separate from the carrier's canonical callback
    /// contract identity.
    receiver_interface_abi_id: String,
    rollback: Option<Box<dyn FnOnce() + Send + 'static>>,
}

impl ServiceLinkableCapabilityProjection {
    pub fn new_with_receiver_interface(
        capability: CallbackCapabilityCarrier,
        receiver_interface_abi_id: impl Into<String>,
        rollback: impl FnOnce() + Send + 'static,
    ) -> Self {
        Self {
            capability,
            receiver_interface_abi_id: receiver_interface_abi_id.into(),
            rollback: Some(Box::new(rollback)),
        }
    }

    pub fn capability(&self) -> &CallbackCapabilityCarrier {
        &self.capability
    }

    pub fn receiver_interface_abi_id(&self) -> &str {
        &self.receiver_interface_abi_id
    }

    fn commit(mut self) {
        self.rollback.take();
    }
}

impl Drop for ServiceLinkableCapabilityProjection {
    fn drop(&mut self) {
        if let Some(rollback) = self.rollback.take() {
            rollback();
        }
    }
}

pub struct FailClosedServiceLinkableCapabilityHooks;

impl ServiceLinkableCapabilityHooks for FailClosedServiceLinkableCapabilityHooks {
    fn project_callback_capability(
        &self,
        _request: ServiceLinkableCapabilityRequest<'_>,
    ) -> Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError> {
        Err(ServiceLinkableMaterializationError::CallbackHookRequired)
    }

    fn project_native_adapter_capability(
        &self,
        _request: ServiceLinkableCapabilityRequest<'_>,
    ) -> Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError> {
        Err(ServiceLinkableMaterializationError::NativeAdapterHookRequired)
    }
}

fn validate_value_plan_shape(
    plan: &BoundaryValuePlan,
) -> Result<(), ServiceLinkableMaterializationError> {
    match plan {
        BoundaryValuePlan::Unsupported { reason } => {
            Err(ServiceLinkableMaterializationError::UnsupportedPlan { reason: *reason })
        }
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: BoundaryValueEncoding::CanonicalValue,
            owner,
            ..
        } if *owner != BoundaryValueOwner::CapabilityOwner => Ok(()),
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::CallbackCapability,
            encoding: BoundaryValueEncoding::OpaqueCapability,
            owner: BoundaryValueOwner::CapabilityOwner,
            lifetime: BoundaryValueLifetime::Request | BoundaryValueLifetime::Stream,
        } => Ok(()),
        BoundaryValuePlan::Linkable { .. } => {
            Err(ServiceLinkableMaterializationError::InvalidPlan {
                message: "invalid carrier/encoding/owner/lifetime combination",
            })
        }
    }
}

fn validate_projected_capability(
    capability: &CallbackCapabilityCarrier,
    lifetime: BoundaryValueLifetime,
) -> Result<(), ServiceLinkableMaterializationError> {
    if capability.owner_runtime_replica_id().is_empty()
        || capability.owner_activation_id().is_empty()
        || capability.interface_or_adapter_contract().is_empty()
        || capability.opaque_capability_id().is_empty()
        || lifetime == BoundaryValueLifetime::Call
    {
        return Err(ServiceLinkableMaterializationError::InvalidProjectedCapability);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServiceLinkableMaterializationError {
    #[error("service-linkable value plan is unsupported: {reason:?}")]
    UnsupportedPlan {
        reason: BoundaryValuePlanUnavailableReason,
    },
    #[error("service-linkable value plan is invalid: {message}")]
    InvalidPlan { message: &'static str },
    #[error("service-linkable owner mismatch: expected {expected:?}, got {actual:?}")]
    OwnerMismatch {
        expected: BoundaryValueOwner,
        actual: BoundaryValueOwner,
    },
    #[error("service-linkable lifetime mismatch: expected {expected:?}, got {actual:?}")]
    LifetimeMismatch {
        expected: BoundaryValueLifetime,
        actual: BoundaryValueLifetime,
    },
    #[error("package boundary schema is missing {package_schema_type_id}")]
    MissingSchema {
        package_schema_type_id: PackageSchemaTypeId,
    },
    #[error("package boundary schema identity mismatch for {requested}: got {actual}")]
    SchemaIdentityMismatch {
        requested: PackageSchemaTypeId,
        actual: PackageSchemaTypeId,
    },
    #[error(
        "package boundary schema reference {package_schema_type_id} expects {expected_package_id}:{expected_stable_schema_key}, got {actual_package_id}:{actual_stable_schema_key}"
    )]
    SchemaOwnerOrKeyMismatch {
        package_schema_type_id: PackageSchemaTypeId,
        expected_package_id: String,
        expected_stable_schema_key: String,
        actual_package_id: String,
        actual_stable_schema_key: String,
    },
    #[error("package boundary schema contains a cycle at {package_schema_type_id}")]
    CyclicSchema {
        package_schema_type_id: PackageSchemaTypeId,
    },
    #[error("package boundary schema contains transparent alias {package_schema_type_id}")]
    AliasSchema {
        package_schema_type_id: PackageSchemaTypeId,
    },
    #[error("ordinary service value cannot contain callback interface {package_schema_type_id}")]
    CallbackInterfaceSchema {
        package_schema_type_id: PackageSchemaTypeId,
    },
    #[error("service-value contract plan is invalid: {message}")]
    InvalidContractPlan { message: String },
    #[error("runtime value does not match the canonical contract type")]
    TypeMismatch,
    #[error("runtime value matches more than one structural union branch")]
    AmbiguousStructuralUnion,
    #[error("service-value codec rejected the payload: {message}")]
    Codec { message: String },
    #[error("detached service materialization rejects {carrier} interface carrier")]
    DetachedInterfaceCarrier { carrier: &'static str },
    #[error("detached service materialization rejects a cyclic value graph")]
    CyclicValueGraph,
    #[error("callback capability projection requires an explicit runtime hook")]
    CallbackHookRequired,
    #[error("native adapter projection requires an explicit runtime hook")]
    NativeAdapterHookRequired,
    #[error("callback/native hook returned an invalid opaque capability")]
    InvalidProjectedCapability,
    #[error("runtime model rejected service materialization: {message}")]
    RuntimeModel { message: String },
}
