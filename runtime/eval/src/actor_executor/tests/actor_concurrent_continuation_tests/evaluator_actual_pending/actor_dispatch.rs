use super::*;

enum ActorReply {
    Ready(CapabilityResult<ActorInvocationOutcome>),
    Pending(oneshot::Receiver<CapabilityResult<ActorInvocationOutcome>>),
}

#[derive(Clone)]
struct RecordingActor {
    state: Arc<RecordingActorState>,
}

struct RecordingActorState {
    reply: Mutex<Option<ActorReply>>,
    starts: AtomicUsize,
    drops_before_completion: AtomicUsize,
}

impl RecordingActor {
    fn ready(outcome: CapabilityResult<ActorInvocationOutcome>) -> Self {
        Self {
            state: Arc::new(RecordingActorState {
                reply: Mutex::new(Some(ActorReply::Ready(outcome))),
                starts: AtomicUsize::new(0),
                drops_before_completion: AtomicUsize::new(0),
            }),
        }
    }

    fn pending() -> (
        Self,
        oneshot::Sender<CapabilityResult<ActorInvocationOutcome>>,
    ) {
        let (sender, receiver) = oneshot::channel();
        (
            Self {
                state: Arc::new(RecordingActorState {
                    reply: Mutex::new(Some(ActorReply::Pending(receiver))),
                    starts: AtomicUsize::new(0),
                    drops_before_completion: AtomicUsize::new(0),
                }),
            },
            sender,
        )
    }

    fn starts(&self) -> usize {
        self.state.starts.load(Ordering::Acquire)
    }
}

struct PendingActorWait {
    state: Arc<RecordingActorState>,
    completed: bool,
}

impl Drop for PendingActorWait {
    fn drop(&mut self) {
        if !self.completed {
            self.state
                .drops_before_completion
                .fetch_add(1, Ordering::AcqRel);
        }
    }
}

impl ActorCapabilityApi for RecordingActor {
    fn owned(&self) -> OwnedActorCapabilityContext {
        ActorCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> ActorCapabilityContext<'_> {
        ActorCapabilityContext::new(self.clone())
    }

    fn runtime_id(&self) -> &str {
        "runtime:f445h-e4r"
    }

    fn service_id(&self) -> &str {
        "skiff.run/counter"
    }

    fn service_version(&self) -> &str {
        "1.0.0"
    }

    fn request_id(&self) -> &str {
        "request:f445h-e4r"
    }

    fn request_target(&self) -> &str {
        "actor.f445h-e4r"
    }

    fn request_build_id(&self) -> &str {
        "build:f445h-e4r"
    }

    fn spawn_service_protocol_identity(&self) -> &str {
        "spawn-protocol:f445h-e4r"
    }

    fn request_service_protocol_identity(&self) -> &str {
        "request-protocol:f445h-e4r"
    }

    fn operation_service_protocol_identity(&self) -> Option<&str> {
        None
    }

    fn activation_identity(&self) -> Option<&ActivationIdentityControl> {
        None
    }

    fn trace_id(&self) -> Option<&str> {
        None
    }

    fn get_or_create_actor<'a>(
        &'a self,
        _request: ActorGetOrCreateControlRequest,
        _bootstrap_payload: Vec<u8>,
    ) -> CapabilityFuture<'a, ActorRef> {
        Box::pin(async { Err(CapabilityError::unsupported("not used")) })
    }

    fn replace_actor<'a>(
        &'a self,
        _request: ActorReplaceControlRequest,
        _bootstrap_payload: Vec<u8>,
    ) -> CapabilityFuture<'a, ActorRef> {
        Box::pin(async { Err(CapabilityError::unsupported("not used")) })
    }

    fn find_actor<'a>(
        &'a self,
        _request: ActorFindControlRequest,
    ) -> CapabilityFuture<'a, Option<ActorRef>> {
        Box::pin(async { Err(CapabilityError::unsupported("not used")) })
    }

    fn remove_actor<'a>(
        &'a self,
        _request: ActorRemoveControlRequest,
    ) -> CapabilityFuture<'a, bool> {
        Box::pin(async { Err(CapabilityError::unsupported("not used")) })
    }

    fn submit_spawn<'a>(
        &'a self,
        _request: SpawnSubmitControlRequest,
        _args_payload: Vec<u8>,
    ) -> CapabilityFuture<'a, ()> {
        Box::pin(async { Err(CapabilityError::unsupported("not used")) })
    }

    fn invoke_actor<'a>(
        &'a self,
        _request: ActorInvocationRequest,
    ) -> CapabilityFuture<'a, ActorInvocationOutcome> {
        self.state.starts.fetch_add(1, Ordering::AcqRel);
        let reply = self
            .state
            .reply
            .lock()
            .expect("Actor reply lock")
            .take()
            .expect("Actor invocation starts once");
        match reply {
            ActorReply::Ready(outcome) => Box::pin(async move { outcome }),
            ActorReply::Pending(receiver) => {
                let state = Arc::clone(&self.state);
                Box::pin(async move {
                    let mut guard = PendingActorWait {
                        state,
                        completed: false,
                    };
                    let outcome = receiver.await.map_err(|_| {
                        CapabilityError::provider_unavailable(
                            "actor.f445h-e4r",
                            "reply channel closed",
                        )
                    })?;
                    guard.completed = true;
                    outcome
                })
            }
        }
    }
}

fn actor_dispatch_fixture() -> EvaluatorFixture {
    EvaluatorFixture::new(
        vec![
            LinkedExprIr::LoadSlot { slot: 0 },
            LinkedExprIr::Literal {
                value: LiteralIr::Number {
                    value: serde_json::Number::from(4),
                },
            },
            LinkedExprIr::Call {
                call: call(
                    LinkedCallTarget::ActorDispatch {
                        plan: skiff_runtime_linked_program::LinkedActorMethodDispatchPlan {
                            declaration_owner: owner(),
                            actor_abi_identity: abi(),
                            actor_implementation_identity: implementation(),
                            method_identity: method_identity(),
                        },
                    },
                    vec![0, 1],
                ),
            },
        ],
        vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 2 },
            },
            LinkedStmtIr::Return { value: None },
        ],
        SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "receiver".to_string(),
                kind: "parameter".to_string(),
            }],
            frame_size: 1,
        },
    )
}

fn actor_dispatch_env(fixture: &EvaluatorFixture) -> Env {
    let mut env = Env::for_program_executable(
        fixture.executable(),
        Some(fixture.file.module_path.clone()),
        0,
    )
    .expect("Actor dispatch env");
    env.declare_binding(
        "receiver",
        Some(0),
        RuntimeValue::ActorRef(ActorRef::new(
            "skiff.run/counter",
            "actors.Counter",
            "builtin:string",
            ACTOR_BOOTSTRAP_ENCODING_V1,
            br#""counter-remote""#.to_vec(),
            "sha256:f445h-e4r-counter",
            Some(7),
        )),
    )
    .expect("Actor receiver binding");
    env
}

fn actor_return(value: i64) -> ActorInvocationOutcome {
    ActorInvocationOutcome::Returned(
        canonical_json_bytes(&json!(value)).expect("Actor return payload"),
    )
}

#[tokio::test]
async fn f445h_e4r_spine_actor_dispatch_ready_keeps_actor_segment() {
    let actor = RecordingActor::ready(Ok(actor_return(11)));
    let fixture = actor_dispatch_fixture();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = actor_dispatch_env(&fixture);
    let addr = executable_addr();
    let context = program_context_with(
        &fixture.interpreter,
        ActorCapabilityContext::new(actor.clone()),
        test_runtime::outbound_context(),
        test_runtime::file_context(),
        DbCapabilityContext::unavailable(),
    );

    fixture
        .eval_context_with(context, frame.clone(), &mut heap, &mut env, &addr)
        .exec_program_executable()
        .await
        .expect("first-Ready Actor dispatch");

    assert_eq!(actor.starts(), 1);
    assert!(
        frame.has_execution_lease(),
        "first-Ready Actor dispatch must keep the current segment"
    );
    frame.finish(heap).expect("finish Actor Ready frame");
}

#[tokio::test]
async fn f445h_e4r_spine_actor_dispatch_pending_reacquires_before_finalize() {
    let (actor, release) = RecordingActor::pending();
    let fixture = actor_dispatch_fixture();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = actor_dispatch_env(&fixture);
    let addr = executable_addr();
    let context = program_context_with(
        &fixture.interpreter,
        ActorCapabilityContext::new(actor.clone()),
        test_runtime::outbound_context(),
        test_runtime::file_context(),
        DbCapabilityContext::unavailable(),
    );
    let mut eval = fixture.eval_context_with(context, frame.clone(), &mut heap, &mut env, &addr);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(actor.starts(), 1);
    assert!(!frame.has_execution_lease());
    release
        .send(Ok(actor_return(12)))
        .expect("release pending Actor invocation");
    execution.await.expect("pending Actor dispatch");
    drop(eval);
    assert!(
        frame.has_execution_lease(),
        "Actor completion must reacquire before return decode/finalize"
    );
    frame.finish(heap).expect("finish Actor Pending frame");
}
