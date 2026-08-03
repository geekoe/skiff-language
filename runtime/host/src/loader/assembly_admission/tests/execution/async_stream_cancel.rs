use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract, BoundaryReturn,
    BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, ContractTypeRef,
};
use skiff_runtime_activation::CallbackCapabilityError;
use skiff_runtime_eval::error::RuntimeError;
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{
        CallbackCapabilityCarrier, HeapHandle, HeapNode, InterfaceCarrier, RuntimeValue,
    },
};

use super::{
    artifacts::TypedExecutionContract, runtime::TypedExecutionRuntime,
    scenario::TypedExecutionFixture,
};

#[tokio::test]
async fn typed_execution_async_stream_cancel_reaches_owned_provider_future_full_chain() {
    let fixture = TypedExecutionFixture::admit_contract(TypedExecutionContract::returning_null(
        async_unary_contract(),
        BTreeMap::new(),
        true,
    ))
    .await;
    let provider = fixture.resolve_provider();
    let receiver_id = fixture
        .eval_target
        .activation_context()
        .activation_id()
        .clone();
    assert_ne!(provider.provider_activation().activation_id(), &receiver_id);

    let runtime = TypedExecutionRuntime::new(
        &fixture
            .eval_target
            .activation_context()
            .identity()
            .deployment
            .service_id,
    );
    let interpreter = runtime.interpreter();
    let context = runtime.context(&interpreter, &fixture.eval_target, &fixture._active);
    let generation = context
        .runtime_assembly_target()
        .unwrap()
        .request_activation()
        .generation();
    let heap = context.request_heap();

    let result = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut skiff_runtime_eval::heap_access::HeapAccess::private(heap),
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect("owned provider future must return the real admitted provider result");

    assert_eq!(
        result,
        RuntimeValue::Null,
        "async service call must propagate the provider's declared void result"
    );
    assert_eq!(
        fixture.eval_target.activation_context().activation_id(),
        &receiver_id,
        "caller must remain in the receiver activation after provider completion"
    );
    assert_eq!(
        provider.provider_request().generation(),
        generation,
        "provider future must retain the explicit request generation"
    );
}

#[tokio::test]
async fn typed_execution_provider_suspension_summary_does_not_select_boundary_lane() {
    for may_suspend in [false, true] {
        let fixture = TypedExecutionFixture::admit_contract(
            TypedExecutionContract::unary().with_provider_may_suspend(may_suspend),
        )
        .await;

        fixture.assert_dynamic_execution_results().await;
    }
}

#[tokio::test]
async fn typed_execution_async_stream_cancel_restores_public_typed_error_from_fixed_carrier() {
    let fixture =
        TypedExecutionFixture::admit_contract(TypedExecutionContract::async_typed_error()).await;
    let runtime = TypedExecutionRuntime::new(
        &fixture
            .eval_target
            .activation_context()
            .identity()
            .deployment
            .service_id,
    );
    let interpreter = runtime.interpreter();
    let context = runtime.context(&interpreter, &fixture.eval_target, &fixture._active);
    let heap = context.request_heap();
    let mut access = skiff_runtime_eval::heap_access::HeapAccess::private(heap);

    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut access,
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect_err("provider throw should cross the async service boundary");
    let RuntimeError::UserException(exception) = error else {
        panic!("linked public error should materialize as a caller user exception: {error}")
    };
    let request = exception.request();
    assert!(
        request.fixed_service_error().is_some(),
        "cross-service materialization must retain the fixed carrier"
    );
    assert!(
        request.local_catch_identity().is_some(),
        "the linked public package type must restore a nominal caller catch identity"
    );
    assert!(!request.correlation().trace_id.is_empty());
    assert!(!request.correlation().error_id.is_empty());
    let RuntimeValue::Heap(payload_handle) = request
        .local_value()
        .expect("linked public error must restore a local payload")
        .value()
    else {
        panic!("restored public error payload must remain a record")
    };
    let HeapNode::Object(payload) = access
        .heap_mut()
        .get(*payload_handle)
        .expect("restored public error record must remain in the caller heap")
    else {
        panic!("restored public error payload must remain an object")
    };
    let Some(RuntimeValue::Heap(messages_handle)) = payload.fields().get("messages").cloned()
    else {
        panic!("restored public error payload must retain its messages array")
    };
    let HeapNode::Array(messages) = access
        .heap_mut()
        .get(messages_handle)
        .expect("restored public error messages must remain in the caller heap")
    else {
        panic!("restored public error messages must remain an array")
    };
    assert_eq!(
        messages,
        &[RuntimeValue::String(
            "provider async typed error".to_string()
        )],
        "typed API must expose the exact restored public payload"
    );
}

#[tokio::test]
async fn typed_execution_async_stream_cancel_spawns_server_stream_from_admitted_target() {
    {
        let fixture = TypedExecutionFixture::admit_contract(
            TypedExecutionContract::returning_null(server_stream_contract(), BTreeMap::new(), true),
        )
        .await;
        let runtime = TypedExecutionRuntime::new(
            &fixture
                .eval_target
                .activation_context()
                .identity()
                .deployment
                .service_id,
        );
        let interpreter = runtime.interpreter();
        let context = runtime.context(&interpreter, &fixture.eval_target, &fixture._active);
        let heap = context.request_heap();
        let mut access = skiff_runtime_eval::heap_access::HeapAccess::private(heap);

        let result = interpreter
            .execute_runtime_assembly_addr(
                context,
                &mut access,
                &fixture.consumer_executable_addr(0),
                Vec::new(),
            )
            .await
            .expect("server stream should reach the async lane from the admitted call target");
        let RuntimeValue::Heap(stream_handle) = result else {
            panic!("server-stream consumer must return the admitted stream carrier")
        };
        let HeapNode::Object(stream_carrier) = access
            .heap_mut()
            .get(stream_handle)
            .expect("returned stream carrier must remain in the consumer heap")
        else {
            panic!("server-stream carrier must retain its canonical object shape")
        };
        assert!(matches!(
            stream_carrier.fields().get("__skiffStreamId"),
            Some(RuntimeValue::String(stream_id)) if stream_id.starts_with("stream-")
        ));
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while crate::eval_capability_adapter::concrete_stream_runtime(
                &interpreter.stream_runtime,
            )
            .active_stream_count()
                != 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("request owner must remove its concrete stream registry entry");
        assert_eq!(
            crate::eval_capability_adapter::concrete_stream_runtime(&interpreter.stream_runtime)
                .active_stream_count(),
            0,
            "request owner must clean up before the still-live root interpreter is dropped"
        );
        assert_eq!(
            crate::eval_capability_adapter::concrete_stream_runtime(&interpreter.stream_runtime)
                .active_stream_count_in_scope(
                    fixture.eval_target.request_activation().generation(),
                ),
            0,
            "the completed request generation must have no live stream entries"
        );
    }
}

#[tokio::test]
async fn typed_execution_service_stream_preserves_two_items_and_generic_substitution_full_chain() {
    let fixture =
        TypedExecutionFixture::admit_contract(TypedExecutionContract::boolean_stream()).await;
    let runtime = TypedExecutionRuntime::new(
        &fixture
            .eval_target
            .activation_context()
            .identity()
            .deployment
            .service_id,
    );
    let interpreter = runtime.interpreter();
    let context = runtime.context(&interpreter, &fixture.eval_target, &fixture._active);
    let heap = context.request_heap();

    let result = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut skiff_runtime_eval::heap_access::HeapAccess::private(heap),
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect("admitted generic Stream<T> call should consume true then false");

    assert_eq!(result, RuntimeValue::Null);
    wait_for_stream_runtime_empty(&interpreter.stream_runtime).await;
}

#[tokio::test]
async fn typed_execution_package_direct_stream_installs_exact_producer_context_full_chain() {
    let fixture =
        TypedExecutionFixture::admit_contract(TypedExecutionContract::boolean_stream()).await;
    let runtime = TypedExecutionRuntime::new(
        &fixture
            .eval_target
            .activation_context()
            .identity()
            .deployment
            .service_id,
    );
    let interpreter = runtime.interpreter();
    let context = runtime.context(&interpreter, &fixture.eval_target, &fixture._active);
    let heap = context.request_heap();

    let result = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut skiff_runtime_eval::heap_access::HeapAccess::private(heap),
            &fixture.consumer_executable_addr(1),
            Vec::new(),
        )
        .await
        .expect("exact package-direct Stream<T> producer should consume true then false");

    assert_eq!(result, RuntimeValue::Null);
    wait_for_stream_runtime_empty(&interpreter.stream_runtime).await;
}

#[tokio::test]
async fn typed_execution_service_stream_propagates_provider_error_full_chain() {
    let fixture =
        TypedExecutionFixture::admit_contract(TypedExecutionContract::boolean_stream_error()).await;
    let runtime = TypedExecutionRuntime::new(
        &fixture
            .eval_target
            .activation_context()
            .identity()
            .deployment
            .service_id,
    );
    let interpreter = runtime.interpreter();
    let context = runtime.context(&interpreter, &fixture.eval_target, &fixture._active);
    let heap = context.request_heap();
    let mut access = skiff_runtime_eval::heap_access::HeapAccess::private(heap);

    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut access,
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect_err("provider failure after its first item must terminate the consumer");

    let RuntimeError::UserException(exception) = error else {
        panic!("public provider stream failure must restore a caller user exception: {error}")
    };
    let request = exception.request();
    assert!(
        request.fixed_service_error().is_some(),
        "provider stream failure must retain its fixed cross-service carrier"
    );
    assert!(!request.correlation().trace_id.is_empty());
    assert!(!request.correlation().error_id.is_empty());
    let RuntimeValue::Heap(payload_handle) = request
        .local_value()
        .expect("linked stream error type must restore its payload")
        .value()
    else {
        panic!("restored stream error payload must be a record")
    };
    let HeapNode::Object(payload) = access
        .heap_mut()
        .get(*payload_handle)
        .expect("restored stream error record must remain in the caller heap")
    else {
        panic!("restored stream error payload must remain an object")
    };
    assert_eq!(
        payload.fields().get("message"),
        Some(&RuntimeValue::String(
            "provider stream typed error".to_string()
        ))
    );
    wait_for_stream_runtime_empty(&interpreter.stream_runtime).await;
}

#[tokio::test]
async fn typed_execution_service_stream_request_cancel_cleans_provider_and_isolates_peer() {
    let baseline = skiff_runtime_eval::provider_stream_tasks_active_for_test();
    let cancelled_fixture =
        TypedExecutionFixture::admit_contract(TypedExecutionContract::unconsumed_boolean_stream())
            .await;
    let peer_fixture =
        TypedExecutionFixture::admit_contract(TypedExecutionContract::boolean_stream()).await;
    let cancelled_runtime = TypedExecutionRuntime::new(
        &cancelled_fixture
            .eval_target
            .activation_context()
            .identity()
            .deployment
            .service_id,
    );
    let cancelled_interpreter = cancelled_runtime.interpreter();
    let cancelled_context = cancelled_runtime.context(
        &cancelled_interpreter,
        &cancelled_fixture.eval_target,
        &cancelled_fixture._active,
    );
    let cancelled_heap = cancelled_context.request_heap();
    let stream = cancelled_interpreter
        .execute_runtime_assembly_addr(
            cancelled_context,
            &mut skiff_runtime_eval::heap_access::HeapAccess::private(cancelled_heap),
            &cancelled_fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect("first admitted call should return its stream before request cancellation");
    assert!(matches!(stream, RuntimeValue::Heap(_)));

    cancelled_runtime.cancel_request();

    let peer_runtime = TypedExecutionRuntime::new(
        &peer_fixture
            .eval_target
            .activation_context()
            .identity()
            .deployment
            .service_id,
    );
    let peer_context = peer_runtime.context(
        &cancelled_interpreter,
        &peer_fixture.eval_target,
        &peer_fixture._active,
    );
    let peer_heap = peer_context.request_heap();
    let peer_result = cancelled_interpreter
        .execute_runtime_assembly_addr(
            peer_context,
            &mut skiff_runtime_eval::heap_access::HeapAccess::private(peer_heap),
            &peer_fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect("cancelling one admitted request must not affect the peer stream");
    assert_eq!(peer_result, RuntimeValue::Null);

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while skiff_runtime_eval::provider_stream_tasks_active_for_test() != baseline {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("request cancellation and peer completion must clean both provider tasks");
    wait_for_stream_runtime_empty(&cancelled_interpreter.stream_runtime).await;
}

#[tokio::test]
async fn typed_execution_service_stream_deadline_releases_provider_task_and_lease() {
    let baseline = skiff_runtime_eval::provider_stream_tasks_active_for_test();
    let fixture =
        TypedExecutionFixture::admit_contract(TypedExecutionContract::unconsumed_boolean_stream())
            .await;
    let provider = fixture.resolve_provider();
    let runtime = TypedExecutionRuntime::new(
        &fixture
            .eval_target
            .activation_context()
            .identity()
            .deployment
            .service_id,
    )
    .with_deadline(std::time::Instant::now() + std::time::Duration::from_millis(250));
    let interpreter = runtime.interpreter();
    let context = runtime.context(&interpreter, &fixture.eval_target, &fixture._active);
    let heap = context.request_heap();

    let stream = interpreter
        .execute_runtime_assembly_addr(
            context.clone(),
            &mut skiff_runtime_eval::heap_access::HeapAccess::private(heap),
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect("service call should return its stream before the request deadline");
    assert!(matches!(stream, RuntimeValue::Heap(_)));

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while skiff_runtime_eval::provider_stream_tasks_active_for_test() == baseline {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("detached provider task must start before deadline cleanup is observed");
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while skiff_runtime_eval::provider_stream_tasks_active_for_test() != baseline {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("deadline must terminate the detached provider task");

    assert!(
        provider.provider_request().open_stream().is_none(),
        "deadline must cancel the provider request and release its stream lease"
    );
    assert_eq!(
        crate::eval_capability_adapter::concrete_stream_runtime(&interpreter.stream_runtime)
            .active_stream_count(),
        1,
        "typed deadline terminal must remain registered until request-scope teardown"
    );
    drop(context);
    wait_for_stream_runtime_empty(&interpreter.stream_runtime).await;
}

#[tokio::test]
async fn typed_execution_async_stream_cancel_runtime_owner_drop_wakes_unconsumed_producer_clone() {
    let runtime = TypedExecutionRuntime::new("example.phase-four.consumer");
    let interpreter = runtime.interpreter();
    let producer_runtime = interpreter.stream_runtime.clone();
    let observer_runtime = interpreter.stream_runtime.clone();
    let (_stream, sink) = interpreter.stream_runtime.channel_stream();
    let cancel_signal = sink.cancel_signal();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let producer = tokio::spawn(async move {
        sink.send(serde_json::json!("unconsumed")).await.unwrap();
        sink.end().await;
        ready_tx.send(()).unwrap();
        cancel_signal.wait_cancelled().await;
        drop(producer_runtime);
    });
    tokio::time::timeout(std::time::Duration::from_millis(100), ready_rx)
        .await
        .expect("producer should publish a buffered item and terminal")
        .expect("producer readiness sender should stay open");

    drop(interpreter);

    tokio::time::timeout(std::time::Duration::from_millis(100), producer)
        .await
        .expect("runtime owner drop must wake the producer runtime clone")
        .expect("producer task must not panic");
    wait_for_stream_runtime_empty(&observer_runtime).await;
}

#[tokio::test]
async fn typed_execution_async_stream_cancel_projects_callback_item_before_json_and_expires_on_end()
{
    let fixture =
        TypedExecutionFixture::admit_contract(TypedExecutionContract::callback_stream()).await;
    let provider = fixture.resolve_provider();
    let provider_activation = provider.provider_activation().clone();
    assert_eq!(
        provider_activation
            .callback_capabilities()
            .active_entry_count(),
        0
    );
    let runtime = TypedExecutionRuntime::new(
        &fixture
            .eval_target
            .activation_context()
            .identity()
            .deployment
            .service_id,
    );
    let interpreter = runtime.interpreter();
    let context = runtime.context(&interpreter, &fixture.eval_target, &fixture._active);
    let heap = context.request_heap();
    let mut access = skiff_runtime_eval::heap_access::HeapAccess::private(heap);

    let result = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut access,
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect("consumer should invoke the provider-owned callback item and observe stream end");
    assert_eq!(result, RuntimeValue::Null);
    let carrier = callback_carrier_in_heap(&*access);
    assert_eq!(
        provider_activation
            .callback_capabilities()
            .active_entry_count(),
        0,
        "normal stream end must close its callback lifetime"
    );
    assert_eq!(
        provider_activation
            .callback_capabilities()
            .lookup(&carrier)
            .unwrap_err(),
        CallbackCapabilityError::CapabilityExpired
    );
    wait_for_stream_runtime_empty(&interpreter.stream_runtime).await;
}

#[tokio::test]
async fn typed_execution_async_stream_cancel_expires_callback_item_on_early_break() {
    let fixture =
        TypedExecutionFixture::admit_contract(TypedExecutionContract::callback_stream_cancel())
            .await;
    let provider = fixture.resolve_provider();
    let provider_activation = provider.provider_activation().clone();
    let runtime = TypedExecutionRuntime::new(
        &fixture
            .eval_target
            .activation_context()
            .identity()
            .deployment
            .service_id,
    );
    let interpreter = runtime.interpreter();
    let context = runtime.context(&interpreter, &fixture.eval_target, &fixture._active);
    let heap = context.request_heap();
    let mut access = skiff_runtime_eval::heap_access::HeapAccess::private(heap);

    let result = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut access,
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect("consumer should invoke one callback item before breaking the stream");
    assert_eq!(result, RuntimeValue::Null);
    let carrier = callback_carrier_in_heap(&*access);
    assert_eq!(
        provider_activation
            .callback_capabilities()
            .lookup(&carrier)
            .unwrap_err(),
        CallbackCapabilityError::CapabilityExpired,
        "early break must close the stream-scoped callback capability"
    );
    assert_eq!(
        provider_activation
            .callback_capabilities()
            .active_entry_count(),
        0
    );
    wait_for_stream_runtime_empty(&interpreter.stream_runtime).await;
}

#[tokio::test]
async fn typed_execution_async_stream_cancel_rejects_callback_item_wrong_mapping_before_owner() {
    let fixture = TypedExecutionFixture::admit_contract(
        TypedExecutionContract::callback_stream_with_operation_key("different"),
    )
    .await;
    let provider = fixture.resolve_provider();
    let provider_activation = provider.provider_activation().clone();
    let runtime = TypedExecutionRuntime::new(
        &fixture
            .eval_target
            .activation_context()
            .identity()
            .deployment
            .service_id,
    );
    let interpreter = runtime.interpreter();
    let context = runtime.context(&interpreter, &fixture.eval_target, &fixture._active);
    let heap = context.request_heap();

    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut skiff_runtime_eval::heap_access::HeapAccess::private(heap),
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect_err("callback stream mapping mismatch must fail before publication");
    let message = error.to_string();
    assert!(
        message.contains("contract operation different has no same-name local method"),
        "stream projection should preserve the stable mapping error: {error}"
    );
    assert!(
        !message.contains("CallbackProbe.invoke missing block entry"),
        "mapping mismatch must fail before owner invocation: {error}"
    );
    assert_eq!(
        provider_activation
            .callback_capabilities()
            .active_entry_count(),
        0,
        "failed projection must roll back callback registration"
    );
}

#[tokio::test]
async fn typed_execution_async_stream_cancel_rejects_callback_item_wrong_tuple_before_owner() {
    let fixture = TypedExecutionFixture::admit_contract(
        TypedExecutionContract::callback_stream_wrong_tuple(),
    )
    .await;
    let provider = fixture.resolve_provider();
    let provider_activation = provider.provider_activation().clone();
    let runtime = TypedExecutionRuntime::new(
        &fixture
            .eval_target
            .activation_context()
            .identity()
            .deployment
            .service_id,
    );
    let interpreter = runtime.interpreter();
    let context = runtime.context(&interpreter, &fixture.eval_target, &fixture._active);
    let heap = context.request_heap();

    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut skiff_runtime_eval::heap_access::HeapAccess::private(heap),
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect_err("callback stream signature mismatch must fail before publication");
    let message = error.to_string();
    assert!(
        message
            .contains("callback contract operation invoke signature does not match local method"),
        "stream projection should reject the exact callback tuple: {error}"
    );
    assert!(
        !message.contains("CallbackProbe.invoke missing block entry"),
        "tuple mismatch must fail before owner invocation: {error}"
    );
    assert_eq!(
        provider_activation
            .callback_capabilities()
            .active_entry_count(),
        0,
        "failed tuple projection must roll back callback registration"
    );
}

fn async_unary_contract() -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: Vec::new(),
        return_value: BoundaryReturn {
            ty: ContractTypeRef::builtin("void"),
            value_plan: detached_plan(BoundaryValueLifetime::Call),
        },
        stream: BoundaryStreamContract::Unary,
        callbacks: BoundaryCallbackContract::None,
        effect_guarantee: detached_effect_guarantee(),
    }
}

fn server_stream_contract() -> BoundaryOperationContract {
    let mut contract = async_unary_contract();
    contract.stream = BoundaryStreamContract::ServerStream {
        item_type: ContractTypeRef::builtin("bool"),
        item_value_plan: detached_plan(BoundaryValueLifetime::Stream),
    };
    contract
}

fn detached_plan(lifetime: BoundaryValueLifetime) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner: BoundaryValueOwner::Provider,
        lifetime,
    }
}

fn detached_effect_guarantee() -> BoundaryEffectGuarantee {
    BoundaryEffectGuarantee {
        detached_parameters: true,
        detached_return: true,
        detached_error: true,
        no_caller_reachable_mutation: true,
        no_caller_value_escape: true,
        no_same_heap_identity: true,
    }
}

fn callback_carrier_in_heap(heap: &RequestHeap) -> CallbackCapabilityCarrier {
    for index in 0..heap.len() {
        let Ok(index) = u32::try_from(index) else {
            break;
        };
        let Ok(HeapNode::Interface(interface)) = heap.get(HeapHandle::new(index, 0)) else {
            continue;
        };
        if let InterfaceCarrier::CallbackCapability(carrier) = interface.carrier() {
            return carrier.clone();
        }
    }
    panic!("consumer heap should retain the projected opaque callback carrier")
}

async fn wait_for_stream_runtime_empty(runtime: &skiff_runtime_eval::capabilities::StreamRuntime) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while crate::eval_capability_adapter::concrete_stream_runtime(runtime).active_stream_count()
            != 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("stream terminal must remove its concrete registry entry");
}
