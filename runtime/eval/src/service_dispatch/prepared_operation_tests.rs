#[path = "prepared_operation_tests/fixture.rs"]
mod fixture;

use serde_json::json;
use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_capability_context::{OutboundResponse, ResponseError};
use skiff_runtime_linked_program::{
    CallIr, ExecutableAddr, LinkedCallTarget, ServiceDependencySymbolRef,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::RuntimeValue,
    type_plan::RuntimeTypePlan,
};

use super::*;
use fixture::*;

#[tokio::test]
async fn outbound_buffered_response_wait_is_static_and_starts_once() {
    let plan = string_plan();
    let api = RecordingOutbound::buffered(OutboundResponse::End {
        payload: encode_response(&plan, &json!("ready")),
    });
    let mut heap = RequestHeap::default();
    let env = Env::new();
    let stream_runtime = crate::actor_executor_test_runtime::runtime_factory().stream_runtime();

    let PreparedOutboundServiceCall::ExternalWait(operation) =
        prepare(&api, "unary", plan, &mut heap, &env, &stream_runtime)
            .expect("prepare should succeed")
    else {
        panic!("unary call must expose an external wait");
    };
    assert_eq!(api.starts(), 1);
    let wait = operation.into_wait();
    assert_heap_free_wait(&wait);
    let completion = wait.await;
    assert_eq!(api.starts(), 1);
    let value = completion
        .finalize(&mut heap, &env)
        .expect("buffered response should finalize");
    assert_eq!(value, RuntimeValue::String("ready".to_string()));
    assert_eq!(api.state.registry.pending_count(), 0);
    assert_eq!(api.state.registry.active_lease_count(), 0);
    assert!(api.cancels().is_empty());
}

#[test]
fn dependency_and_remote_interface_entries_share_the_prepared_owner_contract() {
    let interpreter = outbound_interpreter();
    let caller_addr = ExecutableAddr::service(0, 0);
    let (dependency, operation) = dependency_fixture();
    let symbol = ServiceDependencySymbolRef {
        dependency_ref: dependency.alias.clone(),
        operation: operation.clone(),
    };
    let call = CallIr {
        target: LinkedCallTarget::ServiceDependencySymbol {
            symbol: symbol.clone(),
        },
        site: InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
        },
        args: Vec::new(),
        type_args: Default::default(),
        metadata: Default::default(),
        actor_metadata: None,
    };
    let env = Env::new();

    let dependency_api = RecordingOutbound::pending_with_dependencies(vec![dependency.clone()]);
    let dependency_context = OutboundServiceContext::new(dependency_api.clone());
    let mut dependency_heap = RequestHeap::default();
    let dependency_prepared = prepare_outbound_service(
        &interpreter,
        &dependency_context,
        &interpreter.stream_runtime,
        &mut dependency_heap,
        &env,
        &caller_addr,
        &call,
        &symbol,
        Vec::new(),
    )
    .expect("ordinary dependency should prepare");
    assert!(matches!(
        dependency_prepared,
        PreparedOutboundServiceCall::ExternalWait(_)
    ));
    assert_eq!(dependency_api.starts(), 1);

    let remote_api = RecordingOutbound::pending_with_dependencies(vec![dependency]);
    let remote_context = OutboundServiceContext::new(remote_api.clone());
    let mut remote_heap = RequestHeap::default();
    let remote_prepared = prepare_outbound_service_operation(
        &interpreter,
        &remote_context,
        &interpreter.stream_runtime,
        &mut remote_heap,
        &env,
        &caller_addr,
        "provider",
        &operation.operation_abi_id,
        Vec::new(),
    )
    .expect("remote interface operation should prepare");
    assert!(matches!(
        remote_prepared,
        PreparedOutboundServiceCall::ExternalWait(_)
    ));
    assert_eq!(remote_api.starts(), 1);
}

#[tokio::test]
async fn outbound_pending_wait_does_not_borrow_or_write_the_caller_heap() {
    let plan = string_plan();
    let api = RecordingOutbound::pending();
    let mut heap = RequestHeap::default();
    let env = Env::new();
    let stream_runtime = crate::actor_executor_test_runtime::runtime_factory().stream_runtime();
    let PreparedOutboundServiceCall::ExternalWait(operation) = prepare(
        &api,
        "unary",
        plan.clone(),
        &mut heap,
        &env,
        &stream_runtime,
    )
    .expect("prepare should succeed") else {
        panic!("unary call must expose an external wait");
    };
    let mut wait = Box::pin(operation.into_wait());
    assert!(poll_once(wait.as_mut()).await.is_none());

    heap.alloc_bytes(vec![1, 2, 3])
        .expect("caller heap must remain independently mutable");
    let before_finalize = heap.stats();
    assert!(api.send(OutboundResponse::End {
        payload: encode_response(&plan, &json!("pending")),
    }));
    let completion = wait.await;
    assert_eq!(
        heap.stats(),
        before_finalize,
        "owned wait must not write the caller heap"
    );
    let value = completion
        .finalize(&mut heap, &env)
        .expect("pending response should finalize");
    assert_eq!(value, RuntimeValue::String("pending".to_string()));
    assert_eq!(heap.stats(), before_finalize);
    assert_eq!(api.starts(), 1);
}

#[tokio::test]
async fn outbound_unary_error_and_drop_settle_the_lease_exactly_once() {
    let plan = string_plan();
    let api = RecordingOutbound::buffered(OutboundResponse::Start {
        http_response: skiff_runtime_capability_context::HttpResponseMetadata::new(200, Vec::new()),
    });
    let mut heap = RequestHeap::default();
    let env = Env::new();
    let stream_runtime = crate::actor_executor_test_runtime::runtime_factory().stream_runtime();
    let PreparedOutboundServiceCall::ExternalWait(operation) = prepare(
        &api,
        "unary",
        plan.clone(),
        &mut heap,
        &env,
        &stream_runtime,
    )
    .expect("prepare should succeed") else {
        panic!("unary call must expose an external wait");
    };
    let completion = operation.into_wait().await;
    assert!(matches!(
        completion.finalize(&mut heap, &env),
        Err(RuntimeError::ProviderUnavailable { .. })
    ));
    assert_eq!(api.cancels().len(), 1);
    assert_eq!(api.cancels()[0].1, "unexpected_stream_response");
    assert_eq!(api.state.registry.pending_count(), 0);
    assert_eq!(api.state.registry.active_lease_count(), 0);

    let terminal_api = RecordingOutbound::buffered(OutboundResponse::Error(ResponseError {
        code: "provider.error".to_string(),
        message: "fixed terminal".to_string(),
        status: Some(500),
        details: None,
    }));
    let PreparedOutboundServiceCall::ExternalWait(operation) = prepare(
        &terminal_api,
        "unary",
        plan.clone(),
        &mut heap,
        &env,
        &stream_runtime,
    )
    .expect("prepare should succeed") else {
        panic!("unary call must expose an external wait");
    };
    let completion = operation.into_wait().await;
    assert!(matches!(
        completion.finalize(&mut heap, &env),
        Err(RuntimeError::Protocol { .. })
    ));
    assert!(
        terminal_api.cancels().is_empty(),
        "terminal response errors complete rather than cancel the lease"
    );
    assert_eq!(terminal_api.state.registry.pending_count(), 0);
    assert_eq!(terminal_api.state.registry.active_lease_count(), 0);

    let dropped_api = RecordingOutbound::pending();
    let PreparedOutboundServiceCall::ExternalWait(operation) = prepare(
        &dropped_api,
        "unary",
        plan,
        &mut heap,
        &env,
        &stream_runtime,
    )
    .expect("prepare should succeed") else {
        panic!("unary call must expose an external wait");
    };
    let mut wait = Box::pin(operation.into_wait());
    assert!(poll_once(wait.as_mut()).await.is_none());
    let before_drop = heap.stats();
    drop(wait);
    assert_eq!(
        dropped_api.cancels(),
        vec![(
            "prepared-outbound-1".to_string(),
            "unary_wait_dropped".to_string()
        )]
    );
    assert_eq!(dropped_api.state.registry.pending_count(), 0);
    assert_eq!(dropped_api.state.registry.active_lease_count(), 0);
    assert!(
        !dropped_api.send(OutboundResponse::End {
            payload: Vec::new()
        }),
        "late response must be isolated after the receiver is dropped"
    );
    assert_eq!(heap.stats(), before_drop);
}

#[test]
fn outbound_server_stream_setup_is_a_synchronous_ready_step() {
    let plan = string_plan();
    let api = RecordingOutbound::pending();
    let mut heap = RequestHeap::default();
    let env = Env::new();
    let stream_runtime = pull_setup_runtime();

    let prepared = prepare(&api, "serverStream", plan, &mut heap, &env, &stream_runtime)
        .expect("stream setup should succeed synchronously");
    assert!(matches!(prepared, PreparedOutboundServiceCall::Ready(_)));
    assert_eq!(api.starts(), 1);
    assert_eq!(api.state.registry.pending_count(), 1);
    assert_eq!(api.state.registry.active_lease_count(), 1);
}

#[tokio::test]
async fn outbound_finalize_heap_failure_rolls_back_partial_decode() {
    let plan = RuntimeTypePlan::json_value_plan();
    let api = RecordingOutbound::buffered(OutboundResponse::End {
        payload: encode_response(&plan, &json!([[1]])),
    });
    let mut heap = RequestHeap::new(RequestHeapLimits {
        max_nodes: 2,
        ..RequestHeapLimits::default()
    });
    let env = Env::new();
    let stream_runtime = crate::actor_executor_test_runtime::runtime_factory().stream_runtime();
    let PreparedOutboundServiceCall::ExternalWait(operation) =
        prepare(&api, "unary", plan, &mut heap, &env, &stream_runtime)
            .expect("request prepare should fit the heap")
    else {
        panic!("unary call must expose an external wait");
    };
    let completion = operation.into_wait().await;
    let before = heap.stats();
    let before_len = heap.len();
    assert!(
        completion.finalize(&mut heap, &env).is_err(),
        "nested response should exceed the remaining heap node budget"
    );
    assert_eq!(heap.stats(), before);
    assert_eq!(heap.len(), before_len);
    assert_eq!(api.starts(), 1);
}
