use super::*;

use std::{collections::BTreeSet, fmt, sync::atomic::AtomicU64};

use skiff_artifact_model as artifact;
use skiff_runtime_activation::{
    ActivationContext, ActivationId, ActivationIdentity, ActivationServiceBinding,
    RequestActivationContext,
};
use skiff_runtime_capability_context::StreamCancelSignalApi;

use crate::{RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget};

const CALLER_PACKAGE: &str = "example.f445h.emit-caller";
const PROVIDER_PACKAGE: &str = "example.f445h.emit-provider";
const SERVICE_ID: &str = "example.f445h.emit-service";
const OPERATION_ID: &str = "operation:f445h-e4r:canonical-emit";

#[derive(Debug)]
enum ProbeEvent {
    Item(Value),
    Internal(StreamInternalItem),
    End,
    Fail(StreamRuntimeError),
}

#[derive(Default)]
struct ProbeCounts {
    send_starts: AtomicUsize,
    send_completions: AtomicUsize,
}

struct ProbeState {
    id: u64,
    sender: tokio::sync::mpsc::Sender<ProbeEvent>,
    receiver: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<ProbeEvent>>,
    counts: Arc<ProbeCounts>,
    cancelled: Arc<AtomicBool>,
    lifetime: Mutex<Option<StreamLifetimeGuard>>,
}

#[derive(Clone)]
struct ProbeRuntime {
    state: Arc<ProbeState>,
}

impl fmt::Debug for ProbeRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProbeRuntime")
            .field("id", &self.state.id)
            .finish()
    }
}

impl ProbeRuntime {
    fn new() -> (StreamRuntime, Arc<ProbeCounts>) {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let counts = Arc::new(ProbeCounts::default());
        let runtime = Self {
            state: Arc::new(ProbeState {
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                sender,
                receiver: tokio::sync::Mutex::new(receiver),
                counts: Arc::clone(&counts),
                cancelled: Arc::new(AtomicBool::new(false)),
                lifetime: Mutex::new(None),
            }),
        };
        (StreamRuntime::new(runtime), counts)
    }

    fn stream(&self, lifetime: Option<StreamLifetimeGuard>) -> (Value, StreamSink) {
        *self
            .state
            .lifetime
            .lock()
            .expect("probe stream lifetime lock") = lifetime;
        (
            skiff_runtime_boundary::stream::stream_value(&self.state.id.to_string()),
            StreamSink::new(ProbeSink {
                state: Arc::clone(&self.state),
            }),
        )
    }

    async fn next_event(&self) -> StreamRuntimeResult<StreamPoll> {
        match self.state.receiver.lock().await.recv().await {
            Some(ProbeEvent::Item(value)) => Ok(StreamPoll::Item(value)),
            Some(ProbeEvent::Internal(item)) => Ok(StreamPoll::InternalItem(item)),
            Some(ProbeEvent::End) | None => Ok(StreamPoll::End),
            Some(ProbeEvent::Fail(error)) => Err(error),
        }
    }
}

impl StreamRuntimeApi for ProbeRuntime {
    fn channel_stream(&self) -> (Value, StreamSink) {
        self.stream(None)
    }

    fn channel_stream_with_lifetime(&self, lifetime: StreamLifetimeGuard) -> (Value, StreamSink) {
        self.stream(Some(lifetime))
    }

    fn pull_stream_with_cancellation(
        &self,
        _source: Box<dyn StreamPullSource>,
        _cancellation: CancellationToken,
    ) -> Value {
        panic!("canonical Emit probe does not create pull streams")
    }

    fn buffered_stream(&self, _items: Vec<Value>) -> Value {
        panic!("canonical Emit probe does not create buffered streams")
    }

    fn next_with_cancel<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        Box::pin(self.next_event())
    }

    fn next_with_cancellation<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        Box::pin(self.next_event())
    }

    fn next<'a>(
        &'a self,
        _value: &'a Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        Box::pin(self.next_event())
    }

    fn cancel(&self, _value: &Value) {
        self.state.cancelled.store(true, Ordering::Release);
        self.state
            .lifetime
            .lock()
            .expect("probe stream lifetime lock")
            .take();
    }
}

#[derive(Clone)]
struct ProbeSink {
    state: Arc<ProbeState>,
}

impl fmt::Debug for ProbeSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProbeSink")
            .field("id", &self.state.id)
            .finish()
    }
}

impl ProbeSink {
    fn send_event<'a>(
        &'a self,
        event: ProbeEvent,
        count_send: bool,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move {
            if count_send {
                self.state.counts.send_starts.fetch_add(1, Ordering::AcqRel);
            }
            self.state
                .sender
                .send(event)
                .await
                .map_err(|_| StreamRuntimeError::decode("probe stream receiver dropped"))?;
            if count_send {
                self.state
                    .counts
                    .send_completions
                    .fetch_add(1, Ordering::AcqRel);
            }
            Ok(())
        })
    }
}

impl StreamSinkApi for ProbeSink {
    fn send_internal_with_cancellation<'a>(
        &'a self,
        item: StreamInternalItem,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        self.send_event(ProbeEvent::Internal(item), true)
    }

    fn send<'a>(
        &'a self,
        item: Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        self.send_event(ProbeEvent::Item(item), true)
    }

    fn send_with_cancel<'a>(
        &'a self,
        item: Value,
        _cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        self.send_event(ProbeEvent::Item(item), true)
    }

    fn send_with_cancellation<'a>(
        &'a self,
        item: Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        self.send_event(ProbeEvent::Item(item), true)
    }

    fn end<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let _ = self.state.sender.send(ProbeEvent::End).await;
            self.state
                .lifetime
                .lock()
                .expect("probe stream lifetime lock")
                .take();
        })
    }

    fn fail<'a>(
        &'a self,
        error: StreamRuntimeError,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let _ = self.state.sender.send(ProbeEvent::Fail(error)).await;
            self.state
                .lifetime
                .lock()
                .expect("probe stream lifetime lock")
                .take();
        })
    }

    fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    fn is_same_stream(&self, other: &StreamSink) -> bool {
        other
            .downcast_ref::<Self>()
            .is_some_and(|other| other.state.id == self.state.id)
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.state.cancelled)
    }

    fn cancel_signal(&self) -> StreamCancelSignal {
        StreamCancelSignal::new(NeverCancelled)
    }
}

#[derive(Debug)]
struct NeverCancelled;

impl StreamCancelSignalApi for NeverCancelled {
    fn wait_cancelled<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(std::future::pending())
    }
}

struct ServiceFixture {
    evaluator: EvaluatorFixture,
    target: RuntimeAssemblyEvalTarget,
    caller_addr: ExecutableAddr,
}

fn provider_file(items: usize) -> artifact::FileIrUnit {
    let mut file = artifact::FileIrUnit::empty("emit.provider", "source:f445h-e4r-canonical-emit");
    let expressions = (0..items)
        .map(|index| artifact::ExprIr::Literal {
            value: artifact::LiteralIr::String {
                value: format!("canonical-{index}"),
            },
        })
        .collect::<Vec<_>>();
    let mut statements = (0..items)
        .map(|index| artifact::StmtIr::Emit {
            operation: "emit".to_string(),
            value: artifact::ExprRefIr {
                expression: index as u32,
            },
        })
        .collect::<Vec<_>>();
    statements.push(artifact::StmtIr::Return { value: None });
    file.executables.push(artifact::ExecutableIr {
        kind: artifact::ExecutableKind::Function,
        symbol: "stream".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: artifact::TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![artifact::TypeRefIr::builtin("string")],
        },
        self_type: None,
        slots: artifact::SlotLayout::default(),
        may_suspend: true,
        body: artifact::ExecutableBody {
            blocks: vec![artifact::BlockIr {
                label: "entry".to_string(),
                statements: (0..statements.len())
                    .map(|statement| artifact::StmtRefIr {
                        statement: statement as u32,
                    })
                    .collect(),
            }],
            statements,
            expressions,
        },
        source_span: None,
    });
    skiff_artifact_identity::assign_file_ir_identity(&mut file)
        .expect("canonical Emit provider file identity");
    file
}

fn caller_file(service_call: &artifact::ServiceCallRef) -> artifact::FileIrUnit {
    let mut file = artifact::FileIrUnit::empty("emit.caller", "source:f445h-e4r-canonical-caller");
    file.external_refs
        .service_call_refs
        .push(service_call.clone());
    file.executables.push(artifact::ExecutableIr {
        kind: artifact::ExecutableKind::Function,
        symbol: "anchor".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: artifact::TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![artifact::TypeRefIr::builtin("string")],
        },
        self_type: None,
        slots: artifact::SlotLayout::default(),
        may_suspend: true,
        body: artifact::ExecutableBody {
            blocks: vec![artifact::BlockIr {
                label: "entry".to_string(),
                statements: vec![artifact::StmtRefIr { statement: 0 }],
            }],
            statements: vec![artifact::StmtIr::Return {
                value: Some(artifact::ExprRefIr { expression: 0 }),
            }],
            expressions: vec![artifact::ExprIr::Call {
                call: artifact::CallIr {
                    target: artifact::CallTargetIr::ServiceCall {
                        service_call_ref_index: artifact::ServiceCallRefIndex::new(0),
                    },
                    site: site(),
                    args: Vec::new(),
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                },
            }],
        },
        source_span: None,
    });
    skiff_artifact_identity::assign_file_ir_identity(&mut file)
        .expect("canonical Emit caller file identity");
    file
}

fn detached_plan(
    owner: artifact::BoundaryValueOwner,
    lifetime: artifact::BoundaryValueLifetime,
) -> artifact::BoundaryValuePlan {
    artifact::BoundaryValuePlan::Linkable {
        carrier: artifact::BoundaryValueCarrier::DetachedValueGraph,
        encoding: artifact::BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime,
    }
}

fn service_contract() -> artifact::ServiceContract {
    let operation = artifact::ContractOperationId::new(OPERATION_ID);
    artifact::ServiceContract {
        schema_version: artifact::SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: SERVICE_ID.to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: artifact::ServiceProtocolIdentity::new(
            "protocol:f445h-e4r-canonical-emit",
        ),
        operations: BTreeMap::from([(
            operation.clone(),
            artifact::BoundaryOperationDescriptor {
                operation_id: operation.clone(),
                stable_key: "stream".to_string(),
                contract: artifact::BoundaryOperationContract {
                    parameters: Vec::new(),
                    return_value: artifact::BoundaryReturn {
                        ty: artifact::ContractTypeRef::builtin("void"),
                        value_plan: detached_plan(
                            artifact::BoundaryValueOwner::Provider,
                            artifact::BoundaryValueLifetime::Call,
                        ),
                    },
                    stream: artifact::BoundaryStreamContract::ServerStream {
                        item_type: artifact::ContractTypeRef::builtin("string"),
                        item_value_plan: detached_plan(
                            artifact::BoundaryValueOwner::Provider,
                            artifact::BoundaryValueLifetime::Stream,
                        ),
                    },
                    callbacks: artifact::BoundaryCallbackContract::None,
                    effect_guarantee: artifact::BoundaryEffectGuarantee {
                        detached_parameters: true,
                        detached_return: true,
                        detached_error: true,
                        no_caller_reachable_mutation: true,
                        no_caller_value_escape: true,
                        no_same_heap_identity: true,
                    },
                },
            },
        )]),
        package_type_requirements: Vec::new(),
        diagnostic_text: artifact::ContractDiagnosticText {
            service: "canonical Emit fixture".to_string(),
            operations: BTreeMap::from([(operation, "stream".to_string())]),
            types: BTreeMap::new(),
        },
    }
}

fn contract_ref(contract: &artifact::ServiceContract) -> artifact::ServiceContractRef {
    artifact::ServiceContractRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        service_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

fn contract_requirement(contract: &artifact::ServiceContractRef) -> artifact::ContractRequirement {
    artifact::ContractRequirement {
        alias: "emit".to_string(),
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        expected_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

fn activation_identity(
    assembly_identity: artifact::AssemblyIdentity,
    service_id: &str,
    revision: &str,
) -> ActivationIdentity {
    ActivationIdentity {
        assembly_identity,
        assembly_generation: 1,
        runtime_replica_id: "replica:f445h-e4r-canonical-emit".to_string(),
        deployment: artifact::ServiceDeploymentRef {
            service_id: service_id.to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: artifact::DeploymentRevision::new(revision),
            deployment_artifact_identity: artifact::DeploymentArtifactIdentity::new(format!(
                "deployment:f445h-e4r:{revision}"
            )),
        },
    }
}

struct Resolver {
    activations: BTreeMap<ActivationId, Arc<ActivationContext>>,
    contract: Arc<artifact::ServiceContract>,
    contract_ref: artifact::ServiceContractRef,
    operation: artifact::ContractOperationId,
    provider: ActivationId,
    target: artifact::OperationTargetRef,
}

impl RuntimeAssemblyEvalResolver for Resolver {
    fn activation(&self, id: &ActivationId) -> Option<Arc<ActivationContext>> {
        self.activations.get(id).cloned()
    }

    fn activation_by_opaque_id(&self, id: &str) -> Option<Arc<ActivationContext>> {
        self.activations
            .values()
            .find(|activation| activation.activation_id().as_str() == id)
            .cloned()
    }

    fn contract(
        &self,
        contract: &artifact::ServiceContractRef,
    ) -> Option<Arc<artifact::ServiceContract>> {
        (contract == &self.contract_ref).then(|| Arc::clone(&self.contract))
    }

    fn admitted_schema_records(
        &self,
        contract: &artifact::ServiceContractRef,
    ) -> Option<crate::AdmittedPackageSchemaRecords> {
        (contract == &self.contract_ref).then(|| Arc::new(BTreeMap::new()))
    }

    fn operation_target(
        &self,
        activation: &ActivationId,
        operation: &artifact::ContractOperationId,
    ) -> Option<artifact::OperationTargetRef> {
        (activation == &self.provider && operation == &self.operation).then(|| self.target.clone())
    }
}

fn fixture(items: usize) -> ServiceFixture {
    let contract = Arc::new(service_contract());
    let contract_ref = contract_ref(&contract);
    let operation = artifact::ContractOperationId::new(OPERATION_ID);
    let service_call = artifact::ServiceCallRef {
        service_requirement_slot: 0,
        contract_operation_id: operation.clone(),
        expected_protocol_identity: contract_ref.service_protocol_identity.clone(),
    };
    let caller_file = caller_file(&service_call);
    let provider_file = provider_file(items);

    let requirement = contract_requirement(&contract_ref);
    let mut caller_package = super::callback_matrix::private_package(CALLER_PACKAGE, &caller_file);
    caller_package
        .contract_requirements
        .push(requirement.clone());
    caller_package
        .service_requirements
        .push(artifact::ServiceRequirement {
            contract_requirement: requirement,
            service_binding_slot: 0,
            used_operations: BTreeSet::from([operation.clone()]),
        });
    caller_package.service_call_refs.push(service_call);
    skiff_artifact_identity::assign_package_artifact_identities(&mut caller_package)
        .expect("canonical Emit caller package identities");
    let caller_ref = super::callback_matrix::package_ref(&caller_package);

    let mut provider_package =
        super::callback_matrix::private_package(PROVIDER_PACKAGE, &provider_file);
    skiff_artifact_identity::assign_package_artifact_identities(&mut provider_package)
        .expect("canonical Emit provider package identities");
    let provider_callable = artifact::PackageCallableId::new(OPERATION_ID);
    let provider_target = artifact::OperationTargetRef {
        file_ref: super::callback_matrix::file_ref(&provider_file),
        executable_index: 0,
        callable_abi_id: provider_callable.to_string(),
        callable_kind: artifact::OperationCallableKind::PublicFunction,
    };
    provider_package.callable_links.insert(
        provider_callable.clone(),
        artifact::PackageCallableLinkFact {
            callable_id: provider_callable,
            target: provider_target.clone(),
        },
    );
    let provider_ref = super::callback_matrix::package_ref(&provider_package);
    let assembly_identity = artifact::AssemblyIdentity::new("assembly:f445h-e4r-canonical-emit");
    let assembly = artifact::RuntimeAssembly {
        schema_version: artifact::RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: assembly_identity.clone(),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![caller_ref.clone(), provider_ref.clone()],
        package_link_plan: artifact::CanonicalPackageLinkPlan {
            code_slots: vec![
                artifact::PackageCodeSlot {
                    package: caller_ref.clone(),
                },
                artifact::PackageCodeSlot {
                    package: provider_ref.clone(),
                },
            ],
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let image = crate::test_support::link_package_fixture(
        assembly,
        vec![
            (caller_package, vec![caller_file.clone()]),
            (provider_package, vec![provider_file.clone()]),
        ],
    );
    let target = provider_target;
    let provider = ActivationContext::new(
        activation_identity(
            assembly_identity.clone(),
            SERVICE_ID,
            "canonical-emit-provider-r1",
        ),
        provider_ref.package_build_id.clone(),
        Vec::new(),
    )
    .expect("canonical Emit provider activation");
    let binding = ActivationServiceBinding::new(
        artifact::ServiceRequirementKey {
            caller_package_build_id: caller_ref.package_build_id.clone(),
            service_requirement_slot: 0,
        },
        provider.activation_id().clone(),
        contract_ref.clone(),
        vec![operation.clone()],
    )
    .expect("canonical Emit service binding");
    let caller = ActivationContext::new(
        activation_identity(
            assembly_identity,
            CALLER_PACKAGE,
            "canonical-emit-caller-r1",
        ),
        caller_ref.package_build_id.clone(),
        vec![binding],
    )
    .expect("canonical Emit caller activation");
    let activations = BTreeMap::from([
        (caller.activation_id().clone(), Arc::clone(&caller)),
        (provider.activation_id().clone(), Arc::clone(&provider)),
    ]);
    let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(Resolver {
        activations,
        contract,
        contract_ref,
        operation,
        provider: provider.activation_id().clone(),
        target,
    });
    let instruction = image
        .resolve_activation_relative_service_call(
            &caller_ref.package_build_id,
            &caller_file.file_ir_identity,
            artifact::ServiceCallRefIndex::new(0),
        )
        .expect("canonical Emit service instruction");
    let request =
        RequestActivationContext::begin(caller).expect("canonical Emit request activation");
    let target = RuntimeAssemblyEvalTarget::new(image, request, resolver)
        .expect("canonical Emit eval target");
    let evaluator = EvaluatorFixture::new(
        vec![LinkedExprIr::Call {
            call: call(
                LinkedCallTarget::ActivationRelativeService {
                    instruction: instruction.clone(),
                },
                Vec::new(),
            ),
        }],
        vec![LinkedStmtIr::Return {
            value: Some(ExprRefIr { expression: 0 }),
        }],
        SlotLayoutIr::default(),
    );
    ServiceFixture {
        evaluator,
        target,
        caller_addr: ExecutableAddr::package(0, 0, 0),
    }
}

async fn start_stream(
    fixture: &ServiceFixture,
    runtime: StreamRuntime,
) -> (Value, ActorExecutionFrame) {
    let (frame, mut heap) = fixture.evaluator.actor_frame().await;
    let mut env = Env::new();
    let context = program_context_with_stream(
        &fixture.evaluator.interpreter,
        test_runtime::actor_context(),
        test_runtime::file_context(),
        DbCapabilityContext::unavailable(),
        runtime,
    )
    .with_websocket_capability_rebinder(test_runtime::websocket_rebinder())
    .with_runtime_assembly_target(fixture.target.clone());
    let mut eval = fixture.evaluator.eval_context_with(
        context,
        frame.clone(),
        &mut heap,
        &mut env,
        &fixture.caller_addr,
    );
    let value = eval
        .eval_program_expr_ref(ExprRefIr { expression: 0 })
        .await
        .expect("canonical service stream");
    let value = crate::runtime_ops::runtime_to_wire(value.value(), &*eval.heap)
        .expect("canonical stream wire handle");
    drop(eval);
    assert!(
        frame.has_execution_lease(),
        "frozen activation setup must reacquire its Actor segment"
    );
    frame
        .clone()
        .finish(heap)
        .expect("finish canonical Emit caller frame");
    (value, frame)
}

async fn wait_for_counts(counts: &ProbeCounts, starts: usize, completions: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if counts.send_starts.load(Ordering::Acquire) == starts
                && counts.send_completions.load(Ordering::Acquire) == completions
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("canonical Emit counters reach expected state");
}

#[tokio::test]
async fn f445h_e4r_spine_emit_canonical_wire_ready_completes_first_poll() {
    let fixture = fixture(1);
    let (runtime, counts) = ProbeRuntime::new();
    let (stream, _frame) = start_stream(&fixture, runtime.clone()).await;

    wait_for_counts(&counts, 1, 1).await;
    assert!(matches!(
        runtime.next(&stream).await.expect("canonical item"),
        StreamPoll::Item(Value::String(value)) if value == "canonical-0"
    ));
    assert!(matches!(
        runtime.next(&stream).await.expect("canonical end"),
        StreamPoll::End
    ));
}

#[tokio::test]
async fn f445h_e4r_spine_emit_canonical_wire_pending_resumes_same_send_once() {
    let fixture = fixture(2);
    let (runtime, counts) = ProbeRuntime::new();
    let (stream, _frame) = start_stream(&fixture, runtime.clone()).await;

    wait_for_counts(&counts, 2, 1).await;
    assert!(matches!(
        runtime.next(&stream).await.expect("first canonical item"),
        StreamPoll::Item(Value::String(value)) if value == "canonical-0"
    ));
    wait_for_counts(&counts, 2, 2).await;
    assert!(matches!(
        runtime.next(&stream).await.expect("second canonical item"),
        StreamPoll::Item(Value::String(value)) if value == "canonical-1"
    ));
    assert!(matches!(
        runtime.next(&stream).await.expect("canonical end"),
        StreamPoll::End
    ));
}
