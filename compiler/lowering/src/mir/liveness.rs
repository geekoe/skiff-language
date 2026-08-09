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
//! The File IR expression stream backing the function must be supplied by
//! the caller (the MIR carries expression indices, not expression bodies);
//! only slot reads/writes are observed, never types.
//!
//! The API is consumed by the emitter in Wave 4; until then it is exercised
//! by the test suite only.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{AssignTargetIr, ExprIr, PatternIr};

use super::{MirBlockLiveness, MirFunction, MirLiveness, MirStmtKind};

/// May-liveness for one function, backed by the File IR expression stream.
pub fn compute_liveness(function: &MirFunction, expressions: &[ExprIr]) -> MirLiveness {
    let (uses, defs) = block_use_def_sets(function, expressions);
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

    MirLiveness {
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
    }
}

fn block_use_def_sets(
    function: &MirFunction,
    expressions: &[ExprIr],
) -> (BTreeMap<u32, BTreeSet<u32>>, BTreeMap<u32, BTreeSet<u32>>) {
    let mut uses = BTreeMap::new();
    let mut defs = BTreeMap::new();
    for block in &function.blocks {
        let mut block_uses = BTreeSet::new();
        let mut block_defs = BTreeSet::new();
        for statement in &block.statements {
            statement_use_def(
                &statement.kind,
                &mut block_uses,
                &mut block_defs,
                expressions,
            );
        }
        uses.insert(block.id, block_uses);
        defs.insert(block.id, block_defs);
    }
    (uses, defs)
}

fn statement_use_def(
    statement: &MirStmtKind,
    uses: &mut BTreeSet<u32>,
    defs: &mut BTreeSet<u32>,
    expressions: &[ExprIr],
) {
    fn visit(
        expression: u32,
        uses: &mut BTreeSet<u32>,
        defs: &mut BTreeSet<u32>,
        expressions: &[ExprIr],
    ) {
        expression_use_def(expression, uses, defs, expressions, &mut BTreeSet::new());
    }
    match statement {
        MirStmtKind::Let { slot, value } => {
            defs.insert(*slot);
            visit(value.expression, uses, defs, expressions);
        }
        MirStmtKind::Assign { target, value } => {
            visit(value.expression, uses, defs, expressions);
            match target {
                AssignTargetIr::Slot { slot } => {
                    defs.insert(*slot);
                }
                AssignTargetIr::Field { object, .. } => {
                    visit(object.expression, uses, defs, expressions);
                }
                AssignTargetIr::Index { object, index } => {
                    visit(object.expression, uses, defs, expressions);
                    visit(index.expression, uses, defs, expressions);
                }
                AssignTargetIr::ActorSelfField { .. } => {}
            }
        }
        MirStmtKind::Assert { condition, message } => {
            visit(condition.expression, uses, defs, expressions);
            if let Some(message) = message {
                visit(message.expression, uses, defs, expressions);
            }
        }
        MirStmtKind::Dispatch { call } => visit(call.expression, uses, defs, expressions),
        MirStmtKind::Emit { value, .. } => visit(value.expression, uses, defs, expressions),
        MirStmtKind::Expr { value } => visit(value.expression, uses, defs, expressions),
        MirStmtKind::Return { value } => {
            if let Some(value) = value {
                visit(value.expression, uses, defs, expressions);
            }
        }
        MirStmtKind::Throw { value, .. } => visit(value.expression, uses, defs, expressions),
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
                visit(expected.value.expression, uses, defs, expressions);
            }
            match outcome {
                skiff_artifact_model::TestEffectOutcomeIr::Respond { value, .. } => {
                    visit(value.expression, uses, defs, expressions);
                }
                skiff_artifact_model::TestEffectOutcomeIr::Throw { value, .. } => {
                    visit(value.expression, uses, defs, expressions);
                }
                skiff_artifact_model::TestEffectOutcomeIr::Stream { values, .. } => {
                    for value in values {
                        visit(value.expression, uses, defs, expressions);
                    }
                }
            }
        }
        MirStmtKind::If { condition, .. } => visit(condition.expression, uses, defs, expressions),
        MirStmtKind::ForIn {
            item_slot,
            value_slot,
            iterable,
            ..
        } => {
            defs.insert(*item_slot);
            if let Some(value_slot) = value_slot {
                defs.insert(*value_slot);
            }
            visit(iterable.expression, uses, defs, expressions);
        }
        MirStmtKind::While { condition, .. } => {
            visit(condition.expression, uses, defs, expressions)
        }
        MirStmtKind::Match { value, arms } => {
            visit(value.expression, uses, defs, expressions);
            for arm in arms {
                pattern_defs(&arm.pattern, defs);
            }
        }
        MirStmtKind::Timeout { .. }
        | MirStmtKind::Concurrent { .. }
        | MirStmtKind::Break
        | MirStmtKind::Continue => {}
    }
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
    expressions: &[ExprIr],
    seen: &mut BTreeSet<u32>,
) {
    if !seen.insert(expression) {
        return;
    }
    let Some(expr) = expressions.get(expression as usize) else {
        return;
    };
    fn visit(
        child: u32,
        uses: &mut BTreeSet<u32>,
        defs: &mut BTreeSet<u32>,
        expressions: &[ExprIr],
        seen: &mut BTreeSet<u32>,
    ) {
        expression_use_def(child, uses, defs, expressions, seen);
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
            visit(try_expression.expression, uses, defs, expressions, seen);
            visit(body.expression, uses, defs, expressions, seen);
        }
        ExprIr::Field { object, .. } => visit(object.expression, uses, defs, expressions, seen),
        ExprIr::Construct { fields, .. } => {
            for value in fields.values() {
                visit(value.expression, uses, defs, expressions, seen);
            }
        }
        ExprIr::RepresentationWrap { value, .. } | ExprIr::InterfaceBox { value, .. } => {
            visit(value.expression, uses, defs, expressions, seen)
        }
        ExprIr::MapLiteral { entries } => {
            for value in entries.values() {
                visit(value.expression, uses, defs, expressions, seen);
            }
        }
        ExprIr::ArrayLiteral { items } => {
            for item in items {
                visit(item.expression, uses, defs, expressions, seen);
            }
        }
        ExprIr::Unary { value, .. } => visit(value.expression, uses, defs, expressions, seen),
        ExprIr::Binary { left, right, .. } => {
            visit(left.expression, uses, defs, expressions, seen);
            visit(right.expression, uses, defs, expressions, seen);
        }
        ExprIr::Call { call } => {
            for argument in &call.args {
                visit(argument.expression, uses, defs, expressions, seen);
            }
        }
        ExprIr::Throw { value, .. } => visit(value.expression, uses, defs, expressions, seen),
        ExprIr::Timeout { value, .. } => visit(value.expression, uses, defs, expressions, seen),
        ExprIr::ValueBlock { result, .. } => {
            visit(result.expression, uses, defs, expressions, seen)
        }
        ExprIr::ConcurrentValue { plan } => {
            for lane in &plan.lanes {
                if let skiff_artifact_model::ConcurrentLaneIr::Tail { tail, .. } = lane {
                    visit(tail.expression, uses, defs, expressions, seen);
                }
            }
        }
        ExprIr::DbOperation { operation } => {
            for child in crate::mir::builder::db_operation_child_expressions(operation) {
                visit(child, uses, defs, expressions, seen);
            }
        }
        ExprIr::DbQuery { query } => {
            for child in crate::mir::builder::db_query_child_expressions(&query.query) {
                visit(child, uses, defs, expressions, seen);
            }
        }
        ExprIr::DbTransaction { transaction } => {
            visit(transaction.result.expression, uses, defs, expressions, seen)
        }
        ExprIr::DbLeaseClaim { claim } => {
            visit(claim.key.expression, uses, defs, expressions, seen)
        }
        ExprIr::DbLeaseRead { read } => visit(read.key.expression, uses, defs, expressions, seen),
        ExprIr::Literal { .. }
        | ExprIr::LoadConst { .. }
        | ExprIr::LoadPackageConst { .. }
        | ExprIr::ActorSelfField { .. }
        | ExprIr::Rethrow { .. } => {}
    }
}
