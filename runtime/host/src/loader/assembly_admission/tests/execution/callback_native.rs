use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{BoundaryCallbackOperation, ContractTypeRef};
use skiff_runtime_activation::CallbackLifetime;
use skiff_runtime_linked_program::LinkedInterfaceInstantiationRef;
use skiff_runtime_model::runtime_value::{
    InterfaceCarrier, InterfaceMethodSlot, InterfaceMethodTable, InterfaceMethodTarget,
    InterfaceReceiverCallAbi, InterfaceValue, RuntimeValue,
};
use skiff_runtime_native::callback_adapter::InProcessCallbackAdapter;

use super::{
    artifacts::callback_interface_ref, runtime::TypedExecutionRuntime,
    scenario::TypedExecutionFixture,
};

#[tokio::test]
async fn typed_execution_callback_native() {
    let fixture = TypedExecutionFixture::admit().await;
    let provider = fixture.resolve_provider();
    let provider_target = fixture
        .eval_target
        .with_request_activation(provider.provider_request().clone())
        .expect("provider continuation should retain the admitted execution image");
    let provider_activation_id = provider_target.activation_context().activation_id().clone();
    let runtime = TypedExecutionRuntime::new(
        &fixture
            .eval_target
            .activation_context()
            .identity()
            .deployment
            .service_id,
    );
    let interpreter = runtime.interpreter();
    let provider_context = runtime.context(&interpreter, &provider_target);

    let callback_interface = callback_interface_ref("phase_four.consumer");
    let linked_interface = LinkedInterfaceInstantiationRef {
        interface_abi_id: callback_interface.interface_abi_id.clone(),
        canonical_type_args: Vec::new(),
    };
    let interface_id = skiff_runtime_linked_type_plan::linked_interface_instantiation_runtime_id(
        &linked_interface,
    );
    let method_abi_id =
        skiff_artifact_identity::canonical_interface_method_abi_id(&callback_interface, "invoke");
    let operations = BTreeMap::from([(
        "invoke".to_string(),
        BoundaryCallbackOperation {
            parameters: Vec::new(),
            return_type: ContractTypeRef::builtin("bool"),
            may_suspend: false,
        },
    )]);

    let mut callback_heap = provider_context.request_heap();
    let source_interface = local_callback_interface(
        &interface_id,
        &method_abi_id,
        fixture.consumer_executable_addr(0),
    );
    let adapter = InProcessCallbackAdapter::from_local_interface(
        &interface_id,
        &source_interface,
        &operations,
        &BTreeMap::new(),
        &callback_heap,
    )
    .expect("admitted local callback table should match its declared operation");
    let owner = fixture.eval_target.activation_context();
    let carrier = owner
        .callback_capabilities()
        .register(
            owner,
            fixture.eval_target.request_activation(),
            &interface_id,
            "host-full-chain-callback",
            CallbackLifetime::Request,
            Arc::new(adapter),
        )
        .expect("callback preimage should register under its owner activation");
    let callback_handle = callback_heap
        .alloc_interface(InterfaceValue::new(
            interface_id.clone(),
            InterfaceCarrier::CallbackCapability(carrier),
        ))
        .expect("provider callback wrapper should allocate");

    let entered_owner_error = interpreter
        .execute_runtime_assembly_addr(
            provider_context.clone(),
            &mut callback_heap,
            &fixture.consumer_executable_addr(2),
            vec![RuntimeValue::Heap(callback_handle)],
        )
        .await
        .expect_err("fixture owner target intentionally stops at the ordinary lane checkpoint");
    assert!(
        entered_owner_error.to_string().contains("service-call"),
        "callback must switch to the consumer owner before its admitted target executes: {entered_owner_error}"
    );
    assert_eq!(
        provider_context
            .runtime_assembly_target()
            .unwrap()
            .activation_context()
            .activation_id(),
        &provider_activation_id,
        "callback return/unwind must leave the provider receiver context intact"
    );

    let wrong_interface = local_callback_interface(
        &interface_id,
        "method:wrong-tuple",
        fixture.consumer_executable_addr(0),
    );
    let wrong_adapter = InProcessCallbackAdapter::from_local_interface(
        &interface_id,
        &wrong_interface,
        &operations,
        &BTreeMap::new(),
        &callback_heap,
    )
    .expect("wrong invocation tuple belongs to dispatch, not adapter construction");
    let wrong_carrier = owner
        .callback_capabilities()
        .register(
            owner,
            fixture.eval_target.request_activation(),
            &interface_id,
            "host-wrong-tuple-callback",
            CallbackLifetime::Request,
            Arc::new(wrong_adapter),
        )
        .expect("wrong-tuple fixture capability should register");
    let wrong_handle = callback_heap
        .alloc_interface(InterfaceValue::new(
            interface_id,
            InterfaceCarrier::CallbackCapability(wrong_carrier),
        ))
        .expect("wrong-tuple callback wrapper should allocate");
    let wrong_tuple_error = interpreter
        .execute_runtime_assembly_addr(
            provider_context,
            &mut callback_heap,
            &fixture.consumer_executable_addr(2),
            vec![RuntimeValue::Heap(wrong_handle)],
        )
        .await
        .expect_err("undeclared callback tuple must fail before the owner executable");
    assert!(
        wrong_tuple_error
            .to_string()
            .contains("CapabilityUnavailable"),
        "wrong callback tuple should map to stable unavailable: {wrong_tuple_error}"
    );
    assert!(
        !wrong_tuple_error.to_string().contains("service-call"),
        "wrong callback tuple must not enter the owner executable: {wrong_tuple_error}"
    );
}

fn local_callback_interface(
    interface_id: &str,
    method_abi_id: &str,
    executable: skiff_runtime_linked_program::ExecutableAddr,
) -> InterfaceValue {
    InterfaceValue::new(
        interface_id.to_string(),
        InterfaceCarrier::Local {
            concrete_type: "phase_four.consumer.CallbackProbeImpl".to_string(),
            method_table: InterfaceMethodTable::new(
                "table:phase-four-callback".to_string(),
                interface_id.to_string(),
                vec![InterfaceMethodSlot::new(
                    0,
                    method_abi_id.to_string(),
                    InterfaceMethodTarget::LocalExecutable {
                        executable,
                        receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
                    },
                )],
            ),
            payload: RuntimeValue::Bool(true),
        },
    )
}
