use super::{
    capability_harness::*, common::*, execution_control::*, imports::*,
    runtime_factory::HarnessRuntimeFactory, stream_support::ProbeStreamRuntime,
};

pub(super) fn execution_context(
    interpreter: &Interpreter,
    control: HarnessControl,
    config: HarnessConfig,
) -> ProgramExecutionContext<'static> {
    let execution = ExecutionControl::new(control);
    let effects = EffectDispatchContext::new(HarnessEffects);
    let actor = ActorCapabilityContext::new(HarnessActor);
    ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: ConfigCapabilityContext::new(config),
        db: DbCapabilityContext::unavailable(),
        file: FileCapabilityContext::new(HarnessFile),
        file_source_stream: FileSourceStreamContext::from_api(HarnessFileSourceStream {
            stream_runtime: interpreter.stream_runtime.clone(),
        }),
        time: TimeCapabilityContext::new(execution),
        websocket: EvalWebsocketCapabilityContext::new(HarnessWebsocket),
        effects: effects.clone(),
        http_client: effects.http_client_context(
            interpreter.http_options.clone(),
            interpreter.stream_runtime.clone(),
            interpreter.test_effect_double_context(),
        ),
        test_effect_doubles: interpreter.test_effect_double_context(),
        runtime_activation: Arc::new(RuntimeActivation {
            service: ServiceMeta {
                id: SERVICE_ID.to_string(),
                display_name: None,
                metadata: BTreeMap::new(),
            },
            version: VERSION.to_string(),
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
        outbound: OutboundServiceContext::new(HarnessOutbound {
            cancellation: CancellationToken::new(),
        }),
        request_heap_limits: RequestHeapLimits::default(),
    })
}

pub(super) fn interpreter_for(file: Arc<LinkedFileUnit>) -> (Arc<Interpreter>, ProbeStreamRuntime) {
    let duration = anonymous_type_decl(
        "std.time.Duration",
        LinkedTypeDescriptor::Alias { target: integer() },
    );
    let std_file = Arc::new(LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: "file:f445h-e4r-combined-std".to_string(),
        source_ast_hash: "source:f445h-e4r-combined-std".to_string(),
        module_path: "std".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        actor_declarations: Vec::new(),
        types: vec![duration.clone()],
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
    let duration_addr = TypeAddr {
        unit: UnitAddr::Package(0),
        file: FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let mut types = RuntimeTypeContext::default();
    types.descriptors.insert(duration_addr.clone(), duration);
    types
        .exported_types
        .insert_package(PackageSymbolKey::new(0, "std.time.Duration"), duration_addr);
    let program = Arc::new(EvalRuntimeProgram {
        service_id: SERVICE_ID.to_string(),
        service_files: vec![file],
        packages: vec![Arc::new(PackageUnit::empty(
            "skiff.run/std",
            VERSION,
            "skiff.run/std:build:f445h-e4r-combined",
            "skiff.run/std:abi:f445h-e4r-combined",
        ))],
        package_files: vec![vec![std_file]],
        service_resources: PublicationResourceTable::default(),
        package_resources: vec![PublicationResourceTable::default()],
        spawn_routes: HashMap::new(),
        link_overlay: overlay,
        types,
    });
    let stream = ProbeStreamRuntime::default();
    let interpreter = Interpreter::with_program(
        program,
        EvalRuntimeFactory::new(HarnessRuntimeFactory {
            stream: stream.clone(),
        }),
    );
    (Arc::new(interpreter), stream)
}
