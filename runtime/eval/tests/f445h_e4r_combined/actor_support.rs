use super::{
    activation_support::linked_activation_instruction, common::*, execution_control::*,
    execution_harness::*, imports::*,
};

fn actor_owner() -> LinkedActorDeclarationOwner {
    LinkedActorDeclarationOwner {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(ACTOR_FILE_ID.to_string()),
        actor_symbol: "CombinedActor".to_string(),
    }
}

fn actor_abi() -> ActorAbiIdentity {
    ActorAbiIdentity::new("skiff-actor-abi-v1:sha256:f445h-e4r-combined")
}

fn actor_implementation() -> ActorImplementationIdentity {
    ActorImplementationIdentity::new("skiff-actor-implementation-v1:sha256:f445h-e4r-combined")
}

pub(super) fn method_identity(name: &str) -> ActorMethodIdentity {
    ActorMethodIdentity::new(format!("skiff-actor-method-v1:sha256:f445h-e4r-{name}"))
}

fn ready_pending_executable() -> LinkedExecutable {
    executable(
        "CombinedActor.readyPending",
        vec![
            number(0),
            native_sleep_call(0),
            number(20),
            native_sleep_call(2),
            number(11),
        ],
        vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 1 },
            },
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 3 },
            },
            LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 4 }),
            },
        ],
        Vec::new(),
    )
}

fn timeout_executable() -> LinkedExecutable {
    executable(
        "CombinedActor.timeout",
        vec![
            number(1),
            LinkedExprIr::Timeout {
                duration_ms: 1_000,
                value: ExprRefIr { expression: 0 },
                site: site(),
            },
        ],
        vec![
            LinkedStmtIr::Timeout {
                duration_ms: 1_000,
                body: "timeout_body".to_string(),
                site: site(),
            },
            LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 1 }),
            },
        ],
        vec![BlockIr {
            label: "timeout_body".to_string(),
            statements: Vec::new(),
        }],
    )
}

fn concurrent_executable() -> LinkedExecutable {
    executable(
        "CombinedActor.concurrent",
        vec![
            number(2),
            LinkedExprIr::ConcurrentValue {
                plan: LinkedConcurrentPlanIr {
                    lanes: vec![LinkedConcurrentLaneIr::Tail {
                        source_order: 0,
                        dependencies: Vec::new(),
                        tail: ExprRefIr { expression: 0 },
                        site: site(),
                    }],
                    site: site(),
                },
            },
        ],
        vec![
            LinkedStmtIr::Concurrent {
                plan: LinkedConcurrentPlanIr {
                    lanes: vec![LinkedConcurrentLaneIr::Serial {
                        source_order: 0,
                        dependencies: Vec::new(),
                        body: "serial_body".to_string(),
                        site: site(),
                    }],
                    site: site(),
                },
            },
            LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 1 }),
            },
        ],
        vec![BlockIr {
            label: "serial_body".to_string(),
            statements: Vec::new(),
        }],
    )
}

fn activation_executable(instruction: ActivationRelativeServiceCall) -> LinkedExecutable {
    let mut config_call = call(
        LinkedCallTarget::Builtin {
            op: "config.require".to_string(),
        },
        &[0],
    );
    config_call
        .type_args
        .insert("T0".to_string(), string_type());
    executable(
        "CombinedActor.activation",
        vec![
            LinkedExprIr::Literal {
                value: LiteralIr::String {
                    value: "barrier".to_string(),
                },
            },
            LinkedExprIr::Call { call: config_call },
            LinkedExprIr::Call {
                call: call(
                    LinkedCallTarget::ActivationRelativeService { instruction },
                    &[],
                ),
            },
        ],
        vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 1 },
            },
            LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 2 }),
            },
        ],
        Vec::new(),
    )
}

fn competitor_executable() -> LinkedExecutable {
    executable(
        "CombinedActor.competitor",
        vec![number(3)],
        vec![LinkedStmtIr::Return {
            value: Some(ExprRefIr { expression: 0 }),
        }],
        Vec::new(),
    )
}

fn executable(
    symbol: &str,
    expressions: Vec<LinkedExprIr>,
    statements: Vec<LinkedStmtIr>,
    mut extra_blocks: Vec<BlockIr>,
) -> LinkedExecutable {
    let mut blocks = vec![BlockIr {
        label: "entry".to_string(),
        statements: (0..statements.len())
            .map(|statement| StmtRefIr {
                statement: statement as u32,
            })
            .collect(),
    }];
    blocks.append(&mut extra_blocks);
    LinkedExecutable {
        kind: ExecutableKind::ImplMethod,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(integer()),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: true,
        body: LinkedExecutableBody {
            blocks,
            statements,
            expressions,
        },
    }
}

fn actor_file(include_activation: bool) -> Arc<LinkedFileUnit> {
    let mut names = vec!["readyPending", "timeout", "concurrent"];
    let mut executables = vec![
        ready_pending_executable(),
        timeout_executable(),
        concurrent_executable(),
    ];
    if include_activation {
        names.extend(["activation", "competitor"]);
        executables.push(activation_executable(linked_activation_instruction()));
        executables.push(competitor_executable());
    }
    let public_methods = names
        .iter()
        .enumerate()
        .map(|(index, name)| LinkedActorPublicMethod {
            method_identity: method_identity(name),
            name: (*name).to_string(),
            parameters: Vec::<LinkedFunctionTypeParamIr>::new(),
            return_type: integer(),
            may_suspend: true,
            implementation: LinkedActorMethodImplementation::LocalExecutable {
                executable_index: index as u32,
            },
        })
        .collect();
    Arc::new(LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: ACTOR_FILE_ID.to_string(),
        source_ast_hash: "source:f445h-e4r-combined-actor".to_string(),
        module_path: "combined".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        actor_declarations: vec![LinkedActorDeclaration {
            actor_type: skiff_runtime_linked_program::ServiceSymbolRef {
                module_path: "combined".to_string(),
                symbol: "CombinedActor".to_string(),
            },
            implementation_owner: Some(actor_owner()),
            actor_abi_identity: actor_abi(),
            actor_implementation_identity: actor_implementation(),
            actor_name: "CombinedActor".to_string(),
            actor_id_type: string_type(),
            fields: Vec::new(),
            public_methods,
            actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
        }],
        types: Vec::new(),
        constants: Vec::new(),
        executables,
        external_refs: ExternalRefTable::default(),
    })
}

pub(super) struct ActorHarness {
    pub(super) interpreter: Arc<Interpreter>,
    pub(super) store: ActorInstanceStore,
    pub(super) handle: ActorInstanceHandle,
}

impl ActorHarness {
    pub(super) fn new(include_activation: bool) -> Self {
        let file = actor_file(include_activation);
        let (interpreter, _) = interpreter_for(Arc::clone(&file));
        let store = ActorInstanceStore::new();
        let actor_id = br#""combined-actor""#.to_vec();
        let fence = ActorInstanceFence {
            incarnation: ActorIncarnationKey {
                logical_key: ActorLogicalKey {
                    service_id: SERVICE_ID.to_string(),
                    actor_type_identity: "combined.CombinedActor".to_string(),
                    actor_id_type_identity: "builtin:string".to_string(),
                    actor_id_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
                    actor_id_hash: format!("sha256:{}", hex::encode(Sha256::digest(&actor_id))),
                    canonical_actor_id_key_bytes: actor_id,
                },
                epoch: 1,
            },
            actor_abi_identity: actor_abi(),
            actor_implementation_identity: actor_implementation(),
            declaration_owner: actor_owner(),
        };
        let program = EvalRuntimeProgram {
            service_id: SERVICE_ID.to_string(),
            service_files: vec![file],
            packages: Vec::new(),
            package_files: Vec::new(),
            service_resources: PublicationResourceTable::default(),
            package_resources: Vec::new(),
            spawn_routes: HashMap::new(),
            link_overlay: LinkOverlay::default(),
            types: RuntimeTypeContext::default(),
        };
        let handle = store
            .activate(ActorActivationRequest {
                fence,
                bootstrap_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1,
                bootstrap_payload: b"{}",
                program: program.projection().type_view(),
            })
            .expect("combined Actor activation");
        Self {
            interpreter,
            store,
            handle,
        }
    }

    pub(super) async fn execute(
        &self,
        method: &str,
        control: HarnessControl,
        config: HarnessConfig,
    ) -> std::result::Result<Vec<u8>, String> {
        ActorMethodExecutor::new(&self.store)
            .execute(
                &self.interpreter,
                ActorMethodExecutionRequest {
                    instance: &self.handle,
                    method_identity: &method_identity(method),
                    arguments_payload: b"[]",
                    context: execution_context(&self.interpreter, control, config),
                },
            )
            .await
            .map_err(|error| error.to_string())
    }
}
