use std::collections::HashSet;

use serde_json::Value;

use super::*;

const EXPECTED_OPCODE_MANIFEST: &[(Opcode, u8, &str)] = &[
    (Opcode::Const, 0x00, "const"),
    (Opcode::CopySlot, 0x01, "copy_slot"),
    (Opcode::MoveSlot, 0x02, "move_slot"),
    (Opcode::StoreSlot, 0x03, "store_slot"),
    (Opcode::Drop, 0x04, "drop"),
    (Opcode::Dup, 0x05, "dup"),
    (Opcode::LoadSlot, 0x06, "load_slot"),
    (Opcode::TakeSlot, 0x07, "take_slot"),
    (Opcode::Pop, 0x08, "pop"),
    (Opcode::Jump, 0x10, "jump"),
    (Opcode::JumpIfTrue, 0x11, "jump_if_true"),
    (Opcode::JumpIfFalse, 0x12, "jump_if_false"),
    (Opcode::SwitchTag, 0x13, "switch_tag"),
    (Opcode::BudgetCheckpoint, 0x14, "budget_checkpoint"),
    (Opcode::Trap, 0x15, "trap"),
    (Opcode::CallLocal, 0x20, "call_local"),
    (Opcode::TailCallLocal, 0x21, "tail_call_local"),
    (Opcode::CallService, 0x22, "call_service"),
    (Opcode::CallActor, 0x23, "call_actor"),
    (Opcode::CallInterface, 0x24, "call_interface"),
    (Opcode::Return, 0x25, "return"),
    (Opcode::CallLocalInOut, 0x26, "call_local_inout"),
    (Opcode::InterfaceBoxLocal, 0x30, "interface_box_local"),
    (Opcode::InterfaceBoxRemote, 0x31, "interface_box_remote"),
    (Opcode::MakeCallback, 0x32, "make_callback"),
    (Opcode::InvokeCallback, 0x33, "invoke_callback"),
    (Opcode::NewRecord, 0x40, "new_record"),
    (Opcode::GetDenseField, 0x41, "get_dense_field"),
    (Opcode::SetWritablePath, 0x42, "set_writable_path"),
    (Opcode::RepresentationWrap, 0x43, "representation_wrap"),
    (Opcode::NewArrayBuilder, 0x50, "new_array_builder"),
    (Opcode::ArrayBuilderPush, 0x51, "array_builder_push"),
    (Opcode::FreezeArray, 0x52, "freeze_array"),
    (Opcode::ArrayGet, 0x53, "array_get"),
    (Opcode::ArrayPushOwned, 0x54, "array_push_owned"),
    (Opcode::NewMapBuilder, 0x55, "new_map_builder"),
    (Opcode::MapBuilderPut, 0x56, "map_builder_put"),
    (Opcode::FreezeMap, 0x57, "freeze_map"),
    (Opcode::MapGet, 0x58, "map_get"),
    (Opcode::MapPutOwned, 0x59, "map_put_owned"),
    (Opcode::ArrayLen, 0x5A, "array_len"),
    (Opcode::MapLen, 0x5B, "map_len"),
    (Opcode::MapEntryAt, 0x5C, "map_entry_at"),
    (Opcode::StreamNext, 0x60, "stream_next"),
    (Opcode::EmitStream, 0x61, "emit_stream"),
    (Opcode::Throw, 0x70, "throw"),
    (Opcode::Rethrow, 0x71, "rethrow"),
    (Opcode::EnterRegion, 0x72, "enter_region"),
    (Opcode::LeaveRegion, 0x73, "leave_region"),
    (Opcode::InvokeHost, 0x80, "invoke_host"),
    (Opcode::InvokeIntrinsic, 0x81, "invoke_intrinsic"),
    (Opcode::Not, 0x90, "not"),
    (Opcode::Negate, 0x91, "negate"),
    (Opcode::Add, 0x92, "add"),
    (Opcode::Subtract, 0x93, "subtract"),
    (Opcode::Multiply, 0x94, "multiply"),
    (Opcode::Divide, 0x95, "divide"),
    (Opcode::Equal, 0x96, "equal"),
    (Opcode::NotEqual, 0x97, "not_equal"),
    (Opcode::LessThan, 0x98, "less_than"),
    (Opcode::LessOrEqual, 0x99, "less_or_equal"),
    (Opcode::GreaterThan, 0x9A, "greater_than"),
    (Opcode::GreaterOrEqual, 0x9B, "greater_or_equal"),
];

#[test]
fn opcode_manifest_is_an_exact_63_row_cover() {
    assert_eq!(OPCODE_COUNT, 63);
    assert_eq!(EXPECTED_OPCODE_MANIFEST.len(), OPCODE_COUNT);
    assert_eq!(Opcode::ALL.len(), OPCODE_COUNT);
    assert_eq!(OPCODE_CONTRACTS.len(), OPCODE_COUNT);
    assert_eq!(OPCODE_TABLE.len(), OPCODE_COUNT);

    let mut kinds = HashSet::new();
    let mut encodings = HashSet::new();
    for (index, &(kind, opcode, mnemonic)) in EXPECTED_OPCODE_MANIFEST.iter().enumerate() {
        let contract = &OPCODE_CONTRACTS[index];
        let descriptor = &OPCODE_TABLE[index];
        assert_eq!(Opcode::ALL[index], kind);
        assert_eq!(
            (contract.kind, contract.opcode, contract.mnemonic),
            (kind, opcode, mnemonic)
        );
        assert_eq!(
            (descriptor.kind, descriptor.opcode, descriptor.mnemonic),
            (kind, opcode, mnemonic)
        );
        assert!(kinds.insert(kind), "duplicate opcode kind {kind:?}");
        assert!(
            encodings.insert(opcode),
            "duplicate opcode byte 0x{opcode:02x}"
        );
        assert_eq!(opcode_contract_for(opcode), Some(contract));
        assert_eq!(contract_for_opcode(kind), contract);
        assert_eq!(opcode_for(opcode), Some(descriptor));
        assert_eq!(descriptor_for_opcode(kind), descriptor);
    }
}

#[test]
fn generated_descriptor_and_full_contract_are_consistent() {
    for (contract, descriptor) in OPCODE_CONTRACTS.iter().zip(OPCODE_TABLE) {
        assert_eq!(contract.kind, descriptor.kind);
        assert_eq!(contract.opcode, descriptor.opcode);
        assert_eq!(contract.mnemonic, descriptor.mnemonic);
        assert_eq!(
            contract.operand_word_count(),
            descriptor.operand_word_count()
        );
        assert_eq!(contract.typed.stack_in.len(), descriptor.stack_in.len());
        assert_eq!(contract.typed.stack_out.len(), descriptor.stack_out.len());

        let mut roles = HashSet::new();
        let mut relocations = Vec::new();
        for (position, operand) in contract.operands.iter().enumerate() {
            assert!(
                roles.insert(operand.role),
                "{contract} repeats {:?}",
                operand.role
            );
            assert_eq!(operand.kind, descriptor.operand_layout[position]);
            assert_eq!(operand.role, descriptor.operand_roles[position]);
            assert_eq!(operand.kind, operand.role.operand_kind());
            assert_eq!(operand.kind, operand.linked_kind.operand_kind());
            assert_eq!(
                operand.kind == OperandKind::Reloc,
                !operand.allowed_relocations.is_empty(),
                "{contract} must give every reloc operand, and only reloc operands, an allowlist"
            );
            relocations.extend_from_slice(operand.allowed_relocations);
        }
        assert_eq!(relocations, descriptor.allowed_relocations);

        for (typed, legacy) in contract
            .typed
            .stack_in
            .iter()
            .zip(descriptor.stack_in)
            .chain(contract.typed.stack_out.iter().zip(descriptor.stack_out))
        {
            assert_eq!(typed.arity, legacy.arity);
            if let Arity::Declared(role) = typed.arity {
                let operand = contract
                    .operand(role)
                    .expect("declared arity operand exists");
                assert_eq!(operand.kind, OperandKind::Immediate);
            }
        }

        for group in contract
            .typed
            .stack_in
            .iter()
            .chain(contract.typed.stack_out)
        {
            if let Some(role) = group.value.operand() {
                assert!(
                    contract.operand(role).is_some(),
                    "{contract} value source role {role:?}"
                );
            }
            if let Some(role) = group.value.secondary_operand() {
                assert!(
                    contract.operand(role).is_some(),
                    "{contract} secondary value role {role:?}"
                );
            }
            if let Some(group) = group.value.input_group() {
                assert!(
                    (group as usize) < contract.typed.stack_in.len(),
                    "{contract} input group {group}"
                );
            }
        }

        assert!(!contract
            .operands
            .iter()
            .any(|operand| operand.role == OperandRole::Region));
    }
}

#[test]
fn frame_and_opcode_statement_charge_rules_are_exact() {
    assert_eq!(
        ATTRIBUTION_CHARGE_CONTRACT,
        AttributionChargeContract {
            statement: StatementChargeKind::Statement,
            expression: StatementChargeKind::Expression,
            generated: StatementChargeKind::GeneratedChunk,
        }
    );
    for (attribution, charge_kind) in [
        (
            crate::StatementAttributionClass::Statement,
            StatementChargeKind::Statement,
        ),
        (
            crate::StatementAttributionClass::Expression,
            StatementChargeKind::Expression,
        ),
        (
            crate::StatementAttributionClass::Generated,
            StatementChargeKind::GeneratedChunk,
        ),
    ] {
        assert_eq!(
            default_statement_charge_kind_for_attribution(attribution),
            charge_kind
        );
    }
    assert_eq!(
        FRAME_ENTRY_STATEMENT_CONTRACT,
        FrameEntryStatementContract {
            charge_kind: StatementChargeKind::FunctionEntry,
        }
    );
    let required = [
        (
            Opcode::CallLocal,
            StatementChargeKind::LocalCall,
            crate::StatementAttributionClass::Expression,
        ),
        (
            Opcode::CallLocalInOut,
            StatementChargeKind::LocalCall,
            crate::StatementAttributionClass::Expression,
        ),
        (
            Opcode::TailCallLocal,
            StatementChargeKind::TailHop,
            crate::StatementAttributionClass::Expression,
        ),
        (
            Opcode::BudgetCheckpoint,
            StatementChargeKind::LoopCheck,
            crate::StatementAttributionClass::Generated,
        ),
    ];
    for (opcode, charge_kind, attribution) in required {
        assert_eq!(
            contract_for_opcode(opcode).statement,
            StatementContract::RequiredEvent {
                charge_kind,
                attribution,
            }
        );
    }
    for contract in OPCODE_CONTRACTS {
        if !required
            .iter()
            .any(|(opcode, _, _)| *opcode == contract.kind)
        {
            assert_eq!(contract.statement, StatementContract::None, "{contract}");
        }
    }
}

#[test]
fn canonical_projection_contains_the_decided_runtime_semantics() {
    let projection: Value =
        serde_json::from_slice(&opcode_contract_canonical_json()).expect("canonical JSON");
    assert_eq!(projection["contractFormat"], 2);
    assert_eq!(projection["attributionCharges"]["statement"], "statement");
    assert_eq!(projection["attributionCharges"]["expression"], "expression");
    assert_eq!(
        projection["attributionCharges"]["generated"],
        "generatedChunk"
    );
    assert_eq!(
        projection["frameEntryStatement"]["chargeKind"],
        "functionEntry"
    );
    assert_eq!(projection["opcodes"].as_array().map(Vec::len), Some(63));

    let opcode = |mnemonic: &str| -> &Value {
        projection["opcodes"]
            .as_array()
            .expect("opcode array")
            .iter()
            .find(|row| row["mnemonic"] == mnemonic)
            .expect("opcode projection row")
    };

    let trap = opcode("trap");
    assert_eq!(trap["typed"]["stackIn"][0]["value"]["kind"], "bool");
    assert_eq!(trap["control"]["kind"], "fallthrough");
    assert_eq!(
        trap["exception"]["failures"][0]["trigger"],
        "assertionFalse"
    );
    assert_eq!(
        trap["exception"]["failures"][0]["disposition"]["kind"],
        "uncatchableTerminal"
    );
    assert_eq!(trap["source"]["useKind"], "assertion");

    let divide = opcode("divide");
    assert_eq!(
        divide["exception"]["failures"][0]["trigger"],
        "zeroDivisorIncludingNegativeZero"
    );
    assert_eq!(
        divide["exception"]["failures"][1]["trigger"],
        "nonFiniteResult"
    );
    assert_eq!(divide["source"]["useKind"], "generatedFailure");

    let array_get = opcode("array_get");
    assert_eq!(
        array_get["exception"]["failures"][0]["disposition"]["identity"],
        COLLECTION_INDEX_OUT_OF_BOUNDS_ERROR
    );
    let map_get = opcode("map_get");
    assert_eq!(
        map_get["exception"]["failures"][0]["disposition"]["identity"],
        COLLECTION_MISSING_KEY_ERROR
    );

    let path = opcode("set_writable_path");
    assert_eq!(
        path["exception"]["failures"][1]["trigger"],
        "intermediateMissingKey"
    );
    assert!(path["capabilities"]
        .as_array()
        .expect("capabilities")
        .contains(&Value::String("writablePathFinalMapUpsert".to_string())));

    let entry = opcode("map_entry_at");
    assert_eq!(entry["source"]["origin"], "syntheticOnly");
    assert!(entry["capabilities"]
        .as_array()
        .expect("capabilities")
        .contains(&Value::String("canonicalMapSnapshot".to_string())));

    let checkpoint = opcode("budget_checkpoint");
    assert_eq!(
        checkpoint["checkpoint"]["timeoutAttribution"],
        "activeRegionSite"
    );
    assert_eq!(checkpoint["source"]["useKind"], "generatedFailure");
    assert_eq!(checkpoint["statement"]["kind"], "requiredEvent");
    assert_eq!(checkpoint["statement"]["chargeKind"], "loopCheck");
    assert_eq!(checkpoint["statement"]["attribution"], "generated");

    let local_call = opcode("call_local");
    assert_eq!(local_call["statement"]["chargeKind"], "localCall");
    assert_eq!(local_call["statement"]["attribution"], "expression");

    let tail_call = opcode("tail_call_local");
    assert_eq!(tail_call["statement"]["chargeKind"], "tailHop");
    assert_eq!(tail_call["statement"]["attribution"], "expression");

    assert_eq!(opcode("call_service")["statement"]["kind"], "none");

    let rethrow = opcode("rethrow");
    assert_eq!(rethrow["exception"]["behavior"]["kind"], "preserveOriginal");
    assert_eq!(rethrow["source"]["kind"], "preserveOriginal");

    let inout = opcode("call_local_inout");
    assert_eq!(inout["operands"][0]["linkedKind"], "function");
    assert_eq!(inout["operands"][3]["linkedKind"], "callLoanLayout");
    assert_eq!(
        inout["typed"]["stackIn"][0]["value"]["kind"],
        "inOutCallInputs"
    );
    assert_eq!(inout["typed"]["slots"]["kind"], "inOutCallLoans");
    assert_eq!(inout["pending"]["kind"], "noPendingTarget");
}

#[test]
fn every_top_level_contract_field_changes_the_fingerprint() {
    let baseline = opcode_table_fingerprint();
    let assert_changed = |label: &str, mutate: &dyn Fn(&mut Vec<OpcodeContract>)| {
        let mut contracts = OPCODE_CONTRACTS.to_vec();
        mutate(&mut contracts);
        assert_ne!(
            super::fingerprint::opcode_contracts_fingerprint(&contracts),
            baseline,
            "{label} is missing from the canonical projection"
        );
    };

    assert_changed("kind", &|contracts| contracts[0].kind = Opcode::CopySlot);
    assert_changed("opcode", &|contracts| contracts[0].opcode = 0xFE);
    assert_changed("mnemonic", &|contracts| {
        contracts[0].mnemonic = "mutated_const"
    });
    assert_changed("operands", &|contracts| {
        contracts[0].operands = contract_for_opcode(Opcode::RepresentationWrap).operands;
    });
    assert_changed("typed stack input", &|contracts| {
        contracts[0].typed.stack_in = contract_for_opcode(Opcode::Dup).typed.stack_in;
    });
    assert_changed("typed stack output", &|contracts| {
        contracts[0].typed.stack_out = contract_for_opcode(Opcode::Dup).typed.stack_out;
    });
    assert_changed("typed slots", &|contracts| {
        contracts[0].typed.slots = contract_for_opcode(Opcode::CopySlot).typed.slots;
    });
    assert_changed("control", &|contracts| {
        contracts[0].control = contract_for_opcode(Opcode::Return).control;
    });
    assert_changed("pending", &|contracts| {
        contracts[0].pending = contract_for_opcode(Opcode::CallService).pending;
    });
    assert_changed("checkpoint", &|contracts| {
        contracts[0].checkpoint = contract_for_opcode(Opcode::BudgetCheckpoint).checkpoint;
    });
    assert_changed("exception behavior", &|contracts| {
        contracts[0].exception.behavior = contract_for_opcode(Opcode::Throw).exception.behavior;
    });
    assert_changed("checked failures", &|contracts| {
        contracts[0].exception.failures = contract_for_opcode(Opcode::Divide).exception.failures;
    });
    assert_changed("statement", &|contracts| {
        contracts[0].statement = contract_for_opcode(Opcode::CallLocal).statement;
    });
    assert_changed("source", &|contracts| {
        contracts[0].source = contract_for_opcode(Opcode::Trap).source;
    });
    assert_changed("normal region effect", &|contracts| {
        contracts[0].region.normal = RegionEffect::ExitFunction;
    });
    assert_changed("raised region effect", &|contracts| {
        contracts[0].region.raised = RegionEffect::Unwind;
    });
    assert_changed("capabilities", &|contracts| {
        contracts[0].capabilities = contract_for_opcode(Opcode::NewArrayBuilder).capabilities;
    });

    let changed_frame = FrameEntryStatementContract {
        charge_kind: StatementChargeKind::Statement,
    };
    assert_ne!(
        super::fingerprint::opcode_contracts_fingerprint_with_frame(
            changed_frame,
            OPCODE_CONTRACTS,
        ),
        baseline,
        "frame-entry rule is missing from the canonical projection"
    );

    let changed_attribution = AttributionChargeContract {
        generated: StatementChargeKind::Expression,
        ..ATTRIBUTION_CHARGE_CONTRACT
    };
    assert_ne!(
        super::fingerprint::opcode_contracts_fingerprint_with_statement_authority(
            changed_attribution,
            FRAME_ENTRY_STATEMENT_CONTRACT,
            OPCODE_CONTRACTS,
        ),
        baseline,
        "attribution charge rule is missing from the canonical projection"
    );
}

#[test]
fn opcode_contract_fingerprint_is_frozen() {
    assert_eq!(
        opcode_table_fingerprint(),
        "89d4d4d42abe321353bb4377bdbfa4f641eb82e0d23ed288e03d0da7a4103509"
    );
}
