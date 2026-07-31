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

fn schema() -> PackageSchemaRecords {
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
fn callback_adapter_owned_owner_heap_guard_is_exclusive_and_released_once() {
    let adapter = InProcessCallbackAdapter::from_local_interface(
        package_schema_type(),
        &interface("local:owned-owner-heap".to_string()),
        &operations(),
        &schema(),
        &RequestHeap::default(),
    )
    .expect("callback adapter should construct");

    let first = adapter
        .try_lock_owner_heap_owned()
        .expect("first owned owner-heap guard should be available");
    assert!(matches!(
        adapter.try_lock_owner_heap_owned(),
        Err(CallbackAdapterError::OwnerStateUnavailable)
    ));
    drop(first);

    let mut second = adapter
        .try_lock_owner_heap_owned()
        .expect("dropping the first guard should release the owner heap exactly once");
    second
        .alloc_array(vec![RuntimeValue::String(
            "visible-owner-state".to_string(),
        )])
        .expect("owner heap should remain usable through the owned guard");
    drop(second);

    let third = adapter
        .try_lock_owner_heap_owned()
        .expect("the owner heap should remain reacquirable");
    assert_eq!(third.len(), 1);
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
    let first = package_schema_type_for("example.first", "api.Observer", "schema:first-observer");
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
