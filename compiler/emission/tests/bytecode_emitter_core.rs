#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use skiff_artifact_model::{
        ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity, BoxSourceIr,
        BytecodePoolEntry, BytecodeRelocation, CallIr, CallTargetIr, CallableEffectSummary,
        ContractOperationId, ExprIr, ExprRefIr, ExternalRefTable, FileIrUnit, FunctionTypeParamIr,
        InstructionSourceSite, InterfaceInstantiationRef, InterfaceMethodSlotSignatureIr,
        LiteralIr, NativeTarget, PackageCallableId, PatternIr, RemoteOperationSlotPlanIr,
        RemoteOperationTablePlanIr, ServiceCallRef, ServiceProtocolIdentity, ServiceSymbolRef,
        SourcePosition, SourceSpanRef, SyntheticInstructionSiteReason, TypeDeclIr,
        TypeDescriptorIr, TypeRefIr, ResourceDropPlan, ValueTransferPlan,
    };
    use skiff_compiler_emission::{
        derive_bytecode_value_transfer_plans, emit_bytecode_artifact, BytecodeValueTransferPlans,
        FunctionValueTransferPlans,
    };
    use skiff_compiler_lowering::{
        mir::{
            liveness::compute_liveness, MirBlock, MirExecutableKind, MirExpression,
            MirForInBinding, MirForInFacts, MirForInItemKind, MirFunction, MirIndexAccessFacts,
            MirIndexPolicy, MirIndexReceiverKind, MirLiveness, MirMatchArmIr,
            MirRemoteInterfaceFacts, MirRemoteInterfaceMethodFacts, MirSlot, MirSlotKind,
            MirSourceEventPlan, MirSourceEventUnavailableReason, MirStatementEntry, MirStmt,
            MirStmtKind, MirStreamResultFacts, MirUnit,
        },
        Bounds, ConstEvaluator, FrozenConstantBundle,
    };

    fn expression(index: u32) -> ExprRefIr {
        ExprRefIr { expression: index }
    }

    fn site() -> InstructionSourceSite {
        InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::CompilerDesugaring,
        }
    }

    fn is_void(ty: &TypeRefIr) -> bool {
        matches!(ty, TypeRefIr::Builtin { name, args } if name == "void" && args.is_empty())
    }

    fn span() -> SourceSpanRef {
        SourceSpanRef {
            source_id: 1,
            start: SourcePosition::new(1, 1),
            end: SourcePosition::new(1, 2),
        }
    }

    fn mir_and_bundle(
        module_path: &str,
        type_table: Vec<TypeDeclIr>,
        external_refs: ExternalRefTable,
        function: MirFunction,
    ) -> (MirUnit, FrozenConstantBundle) {
        let mut file_ir = FileIrUnit::empty(module_path, "source-hash");
        file_ir.file_ir_identity = format!("file:{module_path}");
        file_ir.type_table = type_table;
        file_ir.external_refs = external_refs;
        let bundle = ConstEvaluator::new(Bounds::default())
            .evaluate_unit(&file_ir)
            .expect("test bundle evaluates");
        let mut function = function;
        function.origin.file_ir_identity = file_ir.file_ir_identity.clone();
        let unit = MirUnit {
            file_ir_identity: file_ir.file_ir_identity.clone(),
            module_path: file_ir.module_path.clone(),
            actor_declarations: file_ir.actor_declarations.clone(),
            external_refs: file_ir.external_refs.clone(),
            source_map: file_ir.source_map.clone(),
            type_table: file_ir.type_table.clone(),
            link_targets: file_ir.link_targets.clone(),
            constants: Vec::new(),
            functions: vec![function],
        };
        (unit, bundle)
    }

    #[allow(clippy::too_many_arguments)]
    fn function(
        module_path: &str,
        declaration: &str,
        return_type: TypeRefIr,
        slots: Vec<MirSlot>,
        expressions: Vec<MirExpression>,
        blocks: Vec<MirBlock>,
        statements: Vec<MirStatementEntry>,
        index_accesses: BTreeMap<u32, MirIndexAccessFacts>,
        regions: Vec<skiff_compiler_lowering::mir::MirRegion>,
    ) -> MirFunction {
        let mut function = MirFunction {
            executable_index: 0,
            origin: skiff_artifact_model::PackageExecutableCoordinate {
                file_ir_identity: "source-hash".to_string(),
                module_path: module_path.to_string(),
                executable_index: 0,
            },
            symbol: format!("{module_path}.{declaration}"),
            kind: MirExecutableKind::Function,
            type_params: Vec::new(),
            params: Vec::new(),
            return_type,
            self_type: None,
            receiver: None,
            slots,
            index_accesses,
            expressions,
            blocks,
            regions,
            statements,
            stream_result: None,
            liveness: MirLiveness::default(),
            effect_summary_ref: PackageCallableId::new(format!(
                "callable:{module_path}:{declaration}"
            )),
            effect_summary: CallableEffectSummary::analysis_pending(),
            source_span: None,
            source_event_plan: MirSourceEventPlan::unavailable(
                MirSourceEventUnavailableReason::SourceFactsNotProvided,
            ),
        };
        function.liveness = compute_liveness(&function).expect("test liveness computes");
        function
    }

    fn plans(
        function_key: &str,
        slot_types: &[TypeRefIr],
        return_type: &TypeRefIr,
    ) -> BytecodeValueTransferPlans {
        BytecodeValueTransferPlans::new(
            BTreeMap::from([(
                function_key.to_string(),
                FunctionValueTransferPlans {
                    slot_plans: slot_types
                        .iter()
                        .cloned()
                        .map(|ty| ValueTransferPlan::FromType { ty })
                        .collect(),
                    result_plans: if is_void(return_type) {
                        Vec::new()
                    } else {
                        vec![ValueTransferPlan::FromType {
                            ty: return_type.clone(),
                        }]
                    },
                },
            )]),
            BTreeMap::new(),
        )
    }

    fn one_return(expression: ExprRefIr) -> Vec<MirStmt> {
        vec![MirStmt {
            statement_index: 0,
            span: None,
            kind: MirStmtKind::Return {
                value: Some(expression),
            },
        }]
    }

    fn one_return_index(statement_index: u32, expression: ExprRefIr) -> Vec<MirStmt> {
        vec![MirStmt {
            statement_index,
            span: None,
            kind: MirStmtKind::Return {
                value: Some(expression),
            },
        }]
    }

    fn return_statements(statement_index: u32) -> Vec<MirStatementEntry> {
        vec![MirStatementEntry {
            statement_index,
            span: None,
        }]
    }

    #[test]
    fn init_slot_emits_store_slot() {
        let slot_ty = TypeRefIr::builtin("number");
        let expressions = vec![MirExpression {
            index: 0,
            expression: ExprIr::Literal {
                value: LiteralIr::Number {
                    value: serde_json::Number::from(42),
                },
            },
            ty: slot_ty.clone(),
            writable: None,
            direct_call: None,
            stream_result: None,
            remote_interface: None,
        }];
        let function = function(
            "slots",
            "init",
            TypeRefIr::builtin("void"),
            vec![MirSlot {
                slot: 0,
                name: "value".to_string(),
                kind: MirSlotKind::Local,
                writable_local: false,
                ty: Some(slot_ty.clone()),
            }],
            expressions,
            vec![MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: vec![
                    MirStmt {
                        statement_index: 0,
                        span: None,
                        kind: MirStmtKind::InitSlot {
                            slot: 0,
                            value: expression(0),
                        },
                    },
                    MirStmt {
                        statement_index: 1,
                        span: None,
                        kind: MirStmtKind::Return { value: None },
                    },
                ],
                successors: Vec::new(),
            }],
            vec![
                MirStatementEntry {
                    statement_index: 0,
                    span: None,
                },
                MirStatementEntry {
                    statement_index: 1,
                    span: None,
                },
            ],
            BTreeMap::new(),
            Vec::new(),
        );
        let (unit, bundle) =
            mir_and_bundle("slots", Vec::new(), ExternalRefTable::default(), function);
        let artifact = emit_bytecode_artifact(
            &[unit],
            &[bundle],
            &plans("slots::init", &[slot_ty], &TypeRefIr::builtin("void")),
        )
        .expect("init slot body emits");
        let view = skiff_artifact_model::bytecode::structurally_validate(&artifact)
            .expect("init slot body must validate");
        let function = view
            .functions()
            .iter()
            .find(|function| function.function_key == "slots::init")
            .expect("init slot function");
        assert!(function.instructions.iter().any(|instruction| {
            instruction.descriptor.kind == skiff_artifact_model::bytecode::Opcode::StoreSlot
        }));
    }

    #[test]
    fn record_construction_and_field_read_emit_shape_and_dense_field() {
        let type_table = vec![TypeDeclIr {
            name: "Person".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([("name".to_string(), TypeRefIr::builtin("string"))]),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        }];
        let record = TypeRefIr::LocalType { type_index: 0 };
        let expressions = vec![
            MirExpression {
                index: 0,
                expression: ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "Ada".to_string(),
                    },
                },
                ty: TypeRefIr::builtin("string"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 1,
                expression: ExprIr::Construct {
                    type_ref: record.clone(),
                    fields: BTreeMap::from([("name".to_string(), expression(0))]),
                },
                ty: record.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 2,
                expression: ExprIr::Field {
                    object: expression(1),
                    field: "name".to_string(),
                },
                ty: TypeRefIr::builtin("string"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
        ];
        let function = function(
            "records",
            "name",
            TypeRefIr::builtin("string"),
            Vec::new(),
            expressions,
            vec![MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: one_return(expression(2)),
                successors: Vec::new(),
            }],
            return_statements(0),
            BTreeMap::new(),
            Vec::new(),
        );
        let (unit, bundle) =
            mir_and_bundle("records", type_table, ExternalRefTable::default(), function);
        let artifact = emit_bytecode_artifact(
            &[unit],
            &[bundle],
            &plans("records::name", &[], &TypeRefIr::builtin("string")),
        )
        .expect("record body emits");

        assert_eq!(artifact.image.pools.shapes.len(), 1);
        let BytecodePoolEntry::ShapeRef { shape } = &artifact.image.pools.shapes[0] else {
            panic!("shapes pool is homogeneous");
        };
        assert_eq!(shape.fields[0].name, "name");
        assert!(!artifact.image.functions["records::name"].words.is_empty());
    }

    #[test]
    fn array_literal_and_index_emit_builder_and_array_get() {
        let array_ty = TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![TypeRefIr::builtin("number")],
        };
        let expressions = vec![
            MirExpression {
                index: 0,
                expression: ExprIr::Literal {
                    value: LiteralIr::Number {
                        value: serde_json::Number::from(10),
                    },
                },
                ty: TypeRefIr::builtin("number"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 1,
                expression: ExprIr::Literal {
                    value: LiteralIr::Number {
                        value: serde_json::Number::from(20),
                    },
                },
                ty: TypeRefIr::builtin("number"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 2,
                expression: ExprIr::ArrayLiteral {
                    items: vec![expression(0), expression(1)],
                },
                ty: array_ty.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 3,
                expression: ExprIr::Literal {
                    value: LiteralIr::Number {
                        value: serde_json::Number::from(1),
                    },
                },
                ty: TypeRefIr::builtin("number"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 4,
                expression: ExprIr::Index {
                    object: expression(2),
                    index: expression(3),
                },
                ty: TypeRefIr::builtin("number"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
        ];
        let index_accesses = BTreeMap::from([(
            3,
            MirIndexAccessFacts {
                receiver_kind: MirIndexReceiverKind::Array,
                receiver_type: array_ty.clone(),
                selector_type: TypeRefIr::builtin("number"),
                result_type: TypeRefIr::builtin("number"),
                policy: MirIndexPolicy::StrictRead,
                source_span: span(),
            },
        )]);
        let function = function(
            "arrays",
            "second",
            TypeRefIr::builtin("number"),
            Vec::new(),
            expressions,
            vec![MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: one_return(expression(4)),
                successors: Vec::new(),
            }],
            return_statements(0),
            index_accesses,
            Vec::new(),
        );
        let (unit, bundle) =
            mir_and_bundle("arrays", Vec::new(), ExternalRefTable::default(), function);
        let artifact = emit_bytecode_artifact(
            &[unit],
            &[bundle],
            &plans("arrays::second", &[], &TypeRefIr::builtin("number")),
        )
        .expect("array body emits");
        let view = skiff_artifact_model::bytecode::structurally_validate(&artifact)
            .expect("array body must validate");
        let function = view
            .functions()
            .iter()
            .find(|function| function.function_key == "arrays::second")
            .expect("array function");
        let opcodes = function
            .instructions
            .iter()
            .map(|instruction| instruction.descriptor.kind)
            .collect::<Vec<_>>();
        assert!(opcodes.contains(&skiff_artifact_model::bytecode::Opcode::NewArrayBuilder));
        assert_eq!(
            opcodes
                .iter()
                .filter(
                    |opcode| **opcode == skiff_artifact_model::bytecode::Opcode::ArrayBuilderPush
                )
                .count(),
            2
        );
        assert!(opcodes.contains(&skiff_artifact_model::bytecode::Opcode::FreezeArray));
        assert!(opcodes.contains(&skiff_artifact_model::bytecode::Opcode::ArrayGet));
        assert!(opcodes.contains(&skiff_artifact_model::bytecode::Opcode::Return));
    }

    #[test]
    fn map_literal_and_index_emit_builder_and_map_get() {
        let map_ty = TypeRefIr::Builtin {
            name: "Map".to_string(),
            args: vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("number")],
        };
        let expressions = vec![
            MirExpression {
                index: 0,
                expression: ExprIr::Literal {
                    value: LiteralIr::Number {
                        value: serde_json::Number::from(42),
                    },
                },
                ty: TypeRefIr::builtin("number"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 1,
                expression: ExprIr::MapLiteral {
                    entries: BTreeMap::from([("answer".to_string(), expression(0))]),
                },
                ty: map_ty.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 2,
                expression: ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "answer".to_string(),
                    },
                },
                ty: TypeRefIr::builtin("string"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 3,
                expression: ExprIr::Index {
                    object: expression(1),
                    index: expression(2),
                },
                ty: TypeRefIr::builtin("number"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
        ];
        let index_accesses = BTreeMap::from([(
            2,
            MirIndexAccessFacts {
                receiver_kind: MirIndexReceiverKind::Map,
                receiver_type: map_ty.clone(),
                selector_type: TypeRefIr::builtin("string"),
                result_type: TypeRefIr::builtin("number"),
                policy: MirIndexPolicy::StrictRead,
                source_span: span(),
            },
        )]);
        let function = function(
            "maps",
            "answer",
            TypeRefIr::builtin("number"),
            Vec::new(),
            expressions,
            vec![MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: one_return(expression(3)),
                successors: Vec::new(),
            }],
            return_statements(0),
            index_accesses,
            Vec::new(),
        );
        let (unit, bundle) =
            mir_and_bundle("maps", Vec::new(), ExternalRefTable::default(), function);
        let artifact = emit_bytecode_artifact(
            &[unit],
            &[bundle],
            &plans("maps::answer", &[], &TypeRefIr::builtin("number")),
        )
        .expect("map body emits");
        let view = skiff_artifact_model::bytecode::structurally_validate(&artifact)
            .expect("map body must validate");
        let function = view
            .functions()
            .iter()
            .find(|function| function.function_key == "maps::answer")
            .expect("map function");
        let opcodes = function
            .instructions
            .iter()
            .map(|instruction| instruction.descriptor.kind)
            .collect::<Vec<_>>();
        assert!(opcodes.contains(&skiff_artifact_model::bytecode::Opcode::NewMapBuilder));
        assert_eq!(
            opcodes
                .iter()
                .filter(
                    |opcode| **opcode == skiff_artifact_model::bytecode::Opcode::MapBuilderPut
                )
                .count(),
            1
        );
        assert!(opcodes.contains(&skiff_artifact_model::bytecode::Opcode::FreezeMap));
        assert!(opcodes.contains(&skiff_artifact_model::bytecode::Opcode::MapGet));
        assert!(opcodes.contains(&skiff_artifact_model::bytecode::Opcode::Return));
    }

    #[test]
    fn empty_map_literal_emits_new_builder_and_freeze() {
        let map_ty = TypeRefIr::Builtin {
            name: "Map".to_string(),
            args: vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("number")],
        };
        let expressions = vec![MirExpression {
            index: 0,
            expression: ExprIr::MapLiteral {
                entries: BTreeMap::new(),
            },
            ty: map_ty.clone(),
            writable: None,
            direct_call: None,
            stream_result: None,
            remote_interface: None,
        }];
        let function = function(
            "maps",
            "empty",
            map_ty.clone(),
            Vec::new(),
            expressions,
            vec![MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: one_return(expression(0)),
                successors: Vec::new(),
            }],
            return_statements(0),
            BTreeMap::new(),
            Vec::new(),
        );
        let (unit, bundle) =
            mir_and_bundle("maps", Vec::new(), ExternalRefTable::default(), function);
        let artifact = emit_bytecode_artifact(
            &[unit],
            &[bundle],
            &plans("maps::empty", &[], &map_ty),
        )
        .expect("empty map body emits");
        let view = skiff_artifact_model::bytecode::structurally_validate(&artifact)
            .expect("empty map body must validate");
        let function = view
            .functions()
            .iter()
            .find(|function| function.function_key == "maps::empty")
            .expect("empty map function");
        let opcodes = function
            .instructions
            .iter()
            .map(|instruction| instruction.descriptor.kind)
            .collect::<Vec<_>>();
        assert!(opcodes.contains(&skiff_artifact_model::bytecode::Opcode::NewMapBuilder));
        assert!(!opcodes.contains(&skiff_artifact_model::bytecode::Opcode::MapBuilderPut));
        assert!(opcodes.contains(&skiff_artifact_model::bytecode::Opcode::FreezeMap));
        assert!(opcodes.contains(&skiff_artifact_model::bytecode::Opcode::Return));
    }

    #[test]
    fn throw_rethrow_and_assertion_trap_emit() {
        let throw_function = function(
            "throws",
            "boom",
            TypeRefIr::builtin("void"),
            Vec::new(),
            vec![MirExpression {
                index: 0,
                expression: ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "bad".to_string(),
                    },
                },
                ty: TypeRefIr::builtin("string"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            }],
            vec![MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: vec![MirStmt {
                    statement_index: 0,
                    span: None,
                    kind: MirStmtKind::Throw {
                        value: expression(0),
                        payload_type: TypeRefIr::builtin("string"),
                        site: site(),
                    },
                }],
                successors: Vec::new(),
            }],
            return_statements(0),
            BTreeMap::new(),
            Vec::new(),
        );
        let (throw_unit, throw_bundle) = mir_and_bundle(
            "throws",
            Vec::new(),
            ExternalRefTable::default(),
            throw_function,
        );
        emit_bytecode_artifact(
            &[throw_unit],
            &[throw_bundle],
            &plans("throws::boom", &[], &TypeRefIr::builtin("void")),
        )
        .expect("throw body emits");

        let assert_function = function(
            "asserts",
            "check",
            TypeRefIr::builtin("void"),
            Vec::new(),
            vec![MirExpression {
                index: 0,
                expression: ExprIr::Literal {
                    value: LiteralIr::Bool { value: true },
                },
                ty: TypeRefIr::builtin("bool"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            }],
            vec![MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: vec![
                    MirStmt {
                        statement_index: 0,
                        span: None,
                        kind: MirStmtKind::Assert {
                            condition: expression(0),
                            message: None,
                        },
                    },
                    MirStmt {
                        statement_index: 1,
                        span: None,
                        kind: MirStmtKind::Return { value: None },
                    },
                ],
                successors: Vec::new(),
            }],
            vec![
                MirStatementEntry {
                    statement_index: 0,
                    span: None,
                },
                MirStatementEntry {
                    statement_index: 1,
                    span: None,
                },
            ],
            BTreeMap::new(),
            Vec::new(),
        );
        let (assert_unit, assert_bundle) = mir_and_bundle(
            "asserts",
            Vec::new(),
            ExternalRefTable::default(),
            assert_function,
        );
        emit_bytecode_artifact(
            &[assert_unit],
            &[assert_bundle],
            &plans("asserts::check", &[], &TypeRefIr::builtin("void")),
        )
        .expect("assert body emits");
    }

    #[test]
    fn service_and_actor_and_host_calls_emit_relocations_and_resume_rows() {
        let service_ref = ServiceCallRef {
            service_requirement_slot: 0,
            contract_operation_id: ContractOperationId::new("operation:echo"),
            expected_protocol_identity: ServiceProtocolIdentity::new("protocol:echo"),
        };
        let service_call = CallIr {
            target: CallTargetIr::ServiceCall {
                service_call_ref_index: skiff_artifact_model::ServiceCallRefIndex::new(0),
            },
            concrete_receiver: None,
            site: site(),
            args: vec![expression(0)],
            inout_args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };
        let actor_call = CallIr {
            target: CallTargetIr::ActorMethod {
                actor: ServiceSymbolRef {
                    module_path: "actors".to_string(),
                    symbol: "Counter".to_string(),
                },
                actor_abi_identity: ActorAbiIdentity::new("abi:counter"),
                actor_implementation_identity: ActorImplementationIdentity::new("impl:counter"),
                method_identity: ActorMethodIdentity::new("method:inc"),
            },
            concrete_receiver: None,
            site: site(),
            args: vec![expression(0)],
            inout_args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };
        let host_call = CallIr {
            target: CallTargetIr::Native {
                target: NativeTarget {
                    namespace: "std".to_string(),
                    symbol: "string.join".to_string(),
                    binding_key: Some("std.string.join".to_string()),
                    metadata: BTreeMap::new(),
                },
            },
            concrete_receiver: None,
            site: site(),
            args: vec![expression(0), expression(0)],
            inout_args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };
        let expressions = vec![
            MirExpression {
                index: 0,
                expression: ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "x".to_string(),
                    },
                },
                ty: TypeRefIr::builtin("string"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 1,
                expression: ExprIr::Call { call: service_call },
                ty: TypeRefIr::builtin("string"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 2,
                expression: ExprIr::Call { call: actor_call },
                ty: TypeRefIr::builtin("string"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 3,
                expression: ExprIr::Call { call: host_call },
                ty: TypeRefIr::builtin("string"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
        ];
        let function = function(
            "calls",
            "run",
            TypeRefIr::builtin("string"),
            Vec::new(),
            expressions,
            vec![MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: vec![
                    MirStmt {
                        statement_index: 0,
                        span: None,
                        kind: MirStmtKind::Expr {
                            value: expression(1),
                        },
                    },
                    MirStmt {
                        statement_index: 1,
                        span: None,
                        kind: MirStmtKind::Expr {
                            value: expression(2),
                        },
                    },
                    MirStmt {
                        statement_index: 2,
                        span: None,
                        kind: MirStmtKind::Return {
                            value: Some(expression(3)),
                        },
                    },
                ],
                successors: Vec::new(),
            }],
            vec![
                MirStatementEntry {
                    statement_index: 0,
                    span: None,
                },
                MirStatementEntry {
                    statement_index: 1,
                    span: None,
                },
                MirStatementEntry {
                    statement_index: 2,
                    span: None,
                },
            ],
            BTreeMap::new(),
            Vec::new(),
        );
        let external_refs = ExternalRefTable {
            service_call_refs: vec![service_ref],
            ..ExternalRefTable::default()
        };
        let (unit, bundle) = mir_and_bundle("calls", Vec::new(), external_refs, function);
        let artifact = emit_bytecode_artifact(
            &[unit],
            &[bundle],
            &plans("calls::run", &[], &TypeRefIr::builtin("string")),
        )
        .expect("service/actor/host body emits");
        let relocations = &artifact.image.functions["calls::run"].relocations;
        assert!(relocations.iter().any(|relocation| matches!(
            relocation,
            BytecodeRelocation::ServiceOperationRef { .. }
        )));
        assert!(relocations
            .iter()
            .any(|relocation| matches!(relocation, BytecodeRelocation::ActorMethodRef { .. })));
        assert!(relocations
            .iter()
            .any(|relocation| matches!(relocation, BytecodeRelocation::HostEffectRef(_))));
        assert_eq!(artifact.image.pools.resume.len(), 3);
    }

    #[test]
    fn array_for_in_emits_iteration_state_and_backedge() {
        let array_ty = TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![TypeRefIr::builtin("number")],
        };
        let expressions = vec![
            MirExpression {
                index: 0,
                expression: ExprIr::ArrayLiteral {
                    items: vec![expression(1)],
                },
                ty: array_ty.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 1,
                expression: ExprIr::Literal {
                    value: LiteralIr::Number {
                        value: serde_json::Number::from(7),
                    },
                },
                ty: TypeRefIr::builtin("number"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 2,
                expression: ExprIr::LoadSlot { slot: 0 },
                ty: TypeRefIr::builtin("number"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
        ];
        let facts = MirForInFacts {
            iterable_type: array_ty,
            binding: MirForInBinding::Item {
                slot: 0,
                ty: TypeRefIr::builtin("number"),
                kind: MirForInItemKind::ArrayItem,
            },
        };
        let function = function(
            "loops",
            "sum",
            TypeRefIr::builtin("void"),
            vec![MirSlot {
                slot: 0,
                name: "item".to_string(),
                kind: MirSlotKind::Local,
                writable_local: false,
                ty: Some(TypeRefIr::builtin("number")),
            }],
            expressions,
            vec![
                MirBlock {
                    id: 0,
                    label: "header".to_string(),
                    statements: vec![MirStmt {
                        statement_index: 0,
                        span: None,
                        kind: MirStmtKind::ForIn {
                            iterable: expression(0),
                            facts,
                            body: 1,
                            continuation: 2,
                        },
                    }],
                    successors: vec![1, 2],
                },
                MirBlock {
                    id: 1,
                    label: "body".to_string(),
                    statements: vec![MirStmt {
                        statement_index: 1,
                        span: None,
                        kind: MirStmtKind::Expr {
                            value: expression(2),
                        },
                    }],
                    successors: vec![2],
                },
                MirBlock {
                    id: 2,
                    label: "continuation".to_string(),
                    statements: vec![MirStmt {
                        statement_index: 2,
                        span: None,
                        kind: MirStmtKind::Return { value: None },
                    }],
                    successors: Vec::new(),
                },
            ],
            vec![
                MirStatementEntry {
                    statement_index: 0,
                    span: None,
                },
                MirStatementEntry {
                    statement_index: 1,
                    span: None,
                },
                MirStatementEntry {
                    statement_index: 2,
                    span: None,
                },
            ],
            BTreeMap::new(),
            Vec::new(),
        );
        let (unit, bundle) =
            mir_and_bundle("loops", Vec::new(), ExternalRefTable::default(), function);
        emit_bytecode_artifact(
            &[unit],
            &[bundle],
            &plans(
                "loops::sum",
                &[TypeRefIr::builtin("number")],
                &TypeRefIr::builtin("void"),
            ),
        )
        .expect("for-in body emits");
    }

    #[test]
    fn match_literal_and_wildcard_emit_jumps() {
        let expressions = vec![MirExpression {
            index: 0,
            expression: ExprIr::Literal {
                value: LiteralIr::Number {
                    value: serde_json::Number::from(1),
                },
            },
            ty: TypeRefIr::builtin("number"),
            writable: None,
            direct_call: None,
            stream_result: None,
            remote_interface: None,
        }];
        let function = function(
            "matches",
            "classify",
            TypeRefIr::builtin("number"),
            Vec::new(),
            expressions,
            vec![
                MirBlock {
                    id: 0,
                    label: "header".to_string(),
                    statements: vec![MirStmt {
                        statement_index: 0,
                        span: None,
                        kind: MirStmtKind::Match {
                            value: expression(0),
                            arms: vec![
                                MirMatchArmIr {
                                    pattern: PatternIr::Literal {
                                        value: LiteralIr::Number {
                                            value: serde_json::Number::from(1),
                                        },
                                    },
                                    body: 1,
                                },
                                MirMatchArmIr {
                                    pattern: PatternIr::Wildcard,
                                    body: 2,
                                },
                            ],
                        },
                    }],
                    successors: vec![1, 2, 3],
                },
                MirBlock {
                    id: 1,
                    label: "one".to_string(),
                    statements: one_return_index(1, expression(0)),
                    successors: vec![3],
                },
                MirBlock {
                    id: 2,
                    label: "other".to_string(),
                    statements: one_return_index(2, expression(0)),
                    successors: vec![3],
                },
                MirBlock {
                    id: 3,
                    label: "continuation".to_string(),
                    statements: one_return_index(3, expression(0)),
                    successors: Vec::new(),
                },
            ],
            vec![
                MirStatementEntry {
                    statement_index: 0,
                    span: None,
                },
                MirStatementEntry {
                    statement_index: 1,
                    span: None,
                },
                MirStatementEntry {
                    statement_index: 2,
                    span: None,
                },
                MirStatementEntry {
                    statement_index: 3,
                    span: None,
                },
            ],
            BTreeMap::new(),
            Vec::new(),
        );
        let (unit, bundle) =
            mir_and_bundle("matches", Vec::new(), ExternalRefTable::default(), function);
        emit_bytecode_artifact(
            &[unit],
            &[bundle],
            &plans("matches::classify", &[], &TypeRefIr::builtin("number")),
        )
        .expect("match body emits");
    }

    #[test]
    fn affine_stream_local_binding_emits_move_slot() {
        let stream_type = TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::builtin("number")],
        };
        let expressions = vec![MirExpression {
            index: 0,
            expression: ExprIr::LoadSlot { slot: 0 },
            ty: stream_type.clone(),
            writable: None,
            direct_call: None,
            stream_result: Some(MirStreamResultFacts {
                item_type: TypeRefIr::builtin("number"),
            }),
            remote_interface: None,
        }];
        let function = function(
            "streams",
            "bind",
            TypeRefIr::builtin("void"),
            vec![
                MirSlot {
                    slot: 0,
                    name: "values".to_string(),
                    kind: MirSlotKind::Local,
                    writable_local: false,
                    ty: Some(stream_type.clone()),
                },
                MirSlot {
                    slot: 1,
                    name: "stream".to_string(),
                    kind: MirSlotKind::Local,
                    writable_local: false,
                    ty: Some(stream_type.clone()),
                },
            ],
            expressions,
            vec![MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: vec![MirStmt {
                    statement_index: 0,
                    span: None,
                    kind: MirStmtKind::InitSlot {
                        slot: 1,
                        value: expression(0),
                    },
                }],
                successors: Vec::new(),
            }],
            vec![MirStatementEntry {
                statement_index: 0,
                span: None,
            }],
            BTreeMap::new(),
            Vec::new(),
        );
        let (unit, bundle) =
            mir_and_bundle("streams", Vec::new(), ExternalRefTable::default(), function);
        let plans = derive_bytecode_value_transfer_plans(&[unit.clone()])
            .expect("affine binding plans derive");
        let artifact = emit_bytecode_artifact(&[unit], &[bundle], &plans)
            .expect("affine stream binding emits bytecode");
        let view = skiff_artifact_model::bytecode::structurally_validate(&artifact)
            .expect("affine stream binding bytecode must validate");
        let function = view
            .functions()
            .iter()
            .find(|function| function.function_key == "streams::bind")
            .expect("affine binding function");
        assert!(function.instructions.iter().any(|instruction| {
            instruction.descriptor.kind == skiff_artifact_model::bytecode::Opcode::MoveSlot
        }));
        assert!(function.instructions.iter().all(|instruction| {
            instruction.descriptor.kind != skiff_artifact_model::bytecode::Opcode::LoadSlot
        }));
    }

    #[test]
    fn stream_for_in_emits_stream_next_store_and_loop_backedge() {
        let item_type = TypeRefIr::builtin("number");
        let stream_type = TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![item_type.clone()],
        };
        let expressions = vec![
            MirExpression {
                index: 0,
                expression: ExprIr::LoadSlot { slot: 0 },
                ty: stream_type.clone(),
                writable: None,
                direct_call: None,
                stream_result: Some(MirStreamResultFacts {
                    item_type: item_type.clone(),
                }),
                remote_interface: None,
            },
            MirExpression {
                index: 1,
                expression: ExprIr::LoadSlot { slot: 1 },
                ty: item_type.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
        ];
        let facts = MirForInFacts {
            iterable_type: stream_type.clone(),
            binding: MirForInBinding::Item {
                slot: 1,
                ty: item_type.clone(),
                kind: MirForInItemKind::StreamItem,
            },
        };
        let function = function(
            "streams",
            "consume",
            TypeRefIr::builtin("void"),
            vec![
                MirSlot {
                    slot: 0,
                    name: "values".to_string(),
                    kind: MirSlotKind::Local,
                    writable_local: false,
                    ty: Some(stream_type.clone()),
                },
                MirSlot {
                    slot: 1,
                    name: "item".to_string(),
                    kind: MirSlotKind::Local,
                    writable_local: false,
                    ty: Some(item_type.clone()),
                },
            ],
            expressions,
            vec![
                MirBlock {
                    id: 0,
                    label: "header".to_string(),
                    statements: vec![MirStmt {
                        statement_index: 0,
                        span: None,
                        kind: MirStmtKind::ForIn {
                            iterable: expression(0),
                            facts,
                            body: 1,
                            continuation: 2,
                        },
                    }],
                    successors: vec![1, 2],
                },
                MirBlock {
                    id: 1,
                    label: "body".to_string(),
                    statements: vec![MirStmt {
                        statement_index: 1,
                        span: None,
                        kind: MirStmtKind::Expr {
                            value: expression(1),
                        },
                    }],
                    successors: vec![2],
                },
                MirBlock {
                    id: 2,
                    label: "continuation".to_string(),
                    statements: vec![MirStmt {
                        statement_index: 2,
                        span: None,
                        kind: MirStmtKind::Return { value: None },
                    }],
                    successors: Vec::new(),
                },
            ],
            vec![
                MirStatementEntry {
                    statement_index: 0,
                    span: None,
                },
                MirStatementEntry {
                    statement_index: 1,
                    span: None,
                },
                MirStatementEntry {
                    statement_index: 2,
                    span: None,
                },
            ],
            BTreeMap::new(),
            Vec::new(),
        );
        let (unit, bundle) =
            mir_and_bundle("streams", Vec::new(), ExternalRefTable::default(), function);
        let plans = derive_bytecode_value_transfer_plans(&[unit.clone()])
            .expect("stream for-in plans derive");
        assert_eq!(
            plans.function("streams::consume").unwrap().slot_plans[0],
            ValueTransferPlan::AffineResource {
                drop: ResourceDropPlan::ResourceTableRelease,
            }
        );
        let artifact = emit_bytecode_artifact(&[unit], &[bundle], &plans)
            .expect("stream for-in emits bytecode");
        let view = skiff_artifact_model::bytecode::structurally_validate(&artifact)
            .expect("stream for-in bytecode must validate");
        let function = view
            .functions()
            .iter()
            .find(|function| function.function_key == "streams::consume")
            .expect("stream consumer function");
        assert!(function.instructions.iter().any(|instruction| {
            instruction.descriptor.kind == skiff_artifact_model::bytecode::Opcode::StreamNext
        }));
        assert!(function.instructions.iter().any(|instruction| {
            instruction.descriptor.kind == skiff_artifact_model::bytecode::Opcode::MoveSlot
        }));
        assert_eq!(artifact.image.pools.resume.len(), 1);
        let BytecodePoolEntry::ResumeDescriptor(descriptor) = &artifact.image.pools.resume[0]
        else {
            panic!("resume pool is homogeneous");
        };
        assert_eq!(
            descriptor.result_plans,
            vec![ValueTransferPlan::FromType {
                ty: item_type.clone(),
            }]
        );
        assert!(
            descriptor.end_resume_pc.is_some(),
            "StreamNext must carry a natural end resume pc"
        );
    }

    #[test]
    fn stream_return_derives_affine_plan_and_emits_bytecode() {
        let item_type = TypeRefIr::builtin("number");
        let stream_type = TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![item_type.clone()],
        };
        let expressions = vec![MirExpression {
            index: 0,
            expression: ExprIr::Literal {
                value: LiteralIr::Number {
                    value: serde_json::Number::from(7),
                },
            },
            ty: TypeRefIr::builtin("integer"),
            writable: None,
            direct_call: None,
            stream_result: None,
            remote_interface: None,
        }];
        let mut function = function(
            "streams",
            "produce",
            stream_type.clone(),
            Vec::new(),
            expressions,
            vec![MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: vec![
                    MirStmt {
                        statement_index: 0,
                        span: None,
                        kind: MirStmtKind::Emit {
                            operation: String::new(),
                            value: expression(0),
                        },
                    },
                    MirStmt {
                        statement_index: 1,
                        span: None,
                        kind: MirStmtKind::Emit {
                            operation: String::new(),
                            value: expression(0),
                        },
                    },
                    MirStmt {
                        statement_index: 2,
                        span: None,
                        kind: MirStmtKind::Return { value: None },
                    },
                ],
                successors: Vec::new(),
            }],
            vec![
                MirStatementEntry {
                    statement_index: 0,
                    span: None,
                },
                MirStatementEntry {
                    statement_index: 1,
                    span: None,
                },
                MirStatementEntry {
                    statement_index: 2,
                    span: None,
                },
            ],
            BTreeMap::new(),
            Vec::new(),
        );
        function.stream_result = Some(MirStreamResultFacts {
            item_type: item_type.clone(),
        });
        let (unit, bundle) =
            mir_and_bundle("streams", Vec::new(), ExternalRefTable::default(), function);
        let plans = derive_bytecode_value_transfer_plans(&[unit.clone()])
            .expect("stream producer plan derives");
        assert!(
            plans.function("streams::produce").unwrap().result_plans.is_empty(),
            "stream producer body return arity is zero"
        );

        let artifact = emit_bytecode_artifact(&[unit], &[bundle], &plans)
            .expect("stream producer emits bytecode");
        let view = skiff_artifact_model::bytecode::structurally_validate(&artifact)
            .expect("stream producer bytecode must validate");
        let function = view
            .functions()
            .iter()
            .find(|function| function.function_key == "streams::produce")
            .expect("stream producer function");
        assert_eq!(function.frame_layout.result_count, 0);
        assert!(function.frame_layout.result_plans.is_empty());
        let stream_result_type_ref = function
            .frame_layout
            .stream_result_type_ref
            .expect("stream producer frame carries Stream<T> authority");
        let BytecodePoolEntry::TypeRef { ty } =
            &artifact.image.pools.types[stream_result_type_ref as usize]
        else {
            panic!("stream authority type ref must select the types pool");
        };
        assert_eq!(ty, &stream_type);
        assert!(function.instructions.iter().any(|instruction| {
            instruction.descriptor.kind == skiff_artifact_model::bytecode::Opcode::EmitStream
        }));
        assert!(function.instructions.iter().all(|instruction| {
            instruction.descriptor.kind != skiff_artifact_model::bytecode::Opcode::StreamNext
        }));
        assert!(artifact.image.pools.resume.iter().all(|entry| {
            matches!(entry, BytecodePoolEntry::ResumeDescriptor(descriptor)
                if descriptor.end_resume_pc.is_none() && descriptor.result_plans.is_empty())
        }));
    }

    #[test]
    fn remote_interface_box_emits_remote_ref_and_method_rows() {
        let interface = InterfaceInstantiationRef {
            interface_abi_id: "iface:reader".to_string(),
            canonical_type_args: Vec::new(),
        };
        let operation_abi_id = "operation:reader:read".to_string();
        let method_abi_id = "method:interface:pkg.Reader:read".to_string();
        let signature = InterfaceMethodSlotSignatureIr {
            params: vec![FunctionTypeParamIr {
                name: "input".to_string(),
                ty: TypeRefIr::builtin("string"),
            }],
            return_type: TypeRefIr::builtin("string"),
        };
        let source = BoxSourceIr::Remote {
            dependency_ref: "readerService".to_string(),
            public_instance_key: "readers/default".to_string(),
            operations: RemoteOperationTablePlanIr {
                interface: interface.clone(),
                slots: vec![RemoteOperationSlotPlanIr {
                    slot: 0,
                    method_abi_id: method_abi_id.clone(),
                    signature: signature.clone(),
                    operation_abi_id: operation_abi_id.clone(),
                }],
            },
            callee_protocol_identity: "protocol:reader".to_string(),
        };
        let box_type = TypeRefIr::AnyInterface {
            interface: interface.clone(),
        };
        let expressions = vec![
            MirExpression {
                index: 0,
                expression: ExprIr::InterfaceBox {
                    value: expression(1),
                    interface: interface.clone(),
                    source,
                },
                ty: box_type.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: Some(MirRemoteInterfaceFacts {
                    service_requirement_slot: 3,
                    public_instance_key: "readers/default".to_string(),
                    interface: interface.clone(),
                    methods: vec![MirRemoteInterfaceMethodFacts {
                        slot: 0,
                        method_abi_id: method_abi_id.clone(),
                        signature: signature.clone(),
                        contract_operation_id: ContractOperationId::new(operation_abi_id.clone()),
                    }],
                    callee_protocol_identity: ServiceProtocolIdentity::new("protocol:reader"),
                }),
            },
            MirExpression {
                index: 1,
                expression: ExprIr::Literal {
                    value: LiteralIr::Null,
                },
                ty: TypeRefIr::builtin("null"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
        ];
        let function = function(
            "remote",
            "boxReader",
            box_type.clone(),
            Vec::new(),
            expressions,
            vec![MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: one_return(expression(0)),
                successors: Vec::new(),
            }],
            return_statements(0),
            BTreeMap::new(),
            Vec::new(),
        );
        let external_refs = ExternalRefTable {
            service_call_refs: vec![ServiceCallRef {
                service_requirement_slot: 3,
                contract_operation_id: ContractOperationId::new(operation_abi_id.clone()),
                expected_protocol_identity: ServiceProtocolIdentity::new("protocol:reader"),
            }],
            ..ExternalRefTable::default()
        };
        let (unit, bundle) = mir_and_bundle("remote", Vec::new(), external_refs, function);
        let artifact = emit_bytecode_artifact(
            &[unit],
            &[bundle],
            &plans("remote::boxReader", &[], &box_type),
        )
        .expect("remote interface box emits");
        let relocation = artifact.image.functions["remote::boxReader"]
            .relocations
            .iter()
            .find_map(|relocation| match relocation {
                BytecodeRelocation::RemoteInterfaceRef { interface } => Some(interface),
                _ => None,
            })
            .expect("remote interface box emits a RemoteInterfaceRef");
        assert_eq!(relocation.service_requirement_slot, 3);
        assert_eq!(relocation.public_instance_key, "readers/default");
        assert_eq!(relocation.interface, interface);
        assert_eq!(
            relocation.callee_protocol_identity.as_str(),
            "protocol:reader"
        );
        assert_eq!(relocation.methods.len(), 1);
        assert_eq!(relocation.methods[0].slot, 0);
        assert_eq!(relocation.methods[0].method_abi_id, method_abi_id);
        assert_eq!(relocation.methods[0].signature, signature);
        assert_eq!(
            relocation.methods[0].contract_operation_id.as_str(),
            operation_abi_id
        );
    }
}
