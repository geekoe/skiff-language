use crate::heap_access::HeapAccess;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::atomic::{AtomicU64, Ordering},
    sync::{Arc, Mutex},
};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use skiff_artifact_model::{
    AssemblyIdentity, BlockIr, CanonicalPackageLinkPlan, DeploymentArtifactIdentity,
    DeploymentRevision, ExecutableBody, ExecutableIr, ExecutableKind, ExprIr, ExprRefIr, FileIrRef,
    FileIrUnit, MetadataValue, NativeTarget, PackageArtifact, PackageArtifactRef, PackageBuildId,
    PackageCodeSlot, PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
    PackageRuntimeRequirements, PackageSchemaIndexRef, RuntimeAssembly, ServiceContract,
    ServiceContractRef, ServiceDeploymentRef, SlotLayout, StmtIr, StmtRefIr, TypeRefIr,
    PACKAGE_ARTIFACT_SCHEMA_VERSION, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_runtime_activation::{
    ActivationContext, ActivationId, ActivationIdentity, RequestActivationContext,
};
use skiff_runtime_capability_context::{
    ActivationIdentityControl, ActorCapabilityApi, ActorCapabilityContext, ActorFindControlRequest,
    ActorGetOrCreateControlRequest, ActorRemoveControlRequest, ActorReplaceControlRequest,
    CapabilityError, CapabilityFuture, OwnedActorCapabilityContext, OwnedExecutionControl,
    OwnedRequestCapabilityContext, RequestCapabilityApi, RequestCapabilityContext,
    TaskCancelControlRequest, TaskCancelControlResponse, TaskStatusControlRequest,
    TaskStatusControlResponse, TaskSubmitControlRequest, TaskSubmitResponseControl,
    TaskSubmitTimingControl,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{ActorRef, RuntimeValue},
    value::HeapNode,
};

use crate::{
    assembly_execution::ordinary::tests::test_runtime,
    capabilities::TimeCapabilityContext,
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
    Interpreter, RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget,
};

const ASSEMBLY_ID: &str = "assembly:canonical-task-eval";
const PACKAGE_ID: &str = "example.canonical-task";
const TARGET_SYMBOL: &str = "task.fixture.run";

#[derive(Clone)]
struct RecordingActor {
    activation_identity: ActivationIdentityControl,
    submissions: Arc<Mutex<Vec<(TaskSubmitControlRequest, Vec<u8>)>>>,
    execution_receipts: Arc<Mutex<Vec<OwnedExecutionControl>>>,
    replies: Arc<Mutex<VecDeque<Result<TaskSubmitResponseControl, CapabilityError>>>>,
    status_requests: Arc<Mutex<Vec<TaskStatusControlRequest>>>,
    status_replies: Arc<Mutex<VecDeque<Result<TaskStatusControlResponse, CapabilityError>>>>,
    cancel_requests: Arc<Mutex<Vec<TaskCancelControlRequest>>>,
    cancel_replies: Arc<Mutex<VecDeque<Result<TaskCancelControlResponse, CapabilityError>>>>,
    task_seq: Arc<AtomicU64>,
}

impl RecordingActor {
    fn new(
        activation_identity: ActivationIdentityControl,
        submissions: Arc<Mutex<Vec<(TaskSubmitControlRequest, Vec<u8>)>>>,
        execution_receipts: Arc<Mutex<Vec<OwnedExecutionControl>>>,
        status_requests: Arc<Mutex<Vec<TaskStatusControlRequest>>>,
        cancel_requests: Arc<Mutex<Vec<TaskCancelControlRequest>>>,
    ) -> Self {
        Self {
            activation_identity,
            submissions,
            execution_receipts,
            replies: Arc::new(Mutex::new(VecDeque::new())),
            status_requests,
            status_replies: Arc::new(Mutex::new(VecDeque::new())),
            cancel_requests,
            cancel_replies: Arc::new(Mutex::new(VecDeque::new())),
            task_seq: Arc::new(AtomicU64::new(0)),
        }
    }

    fn scripted(self, replies: Vec<Result<TaskSubmitResponseControl, CapabilityError>>) -> Self {
        self.replies
            .lock()
            .expect("task reply script should remain available")
            .extend(replies);
        self
    }

    fn scripted_status(
        self,
        replies: Vec<Result<TaskStatusControlResponse, CapabilityError>>,
    ) -> Self {
        self.status_replies
            .lock()
            .expect("status reply script should remain available")
            .extend(replies);
        self
    }

    fn scripted_cancel(
        self,
        replies: Vec<Result<TaskCancelControlResponse, CapabilityError>>,
    ) -> Self {
        self.cancel_replies
            .lock()
            .expect("cancel reply script should remain available")
            .extend(replies);
        self
    }
}

impl ActorCapabilityApi for RecordingActor {
    fn owned(&self) -> OwnedActorCapabilityContext {
        ActorCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> ActorCapabilityContext<'_> {
        ActorCapabilityContext::new(self.clone())
    }

    fn get_or_create_actor<'a>(
        &'a self,
        _request: ActorGetOrCreateControlRequest,
        _bootstrap_payload: Vec<u8>,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, ActorRef> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "actor getOrCreate is not under test",
            ))
        })
    }

    fn replace_actor<'a>(
        &'a self,
        _request: ActorReplaceControlRequest,
        _bootstrap_payload: Vec<u8>,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, ActorRef> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "actor replace is not under test",
            ))
        })
    }

    fn find_actor<'a>(
        &'a self,
        _request: ActorFindControlRequest,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, Option<ActorRef>> {
        Box::pin(async { Err(CapabilityError::unsupported("actor find is not under test")) })
    }

    fn remove_actor<'a>(
        &'a self,
        _request: ActorRemoveControlRequest,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, bool> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "actor remove is not under test",
            ))
        })
    }

    fn invoke_actor<'a>(
        &'a self,
        _request: skiff_runtime_capability_context::ActorInvocationRequest,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, skiff_runtime_capability_context::ActorInvocationOutcome> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "Actor invocation is not under test",
            ))
        })
    }
}

impl RequestCapabilityApi for RecordingActor {
    fn owned(&self) -> OwnedRequestCapabilityContext {
        RequestCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> RequestCapabilityContext<'_> {
        RequestCapabilityContext::new(self.clone())
    }

    fn runtime_id(&self) -> &str {
        "replica:canonical-task"
    }

    fn service_id(&self) -> &str {
        PACKAGE_ID
    }

    fn service_version(&self) -> &str {
        "1.0.0"
    }

    fn request_id(&self) -> &str {
        "request:canonical-task"
    }

    fn request_target(&self) -> &str {
        "operation:submit"
    }

    fn request_build_id(&self) -> &str {
        "canonical-assembly-build"
    }

    fn task_service_protocol_identity(&self) -> &str {
        "protocol:canonical-task"
    }

    fn request_service_protocol_identity(&self) -> &str {
        "protocol:canonical-task"
    }

    fn operation_service_protocol_identity(&self) -> Option<&str> {
        Some("protocol:canonical-task")
    }

    fn activation_identity(&self) -> Option<&ActivationIdentityControl> {
        Some(&self.activation_identity)
    }

    fn trace_id(&self) -> Option<&str> {
        Some("trace:canonical-task")
    }

    fn submit_task<'a>(
        &'a self,
        request: TaskSubmitControlRequest,
        args_payload: Vec<u8>,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, TaskSubmitResponseControl> {
        let submissions = Arc::clone(&self.submissions);
        let execution_receipts = Arc::clone(&self.execution_receipts);
        let replies = Arc::clone(&self.replies);
        let task_seq = Arc::clone(&self.task_seq);
        Box::pin(async move {
            execution_receipts
                .lock()
                .expect("task execution receipts should remain available")
                .push(execution_control);
            submissions
                .lock()
                .expect("task recorder lock should remain available")
                .push((request.clone(), args_payload));
            let reply = {
                let mut replies = replies
                    .lock()
                    .expect("task reply script should remain available");
                replies.pop_front().unwrap_or_else(|| {
                    let seq = task_seq.fetch_add(1, Ordering::Relaxed) + 1;
                    let task_id = request
                        .task_id
                        .clone()
                        .unwrap_or_else(|| format!("task-{seq}"));
                    Ok(TaskSubmitResponseControl {
                        task_ref: task_ref_for(&request.service_id, &task_id),
                        task_id: task_id.clone(),
                        request_id: format!("request-{seq}"),
                    })
                })
            };
            reply
        })
    }

    fn status_task<'a>(
        &'a self,
        request: TaskStatusControlRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, TaskStatusControlResponse> {
        let execution_receipts = Arc::clone(&self.execution_receipts);
        let status_requests = Arc::clone(&self.status_requests);
        let status_replies = Arc::clone(&self.status_replies);
        Box::pin(async move {
            execution_receipts
                .lock()
                .expect("task execution receipts should remain available")
                .push(execution_control);
            status_requests
                .lock()
                .expect("status recorder lock should remain available")
                .push(request.clone());
            let reply = {
                let mut replies = status_replies
                    .lock()
                    .expect("status reply script should remain available");
                replies.pop_front().unwrap_or_else(|| {
                    Ok(TaskStatusControlResponse {
                        task_ref: request.task_ref,
                        kind: "scheduled".to_string(),
                    })
                })
            };
            reply
        })
    }

    fn cancel_task<'a>(
        &'a self,
        request: TaskCancelControlRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, TaskCancelControlResponse> {
        let execution_receipts = Arc::clone(&self.execution_receipts);
        let cancel_requests = Arc::clone(&self.cancel_requests);
        let cancel_replies = Arc::clone(&self.cancel_replies);
        Box::pin(async move {
            execution_receipts
                .lock()
                .expect("task execution receipts should remain available")
                .push(execution_control);
            cancel_requests
                .lock()
                .expect("cancel recorder lock should remain available")
                .push(request.clone());
            let reply = {
                let mut replies = cancel_replies
                    .lock()
                    .expect("cancel reply script should remain available");
                replies.pop_front().unwrap_or_else(|| {
                    Ok(TaskCancelControlResponse {
                        task_ref: request.task_ref,
                        kind: "canceled".to_string(),
                    })
                })
            };
            reply
        })
    }
}

fn task_ref_for(owner: &str, task_id: &str) -> String {
    format!(
        "skiff-task-v1:{}.{}",
        URL_SAFE_NO_PAD.encode(owner),
        URL_SAFE_NO_PAD.encode(task_id)
    )
}

struct TestResolver {
    activation: Arc<ActivationContext>,
}

impl RuntimeAssemblyEvalResolver for TestResolver {
    fn activation(&self, activation_id: &ActivationId) -> Option<Arc<ActivationContext>> {
        (self.activation.activation_id() == activation_id).then(|| Arc::clone(&self.activation))
    }

    fn activation_by_opaque_id(&self, activation_id: &str) -> Option<Arc<ActivationContext>> {
        (self.activation.activation_id().as_str() == activation_id)
            .then(|| Arc::clone(&self.activation))
    }

    fn contract(&self, _contract: &ServiceContractRef) -> Option<Arc<ServiceContract>> {
        None
    }

    fn admitted_schema_records(
        &self,
        _contract: &ServiceContractRef,
    ) -> Option<crate::AdmittedPackageSchemaRecords> {
        None
    }

    fn operation_target(
        &self,
        _activation_id: &ActivationId,
        _operation: &skiff_artifact_model::ContractOperationId,
    ) -> Option<skiff_artifact_model::OperationTargetRef> {
        None
    }
}

struct CanonicalTaskFixture {
    eval_target: RuntimeAssemblyEvalTarget,
    caller_addr: skiff_runtime_linked_program::ExecutableAddr,
    actor: ActorCapabilityContext<'static>,
    request: RequestCapabilityContext<'static>,
    activation_identity: ActivationIdentityControl,
    submissions: Arc<Mutex<Vec<(TaskSubmitControlRequest, Vec<u8>)>>>,
    status_requests: Arc<Mutex<Vec<TaskStatusControlRequest>>>,
    cancel_requests: Arc<Mutex<Vec<TaskCancelControlRequest>>>,
    execution_receipts: Arc<Mutex<Vec<OwnedExecutionControl>>>,
}

#[tokio::test]
async fn f445h_i6_actor_scope_task_uses_current_projection_and_exact_target() {
    let fixture = canonical_task_fixture(Some(TARGET_SYMBOL));
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(
        &interpreter,
        fixture.actor,
        fixture.request,
        Some(fixture.eval_target),
    );
    let heap = RequestHeap::default();

    let value = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut HeapAccess::private(heap),
            &fixture.caller_addr,
            Vec::new(),
        )
        .await
        .expect("canonical task should submit from the admitted in-memory execution image");
    assert_eq!(
        value,
        RuntimeValue::Null,
        "statement-position dispatch discards the TaskRef"
    );

    let submissions = fixture
        .submissions
        .lock()
        .expect("task submissions should be readable");
    let [(request, payload)] = submissions.as_slice() else {
        panic!("canonical task should submit exactly once");
    };
    assert_eq!(request.target_kind, "function");
    assert_eq!(request.target, format!("function:{TARGET_SYMBOL}"));
    assert_eq!(request.timing, TaskSubmitTimingControl::Immediate);
    assert!(
        request.task_id.is_some(),
        "durable submit must carry a runtime-generated TaskId"
    );
    assert_eq!(request.activation_identity, fixture.activation_identity);
    assert_eq!(
        request.caller_request_id.as_deref(),
        Some("request:canonical-task")
    );
    assert_eq!(request.service_protocol_identity, "protocol:canonical-task");
    assert_eq!(&payload[..4], b"SKRE");
    drop(submissions);
    let receipts = fixture
        .execution_receipts
        .lock()
        .expect("task execution receipts should be readable");
    let [receipt] = receipts.as_slice() else {
        panic!("canonical task should carry exactly one current execution control");
    };
    let scope = receipt
        .execution_scope()
        .expect("task receipt must retain the current execution scope");
    assert_eq!(
        scope.lifecycle_snapshot(),
        skiff_runtime_capability_context::ExecutionScopeLifecycleSnapshot::default()
    );
}

#[tokio::test]
async fn canonical_task_missing_metadata_fails_before_actor_capability() {
    let fixture = canonical_task_fixture(None);
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(
        &interpreter,
        fixture.actor,
        fixture.request,
        Some(fixture.eval_target),
    );

    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut HeapAccess::private(RequestHeap::default()),
            &fixture.caller_addr,
            Vec::new(),
        )
        .await
        .expect_err("canonical task without compiler metadata must fail closed");

    assert!(error
        .to_string()
        .contains("missing compiler dispatchSubmit metadata"));
    assert!(fixture
        .submissions
        .lock()
        .expect("task submissions should be readable")
        .is_empty());
}

#[tokio::test]
async fn canonical_task_missing_execution_projection_fails_before_actor_capability() {
    let fixture = canonical_task_fixture(Some(TARGET_SYMBOL));
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(&interpreter, fixture.actor, fixture.request, None);

    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut HeapAccess::private(RequestHeap::default()),
            &fixture.caller_addr,
            Vec::new(),
        )
        .await
        .expect_err("canonical execution without an admitted projection must fail closed");

    assert!(error
        .to_string()
        .contains("program execution context has no runtime assembly target"));
    assert!(fixture
        .submissions
        .lock()
        .expect("task submissions should be readable")
        .is_empty());
}

#[tokio::test]
async fn canonical_task_statement_after_timing_evaluates_expression_once() {
    let fixture = canonical_task_fixture_with(
        vec![
            caller_executable_with_timing(
                Some(TARGET_SYMBOL),
                Some(("after".to_string(), Some(ExprRefIr { expression: 1 }))),
                vec![ExprIr::Literal {
                    value: skiff_artifact_model::LiteralIr::Number {
                        value: serde_json::Number::from(500u64),
                    },
                }],
            ),
            target_executable(),
        ],
        Vec::new(),
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(
        &interpreter,
        fixture.actor,
        fixture.request,
        Some(fixture.eval_target),
    );
    interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut HeapAccess::private(RequestHeap::default()),
            &fixture.caller_addr,
            Vec::new(),
        )
        .await
        .expect("canonical task after() should submit");

    let submissions = fixture
        .submissions
        .lock()
        .expect("task submissions should be readable");
    let [(request, _payload)] = submissions.as_slice() else {
        panic!("canonical task after() should submit exactly once");
    };
    assert_eq!(
        request.timing,
        TaskSubmitTimingControl::After { duration_ms: 500 }
    );
}

#[tokio::test]
async fn canonical_task_statement_at_timing_evaluates_expression_once() {
    let fixture = canonical_task_fixture_with(
        vec![
            caller_executable_with_timing(
                Some(TARGET_SYMBOL),
                Some(("at".to_string(), Some(ExprRefIr { expression: 1 }))),
                vec![
                    ExprIr::Call {
                        call: skiff_artifact_model::CallIr {
                            target: skiff_artifact_model::CallTargetIr::Native {
                                target: NativeTarget {
                                    namespace: "core.date".to_string(),
                                    symbol: "fromEpochMilliseconds".to_string(),
                                    binding_key: Some("core.date.fromEpochMilliseconds".to_string()),
                                    metadata: BTreeMap::new(),
                                },
                            },
                            site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                                reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                            },
                            args: vec![ExprRefIr { expression: 2 }],
                            type_args: BTreeMap::new(),
                            metadata: BTreeMap::new(),
                        },
                    },
                    ExprIr::Literal {
                        value: skiff_artifact_model::LiteralIr::Number {
                            value: serde_json::Number::from(1_728_000_000u64),
                        },
                    },
                ],
            ),
            target_executable(),
        ],
        Vec::new(),
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(
        &interpreter,
        fixture.actor,
        fixture.request,
        Some(fixture.eval_target),
    );
    interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut HeapAccess::private(RequestHeap::default()),
            &fixture.caller_addr,
            Vec::new(),
        )
        .await
        .expect("canonical task at() should submit");

    let submissions = fixture
        .submissions
        .lock()
        .expect("task submissions should be readable");
    let [(request, _payload)] = submissions.as_slice() else {
        panic!("canonical task at() should submit exactly once");
    };
    assert_eq!(
        request.timing,
        TaskSubmitTimingControl::At {
            utc_millis: 1_728_000_000
        }
    );
}

#[tokio::test]
async fn canonical_task_expression_position_returns_task_ref() {
    let fixture = canonical_task_fixture_with(
        vec![
            caller_expression_return_executable(TARGET_SYMBOL),
            target_executable(),
        ],
        Vec::new(),
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(
        &interpreter,
        fixture.actor,
        fixture.request,
        Some(fixture.eval_target),
    );
    let value = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut HeapAccess::private(RequestHeap::default()),
            &fixture.caller_addr,
            Vec::new(),
        )
        .await
        .expect("canonical dispatch expression should return a TaskRef");
    let RuntimeValue::String(task_ref) = value else {
        panic!("dispatch expression must produce an opaque taskRef string");
    };
    let submissions = fixture
        .submissions
        .lock()
        .expect("task submissions should be readable");
    let [(request, _payload)] = submissions.as_slice() else {
        panic!("canonical dispatch expression should submit exactly once");
    };
    let task_id = request
        .task_id
        .as_deref()
        .expect("TaskId must be generated");
    assert_eq!(
        task_ref.as_str(),
        task_ref_for(&request.service_id, task_id)
    );
}

#[tokio::test]
async fn canonical_task_arguments_evaluate_once_with_nested_dispatch() {
    const INNER_SYMBOL: &str = "task.fixture.inner";
    const OUTER_SYMBOL: &str = "task.fixture.outer";
    let fixture = canonical_task_fixture_with(
        vec![
            caller_nested_arg_executable(INNER_SYMBOL, OUTER_SYMBOL),
            target_executable_named(INNER_SYMBOL),
            target_executable_with_param(OUTER_SYMBOL, TypeRefIr::builtin("TaskRef")),
        ],
        Vec::new(),
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(
        &interpreter,
        fixture.actor,
        fixture.request,
        Some(fixture.eval_target),
    );
    interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut HeapAccess::private(RequestHeap::default()),
            &fixture.caller_addr,
            Vec::new(),
        )
        .await
        .expect("canonical nested dispatch should submit");

    let submissions = fixture
        .submissions
        .lock()
        .expect("task submissions should be readable");
    assert_eq!(
        submissions.len(),
        2,
        "dispatch argument must be evaluated exactly once (inner submission + outer submission)"
    );
    assert_eq!(submissions[0].0.target, format!("function:{INNER_SYMBOL}"));
    assert_eq!(submissions[1].0.target, format!("function:{OUTER_SYMBOL}"));
    assert!(!submissions[1].1.is_empty());
}

#[tokio::test]
async fn canonical_task_transient_store_unavailable_retries_same_task_id() {
    let fixture = canonical_task_fixture_with(
        vec![caller_executable(Some(TARGET_SYMBOL)), target_executable()],
        vec![
            Err(CapabilityError::task_submit_rejected(
                "storeUnavailable",
                "store is down",
            )),
            Ok(TaskSubmitResponseControl {
                task_ref: "skiff-task-v1:b3duZXI.dGFzay0x".to_string(),
                task_id: "task-1".to_string(),
                request_id: "request-1".to_string(),
            }),
        ],
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(
        &interpreter,
        fixture.actor,
        fixture.request,
        Some(fixture.eval_target),
    );
    interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut HeapAccess::private(RequestHeap::default()),
            &fixture.caller_addr,
            Vec::new(),
        )
        .await
        .expect("transient storeUnavailable must recover on a bounded retry");

    let submissions = fixture
        .submissions
        .lock()
        .expect("task submissions should be readable");
    assert_eq!(submissions.len(), 2);
    assert!(submissions[0].0.task_id.is_some());
    assert_eq!(submissions[0].0.task_id, submissions[1].0.task_id);
}

#[tokio::test]
async fn canonical_task_definite_rejection_throws_without_task() {
    let fixture = canonical_task_fixture_with(
        vec![caller_executable(Some(TARGET_SYMBOL)), target_executable()],
        vec![Err(CapabilityError::task_submit_rejected(
            "rejected",
            "control plane refused the submission",
        ))],
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(
        &interpreter,
        fixture.actor,
        fixture.request,
        Some(fixture.eval_target),
    );
    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut HeapAccess::private(RequestHeap::default()),
            &fixture.caller_addr,
            Vec::new(),
        )
        .await
        .expect_err("definite rejection must surface as a platform error");
    assert!(error
        .to_string()
        .contains("task.submit rejected (rejected)"));
    assert_eq!(
        fixture
            .submissions
            .lock()
            .expect("task submissions should be readable")
            .len(),
        1,
        "definite rejection must not be retried"
    );
}

#[tokio::test]
async fn canonical_task_ambiguous_result_after_bounded_retries() {
    let fixture = canonical_task_fixture_with(
        vec![caller_executable(Some(TARGET_SYMBOL)), target_executable()],
        vec![
            Err(CapabilityError::task_submit_rejected(
                "storeUnavailable",
                "store down 1",
            )),
            Err(CapabilityError::task_submit_rejected(
                "storeUnavailable",
                "store down 2",
            )),
            Err(CapabilityError::task_submit_rejected(
                "storeUnavailable",
                "store down 3",
            )),
        ],
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(
        &interpreter,
        fixture.actor,
        fixture.request,
        Some(fixture.eval_target),
    );
    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut HeapAccess::private(RequestHeap::default()),
            &fixture.caller_addr,
            Vec::new(),
        )
        .await
        .expect_err("exhausted ambiguous retries must surface as uncertain");
    assert!(error.to_string().contains("result is uncertain"));
    let submissions = fixture
        .submissions
        .lock()
        .expect("task submissions should be readable");
    assert_eq!(submissions.len(), super::TASK_SUBMIT_MAX_ATTEMPTS);
    for submission in submissions.iter().skip(1) {
        assert_eq!(submission.0.task_id, submissions[0].0.task_id);
    }
}

#[test]
fn canonical_task_function_target_rejects_mismatched_metadata_defensively() {
    let call = skiff_runtime_linked_program::CallIr {
        target: skiff_runtime_linked_program::LinkedCallTarget::Executable {
            addr: skiff_runtime_linked_program::ExecutableAddr::package(0, 0, 1),
        },
        site: skiff_artifact_model::InstructionSourceSite::Synthetic {
            reason:
                skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
        },
        args: Vec::new(),
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
        actor_metadata: None,
    };
    let metadata = super::TaskSubmitTarget {
        kind: "function".to_string(),
        name: "function:task.fixture.other".to_string(),
    };

    let error = super::canonical_task_function_target(
        &call,
        &metadata,
        &skiff_runtime_linked_program::ExecutableKind::Function,
        TARGET_SYMBOL,
    )
    .expect_err("eval must still reject a mismatched target defensively");

    assert!(error
        .to_string()
        .contains("does not match linked executable function:task.fixture.run"));
}

#[tokio::test]
async fn std_task_status_maps_wire_kind_to_user_union_value() {
    let fixture = task_control_fixture_with_replies(
        "task.status",
        "TaskStatus",
        "skiff-task-v1:b3duZXI.dGFzay0x",
        vec![Ok(TaskStatusControlResponse {
            task_ref: "skiff-task-v1:b3duZXI.dGFzay0x".to_string(),
            kind: "running".to_string(),
        })],
        Vec::new(),
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(
        &interpreter,
        fixture.actor,
        fixture.request,
        Some(fixture.eval_target),
    );
    let mut heap = HeapAccess::private(RequestHeap::default());
    let value = interpreter
        .execute_runtime_assembly_addr(context, &mut heap, &fixture.caller_addr, Vec::new())
        .await
        .expect("std.task.status should map to a user union value");
    let RuntimeValue::Heap(handle) = value else {
        panic!("status must return a record")
    };
    let HeapNode::Object(object) = heap
        .heap_mut()
        .get(handle)
        .expect("status record must resolve")
    else {
        panic!("status must return a record")
    };
    assert_eq!(
        object.fields().get("kind"),
        Some(&RuntimeValue::String("running".to_string()))
    );
    let requests = fixture.status_requests.lock().expect("status requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].task_ref, "skiff-task-v1:b3duZXI.dGFzay0x");
}

#[tokio::test]
async fn std_task_cancel_maps_wire_kind_to_user_union_value() {
    let fixture = task_control_fixture_with_replies(
        "task.cancel",
        "TaskCancelResult",
        "skiff-task-v1:b3duZXI.dGFzay0x",
        Vec::new(),
        vec![Ok(TaskCancelControlResponse {
            task_ref: "skiff-task-v1:b3duZXI.dGFzay0x".to_string(),
            kind: "alreadyStarted".to_string(),
        })],
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(
        &interpreter,
        fixture.actor,
        fixture.request,
        Some(fixture.eval_target),
    );
    let mut heap = HeapAccess::private(RequestHeap::default());
    let value = interpreter
        .execute_runtime_assembly_addr(context, &mut heap, &fixture.caller_addr, Vec::new())
        .await
        .expect("std.task.cancel should map to a user union value");
    let RuntimeValue::Heap(handle) = value else {
        panic!("cancel must return a heap record")
    };
    let HeapNode::Object(object) = heap
        .heap_mut()
        .get(handle)
        .expect("cancel record must resolve")
    else {
        panic!("cancel must return a record")
    };
    assert_eq!(
        object.fields().get("kind"),
        Some(&RuntimeValue::String("alreadyStarted".to_string()))
    );
    let requests = fixture.cancel_requests.lock().expect("cancel requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].task_ref, "skiff-task-v1:b3duZXI.dGFzay0x");
}

#[tokio::test]
async fn std_task_status_not_found_projects_to_stable_expired() {
    let fixture = task_control_fixture_with_replies(
        "task.status",
        "TaskStatus",
        "skiff-task-v1:b3duZXIuZG9jcy5jb20.bWlzc2luZw",
        vec![Err(CapabilityError::task_control_rejected(
            "notFound",
            "task reference owner scope is not resolvable",
        ))],
        Vec::new(),
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(
        &interpreter,
        fixture.actor,
        fixture.request,
        Some(fixture.eval_target),
    );
    let mut heap = HeapAccess::private(RequestHeap::default());
    let value = interpreter
        .execute_runtime_assembly_addr(context, &mut heap, &fixture.caller_addr, Vec::new())
        .await
        .expect("notFound must project to stable expired");
    let RuntimeValue::Heap(handle) = value else {
        panic!("expired must be a heap record")
    };
    let HeapNode::Object(object) = heap
        .heap_mut()
        .get(handle)
        .expect("expired record must resolve")
    else {
        panic!("expired must be a record")
    };
    assert_eq!(
        object.fields().get("kind"),
        Some(&RuntimeValue::String("expired".to_string()))
    );
}

#[tokio::test]
async fn std_task_cancel_not_found_projects_to_stable_expired() {
    let fixture = task_control_fixture_with_replies(
        "task.cancel",
        "TaskCancelResult",
        "skiff-task-v1:b3duZXI.dGFzay0x",
        Vec::new(),
        vec![Err(CapabilityError::task_control_rejected(
            "notFound",
            "task record is not found",
        ))],
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(
        &interpreter,
        fixture.actor,
        fixture.request,
        Some(fixture.eval_target),
    );
    let mut heap = HeapAccess::private(RequestHeap::default());
    let value = interpreter
        .execute_runtime_assembly_addr(context, &mut heap, &fixture.caller_addr, Vec::new())
        .await
        .expect("notFound must project to stable expired");
    let RuntimeValue::Heap(handle) = value else {
        panic!("expired must be a heap record")
    };
    let HeapNode::Object(object) = heap
        .heap_mut()
        .get(handle)
        .expect("expired record must resolve")
    else {
        panic!("expired must be a record")
    };
    assert_eq!(
        object.fields().get("kind"),
        Some(&RuntimeValue::String("expired".to_string()))
    );
}

#[tokio::test]
async fn std_task_status_store_unavailable_surfaces_platform_error() {
    let fixture = task_control_fixture_with_replies(
        "task.status",
        "TaskStatus",
        "skiff-task-v1:b3duZXI.dGFzay0x",
        vec![Err(CapabilityError::task_control_rejected(
            "storeUnavailable",
            "task store is unavailable",
        ))],
        Vec::new(),
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(
        &interpreter,
        fixture.actor,
        fixture.request,
        Some(fixture.eval_target),
    );
    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut HeapAccess::private(RequestHeap::default()),
            &fixture.caller_addr,
            Vec::new(),
        )
        .await
        .expect_err("storeUnavailable must surface as a platform error");
    assert!(
        error
            .to_string()
            .contains("task control rejected (storeUnavailable)"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn std_task_cancel_rejects_non_canonical_task_ref_argument() {
    let fixture = task_control_fixture("task.cancel", "TaskCancelResult", "not-a-task-ref");
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(
        &interpreter,
        fixture.actor,
        fixture.request,
        Some(fixture.eval_target),
    );
    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut HeapAccess::private(RequestHeap::default()),
            &fixture.caller_addr,
            Vec::new(),
        )
        .await
        .expect_err("non-canonical TaskRef must fail closed before any control request");
    assert!(
        error.to_string().contains("canonical"),
        "unexpected error: {error}"
    );
    assert_eq!(
        fixture
            .cancel_requests
            .lock()
            .expect("cancel requests")
            .len(),
        0,
        "invalid TaskRef must not produce a control request"
    );
}

fn task_control_fixture(
    binding_suffix: &str,
    return_type: &str,
    task_ref: &str,
) -> CanonicalTaskFixture {
    task_control_fixture_with_replies(
        binding_suffix,
        return_type,
        task_ref,
        Vec::new(),
        Vec::new(),
    )
}

fn task_control_fixture_with_replies(
    binding_suffix: &str,
    return_type: &str,
    task_ref: &str,
    status_replies: Vec<Result<TaskStatusControlResponse, CapabilityError>>,
    cancel_replies: Vec<Result<TaskCancelControlResponse, CapabilityError>>,
) -> CanonicalTaskFixture {
    canonical_task_fixture_with_control_scripts(
        vec![
            task_control_caller_executable(binding_suffix, return_type, task_ref),
            target_executable(),
        ],
        Vec::new(),
        status_replies,
        cancel_replies,
    )
}

fn task_control_caller_executable(
    binding_suffix: &str,
    return_type: &str,
    task_ref: &str,
) -> ExecutableIr {
    let binding_key = format!("std.{binding_suffix}");
    let symbol = binding_suffix.to_string();
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "task.fixture.control".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin(return_type),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: true,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Return {
                value: Some(ExprRefIr { expression: 0 }),
            }],
            expressions: vec![
                ExprIr::Call {
                    call: skiff_artifact_model::CallIr {
                        target: skiff_artifact_model::CallTargetIr::Native {
                            target: NativeTarget {
                                namespace: "std".to_string(),
                                symbol,
                                binding_key: Some(binding_key),
                                metadata: BTreeMap::new(),
                            },
                        },
                        site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                            reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                        },
                        args: vec![ExprRefIr { expression: 1 }],
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                },
                ExprIr::Literal {
                    value: skiff_artifact_model::LiteralIr::String {
                        value: task_ref.to_string(),
                    },
                },
            ],
        },
        source_span: None,
    }
}

fn canonical_task_fixture(metadata_symbol: Option<&str>) -> CanonicalTaskFixture {
    canonical_task_fixture_with(
        vec![caller_executable(metadata_symbol), target_executable()],
        Vec::new(),
    )
}

fn canonical_task_fixture_with(
    executables: Vec<ExecutableIr>,
    replies: Vec<Result<TaskSubmitResponseControl, CapabilityError>>,
) -> CanonicalTaskFixture {
    canonical_task_fixture_with_control_scripts(executables, replies, Vec::new(), Vec::new())
}

fn canonical_task_fixture_with_control_scripts(
    executables: Vec<ExecutableIr>,
    replies: Vec<Result<TaskSubmitResponseControl, CapabilityError>>,
    status_replies: Vec<Result<TaskStatusControlResponse, CapabilityError>>,
    cancel_replies: Vec<Result<TaskCancelControlResponse, CapabilityError>>,
) -> CanonicalTaskFixture {
    let mut file = FileIrUnit::empty("task.fixture", "source:canonical-task");
    file.executables = executables;
    skiff_artifact_identity::assign_file_ir_identity(&mut file)
        .expect("canonical task File IR should receive an identity");
    let mut package = private_package(&file);
    skiff_artifact_identity::assign_package_artifact_identities(&mut package)
        .expect("canonical task package should receive identities");
    let package_ref = package_ref(&package);
    let assembly_identity = AssemblyIdentity::new(ASSEMBLY_ID);
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: assembly_identity.clone(),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![package_ref.clone()],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![PackageCodeSlot {
                package: package_ref.clone(),
            }],
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let image =
        crate::test_support::link_package_fixture(assembly.clone(), vec![(package, vec![file])]);
    let activation = activation_context(
        assembly_identity.clone(),
        package_ref.package_build_id.clone(),
    );
    let request = RequestActivationContext::begin(Arc::clone(&activation))
        .expect("canonical task request generation should begin");
    let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(TestResolver {
        activation: Arc::clone(&activation),
    });
    let eval_target = RuntimeAssemblyEvalTarget::new(image, request, resolver)
        .expect("canonical task eval target should use admitted facts");
    let activation_identity = ActivationIdentityControl {
        assembly_identity,
        generation: activation.identity().assembly_generation,
        runtime_replica_id: activation.identity().runtime_replica_id.clone(),
        deployment_revision: activation.identity().deployment.deployment_revision.clone(),
    };
    let submissions = Arc::new(Mutex::new(Vec::new()));
    let execution_receipts = Arc::new(Mutex::new(Vec::new()));
    let status_requests = Arc::new(Mutex::new(Vec::new()));
    let cancel_requests = Arc::new(Mutex::new(Vec::new()));
    let recording_actor = RecordingActor::new(
        activation_identity.clone(),
        Arc::clone(&submissions),
        Arc::clone(&execution_receipts),
        Arc::clone(&status_requests),
        Arc::clone(&cancel_requests),
    )
    .scripted(replies)
    .scripted_status(status_replies)
    .scripted_cancel(cancel_replies);
    let actor = ActorCapabilityContext::new(recording_actor.clone());
    let request = RequestCapabilityContext::new(recording_actor);
    CanonicalTaskFixture {
        eval_target,
        caller_addr: skiff_runtime_linked_program::ExecutableAddr::package(0, 0, 0),
        actor,
        request,
        activation_identity,
        submissions,
        status_requests,
        cancel_requests,
        execution_receipts,
    }
}

fn caller_executable(metadata_symbol: Option<&str>) -> ExecutableIr {
    caller_executable_with_timing(metadata_symbol, None, Vec::new())
}

fn caller_executable_with_timing(
    metadata_symbol: Option<&str>,
    timing: Option<(String, Option<ExprRefIr>)>,
    extra_expressions: Vec<ExprIr>,
) -> ExecutableIr {
    let mut metadata = BTreeMap::new();
    if let Some(symbol) = metadata_symbol {
        let mut dispatch = BTreeMap::from([
            (
                "targetKind".to_string(),
                MetadataValue::String("function".to_string()),
            ),
            (
                "target".to_string(),
                MetadataValue::String(format!("function:{symbol}")),
            ),
        ]);
        if let Some((kind, expr)) = timing {
            let mut timing_metadata = BTreeMap::new();
            timing_metadata.insert("kind".to_string(), MetadataValue::String(kind));
            if let Some(expr) = expr {
                timing_metadata.insert(
                    "expr".to_string(),
                    MetadataValue::Number(expr.expression.into()),
                );
            }
            dispatch.insert("timing".to_string(), MetadataValue::Object(timing_metadata));
        }
        metadata.insert(
            "dispatchSubmit".to_string(),
            MetadataValue::Object(dispatch),
        );
    }
    let mut expressions = vec![ExprIr::Call {
        call: skiff_artifact_model::CallIr {
            target: skiff_artifact_model::CallTargetIr::LocalExecutable {
                executable_index: 1,
            },
            site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
            },
            args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata,
        },
    }];
    expressions.extend(extra_expressions);
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "task.fixture.submit".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: skiff_artifact_model::TypeRefIr::builtin("null"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: true,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
            }],
            statements: vec![
                StmtIr::Dispatch {
                    call: ExprRefIr { expression: 0 },
                },
                StmtIr::Return { value: None },
            ],
            expressions,
        },
        source_span: None,
    }
}

fn target_executable() -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: TARGET_SYMBOL.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: skiff_artifact_model::TypeRefIr::builtin("null"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody::default(),
        source_span: None,
    }
}

fn target_executable_named(symbol: &str) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("null"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody::default(),
        source_span: None,
    }
}

fn target_executable_with_param(symbol: &str, param_type: TypeRefIr) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: vec![skiff_artifact_model::ParamIr {
            name: "ref".to_string(),
            slot: 0,
            ty: param_type,
        }],
        return_type: skiff_artifact_model::TypeRefIr::builtin("null"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody::default(),
        source_span: None,
    }
}

/// Expression-position dispatch: the caller returns the dispatch expression
/// value (a TaskRef) instead of discarding it.
fn caller_expression_return_executable(metadata_symbol: &str) -> ExecutableIr {
    let metadata = BTreeMap::from([(
        "dispatchSubmit".to_string(),
        MetadataValue::Object(BTreeMap::from([
            (
                "targetKind".to_string(),
                MetadataValue::String("function".to_string()),
            ),
            (
                "target".to_string(),
                MetadataValue::String(format!("function:{metadata_symbol}")),
            ),
        ])),
    )]);
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "task.fixture.submit".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("TaskRef"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: true,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Return {
                value: Some(ExprRefIr { expression: 0 }),
            }],
            expressions: vec![ExprIr::Call {
                call: skiff_artifact_model::CallIr {
                    target: skiff_artifact_model::CallTargetIr::LocalExecutable {
                        executable_index: 1,
                    },
                    site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                        reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                    },
                    args: Vec::new(),
                    type_args: BTreeMap::new(),
                    metadata,
                },
            }],
        },
        source_span: None,
    }
}

/// Statement dispatch whose argument is itself a dispatch expression. The
/// inner dispatch must be evaluated exactly once; double evaluation would
/// produce a third submission.
fn caller_nested_arg_executable(inner_symbol: &str, outer_symbol: &str) -> ExecutableIr {
    let dispatch_metadata = |symbol: &str| {
        MetadataValue::Object(BTreeMap::from([
            (
                "targetKind".to_string(),
                MetadataValue::String("function".to_string()),
            ),
            (
                "target".to_string(),
                MetadataValue::String(format!("function:{symbol}")),
            ),
        ]))
    };
    let inner_call = ExprIr::Call {
        call: skiff_artifact_model::CallIr {
            target: skiff_artifact_model::CallTargetIr::LocalExecutable {
                executable_index: 1,
            },
            site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
            },
            args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::from([(
                "dispatchSubmit".to_string(),
                dispatch_metadata(inner_symbol),
            )]),
        },
    };
    let outer_call = ExprIr::Call {
        call: skiff_artifact_model::CallIr {
            target: skiff_artifact_model::CallTargetIr::LocalExecutable {
                executable_index: 2,
            },
            site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
            },
            args: vec![ExprRefIr { expression: 1 }],
            type_args: BTreeMap::new(),
            metadata: BTreeMap::from([(
                "dispatchSubmit".to_string(),
                dispatch_metadata(outer_symbol),
            )]),
        },
    };
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "task.fixture.submit".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("null"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: true,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
            }],
            statements: vec![
                StmtIr::Dispatch {
                    call: ExprRefIr { expression: 0 },
                },
                StmtIr::Return { value: None },
            ],
            expressions: vec![outer_call, inner_call],
        },
        source_span: None,
    }
}

fn private_package(file: &FileIrUnit) -> PackageArtifact {
    PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: PACKAGE_ID.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: vec![FileIrRef {
            file_ir_identity: file.file_ir_identity.clone(),
            module_path: file.module_path.clone(),
            artifact_path: None,
            source_ast_hash: Some(file.source_ast_hash.clone()),
        }],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: PACKAGE_ID.to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                PACKAGE_ID,
                &BTreeMap::new(),
            )
            .expect("empty Package schema index is canonical"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    }
}

fn package_ref(package: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: package.package_id.clone(),
        package_version: package.package_version.clone(),
        package_build_id: package.package_build_id.clone(),
        package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
    }
}

fn activation_context(
    assembly_identity: AssemblyIdentity,
    package_build_id: PackageBuildId,
) -> Arc<ActivationContext> {
    ActivationContext::new(
        ActivationIdentity {
            assembly_identity,
            assembly_generation: 7,
            runtime_replica_id: "replica:canonical-task".to_string(),
            deployment: ServiceDeploymentRef {
                service_id: PACKAGE_ID.to_string(),
                contract_version: "1.0.0".to_string(),
                deployment_revision: DeploymentRevision::new("canonical-task-r1"),
                deployment_artifact_identity: DeploymentArtifactIdentity::new(
                    "deployment:canonical-task",
                ),
            },
        },
        package_build_id,
        Vec::new(),
    )
    .expect("canonical task activation should build")
}

fn execution_context<'a>(
    interpreter: &Interpreter,
    actor: ActorCapabilityContext<'static>,
    request: RequestCapabilityContext<'static>,
    target: Option<RuntimeAssemblyEvalTarget>,
) -> ProgramExecutionContext<'a> {
    let execution = test_runtime::execution_control();
    let effects = test_runtime::effects_context();
    let context = ProgramExecutionContext::new(ProgramExecutionInput {
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
    });
    match target {
        Some(target) => context.with_runtime_assembly_target(target),
        None => context,
    }
}
