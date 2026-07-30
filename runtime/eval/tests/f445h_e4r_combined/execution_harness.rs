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
        actor: actor.clone(),
        spawn: actor,
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
        packages: vec![runtime_package_fixture("skiff.run/std", std_file)],
        service_resources: PublicationResourceTable::default(),
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

fn runtime_package_fixture(
    package_id: &str,
    file: Arc<LinkedFileUnit>,
) -> Arc<RuntimeExecutionPackage> {
    let artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new(format!("{package_id}:build")),
        files: vec![skiff_artifact_model::FileIrRef {
            file_ir_identity: file.file_ir_identity.clone(),
            module_path: file.module_path.clone(),
            artifact_path: None,
            source_ast_hash: Some(file.source_ast_hash.clone()),
        }],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new(format!("{package_id}:abi")),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                package_id,
                &BTreeMap::new(),
            )
            .expect("empty package schema index is canonical"),
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
    };
    Arc::new(
        RuntimeExecutionPackage::try_new(
            PackageCodeSlotIndex::new(0),
            Arc::new(artifact),
            vec![file],
            PublicationResourceTable::default(),
        )
        .expect("combined runtime package fixture must be exact"),
    )
}
