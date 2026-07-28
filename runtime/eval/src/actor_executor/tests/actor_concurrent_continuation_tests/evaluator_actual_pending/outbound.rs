use super::*;

const OUTBOUND_SERVICE_ID: &str = "skiff.test/evaluator-provider";
const OUTBOUND_PROTOCOL: &str = "protocol:f445h-e4r";
const OUTBOUND_BUILD: &str = "build:f445h-e4r";
const OUTBOUND_ALIAS: &str = "provider";
const LEGACY_OPERATION_ABI: &str = "operation:f445h-e4r:legacy";
const REMOTE_OPERATION_ABI: &str = "operation:f445h-e4r:remote";
const REMOTE_INTERFACE_ABI: &str = "interface:f445h-e4r:remote";
const REMOTE_METHOD_ABI: &str = "method:f445h-e4r:remote";

#[derive(Clone)]
struct RecordingOutbound {
    state: Arc<RecordingOutboundState>,
}

struct RecordingOutboundState {
    dependencies: Vec<ServiceDependencyConstraint>,
    registry: OutboundRequestRegistry,
    starts: AtomicUsize,
    response_sender: Mutex<Option<tokio::sync::mpsc::UnboundedSender<OutboundResponse>>>,
    buffered: Mutex<Option<OutboundResponse>>,
}

#[derive(Debug)]
struct PullSetupRuntime;

impl StreamRuntimeApi for PullSetupRuntime {
    fn channel_stream(&self) -> (Value, StreamSink) {
        unreachable!("serverStream setup only creates a pull source")
    }

    fn channel_stream_with_lifetime(&self, _lifetime: StreamLifetimeGuard) -> (Value, StreamSink) {
        unreachable!("serverStream setup only creates a pull source")
    }

    fn pull_stream_with_cancellation(
        &self,
        _source: Box<dyn StreamPullSource>,
        _cancellation: CancellationToken,
    ) -> Value {
        skiff_runtime_boundary::stream::stream_value("f445h-e4r-server-stream")
    }

    fn buffered_stream(&self, _items: Vec<Value>) -> Value {
        unreachable!("serverStream setup does not create buffered streams")
    }

    fn next_with_cancel<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        unreachable!("serverStream setup does not poll its returned stream")
    }

    fn next_with_cancellation<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        unreachable!("serverStream setup does not poll its returned stream")
    }

    fn next<'a>(
        &'a self,
        _value: &'a Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        unreachable!("serverStream setup does not poll its returned stream")
    }

    fn cancel(&self, _value: &Value) {}
}

fn pull_setup_runtime() -> StreamRuntime {
    StreamRuntime::new(PullSetupRuntime)
}

impl RecordingOutbound {
    fn pending(dependencies: Vec<ServiceDependencyConstraint>) -> Self {
        Self {
            state: Arc::new(RecordingOutboundState {
                dependencies,
                registry: OutboundRequestRegistry::default(),
                starts: AtomicUsize::new(0),
                response_sender: Mutex::new(None),
                buffered: Mutex::new(None),
            }),
        }
    }

    fn buffered(
        dependencies: Vec<ServiceDependencyConstraint>,
        response: OutboundResponse,
    ) -> Self {
        let this = Self::pending(dependencies);
        *this.state.buffered.lock().expect("outbound buffered lock") = Some(response);
        this
    }

    fn starts(&self) -> usize {
        self.state.starts.load(Ordering::Acquire)
    }

    fn send(&self, response: OutboundResponse) {
        self.state
            .response_sender
            .lock()
            .expect("outbound response sender lock")
            .as_ref()
            .expect("outbound request must have started")
            .send(response)
            .expect("outbound response receiver must remain live");
    }
}

impl OutboundServiceApi for RecordingOutbound {
    fn service_dependencies(&self) -> &[ServiceDependencyConstraint] {
        &self.state.dependencies
    }

    fn test_effects_enabled(&self) -> bool {
        false
    }

    fn test_effect_doubles(&self) -> HashMap<String, Vec<RequestEffectDoubleControl>> {
        HashMap::new()
    }

    fn request_heap(&self) -> RequestHeap {
        RequestHeap::default()
    }

    fn effective_timeout_ms(&self, _operation_timeout_ms: Option<u64>) -> Option<u64> {
        None
    }

    fn outbound_deadline_error(&self) -> RuntimeError {
        RuntimeError::Cancelled
    }

    fn start_request(
        &self,
        _start: crate::capabilities::OutboundServiceRequestStart,
        _payload: Vec<u8>,
    ) -> crate::error::Result<OutboundStartedRequest> {
        let ordinal = self.state.starts.fetch_add(1, Ordering::AcqRel) + 1;
        let request_id = format!("f445h-e4r-outbound-{ordinal}");
        let (sender, response_rx) = tokio::sync::mpsc::unbounded_channel();
        let cancel_sender: OutboundRequestCancelSender =
            Arc::new(|_, _| Ok::<(), OutboundRequestCancelSendError>(()));
        let lease = self
            .state
            .registry
            .insert_with_lease(
                request_id.clone(),
                sender.clone(),
                Some(cancel_sender),
                "f445h_e4r_wait_dropped",
            )
            .expect("outbound request lease");
        *self
            .state
            .response_sender
            .lock()
            .expect("outbound response sender lock") = Some(sender.clone());
        if let Some(response) = self
            .state
            .buffered
            .lock()
            .expect("outbound buffered lock")
            .take()
        {
            sender.send(response).expect("buffered outbound response");
        }
        Ok(OutboundStartedRequest {
            request_id,
            response_rx,
            lease,
        })
    }

    fn receive_response<'a>(
        &'a self,
        _lease: &'a skiff_runtime_capability_context::OutboundRequestLease,
        target: &'a str,
        receiver: &'a mut OutboundResponseReceiver,
        _timeout_ms: Option<u64>,
    ) -> EvalCapabilityFuture<'a, OutboundResponse> {
        Box::pin(async move {
            receiver
                .recv()
                .await
                .ok_or_else(|| RuntimeError::ProviderUnavailable {
                    target: target.to_string(),
                    reason: "response channel closed".to_string(),
                })
        })
    }

    fn cancel_signal(&self) -> CancellationToken {
        CancellationToken::new()
    }
}

fn string_plan() -> RuntimeTypePlan {
    RuntimeTypePlan::synthetic_named_builtin("string", RuntimeTypeNode::String, Vec::new())
}

fn encoded_string_response(value: &str) -> OutboundResponse {
    let plan = string_plan();
    let mut heap = RequestHeap::default();
    let value = RuntimeBoundaryCodec::new(
        &plan,
        BoundaryUse::NativeReturn,
        "f445h-e4r outbound response",
    )
    .from_wire_json(&json!(value), &mut heap)
    .expect("outbound response should materialize");
    let payload = encode_payload_plan(
        &value,
        &plan,
        &PayloadBoundary::cross_service(
            PayloadBoundaryKind::InboundServiceCall,
            OUTBOUND_SERVICE_ID,
        ),
        &mut heap,
    )
    .expect("outbound response should encode");
    OutboundResponse::End { payload }
}

fn operation_ref(remote: bool) -> OperationAbiRef {
    if remote {
        OperationAbiRef {
            operation_abi_id: REMOTE_OPERATION_ABI.to_string(),
            kind: PublicationOperationKind::PublicInstanceMethod,
            public_path: "reader.read".to_string(),
            public_instance_key: Some("reader".to_string()),
            interface: Some(InterfaceInstantiationRef {
                interface_abi_id: REMOTE_INTERFACE_ABI.to_string(),
                canonical_type_args: Vec::new(),
            }),
            method_abi_id: Some(REMOTE_METHOD_ABI.to_string()),
            display_name: "reader.read".to_string(),
        }
    } else {
        OperationAbiRef {
            operation_abi_id: LEGACY_OPERATION_ABI.to_string(),
            kind: PublicationOperationKind::PublicFunction,
            public_path: "read".to_string(),
            public_instance_key: None,
            interface: None,
            method_abi_id: None,
            display_name: "read".to_string(),
        }
    }
}

fn outbound_dependency(mode: &str, remote: bool) -> ServiceDependencyConstraint {
    let operation = operation_ref(remote);
    let return_type = if mode == "serverStream" {
        TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::builtin("string")],
        }
    } else {
        TypeRefIr::builtin("string")
    };
    let operation_abi = PublicationOperationAbi {
        operation: operation.clone(),
        public_signature: CanonicalPublicCallableSignature {
            params: Vec::new(),
            return_type,
            may_suspend: true,
        },
        schema_closure: Vec::new(),
        stream_effect_throw_config: BTreeMap::new(),
    };
    let mut publication = PublicationAbiUnit::empty(OUTBOUND_SERVICE_ID, "1.0.0", "abi:f445h-e4r");
    publication.operation_exports.push(operation.clone());
    publication.operation_abi.push(operation_abi);
    if remote {
        publication
            .public_instances
            .push(PublicationPublicInstanceExport {
                public_instance_key: "reader".to_string(),
                interfaces: vec![InterfaceInstantiationRef {
                    interface_abi_id: REMOTE_INTERFACE_ABI.to_string(),
                    canonical_type_args: Vec::new(),
                }],
                source_call_method_index: vec![SourceCallMethodIndexEntry {
                    method_name: "read".to_string(),
                    operation: operation.clone(),
                }],
                method_operations: vec![operation],
            });
    }
    ServiceDependencyConstraint {
        id: OUTBOUND_SERVICE_ID.to_string(),
        version: "1.0.0".to_string(),
        alias: OUTBOUND_ALIAS.to_string(),
        build_id: OUTBOUND_BUILD.to_string(),
        service_protocol_identity: OUTBOUND_PROTOCOL.to_string(),
        publication_abi: publication,
    }
}

fn legacy_outbound_fixture(_mode: &str) -> EvaluatorFixture {
    let symbol = ServiceDependencySymbolRef {
        dependency_ref: OUTBOUND_ALIAS.to_string(),
        operation: operation_ref(false),
    };
    EvaluatorFixture::new(
        vec![LinkedExprIr::Call {
            call: call(
                LinkedCallTarget::ServiceDependencySymbol { symbol },
                Vec::new(),
            ),
        }],
        vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 0 },
            },
            LinkedStmtIr::Return { value: None },
        ],
        SlotLayoutIr::default(),
    )
}

fn remote_outbound_fixture() -> EvaluatorFixture {
    let interface = LinkedInterfaceInstantiationRef {
        interface_abi_id: REMOTE_INTERFACE_ABI.to_string(),
        canonical_type_args: Vec::new(),
    };
    let operations = LinkedRemoteOperationTablePlanIr {
        interface: interface.clone(),
        slots: vec![LinkedRemoteOperationSlotPlanIr {
            slot: 0,
            method_abi_id: REMOTE_METHOD_ABI.to_string(),
            signature: skiff_runtime_linked_program::LinkedInterfaceMethodSlotSignatureIr {
                params: Vec::new(),
                return_type: string_type(),
            },
            operation_abi_id: REMOTE_OPERATION_ABI.to_string(),
        }],
    };
    EvaluatorFixture::new(
        vec![
            LinkedExprIr::Literal {
                value: LiteralIr::Null,
            },
            LinkedExprIr::InterfaceBox {
                value: ExprRefIr { expression: 0 },
                interface: interface.clone(),
                source: skiff_runtime_linked_program::LinkedBoxSourceIr::Remote {
                    dependency_ref: OUTBOUND_ALIAS.to_string(),
                    public_instance_key: "reader".to_string(),
                    operations,
                    callee_protocol_identity: OUTBOUND_PROTOCOL.to_string(),
                },
            },
            LinkedExprIr::Call {
                call: call(
                    LinkedCallTarget::InterfaceMethod {
                        interface,
                        method_abi_id: REMOTE_METHOD_ABI.to_string(),
                        slot: 0,
                    },
                    vec![1],
                ),
            },
        ],
        vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 2 },
            },
            LinkedStmtIr::Return { value: None },
        ],
        SlotLayoutIr::default(),
    )
}

#[tokio::test]
async fn f445h_e4r_spine_remote_interface_ready_keeps_actor_segment() {
    let dependency = outbound_dependency("unary", true);
    let outbound = RecordingOutbound::buffered(vec![dependency], encoded_string_response("ready"));
    let fixture = remote_outbound_fixture();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    let addr = executable_addr();
    let context = program_context_with(
        &fixture.interpreter,
        test_runtime::actor_context(),
        OutboundServiceContext::new(outbound.clone()),
        test_runtime::file_context(),
        DbCapabilityContext::unavailable(),
    );

    fixture
        .eval_context_with(context, frame.clone(), &mut heap, &mut env, &addr)
        .exec_program_executable()
        .await
        .expect("buffered remote interface response");

    assert_eq!(outbound.starts(), 1);
    assert!(
        frame.has_execution_lease(),
        "first-Ready remote wait must keep the Actor segment"
    );
    frame.finish(heap).expect("finish remote Ready frame");
}

#[tokio::test]
async fn f445h_e4r_spine_remote_interface_pending_reacquires_before_finalize() {
    let dependency = outbound_dependency("unary", true);
    let outbound = RecordingOutbound::pending(vec![dependency]);
    let fixture = remote_outbound_fixture();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    let addr = executable_addr();
    let context = program_context_with(
        &fixture.interpreter,
        test_runtime::actor_context(),
        OutboundServiceContext::new(outbound.clone()),
        test_runtime::file_context(),
        DbCapabilityContext::unavailable(),
    );
    let mut eval = fixture.eval_context_with(context, frame.clone(), &mut heap, &mut env, &addr);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(outbound.starts(), 1);
    assert!(!frame.has_execution_lease());
    outbound.send(encoded_string_response("pending"));
    execution.await.expect("pending remote interface response");
    drop(eval);
    assert!(
        frame.has_execution_lease(),
        "remote completion must reacquire before response decode/finalize"
    );
    frame.finish(heap).expect("finish remote Pending frame");
}

#[tokio::test]
async fn f445h_e4r_spine_legacy_unary_pending_and_server_stream_ready() {
    let unary_dependency = outbound_dependency("unary", false);
    let unary_outbound = RecordingOutbound::pending(vec![unary_dependency]);
    let unary = legacy_outbound_fixture("unary");
    let (unary_frame, mut unary_heap) = unary.actor_frame().await;
    let mut unary_env = Env::new();
    let addr = executable_addr();
    let unary_context = program_context_with(
        &unary.interpreter,
        test_runtime::actor_context(),
        OutboundServiceContext::new(unary_outbound.clone()),
        test_runtime::file_context(),
        DbCapabilityContext::unavailable(),
    );
    let mut unary_eval = unary.eval_context_with(
        unary_context,
        unary_frame.clone(),
        &mut unary_heap,
        &mut unary_env,
        &addr,
    );
    let mut unary_execution = Box::pin(unary_eval.exec_program_executable());

    assert!(matches!(
        first_poll(unary_execution.as_mut()),
        Poll::Pending
    ));
    assert!(!unary_frame.has_execution_lease());
    unary_outbound.send(encoded_string_response("legacy-pending"));
    unary_execution.await.expect("legacy unary response");
    drop(unary_eval);
    assert!(unary_frame.has_execution_lease());
    unary_frame
        .finish(unary_heap)
        .expect("finish legacy unary frame");

    let stream_dependency = outbound_dependency("serverStream", false);
    let stream_outbound = RecordingOutbound::pending(vec![stream_dependency]);
    let stream = legacy_outbound_fixture("serverStream");
    let (stream_frame, mut stream_heap) = stream.actor_frame().await;
    let mut stream_env = Env::new();
    let stream_context = program_context_with_stream(
        &stream.interpreter,
        test_runtime::actor_context(),
        OutboundServiceContext::new(stream_outbound.clone()),
        test_runtime::file_context(),
        DbCapabilityContext::unavailable(),
        pull_setup_runtime(),
    );

    stream
        .eval_context_with(
            stream_context,
            stream_frame.clone(),
            &mut stream_heap,
            &mut stream_env,
            &addr,
        )
        .exec_program_executable()
        .await
        .expect("serverStream setup is synchronous");

    assert_eq!(stream_outbound.starts(), 1);
    assert!(
        stream_frame.has_execution_lease(),
        "serverStream setup must not cut the Actor segment"
    );
    stream_frame
        .finish(stream_heap)
        .expect("finish serverStream frame");
}
