use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use skiff_artifact_model::{
    BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner,
    BoundaryValuePlan, BoundaryValuePlanUnavailableReason, ContractSchemaType,
    ContractTypeDescriptor, ContractTypeId, ContractTypeNameability, ContractTypeRef,
    ContractTypeShape,
};
use skiff_runtime_model::value::{
    CallbackCapabilityCarrier, HeapNode, InterfaceCarrier, InterfaceMethodTable, InterfaceValue,
    RuntimeObject, RuntimeObjectFields, RuntimeValue,
};

use super::service_linkable::*;
use crate::request_heap::{RequestHeap, RequestHeapLimits};

fn detached_plan(owner: BoundaryValueOwner, lifetime: BoundaryValueLifetime) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime,
    }
}

fn callback_plan(lifetime: BoundaryValueLifetime) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::CallbackCapability,
        encoding: BoundaryValueEncoding::OpaqueCapability,
        owner: BoundaryValueOwner::CapabilityOwner,
        lifetime,
    }
}

fn callback_schema() -> (
    ContractTypeRef,
    BTreeMap<ContractTypeId, ContractSchemaType>,
) {
    let id = ContractTypeId::new("contract:reader");
    let ty = ContractTypeRef::contract(id.clone());
    let schema = BTreeMap::from([(
        id.clone(),
        ContractSchemaType {
            contract_type_id: id,
            stable_key: "Reader".to_string(),
            shape: ContractTypeShape {
                nameability: ContractTypeNameability::PublicNameable,
                descriptor: ContractTypeDescriptor::CallbackInterface {
                    operations: BTreeMap::new(),
                },
            },
        },
    )]);
    (ty, schema)
}

#[test]
fn service_linkable_detached_materialization_uses_contract_shape_and_isolates_aliases() {
    let ty = ContractTypeRef::Record {
        fields: BTreeMap::from([
            (
                "first".to_string(),
                ContractTypeRef::Builtin {
                    name: "Array".to_string(),
                    arguments: vec![ContractTypeRef::builtin("string")],
                },
            ),
            (
                "second".to_string(),
                ContractTypeRef::Builtin {
                    name: "Array".to_string(),
                    arguments: vec![ContractTypeRef::builtin("string")],
                },
            ),
        ]),
    };
    let schema = BTreeMap::new();
    let plan = detached_plan(BoundaryValueOwner::Caller, BoundaryValueLifetime::Call);
    let contract_plan = ServiceLinkableContractPlan::new(&ty, &schema, &plan)
        .expect("canonical detached plan should build");
    let mut source = RequestHeap::default();
    let shared = source
        .alloc_array(vec![RuntimeValue::String("source".to_string())])
        .expect("shared source array should allocate");
    let root = source
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
            ("first".to_string(), RuntimeValue::Heap(shared)),
            ("second".to_string(), RuntimeValue::Heap(shared)),
        ])))
        .expect("source record should allocate");
    let mut destination = RequestHeap::default();

    let materialized = contract_plan
        .materialize(
            &RuntimeValue::Heap(root),
            &source,
            &mut destination,
            ServiceLinkableMaterializationScope {
                owner: BoundaryValueOwner::Caller,
                lifetime: BoundaryValueLifetime::Call,
            },
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .expect("well-typed ordinary graph should materialize");
    let RuntimeValue::Heap(materialized_root) = materialized else {
        panic!("materialized record should be a heap value");
    };
    let HeapNode::Object(materialized_object) = destination
        .get(materialized_root)
        .expect("materialized record should resolve")
    else {
        panic!("materialized record should remain an object");
    };
    let RuntimeValue::Heap(first) = materialized_object.fields()["first"] else {
        panic!("first field should remain an array");
    };
    let RuntimeValue::Heap(second) = materialized_object.fields()["second"] else {
        panic!("second field should remain an array");
    };
    assert_ne!(
        first, second,
        "caller-visible alias identity must not cross the service boundary"
    );
    destination
        .set_array_item(first, 0, RuntimeValue::String("provider".to_string()))
        .expect("provider copy should be mutable");
    let HeapNode::Array(source_items) = source.get(shared).expect("source alias should resolve")
    else {
        panic!("source alias should remain an array");
    };
    assert_eq!(source_items, &[RuntimeValue::String("source".to_string())]);
    let HeapNode::Array(second_items) = destination
        .get(second)
        .expect("second detached array should resolve")
    else {
        panic!("second detached field should remain an array");
    };
    assert_eq!(second_items, &[RuntimeValue::String("source".to_string())]);
}

#[test]
fn service_linkable_plan_rejects_unsupported_missing_schema_and_invalid_pair() {
    let schema = BTreeMap::new();
    let unsupported = BoundaryValuePlan::Unsupported {
        reason: BoundaryValuePlanUnavailableReason::LanguageUnsupported,
    };
    assert!(matches!(
        ServiceLinkableContractPlan::new(
            &ContractTypeRef::builtin("string"),
            &schema,
            &unsupported
        ),
        Err(ServiceLinkableMaterializationError::UnsupportedPlan { .. })
    ));

    let missing = ContractTypeRef::contract(ContractTypeId::new("missing"));
    assert!(matches!(
        ServiceLinkableContractPlan::new(
            &missing,
            &schema,
            &detached_plan(BoundaryValueOwner::Caller, BoundaryValueLifetime::Call)
        ),
        Err(ServiceLinkableMaterializationError::MissingSchema { .. })
    ));

    let invalid = BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::OpaqueCapability,
        owner: BoundaryValueOwner::Caller,
        lifetime: BoundaryValueLifetime::Call,
    };
    assert!(matches!(
        ServiceLinkableContractPlan::new(&ContractTypeRef::builtin("string"), &schema, &invalid),
        Err(ServiceLinkableMaterializationError::InvalidPlan { .. })
    ));
}

#[test]
fn service_linkable_materialization_rejects_wrong_owner_lifetime_and_local_interface() {
    let schema = BTreeMap::new();
    let ty = ContractTypeRef::builtin("Json");
    let plan = detached_plan(BoundaryValueOwner::Caller, BoundaryValueLifetime::Call);
    let contract_plan = ServiceLinkableContractPlan::new(&ty, &schema, &plan).unwrap();
    let source = RequestHeap::default();
    let mut destination = RequestHeap::default();
    assert!(matches!(
        contract_plan.materialize(
            &RuntimeValue::Null,
            &source,
            &mut destination,
            ServiceLinkableMaterializationScope {
                owner: BoundaryValueOwner::Provider,
                lifetime: BoundaryValueLifetime::Call,
            },
            &FailClosedServiceLinkableCapabilityHooks,
        ),
        Err(ServiceLinkableMaterializationError::OwnerMismatch { .. })
    ));
    assert!(matches!(
        contract_plan.materialize(
            &RuntimeValue::Null,
            &source,
            &mut destination,
            ServiceLinkableMaterializationScope {
                owner: BoundaryValueOwner::Caller,
                lifetime: BoundaryValueLifetime::Request,
            },
            &FailClosedServiceLinkableCapabilityHooks,
        ),
        Err(ServiceLinkableMaterializationError::LifetimeMismatch { .. })
    ));

    let mut interface_heap = RequestHeap::default();
    let local = interface_heap
        .alloc_interface(InterfaceValue::new(
            "pkg.Reader".to_string(),
            InterfaceCarrier::Local {
                concrete_type: "pkg.ReaderImpl".to_string(),
                method_table: InterfaceMethodTable::new(
                    "projection:reader".to_string(),
                    "pkg.Reader".to_string(),
                    Vec::new(),
                ),
                payload: RuntimeValue::Null,
            },
        ))
        .expect("local interface should allocate");
    assert!(matches!(
        contract_plan.materialize(
            &RuntimeValue::Heap(local),
            &interface_heap,
            &mut RequestHeap::default(),
            ServiceLinkableMaterializationScope {
                owner: BoundaryValueOwner::Caller,
                lifetime: BoundaryValueLifetime::Call,
            },
            &FailClosedServiceLinkableCapabilityHooks,
        ),
        Err(ServiceLinkableMaterializationError::DetachedInterfaceCarrier { carrier: "local" })
    ));
}

#[derive(Default)]
struct RecordingCapabilityHooks {
    callback_calls: AtomicUsize,
    native_calls: AtomicUsize,
    rollback_calls: Arc<AtomicUsize>,
}

impl ServiceLinkableCapabilityHooks for RecordingCapabilityHooks {
    fn project_callback_capability(
        &self,
        _request: ServiceLinkableCapabilityRequest<'_>,
    ) -> Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError> {
        self.callback_calls.fetch_add(1, Ordering::SeqCst);
        let rollback_calls = Arc::clone(&self.rollback_calls);
        Ok(
            ServiceLinkableCapabilityProjection::new_with_receiver_interface(
                CallbackCapabilityCarrier::new(
                    "runtime-a",
                    "activation-a",
                    7,
                    "contract:reader",
                    "callback-1",
                ),
                "interface-abi:reader",
                move || {
                    rollback_calls.fetch_add(1, Ordering::SeqCst);
                },
            ),
        )
    }

    fn project_native_adapter_capability(
        &self,
        _request: ServiceLinkableCapabilityRequest<'_>,
    ) -> Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError> {
        self.native_calls.fetch_add(1, Ordering::SeqCst);
        let rollback_calls = Arc::clone(&self.rollback_calls);
        Ok(
            ServiceLinkableCapabilityProjection::new_with_receiver_interface(
                CallbackCapabilityCarrier::new(
                    "runtime-a",
                    "activation-a",
                    7,
                    "adapter:file",
                    "native-1",
                ),
                "interface-abi:native-file",
                move || {
                    rollback_calls.fetch_add(1, Ordering::SeqCst);
                },
            ),
        )
    }
}

struct InvalidCapabilityHooks {
    rollback_calls: Arc<AtomicUsize>,
    payload_drops: Arc<AtomicUsize>,
}

struct RollbackDropProbe(Arc<AtomicUsize>);

impl Drop for RollbackDropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl ServiceLinkableCapabilityHooks for InvalidCapabilityHooks {
    fn project_callback_capability(
        &self,
        _request: ServiceLinkableCapabilityRequest<'_>,
    ) -> Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError> {
        let rollback_calls = Arc::clone(&self.rollback_calls);
        let payload = RollbackDropProbe(Arc::clone(&self.payload_drops));
        Ok(
            ServiceLinkableCapabilityProjection::new_with_receiver_interface(
                CallbackCapabilityCarrier::new(
                    "",
                    "activation-a",
                    7,
                    "contract:reader",
                    "callback-invalid",
                ),
                "interface-abi:reader",
                move || {
                    rollback_calls.fetch_add(1, Ordering::SeqCst);
                    drop(payload);
                },
            ),
        )
    }

    fn project_native_adapter_capability(
        &self,
        _request: ServiceLinkableCapabilityRequest<'_>,
    ) -> Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError> {
        unreachable!("invalid callback fixture must not use native projection")
    }
}

struct AllocationFailureCapabilityHooks {
    rollback_calls: Arc<AtomicUsize>,
    payload_drops: Arc<AtomicUsize>,
}

impl ServiceLinkableCapabilityHooks for AllocationFailureCapabilityHooks {
    fn project_callback_capability(
        &self,
        _request: ServiceLinkableCapabilityRequest<'_>,
    ) -> Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError> {
        let rollback_calls = Arc::clone(&self.rollback_calls);
        let payload = RollbackDropProbe(Arc::clone(&self.payload_drops));
        Ok(
            ServiceLinkableCapabilityProjection::new_with_receiver_interface(
                CallbackCapabilityCarrier::new(
                    "runtime-a",
                    "activation-a",
                    7,
                    "contract:reader",
                    "callback-allocation-failure",
                ),
                "interface-abi:reader",
                move || {
                    rollback_calls.fetch_add(1, Ordering::SeqCst);
                    drop(payload);
                },
            ),
        )
    }

    fn project_native_adapter_capability(
        &self,
        _request: ServiceLinkableCapabilityRequest<'_>,
    ) -> Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError> {
        unreachable!("allocation failure callback fixture must not use native projection")
    }
}

#[test]
fn callback_capability_rollback_covers_validation_and_destination_allocation_failure() {
    let (callback_ty, callback_schema) = callback_schema();
    let callback_value_plan = callback_plan(BoundaryValueLifetime::Request);
    let callback_contract_plan =
        ServiceLinkableContractPlan::new(&callback_ty, &callback_schema, &callback_value_plan)
            .expect("callback plan should build");
    let source = RequestHeap::default();
    let scope = ServiceLinkableMaterializationScope {
        owner: BoundaryValueOwner::CapabilityOwner,
        lifetime: BoundaryValueLifetime::Request,
    };

    let invalid_rollback_calls = Arc::new(AtomicUsize::new(0));
    let invalid_payload_drops = Arc::new(AtomicUsize::new(0));
    let invalid_hooks = InvalidCapabilityHooks {
        rollback_calls: Arc::clone(&invalid_rollback_calls),
        payload_drops: Arc::clone(&invalid_payload_drops),
    };
    assert!(matches!(
        callback_contract_plan.materialize(
            &RuntimeValue::Null,
            &source,
            &mut RequestHeap::default(),
            scope,
            &invalid_hooks,
        ),
        Err(ServiceLinkableMaterializationError::InvalidProjectedCapability)
    ));
    assert_eq!(invalid_rollback_calls.load(Ordering::SeqCst), 1);
    assert_eq!(invalid_payload_drops.load(Ordering::SeqCst), 1);

    let rollback_calls = Arc::new(AtomicUsize::new(0));
    let payload_drops = Arc::new(AtomicUsize::new(0));
    let hooks = AllocationFailureCapabilityHooks {
        rollback_calls: Arc::clone(&rollback_calls),
        payload_drops: Arc::clone(&payload_drops),
    };
    let limits = RequestHeapLimits {
        max_nodes: 1,
        ..RequestHeapLimits::default()
    };
    let mut destination = RequestHeap::new(limits);
    destination
        .alloc_array(Vec::new())
        .expect("fixture should fill destination node capacity");
    let checkpoint_stats = destination.stats();
    assert!(matches!(
        callback_contract_plan.materialize(
            &RuntimeValue::Null,
            &source,
            &mut destination,
            scope,
            &hooks,
        ),
        Err(ServiceLinkableMaterializationError::RuntimeModel { .. })
    ));
    assert_eq!(destination.len(), 1);
    assert_eq!(destination.stats(), checkpoint_stats);
    assert_eq!(rollback_calls.load(Ordering::SeqCst), 1);
    assert_eq!(payload_drops.load(Ordering::SeqCst), 1);
}

#[test]
fn service_linkable_callback_and_native_materialization_only_use_explicit_hooks() {
    let (callback_ty, callback_schema) = callback_schema();
    let callback_value_plan = callback_plan(BoundaryValueLifetime::Request);
    let callback_contract_plan =
        ServiceLinkableContractPlan::new(&callback_ty, &callback_schema, &callback_value_plan)
            .expect("callback plan should build");
    let hooks = RecordingCapabilityHooks::default();
    let source = RequestHeap::default();
    let mut destination = RequestHeap::default();
    let callback = callback_contract_plan
        .materialize(
            &RuntimeValue::Null,
            &source,
            &mut destination,
            ServiceLinkableMaterializationScope {
                owner: BoundaryValueOwner::CapabilityOwner,
                lifetime: BoundaryValueLifetime::Request,
            },
            &hooks,
        )
        .expect("explicit callback hook should project opaque capability");
    let RuntimeValue::Heap(callback) = callback else {
        panic!("callback projection should allocate interface wrapper");
    };
    let HeapNode::Interface(callback) = destination.get(callback).unwrap() else {
        panic!("callback projection should allocate interface wrapper");
    };
    assert_eq!(callback.interface(), "interface-abi:reader");
    let InterfaceCarrier::CallbackCapability(capability) = callback.carrier() else {
        panic!("callback projection should retain an opaque capability carrier");
    };
    assert_eq!(
        capability.interface_or_adapter_contract(),
        "contract:reader"
    );
    assert_eq!(hooks.callback_calls.load(Ordering::SeqCst), 1);
    assert_eq!(hooks.native_calls.load(Ordering::SeqCst), 0);
    assert_eq!(hooks.rollback_calls.load(Ordering::SeqCst), 0);

    let native_ty = ContractTypeRef::builtin("string");
    let native_value_plan = callback_plan(BoundaryValueLifetime::Request);
    let native_schema = BTreeMap::new();
    let native_plan =
        ServiceLinkableContractPlan::new(&native_ty, &native_schema, &native_value_plan)
            .expect("native adapter plan should build");
    native_plan
        .materialize(
            &RuntimeValue::String("native-handle".to_string()),
            &source,
            &mut destination,
            ServiceLinkableMaterializationScope {
                owner: BoundaryValueOwner::CapabilityOwner,
                lifetime: BoundaryValueLifetime::Request,
            },
            &hooks,
        )
        .expect("explicit native hook should project opaque capability");
    assert_eq!(hooks.callback_calls.load(Ordering::SeqCst), 1);
    assert_eq!(hooks.native_calls.load(Ordering::SeqCst), 1);
    assert_eq!(hooks.rollback_calls.load(Ordering::SeqCst), 0);

    assert!(matches!(
        callback_contract_plan.materialize(
            &RuntimeValue::Null,
            &source,
            &mut RequestHeap::default(),
            ServiceLinkableMaterializationScope {
                owner: BoundaryValueOwner::CapabilityOwner,
                lifetime: BoundaryValueLifetime::Request,
            },
            &FailClosedServiceLinkableCapabilityHooks,
        ),
        Err(ServiceLinkableMaterializationError::CallbackHookRequired)
    ));
}
