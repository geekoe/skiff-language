use super::*;
use crate::{ExprRefIr, InstructionSourceSite, NamedUnionBranchIr, SyntheticInstructionSiteReason};

const RETIRED_CANCEL_ERROR: &str = "CancelError";

#[test]
fn file_ir_rejects_retired_cancel_error_in_ordinary_type_ref() {
    let mut unit = FileIrUnit::empty("main", "source");
    unit.type_table.push(TypeDeclIr {
        name: "Legacy".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::from([(
                "cancel".to_string(),
                TypeRefIr::builtin(RETIRED_CANCEL_ERROR),
            )]),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });

    assert_retired_cancel_error_rejected(&unit, "typeTable[0]");
}

#[test]
fn file_ir_rejects_retired_cancel_error_in_throw_payload() {
    let mut unit = FileIrUnit::empty("main", "source");
    unit.constants.push(constant_with_expressions(vec![
        null_expression(),
        ExprIr::Throw {
            value: ExprRefIr { expression: 0 },
            payload_type: TypeRefIr::builtin(RETIRED_CANCEL_ERROR),
            site: synthetic_site(),
        },
    ]));

    assert_retired_cancel_error_rejected(&unit, "constants[0]");
}

#[test]
fn file_ir_rejects_retired_cancel_error_in_catch_type() {
    let mut unit = FileIrUnit::empty("main", "source");
    unit.constants.push(constant_with_expressions(vec![
        null_expression(),
        ExprIr::Catch {
            try_expression: ExprRefIr { expression: 0 },
            catch_slot: 0,
            catch_type: TypeRefIr::builtin(RETIRED_CANCEL_ERROR),
            body: ExprRefIr { expression: 0 },
        },
    ]));

    assert_retired_cancel_error_rejected(&unit, "constants[0]");
}

#[test]
fn file_ir_rejects_retired_cancel_error_in_nested_union() {
    let mut unit = FileIrUnit::empty("main", "source");
    unit.type_table.push(TypeDeclIr {
        name: "LegacyUnion".to_string(),
        descriptor: TypeDescriptorIr::Union {
            branches: vec![NamedUnionBranchIr::SyntheticDiscriminator {
                discriminator_field: "kind".to_string(),
                discriminator_value: "legacy".to_string(),
                payload_type: TypeRefIr::Record {
                    fields: BTreeMap::from([(
                        "nested".to_string(),
                        TypeRefIr::Union {
                            items: vec![
                                TypeRefIr::builtin("TimeoutError"),
                                TypeRefIr::Nullable {
                                    inner: Box::new(TypeRefIr::builtin(RETIRED_CANCEL_ERROR)),
                                },
                            ],
                        },
                    )]),
                },
            }],
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });

    assert_retired_cancel_error_rejected(&unit, "typeTable[0]");
}

#[test]
fn file_ir_keeps_timeout_error_admitted_in_the_same_carriers() {
    let timeout = || TypeRefIr::builtin("TimeoutError");
    let mut unit = FileIrUnit::empty("main", "source");
    unit.type_table.push(TypeDeclIr {
        name: "Timeouts".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::from([(
                "nested".to_string(),
                TypeRefIr::Union {
                    items: vec![
                        timeout(),
                        TypeRefIr::Nullable {
                            inner: Box::new(timeout()),
                        },
                    ],
                },
            )]),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    unit.constants.push(constant_with_expressions(vec![
        null_expression(),
        ExprIr::Throw {
            value: ExprRefIr { expression: 0 },
            payload_type: timeout(),
            site: synthetic_site(),
        },
        ExprIr::Catch {
            try_expression: ExprRefIr { expression: 1 },
            catch_slot: 0,
            catch_type: timeout(),
            body: ExprRefIr { expression: 0 },
        },
    ]));

    validate_file_ir_type_refs(&unit).expect("TimeoutError must remain admitted");
}

fn constant_with_expressions(expressions: Vec<ExprIr>) -> ConstIr {
    ConstIr {
        name: "legacy".to_string(),
        ty: TypeRefIr::builtin("void"),
        body: ExecutableBody {
            expressions,
            ..ExecutableBody::default()
        },
        source_span: None,
    }
}

fn null_expression() -> ExprIr {
    ExprIr::Literal {
        value: crate::LiteralIr::Null,
    }
}

fn synthetic_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

fn assert_retired_cancel_error_rejected(unit: &FileIrUnit, expected_location: &str) {
    let error = validate_file_ir_type_refs(unit)
        .expect_err("retired CancelError File IR spelling must fail admission");
    assert_eq!(error.location, expected_location);
    assert!(
        error.message.contains(RETIRED_CANCEL_ERROR),
        "diagnostic must retain the rejected legacy spelling: {error}"
    );
}
