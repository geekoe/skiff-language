#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::bytecode::{
        emitter::{
            emit_bytecode_artifact_unchecked as emit_bytecode_artifact,
            emit_bytecode_artifact_unchecked_with_service_boundary_plans as emit_bytecode_artifact_with_service_boundary_plans,
        },
        plans::{
            derive_bytecode_value_transfer_plans_unchecked as derive_bytecode_value_transfer_plans,
            derive_test_bytecode_value_transfer_plans,
        },
    };
    use crate::BytecodeValueTransferPlans;
    use skiff_artifact_model::{
        derive_package_schema_type_id, ActorAbiIdentity, ActorImplementationIdentity,
        ActorMethodIdentity, AssignTargetIr, BoxSourceIr, BytecodeIntrinsicRef, BytecodePoolEntry,
        BytecodeRelocation, CallIr, CallTargetIr, CallableEffectSummary, ContractOperationId,
        ContractTypeDescriptor, DbBodyIr, DbOpKindIr, DbOperationIr, DbTargetIr, ExprIr, ExprRefIr,
        ExternalRefTable, FileIrUnit, FunctionTypeParamIr, InstructionSourceSite,
        InterfaceInstantiationRef, InterfaceMethodSlotSignatureIr, LiteralIr, MetadataValue,
        NativeTarget, PackageCallableId, PackageSchemaCanonicalDescriptor, PatternIr,
        RemoteOperationSlotPlanIr, RemoteOperationTablePlanIr, ResourceDropPlan,
        ServiceBoundaryPlan, ServiceCallRef, ServiceProtocolIdentity, ServiceSymbolRef,
        SourcePosition, SourceSpanRef, SyntheticInstructionSiteReason, TypeDeclIr,
        TypeDescriptorIr, TypeRefIr, ValueDropPlan, ValueTransferPlan,
    };
    use skiff_artifact_model::{
        BoundaryDropPlan, BoundaryErrorAdmission, BoundaryErrorFallbackIdentity, BoundaryErrorPlan,
        BoundaryErrorPolicy, BoundaryTransfer, BoundaryValueCarrier, BoundaryValueEncoding,
        BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan, ServiceCallbackPlan,
        ValueProvenance,
    };
    use skiff_compiler_lowering::{
        mir::{
            liveness::compute_liveness, MirBlock, MirDirectCallFacts, MirExecutableKind,
            MirExpression, MirExpressionBlockFact, MirForInBinding, MirForInFacts,
            MirForInItemKind, MirFunction, MirIndexAccessFacts, MirIndexPolicy,
            MirIndexReceiverKind, MirLiveness, MirMatchArmIr, MirRemoteInterfaceFacts,
            MirRemoteInterfaceMethodFacts, MirSlot, MirSlotKind, MirSourceEventPlan,
            MirSourceEventUnavailableReason, MirStatementEntry, MirStmt, MirStmtKind,
            MirStreamResultFacts, MirUnit, MirWritablePlace, MirWritableRoot,
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
            package_type_records: BTreeMap::new(),
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
            native: false,
            type_params: Vec::new(),
            params: Vec::new(),
            return_type,
            self_type: None,
            receiver: None,
            slots,
            index_accesses,
            expression_blocks: BTreeMap::new(),
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
        if !function
            .expressions
            .iter()
            .any(|expression| matches!(expression.expression, ExprIr::ValueBlock { .. }))
        {
            function.liveness = compute_liveness(&function).expect("test liveness computes");
        }
        function
    }

    fn service_boundary_plan() -> ServiceBoundaryPlan {
        ServiceBoundaryPlan {
            arguments: Vec::new(),
            results: Vec::new(),
            error: BoundaryErrorPlan {
                fallback_contract_type: std_service_internal_error(),
                fallback: BoundaryValuePlan::Linkable {
                    carrier: BoundaryValueCarrier::DetachedValueGraph,
                    encoding: BoundaryValueEncoding::CanonicalValue,
                    owner: BoundaryValueOwner::Caller,
                    lifetime: BoundaryValueLifetime::Call,
                },
                policy: BoundaryErrorPolicy::DynamicPublicSchema {
                    admission: BoundaryErrorAdmission::PublicNameableSchemaClosed,
                    fallback_identity: BoundaryErrorFallbackIdentity::StdServiceInternalError,
                },
                transfer: BoundaryTransfer::Move,
                drop: BoundaryDropPlan::SnapshotRelease,
                source: ValueProvenance::Fresh,
            },
            stream_item: None,
            callbacks: ServiceCallbackPlan::None,
            effects: CallableEffectSummary::Analyzed {
                effects: skiff_artifact_model::CallableMayEffects {
                    escapes_caller_value: false,
                    requires_same_heap_identity: false,
                    invokes_unknown_target: false,
                    may_pending: true,
                    pending_effect_categories: vec![
                        skiff_artifact_model::PendingEffectCategory::ServiceCall,
                    ],
                    inout_path_effects: Vec::new(),
                },
            },
        }
    }

    fn std_service_internal_error() -> skiff_artifact_model::ContractTypeRef {
        let descriptor = PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Record {
                fields: BTreeMap::from([
                    (
                        "message".to_string(),
                        skiff_artifact_model::ContractTypeRef::builtin("string"),
                    ),
                    (
                        "traceId".to_string(),
                        skiff_artifact_model::ContractTypeRef::builtin("string"),
                    ),
                    (
                        "errorId".to_string(),
                        skiff_artifact_model::ContractTypeRef::builtin("string"),
                    ),
                ]),
            },
        };
        let type_id = derive_package_schema_type_id(
            "skiff.run/std",
            "std.service.InternalError",
            &descriptor,
        )
        .expect("canonical std.service.InternalError schema derives");
        skiff_artifact_model::ContractTypeRef::package_schema(
            "skiff.run/std",
            "std.service.InternalError",
            type_id,
        )
    }

    fn plans(unit: &MirUnit) -> BytecodeValueTransferPlans {
        derive_test_bytecode_value_transfer_plans(std::slice::from_ref(unit))
            .expect("the source classifier covers the test MIR")
    }

    fn stream_plan(ty: &TypeRefIr) -> Result<ValueTransferPlan, String> {
        let stream_number = TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::builtin("number")],
        };
        if *ty == stream_number {
            return Ok(ValueTransferPlan::AffineResource {
                drop: ResourceDropPlan::ResourceTableRelease,
            });
        }
        if matches!(
            ty,
            TypeRefIr::Builtin { name, args }
                if args.is_empty()
                    && matches!(
                        name.as_str(),
                        "bool" | "integer" | "never" | "null" | "number" | "string" | "void"
                    )
        ) {
            return Ok(ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::Trivial,
            });
        }
        Err(format!("no exact source plan for {ty:?}"))
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
        let plans = plans(&unit);
        let artifact =
            emit_bytecode_artifact(&[unit], &[bundle], &plans).expect("init slot body emits");
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
        let plans = plans(&unit);
        let missing_type_facts =
            BytecodeValueTransferPlans::new(plans.functions().clone(), plans.constants().clone());
        let error = emit_bytecode_artifact(
            std::slice::from_ref(&unit),
            std::slice::from_ref(&bundle),
            &missing_type_facts,
        )
        .expect_err("record emission requires the compiler-owned type lifecycle facts");
        assert!(matches!(
            error,
            crate::BytecodeEmissionError::CanonicalSerialization { message, .. }
                if message.contains("missing exact compiler-owned value-transfer plan")
        ));
        let artifact =
            emit_bytecode_artifact(&[unit], &[bundle], &plans).expect("record body emits");

        assert_eq!(artifact.image.pools.shapes.len(), 1);
        let BytecodePoolEntry::ShapeRef { shape } = &artifact.image.pools.shapes[0] else {
            panic!("shapes pool is homogeneous");
        };
        assert_eq!(
            shape.plan,
            ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::SnapshotRelease,
            }
        );
        assert_eq!(shape.fields[0].name, "name");
        assert_eq!(
            shape.fields[0].plan,
            ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::SnapshotRelease,
            }
        );
        assert!(!artifact.image.functions["records::name"].words.is_empty());
    }

    #[test]
    fn phase_1_bytecode_admission_keeps_array_emission_behind_the_private_backend() {
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
        let plans = plans(&unit);
        let artifact =
            emit_bytecode_artifact(&[unit], &[bundle], &plans).expect("array body emits");
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
        let plans = plans(&unit);
        let artifact = emit_bytecode_artifact(&[unit], &[bundle], &plans).expect("map body emits");
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
                .filter(|opcode| **opcode == skiff_artifact_model::bytecode::Opcode::MapBuilderPut)
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
        let plans = plans(&unit);
        let artifact =
            emit_bytecode_artifact(&[unit], &[bundle], &plans).expect("empty map body emits");
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
        let throw_plans = plans(&throw_unit);
        emit_bytecode_artifact(&[throw_unit], &[throw_bundle], &throw_plans)
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
        let assert_plans = plans(&assert_unit);
        emit_bytecode_artifact(&[assert_unit], &[assert_bundle], &assert_plans)
            .expect("assert body emits");
    }

    #[test]
    fn service_and_actor_and_host_calls_emit_relocations_and_resume_rows() {
        let service_ref = ServiceCallRef {
            service_requirement_slot: 0,
            contract_operation_id: ContractOperationId::new("operation:echo"),
            expected_protocol_identity: ServiceProtocolIdentity::new("protocol:echo"),
        };
        let boundary_plan = service_boundary_plan();
        let service_plans = BTreeMap::from([(service_ref.clone(), boundary_plan.clone())]);
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
            service_call_refs: vec![service_ref.clone()],
            ..ExternalRefTable::default()
        };
        let (unit, bundle) = mir_and_bundle("calls", Vec::new(), external_refs, function);
        let plans = plans(&unit);
        let artifact = emit_bytecode_artifact_with_service_boundary_plans(
            &[unit],
            &[bundle],
            &plans,
            &service_plans,
        )
        .expect("service/actor/host body emits");
        let relocations = &artifact.image.functions["calls::run"].relocations;
        assert!(relocations.iter().any(|relocation| matches!(
            relocation,
            BytecodeRelocation::ServiceOperationRef {
                service_call,
            } if service_call.service_call() == &service_ref
                && service_call.boundary_plan() == &boundary_plan
        )));
        assert!(relocations
            .iter()
            .any(|relocation| matches!(relocation, BytecodeRelocation::ActorMethodRef { .. })));
        assert!(relocations
            .iter()
            .any(|relocation| matches!(relocation, BytecodeRelocation::HostEffectRef(_))));
        assert!(artifact.image.pools.types.iter().any(|entry| matches!(
            entry,
            BytecodePoolEntry::TypeRef {
                ty: TypeRefIr::PackageSchema {
                    package_id,
                    stable_schema_key,
                    ..
                },
                plan: ValueTransferPlan::SnapshotShare {
                    drop: ValueDropPlan::SnapshotRelease
                },
                ..
            } if package_id == "skiff.run/std"
                && stable_schema_key == "std.service.InternalError"
        )));
        assert_eq!(artifact.image.pools.resume.len(), 3);
    }

    #[test]
    fn task_submit_emits_exact_task_relocation_and_target_identity() {
        let task_call = CallIr {
            target: CallTargetIr::LocalExecutable {
                executable_index: 0,
            },
            concrete_receiver: None,
            site: site(),
            args: Vec::new(),
            inout_args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::from([(
                "dispatchSubmit".to_string(),
                MetadataValue::Object(BTreeMap::from([
                    (
                        "targetKind".to_string(),
                        MetadataValue::String("function".to_string()),
                    ),
                    (
                        "target".to_string(),
                        MetadataValue::String("function:main.work".to_string()),
                    ),
                    (
                        "timing".to_string(),
                        MetadataValue::Object(BTreeMap::from([(
                            "kind".to_string(),
                            MetadataValue::String("immediate".to_string()),
                        )])),
                    ),
                ])),
            )]),
        };
        let expressions = vec![MirExpression {
            index: 0,
            expression: ExprIr::Call { call: task_call },
            ty: TypeRefIr::builtin("TaskRef"),
            writable: None,
            direct_call: Some(MirDirectCallFacts {
                concrete_receiver: None,
                receiver_call_abi: None,
                parameter_modes: Vec::new(),
                arguments: Vec::new(),
            }),
            stream_result: None,
            remote_interface: None,
        }];
        let function = function(
            "main",
            "run",
            TypeRefIr::builtin("TaskRef"),
            Vec::new(),
            expressions,
            vec![MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: vec![MirStmt {
                    statement_index: 0,
                    span: None,
                    kind: MirStmtKind::Return {
                        value: Some(expression(0)),
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
            mir_and_bundle("main", Vec::new(), ExternalRefTable::default(), function);
        let plans = derive_test_bytecode_value_transfer_plans(&[unit.clone()])
            .expect("task fixture plans resolve");
        let artifact = emit_bytecode_artifact(&[unit], &[bundle], &plans)
            .expect("task submit emits exact relocation");
        let relocations = &artifact.image.functions["main::run"].relocations;
        assert!(relocations.iter().any(|relocation| matches!(
            relocation,
            BytecodeRelocation::TaskSubmitRef { task }
                if task.target_identity == "function:main.work"
                    && matches!(
                        &task.target,
                        skiff_artifact_model::bytecode::dto::TaskSubmitTargetRef::Function {
                            function_key,
                        } if function_key == "main::run"
                    )
        )));
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
        let plans = plans(&unit);
        emit_bytecode_artifact(&[unit], &[bundle], &plans).expect("for-in body emits");
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
        let plans = plans(&unit);
        emit_bytecode_artifact(&[unit], &[bundle], &plans).expect("match body emits");
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
        let plans = derive_bytecode_value_transfer_plans(&[unit.clone()], |_module_path, ty| {
            stream_plan(ty)
        })
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
        let plans = derive_bytecode_value_transfer_plans(&[unit.clone()], |_module_path, ty| {
            stream_plan(ty)
        })
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
            vec![ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::Trivial,
            }]
        );
        assert!(
            descriptor.end_resume_pc.is_some(),
            "StreamNext must carry a natural end resume pc"
        );
    }

    #[test]
    fn stream_return_without_source_attribution_fails_closed_after_plan_derivation() {
        let item_type = TypeRefIr::Record {
            fields: BTreeMap::new(),
        };
        let stream_type = TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![item_type.clone()],
        };
        let expressions = vec![MirExpression {
            index: 0,
            expression: ExprIr::Construct {
                type_ref: item_type.clone(),
                fields: BTreeMap::new(),
            },
            ty: item_type.clone(),
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
        let plans = derive_bytecode_value_transfer_plans(&[unit.clone()], |_module_path, ty| {
            if ty == &stream_type {
                Ok(ValueTransferPlan::AffineResource {
                    drop: ResourceDropPlan::ResourceTableRelease,
                })
            } else if ty == &item_type {
                Ok(ValueTransferPlan::SnapshotShare {
                    drop: ValueDropPlan::SnapshotRelease,
                })
            } else {
                stream_plan(ty)
            }
        })
        .expect("stream producer plan derives");
        assert!(
            plans
                .function("streams::produce")
                .unwrap()
                .result_plans
                .is_empty(),
            "stream producer body return arity is zero"
        );

        let error = emit_bytecode_artifact(&[unit], &[bundle], &plans)
            .expect_err("hand-built stream MIR without producer source facts must fail closed");
        assert!(matches!(
            error,
            crate::BytecodeEmissionError::UnsupportedConstruct {
                function_key,
                construct: "EmitStream source attribution",
                ..
            } if function_key == "streams::produce"
        ));
    }

    #[test]
    fn stream_emit_rejects_non_construct_item_before_shape_emission() {
        let item_type = TypeRefIr::builtin("number");
        let stream_type = TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![item_type.clone()],
        };
        let expressions = vec![MirExpression {
            index: 0,
            expression: ExprIr::Literal {
                value: LiteralIr::Number {
                    value: serde_json::Number::from(1_u64),
                },
            },
            ty: item_type.clone(),
            writable: None,
            direct_call: None,
            stream_result: None,
            remote_interface: None,
        }];
        let mut function = function(
            "streams",
            "produceNonConstruct",
            stream_type,
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
        function.stream_result = Some(MirStreamResultFacts { item_type });
        let (unit, bundle) =
            mir_and_bundle("streams", Vec::new(), ExternalRefTable::default(), function);
        let plans = derive_bytecode_value_transfer_plans(&[unit.clone()], |_module_path, ty| {
            stream_plan(ty)
        })
        .expect("stream producer plans derive");

        let error = emit_bytecode_artifact(&[unit], &[bundle], &plans)
            .expect_err("non-Construct Emit must not invent a dense layout fact");
        assert!(matches!(
            error,
            crate::BytecodeEmissionError::UnsupportedConstruct {
                function_key,
                construct: "EmitStream item shape",
                location,
            } if function_key == "streams::produceNonConstruct"
                && location.contains("not an exact record construction")
        ));
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
        let plans = plans(&unit);
        let artifact =
            emit_bytecode_artifact(&[unit], &[bundle], &plans).expect("remote interface box emits");
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

    #[test]
    fn value_block_linearizes_branch_completion_to_resume() {
        let slot_ty = TypeRefIr::builtin("number");
        let expressions = vec![
            MirExpression {
                index: 0,
                expression: ExprIr::ValueBlock {
                    block: "pick".to_string(),
                    result: expression(3),
                },
                ty: slot_ty.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 1,
                expression: ExprIr::Literal {
                    value: LiteralIr::Bool { value: true },
                },
                ty: TypeRefIr::builtin("bool"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 2,
                expression: ExprIr::Literal {
                    value: LiteralIr::Number {
                        value: serde_json::Number::from(1),
                    },
                },
                ty: slot_ty.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 3,
                expression: ExprIr::LoadSlot { slot: 0 },
                ty: slot_ty.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 4,
                expression: ExprIr::Literal {
                    value: LiteralIr::Number {
                        value: serde_json::Number::from(2),
                    },
                },
                ty: slot_ty.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
        ];
        let writable = MirWritablePlace {
            root: MirWritableRoot::Slot { slot: 0 },
            path: Vec::new(),
        };
        let function = function(
            "blocks",
            "pick",
            slot_ty.clone(),
            vec![MirSlot {
                slot: 0,
                name: "chosen".to_string(),
                kind: MirSlotKind::Local,
                writable_local: true,
                ty: Some(slot_ty.clone()),
            }],
            expressions,
            vec![
                MirBlock {
                    id: 0,
                    label: "entry".to_string(),
                    statements: vec![MirStmt {
                        statement_index: 0,
                        span: None,
                        kind: MirStmtKind::Return {
                            value: Some(expression(0)),
                        },
                    }],
                    successors: Vec::new(),
                },
                MirBlock {
                    id: 1,
                    label: "pick_body".to_string(),
                    statements: vec![MirStmt {
                        statement_index: 1,
                        span: None,
                        kind: MirStmtKind::If {
                            condition: expression(1),
                            then_block: 4,
                            else_block: Some(3),
                        },
                    }],
                    successors: vec![3, 4],
                },
                MirBlock {
                    id: 2,
                    label: "pick_body".to_string(),
                    statements: Vec::new(),
                    successors: vec![0],
                },
                MirBlock {
                    id: 3,
                    label: "pick_else".to_string(),
                    statements: vec![MirStmt {
                        statement_index: 2,
                        span: None,
                        kind: MirStmtKind::Assign {
                            target: AssignTargetIr::Slot { slot: 0 },
                            place: writable.clone(),
                            value: expression(4),
                        },
                    }],
                    successors: vec![2],
                },
                MirBlock {
                    id: 4,
                    label: "pick_then".to_string(),
                    statements: vec![MirStmt {
                        statement_index: 3,
                        span: None,
                        kind: MirStmtKind::Assign {
                            target: AssignTargetIr::Slot { slot: 0 },
                            place: writable.clone(),
                            value: expression(2),
                        },
                    }],
                    successors: vec![2],
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
        let mut function = function;
        function.expression_blocks.insert(
            0,
            MirExpressionBlockFact {
                body_block: 1,
                continuation_block: 0,
                result: expression(3),
                completion_targets: vec![2],
            },
        );
        function.liveness = compute_liveness(&function).expect("ValueBlock liveness computes");
        let (unit, bundle) =
            mir_and_bundle("blocks", Vec::new(), ExternalRefTable::default(), function);
        let plans = plans(&unit);
        let artifact = emit_bytecode_artifact(&[unit], &[bundle], &plans)
            .expect("ValueBlock linearization emits");
        let view = skiff_artifact_model::bytecode::structurally_validate(&artifact)
            .expect("ValueBlock artifact validates");
        let function = view
            .functions()
            .iter()
            .find(|function| function.function_key == "blocks::pick")
            .expect("pick function");
        assert!(function.instructions.iter().any(|instruction| {
            instruction.descriptor.kind == skiff_artifact_model::bytecode::Opcode::Jump
        }));
        let load_pc = function
            .instructions
            .iter()
            .find(|instruction| {
                instruction.descriptor.kind == skiff_artifact_model::bytecode::Opcode::LoadSlot
            })
            .expect("ValueBlock result loads the branch slot")
            .pc;
        let branch_target = |instruction: &skiff_artifact_model::bytecode::DecodedInstruction| {
            skiff_artifact_model::bytecode::decode_branch_target(
                instruction.pc,
                u32::try_from(instruction.operand_words.len()).unwrap(),
                instruction.operand(0),
            )
            .expect("validated branch has a target")
        };
        let completion_jumps = function
            .instructions
            .iter()
            .filter(|instruction| {
                instruction.descriptor.kind == skiff_artifact_model::bytecode::Opcode::Jump
                    && branch_target(instruction) == load_pc
            })
            .collect::<Vec<_>>();
        let [completion_jump] = completion_jumps.as_slice() else {
            panic!("exactly one ValueBlock completion edge reaches the result load")
        };
        let stores = function
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, instruction)| {
                instruction.descriptor.kind == skiff_artifact_model::bytecode::Opcode::StoreSlot
            })
            .collect::<Vec<_>>();
        assert_eq!(
            stores.len(),
            2,
            "both ternary branches write the result slot"
        );
        for (index, _) in stores {
            let jump = &function.instructions[index + 1];
            assert_eq!(
                jump.descriptor.kind,
                skiff_artifact_model::bytecode::Opcode::Jump
            );
            assert_eq!(branch_target(jump), completion_jump.pc);
        }
    }

    #[test]
    fn db_operation_without_an_admitted_machine_boundary_fails_closed() {
        let record_ty = TypeRefIr::Record {
            fields: BTreeMap::from([("value".to_string(), TypeRefIr::builtin("number"))]),
        };
        let operation = DbOperationIr {
            op: DbOpKindIr::Update,
            many: false,
            target: DbTargetIr {
                type_ref: record_ty.clone(),
                type_name: "Item".to_string(),
            },
            selector: None,
            query: None,
            projection: None,
            body: Some(DbBodyIr::ObjectFields {
                fields: BTreeMap::from([("value".to_string(), expression(1))]),
            }),
            insert_body: None,
            change: None,
            result_type: record_ty.clone(),
            source_span: None,
        };
        let expressions = vec![
            MirExpression {
                index: 0,
                expression: ExprIr::DbOperation { operation },
                ty: record_ty.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 1,
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
        ];
        let function = function(
            "db",
            "insertItem",
            record_ty.clone(),
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
            mir_and_bundle("db", Vec::new(), ExternalRefTable::default(), function);
        let plans = plans(&unit);
        let error = emit_bytecode_artifact(&[unit], &[bundle], &plans)
            .expect_err("DB execution must not mint a semantic owner shape or host boundary");
        assert!(matches!(
            error,
            crate::BytecodeEmissionError::UnsupportedConstruct {
                function_key,
                construct: "DbOperation",
                location,
            } if function_key == "db::insertItem"
                && location == " bytecode F6 facts currently admit single-object db insert only"
        ));
    }

    #[test]
    fn db_insert_emits_exact_intrinsic_facts_and_result_plan() {
        let record_ty = TypeRefIr::Record {
            fields: BTreeMap::from([
                ("id".to_string(), TypeRefIr::builtin("string")),
                ("value".to_string(), TypeRefIr::builtin("string")),
            ]),
        };
        let operation = DbOperationIr {
            op: DbOpKindIr::Insert,
            many: false,
            target: DbTargetIr {
                type_ref: record_ty.clone(),
                type_name: "Item".to_string(),
            },
            selector: None,
            query: None,
            projection: None,
            body: Some(DbBodyIr::ObjectFields {
                fields: BTreeMap::from([
                    ("id".to_string(), expression(1)),
                    ("value".to_string(), expression(2)),
                ]),
            }),
            insert_body: None,
            change: None,
            result_type: record_ty.clone(),
            source_span: None,
        };
        let expressions = vec![
            MirExpression {
                index: 0,
                expression: ExprIr::DbOperation { operation },
                ty: record_ty.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 1,
                expression: ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "item-id".to_string(),
                    },
                },
                ty: TypeRefIr::builtin("string"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 2,
                expression: ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "item-value".to_string(),
                    },
                },
                ty: TypeRefIr::builtin("string"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
        ];
        let function = function(
            "db",
            "insertItem",
            record_ty.clone(),
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
            mir_and_bundle("db", Vec::new(), ExternalRefTable::default(), function);
        let plans = plans(&unit);
        let artifact = emit_bytecode_artifact(&[unit], &[bundle], &plans)
            .expect("single-object db insert emits exact bytecode facts");
        let function = artifact.image.functions.get("db::insertItem").unwrap();
        let (intrinsic, operation) = function
            .relocations
            .iter()
            .find_map(|relocation| match relocation {
                BytecodeRelocation::IntrinsicRef { intrinsic } => intrinsic
                    .db_operation
                    .as_ref()
                    .map(|operation| (intrinsic, operation)),
                _ => None,
            })
            .expect("db insert emits an intrinsic DB operation fact");
        assert_eq!(
            operation.op,
            skiff_artifact_model::bytecode::dto::DbOperationKind::Insert
        );
        assert_eq!(operation.target.type_name, "Item");
        assert_eq!(operation.target.type_ref, record_ty);
        assert_eq!(operation.result_type, record_ty);
        assert_eq!(operation.result_plans.len(), 1);
        assert_eq!(intrinsic.signature.parameter_types, vec![record_ty.clone()]);
        assert_eq!(intrinsic.signature.result_types, vec![record_ty]);
        assert_eq!(intrinsic.signature.result_plans, operation.result_plans);
        assert_eq!(
            intrinsic.target,
            BytecodeIntrinsicRef::Static {
                canonical_key: "std.db.operation".to_string(),
                signature_version: 1,
            }
        );
    }

    #[test]
    fn string_concat_emits_receiver_intrinsic_with_explicit_operand() {
        let string_ty = TypeRefIr::builtin("string");
        let op = skiff_artifact_model::builtin_receiver_op_by_name("string", "concat")
            .expect("string.concat receiver op");
        let expressions = vec![
            MirExpression {
                index: 0,
                expression: ExprIr::Call {
                    call: CallIr {
                        target: CallTargetIr::ReceiverBuiltin { op },
                        concrete_receiver: None,
                        site: site(),
                        args: vec![expression(1), expression(2)],
                        inout_args: Vec::new(),
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                },
                ty: string_ty.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 1,
                expression: ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "a".to_string(),
                    },
                },
                ty: string_ty.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 2,
                expression: ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "b".to_string(),
                    },
                },
                ty: string_ty.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
        ];
        let function = function(
            "strings",
            "concat",
            string_ty.clone(),
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
            mir_and_bundle("strings", Vec::new(), ExternalRefTable::default(), function);
        let plans = plans(&unit);
        let artifact = emit_bytecode_artifact(&[unit], &[bundle], &plans)
            .expect("string.concat intrinsic emits");
        let view = skiff_artifact_model::bytecode::structurally_validate(&artifact)
            .expect("string.concat artifact validates");
        let function = view
            .functions()
            .iter()
            .find(|function| function.function_key == "strings::concat")
            .expect("concat function");
        assert!(function.instructions.iter().any(|instruction| {
            instruction.descriptor.kind == skiff_artifact_model::bytecode::Opcode::InvokeIntrinsic
        }));
        let intrinsic = artifact.image.functions["strings::concat"]
            .relocations
            .iter()
            .find_map(|relocation| match relocation {
                BytecodeRelocation::IntrinsicRef { intrinsic } => Some(intrinsic),
                _ => None,
            })
            .expect("string.concat emits one intrinsic relocation");
        assert!(matches!(
            &intrinsic.target,
            BytecodeIntrinsicRef::Receiver { op }
                if op.canonical_key == "receiver:string.concat@1"
        ));
        assert_eq!(intrinsic.signature.parameter_types.len(), 2);
    }

    #[test]
    fn config_require_emits_generic_host_call() {
        let string_ty = TypeRefIr::builtin("string");
        let expressions = vec![
            MirExpression {
                index: 0,
                expression: ExprIr::Call {
                    call: CallIr {
                        target: CallTargetIr::Native {
                            target: NativeTarget {
                                namespace: "config".to_string(),
                                symbol: "require".to_string(),
                                binding_key: Some("std.config.require".to_string()),
                                metadata: BTreeMap::new(),
                            },
                        },
                        concrete_receiver: None,
                        site: site(),
                        args: vec![expression(1)],
                        inout_args: Vec::new(),
                        type_args: BTreeMap::from([("T0".to_string(), string_ty.clone())]),
                        metadata: BTreeMap::new(),
                    },
                },
                ty: string_ty.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
            MirExpression {
                index: 1,
                expression: ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "app.token".to_string(),
                    },
                },
                ty: string_ty.clone(),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            },
        ];
        let function = function(
            "config",
            "load",
            string_ty.clone(),
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
            mir_and_bundle("config", Vec::new(), ExternalRefTable::default(), function);
        let plans = plans(&unit);
        let artifact = emit_bytecode_artifact(&[unit], &[bundle], &plans)
            .expect("config.require host call emits");
        let view = skiff_artifact_model::bytecode::structurally_validate(&artifact)
            .expect("config.require artifact validates");
        let function = view
            .functions()
            .iter()
            .find(|function| function.function_key == "config::load")
            .expect("load function");
        assert!(function.instructions.iter().any(|instruction| {
            instruction.descriptor.kind == skiff_artifact_model::bytecode::Opcode::InvokeHost
        }));
        let host = artifact.image.functions["config::load"]
            .relocations
            .iter()
            .find_map(|relocation| match relocation {
                BytecodeRelocation::HostEffectRef(effect) => Some(effect),
                _ => None,
            })
            .expect("config.require emits one HostEffectRef");
        assert_eq!(
            host.target.binding_key.as_deref(),
            Some("std.config.require")
        );
        assert!(host.db_operation.is_none());
    }
}
