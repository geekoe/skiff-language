use crate::heap_access::HeapAccess;
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryOperationDescriptor, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractOperationId, ContractTypeRef,
};
use skiff_runtime_boundary::service_linkable::FailClosedServiceLinkableCapabilityHooks;
use skiff_runtime_linked_program::{LinkedCallTarget, LinkedExprIr};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{HeapNode, RuntimeValue},
    service_error::{InternalErrorPayload, OpaqueServiceError, ServiceErrorEnvelope},
};

use crate::{
    assembly_execution::{
        ordinary::tests::{
            service_error_consumer::{
                ConsumerTopology, ProviderFailureKind, ServiceErrorConsumerFixture,
            },
            test_runtime,
        },
        service_error_channel::{
            start_restricted_service_diagnostic_probe_for_test,
            take_restricted_service_diagnostics_for_test,
        },
        RuntimeAssemblyExecutionProjection,
    },
    env::Env,
    error::RuntimeError,
    eval_context::EvalContext,
    Interpreter, RuntimeAssemblyEvalTarget, RuntimeAssemblyServiceCallTarget,
};

use super::*;

fn assert_owned_wait<F>(future: F) -> F
where
    F: Future + Send + 'static,
{
    future
}

fn resolved_error_call(
    fixture: &ServiceErrorConsumerFixture,
    receiver_target: &RuntimeAssemblyEvalTarget,
) -> (
    RuntimeAssemblyExecutionProjection,
    skiff_runtime_linked_program::CallIr,
    RuntimeAssemblyServiceCallTarget,
) {
    let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(
        receiver_target.execution_image(),
    ));
    let caller = projection
        .resolve_executable(fixture.caller_addr())
        .expect("linked caller executable");
    let call = caller
        .executable
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            LinkedExprIr::Call { call }
                if matches!(
                    call.target,
                    LinkedCallTarget::ActivationRelativeService { .. }
                ) =>
            {
                Some(call.clone())
            }
            _ => None,
        })
        .expect("linked service call");
    let instruction = match &call.target {
        LinkedCallTarget::ActivationRelativeService { instruction } => instruction,
        _ => unreachable!("selected call is activation-relative"),
    };
    let target = receiver_target
        .resolve_service_call(instruction)
        .expect("resolved provider target");
    (projection, call, target)
}

#[tokio::test]
async fn prepared_provider_unary_wait_does_not_borrow_caller_heap_or_env() {
    let fixture = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        true,
        false,
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let receiver_target = fixture.caller_eval_target();
    let (projection, call, target) = resolved_error_call(&fixture, &receiver_target);
    let caller = projection
        .resolve_executable(fixture.caller_addr())
        .expect("linked caller executable");
    let context = fixture.execution_context(&interpreter, receiver_target);
    let mut caller_heap = RequestHeap::default();
    let mut caller_access = HeapAccess::Exclusive(&mut caller_heap);
    let mut caller_env = Env::new();
    let mut eval = EvalContext::new(
        &interpreter,
        context,
        &mut caller_access,
        &mut caller_env,
        &caller.addr,
        caller.file.as_ref(),
        caller.executable,
    )
    .expect("caller eval context");

    let prepared = prepare_provider_unary(&mut eval, &call, target, Vec::new())
        .expect("provider unary prepares synchronously");
    let wait = assert_owned_wait(prepared.wait());

    let caller_value = eval
        .heap
        .alloc_array(vec![RuntimeValue::String("caller-still-owned".to_string())])
        .expect("caller heap remains independently mutable");
    eval.env.current_module = Some("caller.changed.after.prepare".to_string());
    assert_eq!(eval.heap.len(), 1);
    assert!(matches!(
        eval.heap.get(caller_value),
        Ok(HeapNode::Array(items))
            if items == &[RuntimeValue::String("caller-still-owned".to_string())]
    ));
    assert_eq!(
        eval.env.current_module.as_deref(),
        Some("caller.changed.after.prepare")
    );

    drop(wait);
}

#[tokio::test]
async fn provider_user_error_stays_owned_until_finalize_and_exports_once() {
    let fixture = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        true,
        false,
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let receiver_target = fixture.caller_eval_target();
    let generation = receiver_target.request_activation().generation();
    let (projection, call, target) = resolved_error_call(&fixture, &receiver_target);
    let caller = projection
        .resolve_executable(fixture.caller_addr())
        .expect("linked caller executable");
    let context = fixture.execution_context(&interpreter, receiver_target);
    let mut caller_heap = RequestHeap::default();
    let sentinel = caller_heap
        .alloc_array(vec![RuntimeValue::String("caller".to_string())])
        .expect("caller sentinel");
    let mut caller_env = Env::new();
    let mut caller_access = HeapAccess::Exclusive(&mut caller_heap);
    let mut eval = EvalContext::new(
        &interpreter,
        context,
        &mut caller_access,
        &mut caller_env,
        &caller.addr,
        caller.file.as_ref(),
        caller.executable,
    )
    .expect("caller eval context");
    start_restricted_service_diagnostic_probe_for_test(generation);

    let prepared = prepare_provider_unary(&mut eval, &call, target, Vec::new())
        .expect("provider unary prepares synchronously");
    assert!(
        !prepared.provider_context_has_actor_frame(),
        "owned provider context must not retain the caller Actor frame"
    );
    let caller_checkpoint = eval.heap.checkpoint();
    let caller_stats = eval.heap.stats();
    let completed = assert_owned_wait(prepared.wait()).await;
    assert_eq!(
        eval.heap.checkpoint(),
        caller_checkpoint,
        "owned provider wait must not write the caller heap"
    );
    assert_eq!(eval.heap.stats(), caller_stats);

    let error = completed
        .finalize(eval.heap)
        .expect_err("provider user error must become a fixed service failure");
    assert!(matches!(error, RuntimeError::FixedServiceFailure(_)));
    assert_eq!(
        eval.heap.checkpoint(),
        caller_checkpoint,
        "fixed failure export must not partially import into the caller heap"
    );
    assert!(matches!(
        eval.heap.get(sentinel),
        Ok(HeapNode::Array(items)) if items == &[RuntimeValue::String("caller".to_string())]
    ));
    assert_eq!(
        take_restricted_service_diagnostics_for_test(generation).len(),
        1,
        "provider failure diagnostic must be submitted exactly once during finalize"
    );
}

#[test]
fn provider_normal_and_fixed_outcomes_are_deferred_to_finalize() {
    let normal_fixture = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        true,
        false,
    );
    let normal_interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let normal_receiver = normal_fixture.caller_eval_target();
    let (normal_projection, normal_call, normal_target) =
        resolved_error_call(&normal_fixture, &normal_receiver);
    let normal_caller = normal_projection
        .resolve_executable(normal_fixture.caller_addr())
        .expect("linked caller executable");
    let normal_context = normal_fixture.execution_context(&normal_interpreter, normal_receiver);
    let mut normal_heap = RequestHeap::default();
    let mut normal_access = HeapAccess::Exclusive(&mut normal_heap);
    let mut normal_env = Env::new();
    let mut normal_eval = EvalContext::new(
        &normal_interpreter,
        normal_context,
        &mut normal_access,
        &mut normal_env,
        &normal_caller.addr,
        normal_caller.file.as_ref(),
        normal_caller.executable,
    )
    .expect("normal caller eval context");
    let normal = prepare_provider_unary(&mut normal_eval, &normal_call, normal_target, Vec::new())
        .expect("normal provider prepares")
        .complete_for_test(Ok(RuntimeValue::String("provider-result".to_string())));
    assert!(normal_eval.heap.is_empty());
    assert_eq!(
        normal
            .finalize(normal_eval.heap)
            .expect("normal provider result materializes during finalize"),
        RuntimeValue::String("provider-result".to_string())
    );

    let fixed_fixture = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        true,
        false,
    );
    let fixed_interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let fixed_receiver = fixed_fixture.caller_eval_target();
    let fixed_generation = fixed_receiver.request_activation().generation();
    let (fixed_projection, fixed_call, fixed_target) =
        resolved_error_call(&fixed_fixture, &fixed_receiver);
    let fixed_caller = fixed_projection
        .resolve_executable(fixed_fixture.caller_addr())
        .expect("linked caller executable");
    let fixed_context = fixed_fixture.execution_context(&fixed_interpreter, fixed_receiver);
    let mut fixed_heap = RequestHeap::default();
    let mut fixed_access = HeapAccess::Exclusive(&mut fixed_heap);
    let mut fixed_env = Env::new();
    let mut fixed_eval = EvalContext::new(
        &fixed_interpreter,
        fixed_context,
        &mut fixed_access,
        &mut fixed_env,
        &fixed_caller.addr,
        fixed_caller.file.as_ref(),
        fixed_caller.executable,
    )
    .expect("fixed caller eval context");
    let fixed = fixed_service_error();
    let completed = prepare_provider_unary(&mut fixed_eval, &fixed_call, fixed_target, Vec::new())
        .expect("fixed provider prepares")
        .complete_for_test(Err(RuntimeError::FixedServiceFailure(fixed.clone())));
    assert!(fixed_eval.heap.is_empty());
    start_restricted_service_diagnostic_probe_for_test(fixed_generation);
    let error = completed
        .finalize(fixed_eval.heap)
        .expect_err("fixed provider result stays on the error path");
    assert!(matches!(
        error,
        RuntimeError::FixedServiceFailure(actual) if actual == fixed
    ));
    assert!(fixed_eval.heap.is_empty());
    assert!(
        take_restricted_service_diagnostics_for_test(fixed_generation).is_empty(),
        "an already-fixed failure must not submit a second provider diagnostic"
    );
}

#[test]
fn dropping_unpolled_provider_wait_cancels_the_provider_request() {
    let fixture = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        true,
        false,
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let receiver_target = fixture.caller_eval_target();
    let (projection, call, target) = resolved_error_call(&fixture, &receiver_target);
    let provider_request = target.provider_request().clone();
    let caller = projection
        .resolve_executable(fixture.caller_addr())
        .expect("linked caller executable");
    let context = fixture.execution_context(&interpreter, receiver_target);
    let mut caller_heap = RequestHeap::default();
    let mut caller_access = HeapAccess::Exclusive(&mut caller_heap);
    let mut caller_env = Env::new();
    let mut eval = EvalContext::new(
        &interpreter,
        context,
        &mut caller_access,
        &mut caller_env,
        &caller.addr,
        caller.file.as_ref(),
        caller.executable,
    )
    .expect("caller eval context");

    let prepared = prepare_provider_unary(&mut eval, &call, target, Vec::new())
        .expect("provider unary prepares");
    let wait = assert_owned_wait(prepared.wait());
    assert!(
        provider_request.open_stream().is_some(),
        "provider request is live before the owned wait is dropped"
    );
    drop(wait);
    assert!(
        provider_request.open_stream().is_none(),
        "dropping the owned wait must cancel its provider request"
    );
}

#[tokio::test]
async fn cancelling_provider_wait_drops_the_in_flight_future_once_and_rejects_late_result() {
    struct DropProbe(Arc<AtomicUsize>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    let execution = test_runtime::execution_control();
    let cancellation = execution.cancellation_token();
    let fixture = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        true,
        false,
    );
    let receiver_target = fixture.caller_eval_target();
    let (_, _, target) = resolved_error_call(&fixture, &receiver_target);
    let request = target.provider_request().clone();
    let drops = Arc::new(AtomicUsize::new(0));
    let (started, provider_started) = tokio::sync::oneshot::channel::<()>();
    let (complete, completed) = tokio::sync::oneshot::channel::<RuntimeValue>();
    let waiter = tokio::spawn({
        let execution = execution.clone();
        let request = request.clone();
        let drops = Arc::clone(&drops);
        async move {
            await_provider_unary(&execution, &request, async move {
                let _probe = DropProbe(drops);
                started.send(()).expect("provider start observer");
                Ok(completed.await.expect("provider completion"))
            })
            .await
            .into_result()
        }
    });
    provider_started
        .await
        .expect("provider future must start exactly once");
    cancellation.cancel();
    let error = waiter
        .await
        .expect("provider waiter must not panic")
        .expect_err("caller cancellation must win");
    assert!(error.is_cancelled());
    assert_eq!(
        drops.load(Ordering::Acquire),
        1,
        "the in-flight provider future must be dropped exactly once"
    );
    assert!(
        complete.send(RuntimeValue::Bool(true)).is_err(),
        "late provider completion must not re-enter caller finalization"
    );
    assert!(request.open_stream().is_none());
}

#[test]
fn provider_result_materialization_rolls_back_partial_caller_import() {
    let descriptor = BoundaryOperationDescriptor {
        operation_id: ContractOperationId::new("operation:o3-atomic-result"),
        stable_key: "o3AtomicResult".to_string(),
        contract: BoundaryOperationContract {
            parameters: Vec::new(),
            return_value: BoundaryReturn {
                ty: ContractTypeRef::Builtin {
                    name: "Array".to_string(),
                    arguments: vec![ContractTypeRef::Builtin {
                        name: "Array".to_string(),
                        arguments: vec![ContractTypeRef::builtin("string")],
                    }],
                },
                value_plan: detached_call_plan(BoundaryValueOwner::Provider),
            },
            stream: BoundaryStreamContract::Unary,
            callbacks: BoundaryCallbackContract::None,
            effect_guarantee: BoundaryEffectGuarantee {
                detached_parameters: true,
                detached_return: true,
                detached_error: true,
                no_caller_reachable_mutation: true,
                no_caller_value_escape: true,
                no_same_heap_identity: true,
            },
        },
    };
    let schema = BTreeMap::new();
    let boundary = CanonicalServiceBoundaryPlan::new(&descriptor, &schema, 0)
        .expect("canonical provider result plan");
    let mut provider_heap = RequestHeap::default();
    let child = provider_heap
        .alloc_array(vec![RuntimeValue::String("provider".to_string())])
        .expect("provider child");
    let root = provider_heap
        .alloc_array(vec![RuntimeValue::Heap(child)])
        .expect("provider root");
    let mut caller_heap = RequestHeap::new(RequestHeapLimits {
        max_nodes: 2,
        ..RequestHeapLimits::default()
    });
    caller_heap
        .alloc_array(vec![RuntimeValue::String("sentinel".to_string())])
        .expect("caller sentinel");
    let checkpoint = caller_heap.checkpoint();
    let stats = caller_heap.stats();

    boundary
        .materialize_provider_result(
            Ok(RuntimeValue::Heap(root)),
            &mut provider_heap,
            &mut caller_heap,
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .expect_err("nested result must exhaust the remaining caller node");
    assert_eq!(caller_heap.checkpoint(), checkpoint);
    assert_eq!(caller_heap.stats(), stats);
}

fn detached_call_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

fn fixed_service_error() -> OpaqueServiceError {
    let envelope = ServiceErrorEnvelope::InternalError {
        payload: InternalErrorPayload {
            message: "Internal service error".to_string(),
            trace_id: "trace:o3-fixed".to_string(),
            error_id: "error:o3-fixed".to_string(),
        },
    };
    OpaqueServiceError::decode(
        skiff_canonical_json::canonical_json_bytes(&envelope).expect("canonical fixed error"),
    )
    .expect("opaque fixed error")
}
