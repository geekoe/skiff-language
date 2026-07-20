use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryErrorContract, BoundaryFeatureUnavailableReason, BoundaryOperationContract,
    BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding,
    BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan, ContractTypeRef,
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
    artifacts::{ProjectedFixture, TypedExecutionContract},
    runtime::TypedExecutionRuntime,
    scenario::TypedExecutionFixture,
};

#[tokio::test]
async fn typed_execution_async_stream_cancel_reaches_owned_provider_future_full_chain() {
    let fixture = TypedExecutionFixture::admit_contract(TypedExecutionContract::returning_null(
        async_unary_contract(),
        BTreeMap::new(),
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
    let context = runtime.context(&interpreter, &fixture.eval_target);
    let generation = context
        .runtime_assembly_target()
        .unwrap()
        .request_activation()
        .generation();
    let mut heap = context.request_heap();

    let result = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
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
async fn typed_execution_async_stream_cancel_detaches_declared_typed_error_with_shared_planner() {
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
    let context = runtime.context(&interpreter, &fixture.eval_target);
    let mut heap = context.request_heap();

    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect_err("declared provider throw should cross the async service boundary");
    let RuntimeError::UserException(exception) = error else {
        panic!("declared async typed error should retain its user-exception class: {error}")
    };
    assert_eq!(
        exception.error_payload().unwrap().get("messages"),
        Some(&serde_json::json!(["provider async typed error"])),
        "shared planner must materialize the declared payload shape into the caller error"
    );
}

#[tokio::test]
async fn typed_execution_async_stream_cancel_spawns_server_stream_from_admitted_target() {
    {
        let fixture = TypedExecutionFixture::admit_contract(
            TypedExecutionContract::returning_null(server_stream_contract(), BTreeMap::new()),
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
        let context = runtime.context(&interpreter, &fixture.eval_target);
        let mut heap = context.request_heap();

        let result = interpreter
            .execute_runtime_assembly_addr(
                context,
                &mut heap,
                &fixture.consumer_executable_addr(0),
                Vec::new(),
            )
            .await
            .expect("server stream should reach the async lane from the admitted call target");
        let RuntimeValue::Heap(stream_handle) = result else {
            panic!("server-stream consumer must return the admitted stream carrier")
        };
        let HeapNode::Object(stream_carrier) = heap
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
    let context = runtime.context(&interpreter, &fixture.eval_target);
    let mut heap = context.request_heap();

    let result = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect("consumer should invoke the provider-owned callback item and observe stream end");
    assert_eq!(result, RuntimeValue::Null);
    let carrier = callback_carrier_in_heap(&heap);
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
    let context = runtime.context(&interpreter, &fixture.eval_target);
    let mut heap = context.request_heap();

    let result = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect("consumer should invoke one callback item before breaking the stream");
    assert_eq!(result, RuntimeValue::Null);
    let carrier = callback_carrier_in_heap(&heap);
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
    let context = runtime.context(&interpreter, &fixture.eval_target);
    let mut heap = context.request_heap();

    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
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
    let context = runtime.context(&interpreter, &fixture.eval_target);
    let mut heap = context.request_heap();

    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
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

#[test]
fn typed_execution_async_stream_cancel_rejects_unsupported_descriptor_before_provider() {
    let mut contract = async_unary_contract();
    contract.cancellation = BoundaryCancellationContract::Unsupported {
        reason: BoundaryFeatureUnavailableReason::UnknownSemantics,
    };
    let rejected = std::panic::catch_unwind(|| {
        ProjectedFixture::new(TypedExecutionContract::returning_null(
            contract,
            BTreeMap::new(),
        ))
    });
    assert!(
        rejected.is_err(),
        "unsupported cancellation descriptor must fail during typed projection"
    );
}

fn async_unary_contract() -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: Vec::new(),
        return_value: BoundaryReturn {
            ty: ContractTypeRef::builtin("void"),
            value_plan: detached_plan(BoundaryValueLifetime::Call),
        },
        errors: BoundaryErrorContract::None,
        stream: BoundaryStreamContract::Unary,
        cancellation: BoundaryCancellationContract::Cooperative,
        callbacks: BoundaryCallbackContract::None,
        may_suspend: true,
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
