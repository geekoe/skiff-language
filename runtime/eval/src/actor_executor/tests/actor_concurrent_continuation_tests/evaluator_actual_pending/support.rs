use super::*;
use skiff_runtime_capability_context::OwnedExecutionControl;

pub(super) struct EvaluatorFixture {
    pub(super) actor: Fixture,
    pub(super) interpreter: Interpreter,
    pub(super) file: Arc<LinkedFileUnit>,
}

impl EvaluatorFixture {
    pub(super) fn new(
        expressions: Vec<LinkedExprIr>,
        statements: Vec<LinkedStmtIr>,
        slots: SlotLayoutIr,
    ) -> Self {
        let actor = fixture(integer(), true);
        let mut file = (*actor_file(integer(), true)).clone();
        let executable = &mut file.executables[0];
        executable.return_type = None;
        executable.slots = slots;
        executable.body = LinkedExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: (0..statements.len())
                    .map(|statement| StmtRefIr {
                        statement: statement as u32,
                    })
                    .collect(),
            }],
            statements,
            expressions,
        };
        let file = Arc::new(file);
        let interpreter = interpreter_with_std_types(Arc::clone(&file));
        Self {
            actor,
            interpreter,
            file,
        }
    }

    pub(super) fn executable(&self) -> &LinkedExecutable {
        &self.file.executables[0]
    }

    pub(super) async fn actor_frame(&self) -> (ActorExecutionFrame, RequestHeap) {
        execution_frame(&self.actor).await
    }

    pub(super) fn eval_context<'a>(
        &'a self,
        frame: ActorExecutionFrame,
        heap: &'a mut RequestHeap,
        env: &'a mut Env,
        addr: &'a ExecutableAddr,
    ) -> EvalContext<'a> {
        EvalContext::new(
            &self.interpreter,
            context(&self.interpreter).with_actor_execution_frame(frame),
            heap,
            env,
            addr,
            &self.file,
            self.executable(),
        )
        .expect("evaluator context")
    }

    pub(super) fn eval_context_with<'a>(
        &'a self,
        context: ProgramExecutionContext<'static>,
        frame: ActorExecutionFrame,
        heap: &'a mut RequestHeap,
        env: &'a mut Env,
        addr: &'a ExecutableAddr,
    ) -> EvalContext<'a> {
        EvalContext::new(
            &self.interpreter,
            context.with_actor_execution_frame(frame),
            heap,
            env,
            addr,
            &self.file,
            self.executable(),
        )
        .expect("evaluator context")
    }
}

pub(super) fn program_context_with(
    interpreter: &Interpreter,
    actor: ActorCapabilityContext<'static>,
    outbound: OutboundServiceContext,
    file: FileCapabilityContext,
    db: DbCapabilityContext,
) -> ProgramExecutionContext<'static> {
    program_context_with_stream(
        interpreter,
        actor,
        outbound,
        file,
        db,
        interpreter.stream_runtime.clone(),
    )
}

pub(super) fn program_context_with_stream(
    interpreter: &Interpreter,
    actor: ActorCapabilityContext<'static>,
    outbound: OutboundServiceContext,
    file: FileCapabilityContext,
    db: DbCapabilityContext,
    stream_runtime: StreamRuntime,
) -> ProgramExecutionContext<'static> {
    let execution = test_runtime::execution_control();
    let effects = test_runtime::effects_context();
    ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: test_runtime::config_context(),
        db,
        file,
        file_source_stream: FileSourceStreamContext::from_api(RuntimeFileSourceStream {
            stream_runtime: stream_runtime.clone(),
        }),
        time: TimeCapabilityContext::new(execution),
        websocket: test_runtime::websocket_context(),
        effects: effects.clone(),
        http_client: effects.http_client_context(
            interpreter.http_options.clone(),
            stream_runtime,
            interpreter.test_effect_double_context(),
        ),
        test_effect_doubles: interpreter.test_effect_double_context(),
        runtime_activation: Arc::new(RuntimeActivation {
            service: ServiceMeta {
                id: "skiff.run/counter".to_string(),
                display_name: None,
                metadata: BTreeMap::new(),
            },
            version: "1.0.0".to_string(),
            package_configs: Vec::new(),
            service_dependencies: Vec::new(),
            timeout: Default::default(),
            operation_route_bindings: Vec::new(),
            db: Vec::new(),
            actors: Vec::new(),
            gateway: Default::default(),
        }),
        actor: actor.clone(),
        spawn: actor,
        outbound,
        request_heap_limits: RequestHeapLimits::default(),
    })
}

#[derive(Clone)]
struct RuntimeFileSourceStream {
    stream_runtime: StreamRuntime,
}

impl FileSourceStreamApi for RuntimeFileSourceStream {
    fn stream_runtime_handle(&self) -> StreamRuntime {
        self.stream_runtime.clone()
    }

    fn next_file_source_stream_item<'a>(
        &'a self,
        stream: &'a Value,
        _execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Option<Value>> {
        Box::pin(async move {
            match self.stream_runtime.next(stream).await? {
                StreamPoll::Item(item) => Ok(Some(item)),
                StreamPoll::End => Ok(None),
                StreamPoll::InternalItem(item) => {
                    let (value, heap) = item.into_parts();
                    let value = match value {
                        RuntimeValue::Heap(handle) => heap
                            .local_carrier_cell(handle)
                            .map_err(|error| {
                                skiff_runtime_capability_context::FileCapabilityError::decode(
                                    error.to_string(),
                                )
                            })?
                            .map(|carrier| carrier.into_value())
                            .unwrap_or(RuntimeValue::Heap(handle)),
                        value => value,
                    };
                    crate::runtime_ops::runtime_to_wire(&value, &heap)
                        .map(Some)
                        .map_err(|error| {
                            skiff_runtime_capability_context::FileCapabilityError::decode(
                                error.to_string(),
                            )
                        })
                }
            }
        })
    }
}

pub(super) fn default_program_context(
    interpreter: &Interpreter,
) -> ProgramExecutionContext<'static> {
    program_context_with(
        interpreter,
        test_runtime::actor_context(),
        test_runtime::outbound_context(),
        test_runtime::file_context(),
        DbCapabilityContext::unavailable(),
    )
}

pub(super) fn interpreter_with_std_types(file: Arc<LinkedFileUnit>) -> Interpreter {
    let duration = anonymous_type_decl(
        "std.time.Duration",
        LinkedTypeDescriptor::Alias { target: integer() },
    );
    let nullable_string = LinkedTypeRef::Nullable {
        inner: Box::new(string_type()),
    };
    let immutable_file = anonymous_type_decl(
        "std.file.ImmutableFile",
        LinkedTypeDescriptor::Record {
            fields: BTreeMap::from([
                ("contentType".to_string(), nullable_string.clone()),
                ("id".to_string(), string_type()),
                ("sha256".to_string(), string_type()),
                ("size".to_string(), integer()),
            ]),
        },
    );
    let create_options = anonymous_type_decl(
        "std.file.CreateOptions",
        LinkedTypeDescriptor::Record {
            fields: BTreeMap::from([
                ("contentType".to_string(), nullable_string.clone()),
                ("purpose".to_string(), nullable_string),
            ]),
        },
    );
    let declarations = [
        ("std.time.Duration", duration),
        ("std.file.ImmutableFile", immutable_file),
        ("std.file.CreateOptions", create_options),
    ];
    let std_file = Arc::new(LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: "file:f445h-e4r-std".to_string(),
        source_ast_hash: "source:f445h-e4r-std".to_string(),
        module_path: "std".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        actor_declarations: Vec::new(),
        types: declarations
            .iter()
            .map(|(_, declaration)| declaration.clone())
            .collect(),
        constants: Vec::new(),
        executables: Vec::new(),
        external_refs: ExternalRefTable::default(),
    });
    let mut overlay = LinkOverlay::default();
    overlay
        .package_slots_by_id
        .insert("skiff.run/std".to_string(), 0);
    overlay
        .package_slots_by_dependency_ref
        .insert("std".to_string(), 0);
    let mut types = RuntimeTypeContext::default();
    for (type_index, (symbol, declaration)) in declarations.into_iter().enumerate() {
        let addr = TypeAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            type_index,
        };
        types.descriptors.insert(addr.clone(), declaration);
        types
            .exported_types
            .insert_package(PackageSymbolKey::new(0, symbol), addr);
    }
    let program = Arc::new(EvalRuntimeProgram::new(
        "skiff.run/counter",
        vec![file],
        vec![Arc::new(PackageUnit::empty(
            "skiff.run/std",
            "1.0.0",
            "skiff.run/std:build:f445h-e4r",
            "skiff.run/std:abi:f445h-e4r",
        ))],
        vec![vec![std_file]],
        PublicationResourceTable::default(),
        vec![PublicationResourceTable::default()],
        Default::default(),
        overlay,
        types,
    ));
    Interpreter::with_program(program, test_runtime::runtime_factory())
}

pub(super) fn site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

pub(super) fn call(target: LinkedCallTarget, args: Vec<u32>) -> CallIr {
    CallIr {
        target,
        site: site(),
        args: args
            .into_iter()
            .map(|expression| ExprRefIr { expression })
            .collect(),
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
        actor_metadata: None,
    }
}

pub(super) fn native_target(namespace: &str, symbol: &str, binding_key: &str) -> LinkedCallTarget {
    LinkedCallTarget::Native {
        target: NativeTarget {
            namespace: namespace.to_string(),
            symbol: symbol.to_string(),
            binding_key: Some(binding_key.to_string()),
            metadata: BTreeMap::new(),
        },
    }
}

pub(super) fn native_executable(
    target: LinkedCallTarget,
    args: Vec<LiteralIr>,
) -> EvaluatorFixture {
    let call_index = args.len() as u32;
    let mut expressions = args
        .into_iter()
        .map(|value| LinkedExprIr::Literal { value })
        .collect::<Vec<_>>();
    expressions.push(LinkedExprIr::Call {
        call: call(target, (0..call_index).collect()),
    });
    EvaluatorFixture::new(
        expressions,
        vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr {
                    expression: call_index,
                },
            },
            LinkedStmtIr::Return { value: None },
        ],
        SlotLayoutIr::default(),
    )
}

pub(super) fn string_type() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "string".to_string(),
        args: Vec::new(),
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

pub(super) fn first_poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    future.poll(&mut Context::from_waker(&waker))
}
