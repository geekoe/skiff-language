use skiff_artifact_model::{
    ExprIr, InstructionSourceSite, StatementAttributionId, SyntheticInstructionSiteReason,
};

use super::{function, lower_sources, lower_sources_for_package};
use crate::mir::{MirEmissionAnchor, MirStmtKind};

#[test]
fn ternary_desugaring_has_exact_generated_statement_coverage() {
    let module = "internal.ternary_events";
    let lowered = lower_sources(&[(
        "internal/ternary_events.skiff",
        module,
        r#"
          function choose(flag: boolean) -> number {
            return flag ? 1 : 2
          }
        "#,
    )]);
    let choose = function(&lowered, module, "choose");
    let events = choose
        .source_event_plan
        .events()
        .expect("ternary fixture has a checked available plan");
    let mut generated = events
        .iter()
        .filter_map(|event| match event.attribution_id {
            StatementAttributionId::Generated { ordinal } => Some((ordinal, event)),
            _ => None,
        })
        .collect::<Vec<_>>();
    generated.sort_by_key(|(ordinal, _)| *ordinal);
    assert_eq!(generated.len(), 3);
    for (expected, (ordinal, event)) in generated.iter().enumerate() {
        assert_eq!(*ordinal, u32::try_from(expected).unwrap());
        assert!(matches!(
            &event.site,
            InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerDesugaring,
            }
        ));
        let MirEmissionAnchor::GeneratedStatement {
            statement_index, ..
        } = event.anchor
        else {
            panic!("ternary generated event has a statement placement")
        };
        assert!(choose
            .statements
            .iter()
            .any(|statement| statement.statement_index == statement_index));
    }

    let value_blocks = choose
        .expressions
        .iter()
        .filter_map(|expression| match &expression.expression {
            ExprIr::ValueBlock { block, result } => Some((expression.index, block.clone(), *result)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(value_blocks.len(), 1);
    let (index, block_label, result) = value_blocks[0].clone();
    let fact = choose
        .expression_blocks
        .get(&index)
        .expect("ternary ValueBlock has an exact completion fact");
    assert_eq!(fact.result, result);
    assert_eq!(choose.block(fact.body_block).expect("body block").label, block_label);
    assert_eq!(fact.completion_targets.len(), 1);
    choose.block(fact.completion_targets[0]).expect("completion target block");
    choose
        .validate_expression_block_facts()
        .expect("ternary expression block facts stay contract-valid");
}

#[test]
fn native_wrapper_has_one_explicit_generated_statement_event() {
    let module = "std.wrapper_events";
    let lowered = lower_sources_for_package(
        "skiff.run/std",
        &[(
            "std/wrapper_events.skiff",
            module,
            "native function passthrough(value: string) -> string",
        )],
    );
    let wrapper = function(&lowered, module, "passthrough");
    let events = wrapper
        .source_event_plan
        .events()
        .expect("registered zero-row native owner produces an available plan");
    let [event] = events else {
        panic!("native wrapper has exactly one generated event")
    };
    assert_eq!(
        event.attribution_id,
        StatementAttributionId::Generated { ordinal: 0 }
    );
    assert!(matches!(
        event.site,
        InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
        }
    ));
    let MirEmissionAnchor::GeneratedStatement {
        statement_index, ..
    } = event.anchor
    else {
        panic!("native wrapper event has a generated statement anchor")
    };
    assert!(matches!(
        wrapper
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find(|statement| statement.statement_index == statement_index)
            .map(|statement| &statement.kind),
        Some(MirStmtKind::Return { .. })
    ));
}
