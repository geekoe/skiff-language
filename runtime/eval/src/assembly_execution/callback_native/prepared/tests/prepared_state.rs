use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use skiff_artifact_model::{
    ContractTypeRef, InstructionSourceSite, SyntheticInstructionSiteReason,
};
use skiff_runtime_linked_program::{FileAddr, ServiceMeta, UnitAddr};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{
        CallbackCapabilityCarrier, HeapHandle, HeapNode, RuntimeObject, RuntimeObjectFields,
        RuntimeValue, RuntimeValueCarrier,
    },
    service_error::{
        ErrorCorrelation, ExceptionStackFrame, OpaqueServiceError, PlatformBuiltinErrorIdentity,
        RequestException, RequestExceptionCause, ServiceErrorEnvelope,
    },
};
use tokio::sync::{oneshot, Mutex};

use super::*;
use crate::{
    assembly_execution::ordinary::tests::test_runtime,
    capabilities::TimeCapabilityContext,
    env::Env,
    error::RuntimeError,
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
};

fn owner_wait() -> (Arc<Mutex<RequestHeap>>, CallbackOwnerWait) {
    let owner_heap = Arc::new(Mutex::new(RequestHeap::default()));
    let guard = Arc::clone(&owner_heap)
        .try_lock_owned()
        .expect("fresh owner heap should lock");
    (owner_heap, CallbackOwnerWait::new(guard))
}

fn completed(
    owner: CallbackOwnerWaitOutcome,
    return_type: ContractTypeRef,
) -> CompletedCallbackInvocation {
    CompletedCallbackInvocation {
        owner,
        return_type,
        package_schema_records: BTreeMap::new(),
    }
}

fn callback_user_exception(payload: RuntimeValue) -> RuntimeError {
    let site = InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    };
    RuntimeError::UserException(crate::error::UserException::new(
        RequestException::local(
            RuntimeValueCarrier::identified(
                payload,
                PlatformBuiltinErrorIdentity::Http.catch_identity(),
            ),
            site.clone(),
            vec![ExceptionStackFrame::Local { site }],
            ErrorCorrelation {
                trace_id: "callback-cross-heap-trace".to_string(),
                error_id: "callback-cross-heap-error".to_string(),
            },
        )
        .expect("test callback exception should be valid"),
    ))
}

fn owner_nested_exception_graph(heap: &mut RequestHeap) -> RuntimeError {
    let leaf = heap
        .alloc_array(vec![RuntimeValue::String("owner-leaf".to_string())])
        .expect("owner leaf should allocate");
    let imported_local = heap
        .alloc_array(vec![RuntimeValue::String("imported-local".to_string())])
        .expect("imported local payload should allocate");
    let envelope = ServiceErrorEnvelope::PlatformError {
        builtin_error_identity: PlatformBuiltinErrorIdentity::DbConflict,
        encoded_payload: br#"{"retryable":true}"#.to_vec(),
        trace_id: "callback-imported-trace".to_string(),
        error_id: "callback-imported-error".to_string(),
    };
    let opaque = OpaqueServiceError::decode(
        serde_json::to_vec(&envelope).expect("test service envelope should encode"),
    )
    .expect("test service envelope should decode strictly");
    let imported = RequestException::imported(
        opaque,
        Some(RuntimeValueCarrier::identified(
            RuntimeValue::Heap(imported_local),
            PlatformBuiltinErrorIdentity::DbConflict.catch_identity(),
        )),
        InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
        },
        Vec::new(),
    )
    .expect("imported exception should retain its linked local projection");
    let imported = heap
        .alloc_exception(imported)
        .expect("nested imported exception should allocate");
    let object = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
            ("nested".to_string(), RuntimeValue::Heap(leaf)),
            ("imported".to_string(), RuntimeValue::Heap(imported)),
            (
                "ownerMarker".to_string(),
                RuntimeValue::String("owner-object".to_string()),
            ),
        ])))
        .expect("owner object should allocate");
    let root = heap
        .alloc_array(vec![
            RuntimeValue::Heap(object),
            RuntimeValue::String("owner-root".to_string()),
        ])
        .expect("owner root should allocate");
    callback_user_exception(RuntimeValue::Heap(root))
}

fn assert_caller_local_nested_exception(error: RuntimeError, caller_heap: &RequestHeap) {
    let RuntimeError::UserException(exception) = error else {
        panic!("callback should preserve the typed user-exception terminal")
    };
    assert_eq!(
        exception.actual_payload_type(),
        Some(&PlatformBuiltinErrorIdentity::Http.catch_identity())
    );
    let Some(_) = exception.request().local_value() else {
        panic!("typed callback exception should retain its local payload")
    };
    let RuntimeValue::Heap(root) = exception
        .request()
        .local_value()
        .expect("typed callback exception payload")
        .value()
    else {
        panic!("typed callback exception payload should remain heap-backed")
    };
    let HeapNode::Array(root_items) = caller_heap
        .get(*root)
        .expect("exception root must resolve in caller heap")
    else {
        panic!("exception root should remain an array")
    };
    assert_eq!(
        root_items.get(1),
        Some(&RuntimeValue::String("owner-root".to_string()))
    );
    let Some(RuntimeValue::Heap(object)) = root_items.first() else {
        panic!("exception root should retain its nested object")
    };
    let HeapNode::Object(object) = caller_heap
        .get(*object)
        .expect("nested object must resolve in caller heap")
    else {
        panic!("nested exception value should remain an object")
    };
    assert_eq!(
        object.fields().get("ownerMarker"),
        Some(&RuntimeValue::String("owner-object".to_string()))
    );
    let Some(RuntimeValue::Heap(leaf)) = object.fields().get("nested") else {
        panic!("nested object should retain its leaf handle")
    };
    assert!(matches!(
        caller_heap.get(*leaf),
        Ok(HeapNode::Array(items))
            if items == &[RuntimeValue::String("owner-leaf".to_string())]
    ));
    let Some(RuntimeValue::Heap(imported)) = object.fields().get("imported") else {
        panic!("nested object should retain its imported exception")
    };
    let HeapNode::Exception(imported) = caller_heap
        .get(*imported)
        .expect("nested imported exception must resolve in caller heap")
    else {
        panic!("nested imported exception should remain an exception node")
    };
    let RequestExceptionCause::OpaqueService {
        local_value: Some(local_value),
        ..
    } = imported.cause()
    else {
        panic!("nested exception should preserve its imported local projection")
    };
    assert_eq!(
        local_value.catch_identity(),
        Some(&PlatformBuiltinErrorIdentity::DbConflict.catch_identity())
    );
    let RuntimeValue::Heap(imported_local) = local_value.value() else {
        panic!("imported local projection should remain heap-backed")
    };
    assert!(matches!(
        caller_heap.get(*imported_local),
        Ok(HeapNode::Array(items))
            if items == &[RuntimeValue::String("imported-local".to_string())]
    ));
}

fn program_context(interpreter: &Interpreter) -> ProgramExecutionContext<'static> {
    let execution = test_runtime::execution_control();
    let effects = test_runtime::effects_context();
    let actor = test_runtime::actor_context();
    let request = test_runtime::request_context();
    ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: test_runtime::config_context(),
        db: skiff_runtime_capability_context::DbCapabilityContext::unavailable(),
        file: test_runtime::file_context(),
        file_source_stream: test_runtime::file_source_stream_context(
            interpreter.stream_runtime.clone(),
        ),
        time: TimeCapabilityContext::new(execution),
        websocket: test_runtime::websocket_context(),
        effects: effects.clone(),
        http_client: effects.http_client_context(
            interpreter.http_options.clone(),
            interpreter.stream_runtime.clone(),
            interpreter.test_effect_double_context(),
        ),
        test_effect_doubles: interpreter.test_effect_double_context(),
        actor: actor.clone(),
        request,
        request_heap_limits: RequestHeapLimits::default(),
    })
}

#[tokio::test]
async fn callback_native_ready_wait_invokes_once_and_finalizes_once() {
    let (owner_heap, wait) = owner_wait();
    let invocations = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&invocations);

    let outcome = wait
        .run(move |_heap| {
            Box::pin(async move {
                observed.fetch_add(1, Ordering::SeqCst);
                Ok(RuntimeValue::String("ready".to_string()))
            })
        })
        .await;
    assert_eq!(invocations.load(Ordering::SeqCst), 1);

    let mut caller_heap = RequestHeap::default();
    let result = completed(outcome, ContractTypeRef::builtin("string"))
        .finalize(&mut caller_heap)
        .expect("ready callback should finalize");
    assert_eq!(result, RuntimeValue::String("ready".to_string()));
    assert_eq!(caller_heap.len(), 0);
    assert!(
        owner_heap.try_lock().is_ok(),
        "finalize must release the owner guard"
    );
}

#[tokio::test]
async fn callback_native_finalize_imports_owner_heap_result_before_releasing_guard() {
    let (owner_heap, wait) = owner_wait();
    let outcome = wait
        .run(|heap| {
            Box::pin(async move {
                let value = heap
                    .alloc_array(vec![RuntimeValue::String("owner-result".to_string())])
                    .expect("owner result should allocate");
                Ok(RuntimeValue::Heap(value))
            })
        })
        .await;
    assert!(
        owner_heap.try_lock().is_err(),
        "completed outcome retains its owner heap through finalize"
    );

    let mut caller_heap = RequestHeap::default();
    let result = completed(
        outcome,
        ContractTypeRef::Builtin {
            name: "Array".to_string(),
            arguments: vec![ContractTypeRef::builtin("string")],
        },
    )
    .finalize(&mut caller_heap)
    .expect("owner result should import into caller heap");
    let RuntimeValue::Heap(result) = result else {
        panic!("array result should materialize as a caller heap handle")
    };
    assert!(matches!(
        caller_heap.get(result),
        Ok(HeapNode::Array(items))
            if items == &[RuntimeValue::String("owner-result".to_string())]
    ));
    assert!(owner_heap.try_lock().is_ok());
}

#[tokio::test]
async fn callback_native_finalize_rematerializes_nested_error_when_foreign_handle_collides() {
    let (owner_heap, wait) = owner_wait();
    let outcome = wait
        .run(|heap| Box::pin(async move { Err(owner_nested_exception_graph(heap)) }))
        .await;

    let mut caller_heap = RequestHeap::default();
    for marker in [
        "caller-zero",
        "nested-imported-handle-collision",
        "caller-two",
        "caller-three",
        "foreign-root-handle-collision",
    ] {
        caller_heap
            .alloc_array(vec![RuntimeValue::String(marker.to_string())])
            .expect("caller collision fixture should allocate");
    }
    let owner_payload_handle = HeapHandle::new(4, 0);
    assert!(matches!(
        caller_heap.get(owner_payload_handle),
        Ok(HeapNode::Array(items))
            if items == &[RuntimeValue::String("foreign-root-handle-collision".to_string())]
    ));
    assert!(matches!(
        caller_heap.get(HeapHandle::new(1, 0)),
        Ok(HeapNode::Array(items))
            if items == &[RuntimeValue::String("nested-imported-handle-collision".to_string())]
    ));

    let error = completed(outcome, ContractTypeRef::builtin("string"))
        .finalize(&mut caller_heap)
        .expect_err("typed callback exception should remain an error");
    assert_caller_local_nested_exception(error, &caller_heap);
    assert!(matches!(
        caller_heap.get(owner_payload_handle),
        Ok(HeapNode::Array(items))
            if items == &[RuntimeValue::String("foreign-root-handle-collision".to_string())]
    ));
    assert!(matches!(
        caller_heap.get(HeapHandle::new(1, 0)),
        Ok(HeapNode::Array(items))
            if items == &[RuntimeValue::String("nested-imported-handle-collision".to_string())]
    ));
    assert!(
        owner_heap.try_lock().is_ok(),
        "error rematerialization must release the owner guard"
    );
}

#[tokio::test]
async fn callback_native_finalize_rematerializes_nested_error_when_foreign_handle_is_oob() {
    let (owner_heap, wait) = owner_wait();
    let outcome = wait
        .run(|heap| Box::pin(async move { Err(owner_nested_exception_graph(heap)) }))
        .await;

    let mut caller_heap = RequestHeap::default();
    let owner_payload_handle = HeapHandle::new(4, 0);
    assert!(
        caller_heap.get(owner_payload_handle).is_err(),
        "owner handle must be out of bounds in the empty caller heap"
    );

    let error = completed(outcome, ContractTypeRef::builtin("string"))
        .finalize(&mut caller_heap)
        .expect_err("typed callback exception should remain an error");
    assert_caller_local_nested_exception(error, &caller_heap);
    assert!(
        owner_heap.try_lock().is_ok(),
        "error rematerialization must release the owner guard"
    );
}

#[tokio::test]
async fn callback_native_error_rematerialization_failure_restores_caller_checkpoint() {
    let (owner_heap, wait) = owner_wait();
    let outcome = wait
        .run(|heap| Box::pin(async move { Err(owner_nested_exception_graph(heap)) }))
        .await;

    let mut limits = RequestHeapLimits::default();
    limits.max_nodes = 2;
    let mut caller_heap = RequestHeap::new(limits);
    let sentinel = caller_heap
        .alloc_array(vec![RuntimeValue::String("caller-sentinel".to_string())])
        .expect("caller sentinel should fit before rematerialization");
    let checkpoint = caller_heap.checkpoint();
    let stats = caller_heap.stats();

    let error = completed(outcome, ContractTypeRef::builtin("string"))
        .finalize(&mut caller_heap)
        .expect_err("the nested graph should exceed the caller heap node limit");
    assert!(matches!(
        error,
        RuntimeError::ResourceLimitExceeded {
            resource,
            reason,
            limit: 2,
            ..
        } if resource == "requestHeap" && reason == "max heap nodes"
    ));
    assert_eq!(caller_heap.checkpoint(), checkpoint);
    assert_eq!(caller_heap.stats(), stats);
    assert!(matches!(
        caller_heap.get(sentinel),
        Ok(HeapNode::Array(items))
            if items == &[RuntimeValue::String("caller-sentinel".to_string())]
    ));
    assert!(
        owner_heap.try_lock().is_ok(),
        "failed error rematerialization must release the owner guard"
    );
}

#[tokio::test]
async fn callback_native_pending_wait_owns_only_owner_state_and_invokes_once() {
    let (owner_heap, wait) = owner_wait();
    let invocations = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&invocations);
    let (started_tx, started_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();

    let task = tokio::spawn(wait.run(move |heap| {
        Box::pin(async move {
            observed.fetch_add(1, Ordering::SeqCst);
            heap.alloc_array(vec![RuntimeValue::String("owner".to_string())])
                .expect("owner mutation before Pending should succeed");
            started_tx
                .send(())
                .expect("test should still await the start signal");
            release_rx
                .await
                .expect("test should release the pending callback");
            Ok(RuntimeValue::String("pending-complete".to_string()))
        })
    }));
    started_rx
        .await
        .expect("callback wait should reach its pending point");

    let mut caller_heap = RequestHeap::default();
    let caller_value = caller_heap
        .alloc_array(vec![RuntimeValue::String("caller".to_string())])
        .expect("caller heap must remain independently accessible");
    let mut caller_env = Env::new();
    caller_env.current_module = Some("caller.env.remains.available".to_string());
    assert_eq!(
        caller_env.current_module.as_deref(),
        Some("caller.env.remains.available")
    );
    assert!(matches!(
        caller_heap.get(caller_value),
        Ok(HeapNode::Array(items))
            if items == &[RuntimeValue::String("caller".to_string())]
    ));
    assert!(
        owner_heap.try_lock().is_err(),
        "the pending wait must retain only its owned owner-heap guard"
    );

    release_tx
        .send(())
        .expect("pending callback should still be live");
    let outcome = task.await.expect("callback wait task should finish");
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
    let result = completed(outcome, ContractTypeRef::builtin("string"))
        .finalize(&mut caller_heap)
        .expect("pending callback should finalize");
    assert_eq!(result, RuntimeValue::String("pending-complete".to_string()));
    assert!(owner_heap.try_lock().is_ok());
}

#[tokio::test]
async fn callback_native_prepared_recursive_wait_owns_context_without_actor_frame() {
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = program_context(&interpreter);
    let (owner_heap, owner) = owner_wait();
    let addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::LoadedFileIndex(0),
        executable: 0,
    };
    let prepared = PreparedCallbackInvocation {
        owner,
        owner_context: OwnedProgramExecutionContext::capture(&context),
        owner_call_env: Env::new(),
        caller_addr: addr.clone(),
        executable: addr,
        type_args: BTreeMap::new(),
        receiver: RuntimeValue::Null,
        args: Vec::new(),
        return_type: ContractTypeRef::builtin("string"),
        package_schema_records: BTreeMap::new(),
    };
    assert!(
        prepared
            .owner_context
            .borrow()
            .actor_execution_frame()
            .is_none(),
        "callback owner context must not contain an Actor frame"
    );
    drop(context);

    let outcome = prepared.wait(&interpreter).await;
    assert!(
        owner_heap.try_lock().is_err(),
        "completed recursive wait retains the owner guard until finalize"
    );
    let error = outcome
        .finalize(&mut RequestHeap::default())
        .expect_err("the intentionally empty assembly has no callback executable");
    assert!(matches!(error, RuntimeError::InvalidArtifact(_)));
    assert!(
        owner_heap.try_lock().is_ok(),
        "recursive evaluator error must release the owner guard"
    );
}

#[test]
fn callback_native_parameter_prepare_failure_restores_owner_checkpoint() {
    let mut source_heap = RequestHeap::default();
    let source_array = source_heap
        .alloc_array(vec![RuntimeValue::String("first".to_string())])
        .expect("source array should allocate");
    let mut owner_heap = RequestHeap::default();
    owner_heap
        .alloc_array(vec![RuntimeValue::String("existing".to_string())])
        .expect("pre-existing owner state should allocate");
    let checkpoint = owner_heap.checkpoint();
    let stats = owner_heap.stats();
    let len = owner_heap.len();
    let array_type = ContractTypeRef::Builtin {
        name: "Array".to_string(),
        arguments: vec![ContractTypeRef::builtin("string")],
    };

    let error = prepare_owner_arguments(
        &[array_type, ContractTypeRef::builtin("string")],
        &BTreeMap::new(),
        &[RuntimeValue::Heap(source_array), RuntimeValue::Number(7.0)],
        &source_heap,
        &mut owner_heap,
    )
    .expect_err("the second mismatched argument must fail after the first allocation");
    assert!(matches!(error, RuntimeError::Protocol { .. }));
    assert_eq!(owner_heap.checkpoint(), checkpoint);
    assert_eq!(owner_heap.stats(), stats);
    assert_eq!(owner_heap.len(), len);
}

#[tokio::test]
async fn callback_native_method_error_and_cancel_release_guard_without_rollback() {
    for error in [
        RuntimeError::InvalidArtifact("callback method failed".to_string()),
        RuntimeError::Cancelled,
    ] {
        let expected_cancellation_terminal = error.is_cancellation_terminal();
        let (owner_heap, wait) = owner_wait();
        let outcome = wait
            .run(move |heap| {
                Box::pin(async move {
                    heap.alloc_array(vec![RuntimeValue::String("visible".to_string())])
                        .expect("method mutation should allocate");
                    Err(error)
                })
            })
            .await;
        let mut caller_heap = RequestHeap::default();
        let error = completed(outcome, ContractTypeRef::builtin("string"))
            .finalize(&mut caller_heap)
            .expect_err("method error/cancel must remain an error terminal");
        assert_eq!(
            error.is_cancellation_terminal(),
            expected_cancellation_terminal,
            "error rematerialization must not change cancellation classification"
        );
        assert!(
            caller_heap.is_empty(),
            "non-user terminals must not materialize caller heap values"
        );
        let owner = owner_heap
            .try_lock()
            .expect("error finalization must release the owner guard");
        assert_eq!(
            owner.len(),
            1,
            "method error/cancel must preserve successful owner mutations"
        );
    }
}

#[tokio::test]
async fn callback_native_dropped_pending_wait_releases_guard_once_without_restart() {
    let (owner_heap, wait) = owner_wait();
    let invocations = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&invocations);
    let (started_tx, started_rx) = oneshot::channel();
    let (_release_tx, release_rx) = oneshot::channel::<()>();

    let task = tokio::spawn(wait.run(move |heap| {
        Box::pin(async move {
            observed.fetch_add(1, Ordering::SeqCst);
            heap.alloc_array(vec![RuntimeValue::String("before-drop".to_string())])
                .expect("owner mutation before Pending should succeed");
            started_tx
                .send(())
                .expect("test should await the start signal");
            let _ = release_rx.await;
            Ok(RuntimeValue::Null)
        })
    }));
    started_rx
        .await
        .expect("callback wait should reach its pending point");
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));

    assert_eq!(invocations.load(Ordering::SeqCst), 1);
    let owner = owner_heap
        .try_lock()
        .expect("dropping the wait must release its owned guard");
    assert_eq!(
        owner.len(),
        1,
        "drop must preserve owner mutations made before Pending"
    );
}

#[test]
fn callback_native_generation_mismatch_keeps_the_fixed_unavailable_error() {
    let carrier = CallbackCapabilityCarrier::new("runtime", "owner", 11, "contract", "callback");
    validate_callback_request_generation(11, &carrier)
        .expect("matching generation should remain admissible");
    let error = validate_callback_request_generation(12, &carrier)
        .expect_err("mismatched generation must fail closed");
    assert!(matches!(
        error,
        RuntimeError::ProviderUnavailable { reason, .. }
            if reason == "CapabilityUnavailable"
    ));
}
