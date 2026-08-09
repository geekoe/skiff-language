use serde_json::json;

use super::*;
use crate::refs::SourcePosition;

fn source_site() -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id: 3,
            start: SourcePosition::new(8, 2),
            end: SourcePosition::new(8, 11),
        },
    }
}

#[test]
fn throw_and_call_round_trip_required_source_sites() {
    let statement = StmtIr::Throw {
        value: ExprRefIr { expression: 1 },
        payload_type: TypeRefIr::builtin("string"),
        site: source_site(),
    };
    let call = CallIr {
        target: CallTargetIr::LocalExecutable {
            executable_index: 2,
        },
        site: InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
        },
        args: Vec::new(),
        inout_args: Vec::new(),
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
    };

    for expected in [
        serde_json::to_value(&statement).unwrap(),
        serde_json::to_value(&call).unwrap(),
    ] {
        assert!(expected.get("site").is_some());
    }
    assert_eq!(
        serde_json::from_value::<StmtIr>(serde_json::to_value(&statement).unwrap()).unwrap(),
        statement
    );
    assert_eq!(
        serde_json::from_value::<CallIr>(serde_json::to_value(&call).unwrap()).unwrap(),
        call
    );
}

#[test]
fn source_owned_instructions_reject_missing_or_invalid_sites() {
    let missing_throw_site = json!({
        "kind": "throw",
        "value": { "expression": 0 },
        "payloadType": { "kind": "builtin", "name": "string" }
    });
    let missing_call_site = json!({
        "target": { "kind": "localExecutable", "executableIndex": 0 }
    });
    let forged_synthetic_source = json!({
        "kind": "synthetic",
        "reason": "compilerDesugaring",
        "span": {
            "sourceId": 1,
            "start": { "line": 1, "column": 1 },
            "end": { "line": 1, "column": 2 }
        }
    });
    let unknown_reason = json!({
        "kind": "synthetic",
        "reason": "futureReason"
    });

    assert!(serde_json::from_value::<StmtIr>(missing_throw_site.clone()).is_err());
    assert!(serde_json::from_value::<ExprIr>(missing_throw_site).is_err());
    assert!(serde_json::from_value::<CallIr>(missing_call_site).is_err());
    assert!(serde_json::from_value::<InstructionSourceSite>(forged_synthetic_source).is_err());
    assert!(serde_json::from_value::<InstructionSourceSite>(unknown_reason).is_err());
}

#[test]
fn catch_type_is_required_and_never_null() {
    let valid = json!({
        "kind": "catch",
        "tryExpression": { "expression": 0 },
        "catchSlot": 1,
        "catchType": { "kind": "builtin", "name": "string" },
        "body": { "expression": 2 }
    });
    assert!(serde_json::from_value::<ExprIr>(valid.clone()).is_ok());

    for replacement in [None, Some(serde_json::Value::Null)] {
        let mut invalid = valid.clone();
        match replacement {
            None => {
                invalid.as_object_mut().unwrap().remove("catchType");
            }
            Some(value) => invalid["catchType"] = value,
        }
        assert!(serde_json::from_value::<ExprIr>(invalid).is_err());
    }
}

#[test]
fn representation_wrap_has_one_required_wire_shape() {
    let expected = ExprIr::RepresentationWrap {
        value: ExprRefIr { expression: 4 },
        type_ref: TypeRefIr::AppliedNominal {
            base: crate::NominalTypeRefBaseIr::LocalType { type_index: 2 },
            arguments: vec![TypeRefIr::builtin("string")],
        },
    };
    let wire = json!({
        "kind": "representationWrap",
        "value": { "expression": 4 },
        "typeRef": {
            "kind": "appliedNominal",
            "base": { "kind": "localType", "typeIndex": 2 },
            "arguments": [{ "kind": "builtin", "name": "string" }]
        }
    });
    assert_eq!(serde_json::to_value(&expected).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<ExprIr>(wire.clone()).unwrap(),
        expected
    );

    let mut invalid = Vec::new();
    for missing in ["value", "typeRef"] {
        let mut candidate = wire.clone();
        candidate.as_object_mut().unwrap().remove(missing);
        invalid.push(candidate);
    }
    let mut null_type = wire.clone();
    null_type["typeRef"] = serde_json::Value::Null;
    invalid.push(null_type);
    for forbidden in ["display", "fields", "site", "identity"] {
        let mut candidate = wire.clone();
        candidate[forbidden] = json!("forbidden");
        invalid.push(candidate);
    }
    let mut legacy_type = wire;
    legacy_type["type"] = legacy_type["typeRef"].clone();
    legacy_type.as_object_mut().unwrap().remove("typeRef");
    invalid.push(legacy_type);

    for candidate in invalid {
        assert!(
            serde_json::from_value::<ExprIr>(candidate.clone()).is_err(),
            "strict representationWrap wire must reject {candidate}"
        );
    }
}

#[test]
fn slot_ty_and_executable_type_facts_are_incremental_file_ir_fields() {
    // Empty fields are skipped on the wire so legacy File IR stays
    // byte-identical when lowering has no facts to record.
    let slot = SlotIr {
        index: 0,
        name: "x".to_string(),
        kind: SlotKind::Local,
        ty: None,
    };
    let slot_wire = serde_json::to_value(&slot).unwrap();
    assert_eq!(slot_wire.get("ty"), None);
    let executable = ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "root.mod.f".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("void"),
        self_type: None,
        slots: SlotLayout {
            slots: vec![slot],
            frame_size: 1,
        },
        may_suspend: false,
        body: ExecutableBody::default(),
        expression_types: Vec::new(),
        statement_spans: Vec::new(),
        source_span: None,
    };
    let wire = serde_json::to_value(&executable).unwrap();
    assert_eq!(wire.get("expressionTypes"), None);
    assert_eq!(wire.get("statementSpans"), None);

    // Legacy JSON without the new fields decodes with empty defaults.
    let mut legacy = wire.clone();
    legacy.as_object_mut().unwrap().insert(
        "slots".to_string(),
        serde_json::json!({
            "slots": [{ "index": 0, "name": "x", "kind": "local" }],
            "frameSize": 1
        }),
    );
    let decoded = serde_json::from_value::<ExecutableIr>(legacy).unwrap();
    assert_eq!(decoded.expression_types, Vec::<TypeRefIr>::new());
    assert_eq!(decoded.statement_spans, Vec::<Option<SourceSpanRef>>::new());
    assert_eq!(decoded.slots.slots[0].ty, None);

    // Non-empty facts serialize and round-trip in index-aligned order.
    let typed = ExecutableIr {
        expression_types: vec![TypeRefIr::builtin("number"), TypeRefIr::builtin("string")],
        statement_spans: vec![
            Some(SourceSpanRef {
                source_id: 1,
                start: SourcePosition::new(2, 1),
                end: SourcePosition::new(2, 5),
            }),
            None,
        ],
        ..executable.clone()
    };
    let typed_wire = serde_json::to_value(&typed).unwrap();
    assert_eq!(
        typed_wire.get("expressionTypes"),
        Some(&serde_json::json!([
            { "kind": "builtin", "name": "number" },
            { "kind": "builtin", "name": "string" }
        ]))
    );
    assert_eq!(
        serde_json::from_value::<ExecutableIr>(typed_wire).unwrap(),
        typed
    );
}

#[test]
fn representation_wrap_type_visitor_reaches_all_nested_arguments() {
    let nested_argument = TypeRefIr::AppliedNominal {
        base: crate::NominalTypeRefBaseIr::LocalType { type_index: 1 },
        arguments: vec![TypeRefIr::builtin("string")],
    };
    let body = ExecutableBody {
        expressions: vec![ExprIr::RepresentationWrap {
            value: ExprRefIr { expression: 0 },
            type_ref: TypeRefIr::AppliedNominal {
                base: crate::NominalTypeRefBaseIr::LocalType { type_index: 0 },
                arguments: vec![nested_argument.clone()],
            },
        }],
        ..ExecutableBody::default()
    };
    let mut visited = Vec::new();
    visit_executable_body_type_refs(&body, &mut |ty| {
        visited.push(ty.clone());
        Ok::<(), ()>(())
    })
    .unwrap();

    assert_eq!(visited.len(), 3);
    assert!(matches!(
        &visited[0],
        TypeRefIr::AppliedNominal {
            base: crate::NominalTypeRefBaseIr::LocalType { type_index: 0 },
            ..
        }
    ));
    assert_eq!(visited[1], nested_argument);
    assert_eq!(visited[2], TypeRefIr::builtin("string"));
}
mod timeout_execution;
