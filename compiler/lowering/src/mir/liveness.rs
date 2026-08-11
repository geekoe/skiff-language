//! Standard may-liveness dataflow at slot granularity (design §2.4).
//!
//! For every block: `use` = slots read by the block's statements and their
//! expression DAGs; `def` = slots written by statements (let / slot assign /
//! for-in bindings / match pattern bindings / catch slots). A slot is
//! live-in to a block when it may be used before redefinition on some path:
//! `live_in(b) = use(b) ∪ (live_out(b) − def(b))` with
//! `live_out(b) = ∪ live_in(s) over successors(b)`. Iterates to a fixed
//! point; final sets are sorted for deterministic output.
//!
//! Expressions are read exclusively through the function-owned MIR expression
//! table. No external File IR slice is accepted; only slot reads/writes are
//! observed, never types.
//!
//! The API is consumed by the emitter in Wave 4; until then it is exercised
//! by the test suite only.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{AssignTargetIr, ExprIr, ExprRefIr, PatternIr};

use super::{MirBlockLiveness, MirContractError, MirFunction, MirLiveness, MirStmtKind};

type BlockSlotSets = BTreeMap<u32, BTreeSet<u32>>;

/// May-liveness for one function, backed only by its owned MIR expressions.
pub fn compute_liveness(function: &MirFunction) -> Result<MirLiveness, MirContractError> {
    function.validate_expression_indices()?;
    function.validate_expression_block_facts()?;
    validate_cfg(function)?;
    let (uses, defs) = block_use_def_sets(function)?;
    let mut live_in: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let mut live_out: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for block in &function.blocks {
        live_in.insert(block.id, uses.get(&block.id).cloned().unwrap_or_default());
        live_out.insert(block.id, BTreeSet::new());
    }

    loop {
        let mut changed = false;
        for block in &function.blocks {
            let out = block
                .successors
                .iter()
                .flat_map(|successor| live_in.get(successor).into_iter().flatten().copied())
                .collect::<BTreeSet<u32>>();
            if out != live_out[&block.id] {
                live_out.insert(block.id, out);
                changed = true;
            }
            let block_uses = uses.get(&block.id).cloned().unwrap_or_default();
            let block_defs = defs.get(&block.id).cloned().unwrap_or_default();
            let live = block_uses
                .union(
                    &live_out[&block.id]
                        .difference(&block_defs)
                        .copied()
                        .collect(),
                )
                .copied()
                .collect::<BTreeSet<u32>>();
            if live != live_in[&block.id] {
                live_in.insert(block.id, live);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    Ok(MirLiveness {
        blocks: function
            .blocks
            .iter()
            .map(|block| {
                let liveness = MirBlockLiveness {
                    live_in: live_in[&block.id].iter().copied().collect(),
                    live_out: live_out[&block.id].iter().copied().collect(),
                };
                (block.id, liveness)
            })
            .collect(),
    })
}

fn validate_cfg(function: &MirFunction) -> Result<(), MirContractError> {
    for (expected, block) in function.blocks.iter().enumerate() {
        let expected =
            u32::try_from(expected).map_err(|_| MirContractError::BlockIndexOverflow {
                function: function.symbol.clone(),
            })?;
        if block.id != expected {
            return Err(MirContractError::BlockIndexMismatch {
                function: function.symbol.clone(),
                expected,
                stored: block.id,
            });
        }
        for successor in &block.successors {
            if function
                .blocks
                .get(*successor as usize)
                .is_none_or(|target| target.id != *successor)
            {
                return Err(MirContractError::MissingSuccessorBlock {
                    function: function.symbol.clone(),
                    block: block.id,
                    successor: *successor,
                });
            }
        }
    }
    Ok(())
}

fn block_use_def_sets(
    function: &MirFunction,
) -> Result<(BlockSlotSets, BlockSlotSets), MirContractError> {
    let mut uses = BTreeMap::new();
    let mut defs = BTreeMap::new();
    for block in &function.blocks {
        let mut block_uses = BTreeSet::new();
        let mut block_defs = BTreeSet::new();
        for statement in &block.statements {
            statement_use_def(function, &statement.kind, &mut block_uses, &mut block_defs)?;
        }
        uses.insert(block.id, block_uses);
        defs.insert(block.id, block_defs);
    }
    Ok((uses, defs))
}

fn statement_use_def(
    function: &MirFunction,
    statement: &MirStmtKind,
    uses: &mut BTreeSet<u32>,
    defs: &mut BTreeSet<u32>,
) -> Result<(), MirContractError> {
    fn visit(
        expression: u32,
        uses: &mut BTreeSet<u32>,
        defs: &mut BTreeSet<u32>,
        function: &MirFunction,
    ) -> Result<(), MirContractError> {
        expression_use_def(expression, uses, defs, function, &mut BTreeSet::new())
    }
    match statement {
        MirStmtKind::InitSlot { slot, value } => {
            defs.insert(*slot);
            visit(value.expression, uses, defs, function)?;
        }
        MirStmtKind::Assign { target, value, .. } => {
            visit(value.expression, uses, defs, function)?;
            match target {
                AssignTargetIr::Slot { slot } => {
                    defs.insert(*slot);
                }
                AssignTargetIr::Field { object, .. } => {
                    visit(object.expression, uses, defs, function)?;
                }
                AssignTargetIr::Index { object, index } => {
                    visit(object.expression, uses, defs, function)?;
                    visit(index.expression, uses, defs, function)?;
                }
                AssignTargetIr::ActorSelfField { .. } => {}
            }
        }
        MirStmtKind::Assert { condition, message } => {
            visit(condition.expression, uses, defs, function)?;
            if let Some(message) = message {
                visit(message.expression, uses, defs, function)?;
            }
        }
        MirStmtKind::Dispatch { call } => visit(call.expression, uses, defs, function)?,
        MirStmtKind::Emit { value, .. } => visit(value.expression, uses, defs, function)?,
        MirStmtKind::StreamNext { endpoint_slot, .. } => {
            uses.insert(*endpoint_slot);
        }
        MirStmtKind::Expr { value } => visit(value.expression, uses, defs, function)?,
        MirStmtKind::Return { value } => {
            if let Some(value) = value {
                visit(value.expression, uses, defs, function)?;
            }
        }
        MirStmtKind::Throw { value, .. } => visit(value.expression, uses, defs, function)?,
        MirStmtKind::Rethrow { exception_slot } => {
            uses.insert(*exception_slot);
        }
        MirStmtKind::TestEffectRegister {
            expect,
            step_expect,
            outcome,
            ..
        } => {
            for expected in expect.iter().chain(step_expect.iter()) {
                visit(expected.value.expression, uses, defs, function)?;
            }
            match outcome {
                skiff_artifact_model::TestEffectOutcomeIr::Respond { value, .. } => {
                    visit(value.expression, uses, defs, function)?;
                }
                skiff_artifact_model::TestEffectOutcomeIr::Throw { value, .. } => {
                    visit(value.expression, uses, defs, function)?;
                }
                skiff_artifact_model::TestEffectOutcomeIr::Stream { values, .. } => {
                    for value in values {
                        visit(value.expression, uses, defs, function)?;
                    }
                }
            }
        }
        MirStmtKind::If { condition, .. } => visit(condition.expression, uses, defs, function)?,
        MirStmtKind::ForIn {
            facts, iterable, ..
        } => {
            match &facts.binding {
                super::MirForInBinding::Item { slot, .. } => {
                    defs.insert(*slot);
                }
                super::MirForInBinding::MapEntry {
                    key_slot,
                    value_slot,
                    ..
                } => {
                    defs.insert(*key_slot);
                    defs.insert(*value_slot);
                }
            }
            visit(iterable.expression, uses, defs, function)?;
        }
        MirStmtKind::While { condition, .. } => {
            visit(condition.expression, uses, defs, function)?;
        }
        MirStmtKind::Match { value, arms } => {
            visit(value.expression, uses, defs, function)?;
            for arm in arms {
                pattern_defs(&arm.pattern, defs);
            }
        }
        MirStmtKind::Concurrent { plan } => {
            for lane in &plan.lanes {
                if let super::MirConcurrentLaneIr::Tail { tail, .. } = lane {
                    visit(tail.expression, uses, defs, function)?;
                }
            }
        }
        MirStmtKind::Timeout { .. } | MirStmtKind::Break | MirStmtKind::Continue => {}
    }
    Ok(())
}

fn pattern_defs(pattern: &PatternIr, defs: &mut BTreeSet<u32>) {
    match pattern {
        PatternIr::Binding { slot } => {
            defs.insert(*slot);
        }
        PatternIr::Record { fields } => {
            for field in fields {
                pattern_defs(&field.pattern, defs);
            }
        }
        PatternIr::Wildcard | PatternIr::Literal { .. } | PatternIr::Type { .. } => {}
    }
}

fn expression_use_def(
    expression: u32,
    uses: &mut BTreeSet<u32>,
    defs: &mut BTreeSet<u32>,
    function: &MirFunction,
    seen: &mut BTreeSet<u32>,
) -> Result<(), MirContractError> {
    if !seen.insert(expression) {
        return Ok(());
    }
    let expr = &function.expression(ExprRefIr { expression })?.expression;
    fn visit(
        child: u32,
        uses: &mut BTreeSet<u32>,
        defs: &mut BTreeSet<u32>,
        function: &MirFunction,
        seen: &mut BTreeSet<u32>,
    ) -> Result<(), MirContractError> {
        expression_use_def(child, uses, defs, function, seen)
    }
    match expr {
        ExprIr::LoadSlot { slot } => {
            uses.insert(*slot);
        }
        ExprIr::Catch {
            try_expression,
            catch_slot,
            body,
            ..
        } => {
            // The runtime writes the caught exception into the catch slot.
            defs.insert(*catch_slot);
            visit(try_expression.expression, uses, defs, function, seen)?;
            visit(body.expression, uses, defs, function, seen)?;
        }
        ExprIr::Field { object, .. } => {
            visit(object.expression, uses, defs, function, seen)?;
        }
        ExprIr::Index { object, index } => {
            visit(object.expression, uses, defs, function, seen)?;
            visit(index.expression, uses, defs, function, seen)?;
        }
        ExprIr::Construct { fields, .. } => {
            for value in fields.values() {
                visit(value.expression, uses, defs, function, seen)?;
            }
        }
        ExprIr::RepresentationWrap { value, .. } | ExprIr::InterfaceBox { value, .. } => {
            visit(value.expression, uses, defs, function, seen)?;
        }
        ExprIr::MapLiteral { entries } => {
            for value in entries.values() {
                visit(value.expression, uses, defs, function, seen)?;
            }
        }
        ExprIr::ArrayLiteral { items } => {
            for item in items {
                visit(item.expression, uses, defs, function, seen)?;
            }
        }
        ExprIr::Unary { value, .. } => {
            visit(value.expression, uses, defs, function, seen)?;
        }
        ExprIr::Binary { left, right, .. } => {
            visit(left.expression, uses, defs, function, seen)?;
            visit(right.expression, uses, defs, function, seen)?;
        }
        ExprIr::Call { call } => {
            for argument in &call.args {
                visit(argument.expression, uses, defs, function, seen)?;
            }
            for argument in &call.inout_args {
                uses.insert(argument.root_slot);
                defs.insert(argument.root_slot);
                for segment in &argument.path {
                    if let skiff_artifact_model::InOutPathSegmentIr::Index { selector } = segment {
                        visit(selector.expression, uses, defs, function, seen)?;
                    }
                }
            }
        }
        ExprIr::Throw { value, .. } => {
            visit(value.expression, uses, defs, function, seen)?;
        }
        ExprIr::Timeout { value, .. } => {
            visit(value.expression, uses, defs, function, seen)?;
        }
        ExprIr::ValueBlock { result, .. } => {
            visit(result.expression, uses, defs, function, seen)?;
        }
        ExprIr::ConcurrentValue { plan } => {
            for lane in &plan.lanes {
                if let skiff_artifact_model::ConcurrentLaneIr::Tail { tail, .. } = lane {
                    visit(tail.expression, uses, defs, function, seen)?;
                }
            }
        }
        ExprIr::DbOperation { operation } => {
            for child in crate::mir::builder::db_operation_child_expressions(operation) {
                visit(child, uses, defs, function, seen)?;
            }
        }
        ExprIr::DbQuery { query } => {
            for child in crate::mir::builder::db_query_child_expressions(&query.query) {
                visit(child, uses, defs, function, seen)?;
            }
        }
        ExprIr::DbTransaction { transaction } => {
            visit(transaction.result.expression, uses, defs, function, seen)?;
        }
        ExprIr::DbLeaseClaim { claim } => {
            visit(claim.key.expression, uses, defs, function, seen)?;
        }
        ExprIr::DbLeaseRead { read } => {
            visit(read.key.expression, uses, defs, function, seen)?;
        }
        ExprIr::Literal { .. }
        | ExprIr::LoadConst { .. }
        | ExprIr::LoadPackageConst { .. }
        | ExprIr::ActorSelfField { .. }
        | ExprIr::Rethrow { .. } => {}
    }
    Ok(())
}
