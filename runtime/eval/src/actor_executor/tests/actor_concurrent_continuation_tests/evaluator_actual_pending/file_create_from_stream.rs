use super::*;
use skiff_runtime_capability_context::OwnedExecutionControl;

#[derive(Clone)]
struct RecordingFile {
    state: Arc<RecordingFileState>,
}

struct RecordingFileState {
    starts: AtomicUsize,
    completions: AtomicUsize,
    drops_before_completion: AtomicUsize,
    scope_pending: AtomicBool,
}

impl RecordingFile {
    fn new() -> Self {
        Self {
            state: Arc::new(RecordingFileState {
                starts: AtomicUsize::new(0),
                completions: AtomicUsize::new(0),
                drops_before_completion: AtomicUsize::new(0),
                scope_pending: AtomicBool::new(false),
            }),
        }
    }

    fn hold_pending_until_scope_terminal(&self) {
        self.state.scope_pending.store(true, Ordering::Release);
    }

    fn starts(&self) -> usize {
        self.state.starts.load(Ordering::Acquire)
    }

    fn completions(&self) -> usize {
        self.state.completions.load(Ordering::Acquire)
    }

    fn drops_before_completion(&self) -> usize {
        self.state.drops_before_completion.load(Ordering::Acquire)
    }
}

#[derive(Clone)]
struct RecordingFileSource {
    file: RecordingFile,
}

impl FileCapabilitySourceApi for RecordingFileSource {
    fn context_for_request(&self, _db_context: DbCapabilityContext) -> FileCapabilityContext {
        FileCapabilityContext::new(self.file.clone())
    }
}

struct PendingFileWait {
    state: Arc<RecordingFileState>,
    completed: bool,
}

impl Drop for PendingFileWait {
    fn drop(&mut self) {
        if !self.completed {
            self.state
                .drops_before_completion
                .fetch_add(1, Ordering::AcqRel);
        }
    }
}

impl FileCapabilityApi for RecordingFile {
    fn source(&self) -> FileCapabilitySource {
        FileCapabilitySource::new(RecordingFileSource { file: self.clone() })
    }

    fn create_file<'a>(
        &'a self,
        _target: &'a str,
        _input: Bytes,
        _options: FileCreateOptions,
        _execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Value> {
        Box::pin(async {
            Err(skiff_runtime_capability_context::FileCapabilityError::file(
                "not used",
            ))
        })
    }

    fn read_file_wire<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
        _execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Value> {
        Box::pin(async {
            Err(skiff_runtime_capability_context::FileCapabilityError::file(
                "not used",
            ))
        })
    }

    fn read_text_file<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
        _execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Value> {
        Box::pin(async {
            Err(skiff_runtime_capability_context::FileCapabilityError::file(
                "not used",
            ))
        })
    }

    fn file_info<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
        _execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Value> {
        Box::pin(async {
            Err(skiff_runtime_capability_context::FileCapabilityError::file(
                "not used",
            ))
        })
    }

    fn delete_file<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
        _execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, ()> {
        Box::pin(async {
            Err(skiff_runtime_capability_context::FileCapabilityError::file(
                "not used",
            ))
        })
    }

    fn create_file_from_chunks<'a>(
        &'a self,
        _target: &'a str,
        _options: FileCreateOptions,
        mut next_chunk: FileChunkSource<'a>,
        execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Value> {
        self.state.starts.fetch_add(1, Ordering::AcqRel);
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let mut guard = PendingFileWait {
                state: Arc::clone(&state),
                completed: false,
            };
            if state.scope_pending.load(Ordering::Acquire) {
                let scope = execution_control.execution_scope().map_err(|error| {
                    skiff_runtime_capability_context::FileCapabilityError::decode(error.to_string())
                })?;
                let (lease, _completion) = scope.acquire_lease();
                let _terminal = lease.wait().await;
                let error = execution_control
                    .borrow()
                    .poll_execution_budget()
                    .expect_err("scope terminal must fail the post-await checkpoint");
                assert!(
                    matches!(
                        error,
                        skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(
                            skiff_runtime_capability_context::ExecutionBudgetFailure {
                                reason:
                                    skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded,
                                ..
                            }
                        )
                    ),
                    "file lower owner must observe the current deadline, got {error:?}"
                );
                return Err(
                    skiff_runtime_capability_context::FileCapabilityError::Execution(error),
                );
            }
            let mut size = 0usize;
            while let Some(chunk) = next_chunk().await? {
                size += chunk.len();
            }
            state.completions.fetch_add(1, Ordering::AcqRel);
            guard.completed = true;
            Ok(immutable_file_wire(ImmutableFileRef {
                id: "file:f445h-e4r".to_string(),
                size: size as i64,
                sha256: "sha256:f445h-e4r".to_string(),
                content_type: None,
            }))
        })
    }
}

fn bytes_type() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "bytes".to_string(),
        args: Vec::new(),
    }
}

fn create_from_stream_fixture() -> EvaluatorFixture {
    let actor = fixture(integer(), true);
    let mut file = (*actor_file(integer(), true)).clone();
    let producer_addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
        executable: 1,
    };
    file.executables[0].return_type = None;
    file.executables[0].slots = SlotLayoutIr::default();
    file.executables[0].body = LinkedExecutableBody {
        blocks: vec![BlockIr {
            label: "entry".to_string(),
            statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
        }],
        statements: vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 2 },
            },
            LinkedStmtIr::Return { value: None },
        ],
        expressions: vec![
            LinkedExprIr::Call {
                call: call(
                    LinkedCallTarget::Executable {
                        addr: producer_addr,
                    },
                    Vec::new(),
                ),
            },
            LinkedExprIr::Literal {
                value: LiteralIr::Null,
            },
            LinkedExprIr::Call {
                call: call(
                    native_target("std.file", "createFromStream", "std.file.createFromStream"),
                    vec![0, 1],
                ),
            },
        ],
    };
    file.executables.push(LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "bytesProducer".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(LinkedTypeRef::Native {
            name: "Stream".to_string(),
            args: vec![bytes_type()],
        }),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: true,
        body: LinkedExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
            }],
            statements: vec![
                LinkedStmtIr::Emit {
                    operation: "emit".to_string(),
                    value: ExprRefIr { expression: 1 },
                },
                LinkedStmtIr::Return { value: None },
            ],
            expressions: vec![
                LinkedExprIr::Literal {
                    value: LiteralIr::String {
                        value: "6869".to_string(),
                    },
                },
                LinkedExprIr::Call {
                    call: call(
                        native_target("std.bytes", "fromHex", "core.bytes.fromHex"),
                        vec![0],
                    ),
                },
            ],
        },
    });
    let file = Arc::new(file);
    let interpreter = interpreter_with_std_types(Arc::clone(&file));
    EvaluatorFixture {
        actor,
        interpreter,
        file,
    }
}

#[tokio::test]
async fn f445h_e4r_spine_create_from_stream_pending_reacquires_and_finalizes_once() {
    let file = RecordingFile::new();
    let fixture = create_from_stream_fixture();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    let addr = executable_addr();
    let context = program_context_with(
        &fixture.interpreter,
        test_runtime::actor_context(),
        FileCapabilityContext::new(file.clone()),
        DbCapabilityContext::unavailable(),
    );
    let mut eval = fixture.eval_context_with(context, frame.clone(), &mut heap, &mut env, &addr);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(file.starts(), 1);
    assert!(!frame.has_execution_lease());
    tokio::time::timeout(Duration::from_secs(1), execution)
        .await
        .expect("createFromStream completes")
        .expect("createFromStream finalizes");
    drop(eval);
    assert_eq!(file.starts(), 1);
    assert_eq!(file.completions(), 1);
    assert_eq!(file.drops_before_completion(), 0);
    assert!(
        frame.has_execution_lease(),
        "file result must reacquire before native return finalization"
    );
    frame
        .finish(heap)
        .expect("finish createFromStream success frame");
}

#[tokio::test]
async fn f445h_e4r_spine_create_from_stream_pending_drop_settles_once() {
    let file = RecordingFile::new();
    let fixture = create_from_stream_fixture();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    let addr = executable_addr();
    let context = program_context_with(
        &fixture.interpreter,
        test_runtime::actor_context(),
        FileCapabilityContext::new(file.clone()),
        DbCapabilityContext::unavailable(),
    );
    let mut eval = fixture.eval_context_with(context, frame.clone(), &mut heap, &mut env, &addr);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(file.starts(), 1);
    assert!(!frame.has_execution_lease());
    drop(execution);
    drop(eval);
    assert_eq!(file.completions(), 0);
    assert_eq!(
        file.drops_before_completion(),
        1,
        "dropping the evaluator future must drop the prepared file wait once"
    );
    drop(heap);
    drop(frame);
}

#[tokio::test]
async fn f445h_i6_file_projection_to_pending_preserves_current_deadline_owner() {
    let file = RecordingFile::new();
    file.hold_pending_until_scope_terminal();
    let fixture = create_from_stream_fixture();
    let mut heap = RequestHeap::new(RequestHeapLimits::default());
    let mut env = Env::new();
    let addr = executable_addr();
    let deadline = (tokio::time::Instant::now() + Duration::from_millis(20)).into_std();
    let current = test_runtime::execution_control()
        .derive_scope(deadline, site())
        .expect("current file scope");
    let current_scope = current.execution_scope().expect("current scope");
    let context = program_context_with(
        &fixture.interpreter,
        test_runtime::actor_context(),
        FileCapabilityContext::new(file.clone()),
        DbCapabilityContext::unavailable(),
    )
    .with_execution_control(current);
    let checkpoint_context = context.clone();
    let mut eval = fixture.eval_context_with_unframed(context, &mut heap, &mut env, &addr);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(file.starts(), 1);
    assert_eq!(current_scope.lifecycle_snapshot().active_leases, 1);
    let error = tokio::time::timeout(Duration::from_secs(1), execution)
        .await
        .expect("current deadline wakes")
        .expect_err("current deadline terminates createFromStream");
    drop(eval);

    assert!(
        matches!(error, RuntimeError::Cancelled),
        "the native wait exposes the internal cancellation terminal, got {error:?}"
    );
    let checkpoint_error = checkpoint_context
        .checkpoint(crate::program_execution::ExecutionCheckpoint::new(
            crate::program_execution::ExecutionCheckpointKind::GeneratedChunk,
            0,
        ))
        .expect_err("post-await checkpoint sees the current terminal");
    let RuntimeError::ScopeTerminal(terminal) = checkpoint_error else {
        panic!("expected exact checkpoint scope terminal, got {checkpoint_error:?}");
    };
    assert!(terminal.is_owned_by(&current_scope));
    assert_eq!(
        current_scope.lifecycle_snapshot(),
        skiff_runtime_capability_context::ExecutionScopeLifecycleSnapshot::default(),
    );
    assert_eq!(file.completions(), 0);
    assert_eq!(file.drops_before_completion(), 1);
}
