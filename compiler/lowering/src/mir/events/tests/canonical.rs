use std::collections::BTreeMap;

use skiff_artifact_model::{
    CallIr, CallTargetIr, ExprIr, InstructionSourceSite, SourcePosition, SourceSpanRef,
    StatementAttributionId, TypeRefIr,
};

use crate::mir::{
    finalize_mir_source_event_plan, MirEmissionAnchor, MirExpression, MirSourceEvent,
    MirSourceEventPlan,
};

#[test]
fn local_call_event_site_must_equal_the_final_call_site() {
    let call_site = source_site(1);
    let event_site = source_site(2);
    let plan = MirSourceEventPlan::checked_available(vec![MirSourceEvent {
        attribution_id: StatementAttributionId::Expression {
            expression_index: 0,
            occurrence_ordinal: 0,
        },
        site: event_site,
        anchor: MirEmissionAnchor::LocalCall {
            expression_index: 0,
            occurrence_ordinal: 0,
        },
    }])
    .unwrap();
    let expressions = vec![direct_call_expression(call_site)];
    let error = finalize_mir_source_event_plan(plan, &expressions, &[])
        .expect_err("mismatched local-call site must fail closed");
    assert!(error.message().contains("does not match call expression 0"));
}

#[test]
fn specialized_local_call_cannot_use_a_later_occurrence() {
    let site = source_site(1);
    let error = MirSourceEventPlan::checked_available(vec![
        MirSourceEvent {
            attribution_id: StatementAttributionId::Expression {
                expression_index: 0,
                occurrence_ordinal: 0,
            },
            site: site.clone(),
            anchor: MirEmissionAnchor::Expression {
                expression_index: 0,
                occurrence_ordinal: 0,
            },
        },
        MirSourceEvent {
            attribution_id: StatementAttributionId::Expression {
                expression_index: 0,
                occurrence_ordinal: 1,
            },
            site,
            anchor: MirEmissionAnchor::LocalCall {
                expression_index: 0,
                occurrence_ordinal: 1,
            },
        },
    ])
    .expect_err("specialized local-call anchor must be the root occurrence");
    assert!(error.message().contains("occurrence ordinal zero"));
}

fn direct_call_expression(site: InstructionSourceSite) -> MirExpression {
    MirExpression {
        index: 0,
        expression: ExprIr::Call {
            call: CallIr {
                target: CallTargetIr::LocalExecutable {
                    executable_index: 0,
                },
                concrete_receiver: None,
                site,
                args: Vec::new(),
                inout_args: Vec::new(),
                type_args: BTreeMap::new(),
                metadata: BTreeMap::new(),
            },
        },
        ty: TypeRefIr::builtin("void"),
        writable: None,
        direct_call: None,
    }
}

fn source_site(line: u32) -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id: 0,
            start: SourcePosition::new(line, 1),
            end: SourcePosition::new(line, 2),
        },
    }
}
