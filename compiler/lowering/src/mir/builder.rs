//! MIR construction: a post-pass over `FileIrUnit` plus source-owned effect
//! facts (design §2.4). See `super` module docs for the CFG rules.

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::CallableEffectSummary;
use skiff_artifact_model::{
    AssignTargetIr, ConcurrentLaneIr, ConcurrentPlanIr, DbBodyIr, DbChangeOpIr, DbPredicateIr,
    DbSelectorIr, ExecutableIr, ExprIr, FileIrUnit, SourceSpanRef, StmtIr,
};
use skiff_compiler_source::{SourceCallableEffectFacts, SourceSymbolKey};

use super::{
    MirBlock, MirConcurrentLaneIr, MirConcurrentPlanIr, MirExecutableKind, MirFunction,
    MirMatchArmIr, MirParam, MirParamMode, MirRegion, MirSlot, MirSlotKind, MirStatementEntry,
    MirStmt, MirStmtKind, MirUnit,
};

/// Per-callable effect facts resolved from the source model. The MIR never
/// infers effects from File IR (design §2.4 stop condition);
/// `effect_summary_ref` is the stable `"{module_path}::{symbol}"` rendering of
/// the callable identity (see `super` module docs).
pub type CallableEffectMap = BTreeMap<SourceSymbolKey, (bool, String)>;

/// One MIR unit per File IR unit; functions are ordered by declaration name
/// (deterministic `BTreeMap` iteration).
pub fn build_mir_units(
    units: &[FileIrUnit],
    effects: &SourceCallableEffectFacts,
) -> Result<Vec<MirUnit>, String> {
    let mut per_callable = CallableEffectMap::new();
    for (source_key, summary) in effects.operations() {
        let may_pending = match summary {
            CallableEffectSummary::Analyzed { effects } => effects.may_pending(),
            // `AnalysisPending` never occurs on the production pipeline; treat
            // it conservatively as potentially pending.
            CallableEffectSummary::Unknown { .. } => true,
        };
        let symbol = format!(
            "{module_path}.{symbol}",
            module_path = source_key.module_path(),
            symbol = source_key.symbol()
        );
        per_callable.insert(
            source_key.clone(),
            (
                may_pending,
                format!("{}::{symbol}", source_key.module_path()),
            ),
        );
    }
    units
        .iter()
        .map(|unit| build_mir_unit_with_effect_map(unit, &per_callable))
        .collect()
}

/// Builder core with an already-resolved per-callable effect map (test seam).
pub(crate) fn build_mir_unit_with_effect_map(
    unit: &FileIrUnit,
    per_callable: &CallableEffectMap,
) -> Result<MirUnit, String> {
    let mut functions = Vec::with_capacity(unit.declarations.executables.len());
    for (declaration_name, declaration) in &unit.declarations.executables {
        let executable = unit
            .executables
            .get(declaration.executable_index as usize)
            .ok_or_else(|| {
                format!(
                    "MIR build for {module_path}::{declaration_name} references missing executable index {index}",
                    module_path = unit.module_path,
                    index = declaration.executable_index
                )
            })?;
        functions.push(build_mir_function(
            unit,
            declaration_name,
            executable,
            per_callable,
        )?);
    }
    Ok(MirUnit {
        module_path: unit.module_path.clone(),
        functions,
    })
}

fn callable_effect_facts(
    module_path: &str,
    declaration_name: &str,
    per_callable: &CallableEffectMap,
) -> Result<(bool, String), String> {
    let source_key = SourceSymbolKey::new(module_path, declaration_name);
    per_callable.get(&source_key).cloned().ok_or_else(|| {
        format!(
            "MIR build requires source-owned callable effect facts for {source_key} (missing from callable effects)"
        )
    })
}

fn build_mir_function(
    unit: &FileIrUnit,
    declaration_name: &str,
    executable: &ExecutableIr,
    per_callable: &CallableEffectMap,
) -> Result<MirFunction, String> {
    let (may_pending, effect_summary_ref) =
        callable_effect_facts(&unit.module_path, declaration_name, per_callable)?;
    let mut cfg = FunctionCfg::new(unit, executable);
    cfg.build_blocks()?;
    let (blocks, regions, statements) = cfg.finish();
    Ok(MirFunction {
        symbol: executable.symbol.clone(),
        kind: match executable.kind {
            skiff_artifact_model::ExecutableKind::Function => MirExecutableKind::Function,
            skiff_artifact_model::ExecutableKind::ImplMethod => MirExecutableKind::ImplMethod,
        },
        type_params: executable.type_params.clone(),
        params: executable
            .params
            .iter()
            .map(|param| MirParam {
                name: param.name.clone(),
                slot: param.slot,
                ty: param.ty.clone(),
                mode: match param.mode {
                    skiff_artifact_model::ParamModeIr::Value => MirParamMode::Value,
                    skiff_artifact_model::ParamModeIr::InOut => MirParamMode::InOut,
                },
            })
            .collect(),
        return_type: executable.return_type.clone(),
        self_type: executable.self_type.clone(),
        slots: executable
            .slots
            .slots
            .iter()
            .map(|slot| MirSlot {
                slot: slot.index,
                name: slot.name.clone(),
                kind: match slot.kind {
                    skiff_artifact_model::SlotKind::Param => MirSlotKind::Param,
                    skiff_artifact_model::SlotKind::SelfValue => MirSlotKind::SelfValue,
                    skiff_artifact_model::SlotKind::Local => MirSlotKind::Local,
                    skiff_artifact_model::SlotKind::Temp => MirSlotKind::Temp,
                    skiff_artifact_model::SlotKind::Pattern => MirSlotKind::Pattern,
                },
                ty: slot.ty.clone(),
            })
            .collect(),
        blocks,
        regions,
        statements,
        may_pending,
        effect_summary_ref,
        source_span: executable.source_span.clone(),
    })
}

/// Two-pass CFG construction:
///
/// - Pass A walks `ExecutableBody.blocks` in reverse (parents before
///   children, since the vector is pushed post-order with entry last),
///   creating fragments and recording loop contexts / continuations. Targets
///   are not resolved here: their fragments may not exist yet.
/// - Pass B converts every statement's branch targets from labels to block
///   ids (all fragments now exist) and computes the complete successor edges.
struct FunctionCfg<'a> {
    unit: &'a FileIrUnit,
    executable: &'a ExecutableIr,
    blocks: Vec<MirBlock>,
    /// File IR block label -> MirBlock ids of its fragments, in order.
    blocks_by_label: BTreeMap<String, Vec<u32>>,
    /// File IR block label -> statement continuation (MirBlock id). Absent
    /// for the entry block.
    continuations: BTreeMap<String, u32>,
    /// File IR block label -> (loop header, loop exit) for break/continue.
    loop_contexts: BTreeMap<String, (u32, u32)>,
    /// File IR block label -> loop header (loop-back edge for loop bodies).
    loop_backs: BTreeMap<String, u32>,
    /// MirBlock id -> raw statements collected in pass A, converted in pass B.
    pending_statements: BTreeMap<u32, Vec<(u32, Option<SourceSpanRef>, StmtIr)>>,
    statement_entries: Vec<MirStatementEntry>,
    regions: Vec<MirRegion>,
    next_block_id: u32,
    next_region_id: u32,
}

impl<'a> FunctionCfg<'a> {
    fn new(unit: &'a FileIrUnit, executable: &'a ExecutableIr) -> Self {
        Self {
            unit,
            executable,
            blocks: Vec::new(),
            blocks_by_label: BTreeMap::new(),
            continuations: BTreeMap::new(),
            loop_contexts: BTreeMap::new(),
            loop_backs: BTreeMap::new(),
            pending_statements: BTreeMap::new(),
            statement_entries: Vec::new(),
            regions: Vec::new(),
            next_block_id: 0,
            next_region_id: 0,
        }
    }

    fn new_block(&mut self, label: &str) -> u32 {
        let id = self.next_block_id;
        self.next_block_id += 1;
        self.blocks.push(MirBlock {
            id,
            label: label.to_string(),
            statements: Vec::new(),
            successors: Vec::new(),
        });
        id
    }

    fn finish(self) -> (Vec<MirBlock>, Vec<MirRegion>, Vec<MirStatementEntry>) {
        (self.blocks, self.regions, self.statement_entries)
    }

    fn build_blocks(&mut self) -> Result<(), String> {
        self.build_fragments()?;
        self.convert_and_resolve()
    }

    /// Pass A: fragments, loop contexts, continuations, regions.
    fn build_fragments(&mut self) -> Result<(), String> {
        let file_blocks = self.executable.body.blocks.clone();
        for file_block in file_blocks.iter().rev() {
            let label = file_block.label.clone();
            let loop_context = self.loop_contexts.get(&label).copied();
            let mut fragments: Vec<u32> = Vec::new();
            let mut current = self.new_block(&label);
            fragments.push(current);
            let mut pending = Vec::new();

            let statements = file_block.statements.clone();
            let mut index = 0;
            while index < statements.len() {
                let stmt_ref = statements[index];
                let stmt = self
                    .executable
                    .body
                    .statements
                    .get(stmt_ref.statement as usize)
                    .cloned()
                    .ok_or_else(|| {
                        format!(
                            "MIR build in {module}:{symbol} references missing statement index {}",
                            stmt_ref.statement,
                            module = self.unit.module_path,
                            symbol = self.executable.symbol
                        )
                    })?;
                let is_terminator = is_statement_terminator(&stmt);
                let is_branch = is_branch_statement(&stmt);
                // Continuations are created eagerly for branches so loop exit
                // edges and the implicit else target always exist.
                let branch_continuation = if is_branch {
                    Some(self.new_block(&label))
                } else {
                    None
                };
                // A plain statement completes inside its own fragment; an
                // expression-inlined block's return point is a pc decided by
                // the emitter, so the recorded edge is a conservative
                // statement-level approximation (self edge).
                let statement_continuation = branch_continuation.or(Some(current));
                // Terminators carry expressions too (return/throw values);
                // the walk only records regions and block references.
                self.walk_statement_expressions(&stmt, loop_context, statement_continuation)?;
                let span = self
                    .executable
                    .statement_spans
                    .get(stmt_ref.statement as usize)
                    .and_then(|span| span.clone());
                if is_terminator {
                    // Statements after a terminator are unreachable; drop them.
                    self.statement_entries.push(MirStatementEntry {
                        statement_index: stmt_ref.statement,
                        span: span.clone(),
                    });
                    pending.push((stmt_ref.statement, span, stmt));
                    break;
                }
                self.statement_entries.push(MirStatementEntry {
                    statement_index: stmt_ref.statement,
                    span: span.clone(),
                });
                if let Some(branch_continuation) = branch_continuation {
                    self.record_branch_contexts(&stmt, current, branch_continuation, loop_context)?;
                    pending.push((stmt_ref.statement, span, stmt));
                    fragments.push(branch_continuation);
                    self.pending_statements.insert(current, pending);
                    pending = Vec::new();
                    current = branch_continuation;
                } else {
                    pending.push((stmt_ref.statement, span, stmt));
                }
                index += 1;
            }
            self.pending_statements.insert(current, pending);
            self.blocks_by_label.insert(label, fragments);
        }
        Ok(())
    }

    /// Record loop contexts and continuations for every target of a branch
    /// statement. The target's fragments are built later in the reverse walk,
    /// so these lookups are always resolved before use.
    fn record_branch_contexts(
        &mut self,
        stmt: &StmtIr,
        header: u32,
        continuation: u32,
        enclosing_loop: Option<(u32, u32)>,
    ) -> Result<(), String> {
        match stmt {
            StmtIr::If {
                then_block,
                else_block,
                ..
            } => {
                self.record_target_loop_context(then_block, enclosing_loop)?;
                if let Some(else_block) = else_block {
                    self.record_target_loop_context(else_block, enclosing_loop)?;
                }
                self.record_continuation(then_block, continuation)?;
                if let Some(else_block) = else_block {
                    self.record_continuation(else_block, continuation)?;
                }
            }
            StmtIr::While { body, .. } | StmtIr::ForIn { body, .. } => {
                self.record_target_loop_context(body, Some((header, continuation)))?;
                self.record_continuation(body, continuation)?;
                self.record_loop_back(body, header)?;
            }
            StmtIr::Match { arms, .. } => {
                for arm in arms {
                    self.record_target_loop_context(&arm.body, enclosing_loop)?;
                    self.record_continuation(&arm.body, continuation)?;
                }
            }
            StmtIr::Timeout { body, .. } => {
                self.record_target_loop_context(body, enclosing_loop)?;
                self.record_continuation(body, continuation)?;
            }
            StmtIr::Concurrent { plan } => {
                for lane in &plan.lanes {
                    if let ConcurrentLaneIr::Statement { body, .. }
                    | ConcurrentLaneIr::Serial { body, .. } = lane
                    {
                        self.record_target_loop_context(body, enclosing_loop)?;
                        self.record_continuation(body, continuation)?;
                    }
                }
            }
            StmtIr::Let { .. }
            | StmtIr::Assign { .. }
            | StmtIr::Assert { .. }
            | StmtIr::Dispatch { .. }
            | StmtIr::Emit { .. }
            | StmtIr::Expr { .. }
            | StmtIr::Return { .. }
            | StmtIr::Throw { .. }
            | StmtIr::Rethrow { .. }
            | StmtIr::TestEffectRegister { .. }
            | StmtIr::Break
            | StmtIr::Continue => {}
        }
        Ok(())
    }

    fn record_target_loop_context(
        &mut self,
        label: &str,
        context: Option<(u32, u32)>,
    ) -> Result<(), String> {
        if let Some(context) = context {
            if self
                .loop_contexts
                .insert(label.to_string(), context)
                .is_some()
            {
                return Err(format!(
                    "MIR build: block label `{label}` referenced more than once"
                ));
            }
        }
        Ok(())
    }

    fn record_continuation(&mut self, label: &str, continuation: u32) -> Result<(), String> {
        if self
            .continuations
            .insert(label.to_string(), continuation)
            .is_some()
        {
            return Err(format!(
                "MIR build: block label `{label}` continuation recorded more than once"
            ));
        }
        Ok(())
    }

    fn record_loop_back(&mut self, label: &str, header: u32) -> Result<(), String> {
        if self.loop_backs.insert(label.to_string(), header).is_some() {
            return Err(format!(
                "MIR build: block label `{label}` loop-back recorded more than once"
            ));
        }
        Ok(())
    }

    /// Pass B: convert every statement's branch targets from labels to block
    /// ids, then compute the complete successor edge set of every fragment.
    fn convert_and_resolve(&mut self) -> Result<(), String> {
        let labels = self.blocks_by_label.clone();
        for (_label, fragments) in &labels {
            for (index, fragment) in fragments.iter().enumerate() {
                let pending = self.pending_statements.remove(fragment).unwrap_or_default();
                // The continuation fragment is created eagerly right after a
                // branch statement, so it is always the next fragment.
                let branch_continuation = fragments.get(index + 1).copied();
                let statements = pending
                    .into_iter()
                    .map(|(statement_index, span, stmt)| {
                        Ok(MirStmt {
                            statement_index,
                            span,
                            kind: self.convert_statement(stmt, branch_continuation)?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                self.blocks[*fragment as usize].statements = statements;
            }
        }
        self.resolve_successors(&labels)
    }

    fn convert_statement(
        &self,
        stmt: StmtIr,
        branch_continuation: Option<u32>,
    ) -> Result<MirStmtKind, String> {
        let kind = match stmt {
            StmtIr::Let { slot, value } => MirStmtKind::Let { slot, value },
            StmtIr::Assign { target, value } => MirStmtKind::Assign { target, value },
            StmtIr::Assert { condition, message } => MirStmtKind::Assert { condition, message },
            StmtIr::Dispatch { call } => MirStmtKind::Dispatch { call },
            StmtIr::Emit { operation, value } => MirStmtKind::Emit { operation, value },
            StmtIr::Expr { value } => MirStmtKind::Expr { value },
            StmtIr::Return { value } => MirStmtKind::Return { value },
            StmtIr::Throw {
                value,
                payload_type,
                site,
            } => MirStmtKind::Throw {
                value,
                payload_type,
                site,
            },
            StmtIr::Rethrow { exception_slot } => MirStmtKind::Rethrow { exception_slot },
            StmtIr::TestEffectRegister {
                target,
                expect,
                step_expect,
                outcome,
            } => MirStmtKind::TestEffectRegister {
                target,
                expect,
                step_expect,
                outcome,
            },
            StmtIr::If {
                condition,
                then_block,
                else_block,
            } => {
                let then_id = self.first_fragment_of(&then_block)?;
                let else_id = match else_block {
                    Some(else_block) => Some(self.first_fragment_of(&else_block)?),
                    // Implicit else: the statement continuation created right
                    // after this branch statement.
                    None => branch_continuation,
                };
                MirStmtKind::If {
                    condition,
                    then_block: then_id,
                    else_block: else_id,
                }
            }
            StmtIr::While { condition, body } => MirStmtKind::While {
                condition,
                body: self.first_fragment_of(&body)?,
            },
            StmtIr::ForIn {
                item_slot,
                item_type,
                value_slot,
                iterable,
                body,
            } => MirStmtKind::ForIn {
                item_slot,
                item_type,
                value_slot,
                iterable,
                body: self.first_fragment_of(&body)?,
            },
            StmtIr::Match { value, arms } => {
                let mut mir_arms = Vec::with_capacity(arms.len());
                for arm in arms {
                    mir_arms.push(MirMatchArmIr {
                        pattern: arm.pattern,
                        body: self.first_fragment_of(&arm.body)?,
                    });
                }
                MirStmtKind::Match {
                    value,
                    arms: mir_arms,
                }
            }
            StmtIr::Timeout {
                duration_ms,
                body,
                site,
            } => MirStmtKind::Timeout {
                duration_ms,
                body: self.first_fragment_of(&body)?,
                site,
            },
            StmtIr::Concurrent { plan } => MirStmtKind::Concurrent {
                plan: self.convert_concurrent_plan(plan)?,
            },
            StmtIr::Break => MirStmtKind::Break,
            StmtIr::Continue => MirStmtKind::Continue,
        };
        Ok(kind)
    }

    fn convert_concurrent_plan(
        &self,
        plan: ConcurrentPlanIr,
    ) -> Result<MirConcurrentPlanIr, String> {
        let mut lanes = Vec::with_capacity(plan.lanes.len());
        for lane in plan.lanes {
            lanes.push(match lane {
                ConcurrentLaneIr::Statement {
                    source_order,
                    dependencies,
                    body,
                    site,
                } => MirConcurrentLaneIr::Statement {
                    source_order,
                    dependencies,
                    body: self.first_fragment_of(&body)?,
                    site,
                },
                ConcurrentLaneIr::Serial {
                    source_order,
                    dependencies,
                    body,
                    site,
                } => MirConcurrentLaneIr::Serial {
                    source_order,
                    dependencies,
                    body: self.first_fragment_of(&body)?,
                    site,
                },
                ConcurrentLaneIr::Tail {
                    source_order,
                    dependencies,
                    tail,
                    site,
                } => MirConcurrentLaneIr::Tail {
                    source_order,
                    dependencies,
                    tail,
                    site,
                },
            });
        }
        Ok(MirConcurrentPlanIr { lanes })
    }

    fn first_fragment_of(&self, label: &str) -> Result<u32, String> {
        self.blocks_by_label
            .get(label)
            .and_then(|fragments| fragments.first())
            .copied()
            .ok_or_else(|| {
                format!(
                    "MIR build in {module}:{symbol}: block label `{label}` has no built fragments",
                    module = self.unit.module_path,
                    symbol = self.executable.symbol
                )
            })
    }

    /// Compute the complete successor edge set for every fragment.
    ///
    /// - terminator-end: `Return`/`Throw`/`Rethrow` have no successors;
    ///   `Break`/`Continue` jump to the loop exit/header of the enclosing
    ///   loop context.
    /// - branch-end: the structured branch targets plus the statement
    ///   continuation (loop-exit / no-match edge).
    /// - plain-end / empty: the next fragment, or the File IR block's
    ///   continuation; loop bodies instead loop back to their header.
    fn resolve_successors(&mut self, labels: &BTreeMap<String, Vec<u32>>) -> Result<(), String> {
        let continuations = self.continuations.clone();
        let loop_contexts = self.loop_contexts.clone();
        let loop_backs = self.loop_backs.clone();
        let blocks = self.blocks.clone();
        for (label, fragments) in labels {
            for (index, fragment) in fragments.iter().enumerate() {
                let block = &blocks[*fragment as usize];
                let successors = match block.statements.last().map(|stmt| &stmt.kind) {
                    Some(
                        MirStmtKind::Return { .. }
                        | MirStmtKind::Throw { .. }
                        | MirStmtKind::Rethrow { .. },
                    ) => Vec::new(),
                    Some(MirStmtKind::Break) => {
                        let (_, exit) = self.loop_context(label, &loop_contexts)?;
                        vec![exit]
                    }
                    Some(MirStmtKind::Continue) => {
                        let (header, _) = self.loop_context(label, &loop_contexts)?;
                        vec![header]
                    }
                    Some(MirStmtKind::If {
                        then_block,
                        else_block,
                        ..
                    }) => {
                        let mut successors = vec![*then_block];
                        successors.extend(else_block);
                        successors
                    }
                    Some(MirStmtKind::While { body, .. } | MirStmtKind::ForIn { body, .. }) => {
                        // The loop-exit edge: the eagerly created continuation
                        // fragment right after this branch statement.
                        vec![*body, fragments[index + 1]]
                    }
                    Some(MirStmtKind::Match { arms, .. }) => {
                        let mut successors = arms.iter().map(|arm| arm.body).collect::<Vec<u32>>();
                        // The no-match edge: the statement continuation.
                        successors.push(fragments[index + 1]);
                        successors
                    }
                    Some(MirStmtKind::Timeout { body, .. }) => vec![*body],
                    Some(MirStmtKind::Concurrent { plan }) => plan
                        .lanes
                        .iter()
                        .filter_map(|lane| match lane {
                            MirConcurrentLaneIr::Statement { body, .. }
                            | MirConcurrentLaneIr::Serial { body, .. } => Some(*body),
                            MirConcurrentLaneIr::Tail { .. } => None,
                        })
                        .collect(),
                    // Plain statements and empty fragments fall through.
                    Some(
                        MirStmtKind::Let { .. }
                        | MirStmtKind::Assign { .. }
                        | MirStmtKind::Assert { .. }
                        | MirStmtKind::Dispatch { .. }
                        | MirStmtKind::Emit { .. }
                        | MirStmtKind::Expr { .. }
                        | MirStmtKind::TestEffectRegister { .. },
                    )
                    | None => {
                        if index + 1 < fragments.len() {
                            vec![fragments[index + 1]]
                        } else if let Some(header) = loop_backs.get(label) {
                            // Loop body completion: next iteration.
                            vec![*header]
                        } else if let Some(continuation) = continuations.get(label) {
                            vec![*continuation]
                        } else {
                            Vec::new()
                        }
                    }
                };
                let mut successors = successors;
                successors.sort_unstable();
                successors.dedup();
                self.blocks[*fragment as usize].successors = successors;
            }
        }
        Ok(())
    }

    fn loop_context(
        &self,
        label: &str,
        contexts: &BTreeMap<String, (u32, u32)>,
    ) -> Result<(u32, u32), String> {
        contexts.get(label).copied().ok_or_else(|| {
            format!(
                "MIR build in {module}:{symbol}: `break`/`continue` in block `{label}` has no enclosing loop",
                module = self.unit.module_path,
                symbol = self.executable.symbol
            )
        })
    }
}

fn is_statement_terminator(stmt: &StmtIr) -> bool {
    matches!(
        stmt,
        StmtIr::Return { .. }
            | StmtIr::Throw { .. }
            | StmtIr::Rethrow { .. }
            | StmtIr::Break
            | StmtIr::Continue
    )
}

fn is_branch_statement(stmt: &StmtIr) -> bool {
    matches!(
        stmt,
        StmtIr::If { .. }
            | StmtIr::ForIn { .. }
            | StmtIr::While { .. }
            | StmtIr::Match { .. }
            | StmtIr::Timeout { .. }
            | StmtIr::Concurrent { .. }
    )
}

pub(crate) fn db_operation_child_expressions(
    operation: &skiff_artifact_model::DbOperationIr,
) -> Vec<u32> {
    let mut children = Vec::new();
    match &operation.selector {
        Some(DbSelectorIr::Key { value }) => children.push(value.expression),
        Some(DbSelectorIr::Query { query }) => {
            db_query_child_expressions_into(query, &mut children)
        }
        None => {}
    }
    if let Some(query) = &operation.query {
        db_query_child_expressions_into(query, &mut children);
    }
    for body in operation.body.iter().chain(operation.insert_body.iter()) {
        db_body_child_expressions(body, &mut children);
    }
    if let Some(change) = &operation.change {
        for op in &change.ops {
            match op {
                DbChangeOpIr::Set { value, .. }
                | DbChangeOpIr::Inc { value, .. }
                | DbChangeOpIr::AddToSet { value, .. }
                | DbChangeOpIr::Remove { value, .. } => children.push(value.expression),
                DbChangeOpIr::Unset { .. } => {}
            }
        }
    }
    children
}

pub(crate) fn db_query_child_expressions(query: &skiff_artifact_model::DbQueryIr) -> Vec<u32> {
    let mut children = Vec::new();
    db_query_child_expressions_into(query, &mut children);
    children
}

fn db_query_child_expressions_into(
    query: &skiff_artifact_model::DbQueryIr,
    children: &mut Vec<u32>,
) {
    for predicate in &query.where_clauses {
        db_predicate_child_expressions(predicate, children);
    }
    for entry in [&query.limit, &query.offset, &query.after] {
        if let Some(expression) = entry {
            children.push(expression.expression);
        }
    }
}

fn db_predicate_child_expressions(predicate: &DbPredicateIr, children: &mut Vec<u32>) {
    match predicate {
        DbPredicateIr::Compare { value, .. } => children.push(value.expression),
        DbPredicateIr::Regex {
            pattern, options, ..
        } => {
            children.push(pattern.expression);
            if let Some(options) = options {
                children.push(options.expression);
            }
        }
        DbPredicateIr::And { predicates } | DbPredicateIr::Or { predicates } => {
            for predicate in predicates {
                db_predicate_child_expressions(predicate, children);
            }
        }
        DbPredicateIr::Not { predicate } => db_predicate_child_expressions(predicate, children),
        DbPredicateIr::Conditional {
            condition,
            predicate,
        } => {
            children.push(condition.expression);
            db_predicate_child_expressions(predicate, children);
        }
    }
}

fn db_body_child_expressions(body: &DbBodyIr, children: &mut Vec<u32>) {
    match body {
        DbBodyIr::ObjectFields { fields } => {
            children.extend(fields.values().map(|value| value.expression));
        }
        DbBodyIr::Values { value } => children.push(value.expression),
    }
}

impl<'a> FunctionCfg<'a> {
    fn walk_statement_expressions(
        &mut self,
        stmt: &StmtIr,
        loop_context: Option<(u32, u32)>,
        statement_continuation: Option<u32>,
    ) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        let mut visit = |cfg: &mut Self, expression: u32| -> Result<(), String> {
            cfg.walk_expression(
                expression,
                0,
                loop_context,
                statement_continuation,
                &mut seen,
            )
        };
        match stmt {
            StmtIr::Let { value, .. } => visit(self, value.expression)?,
            StmtIr::Assign { target, value } => {
                visit(self, value.expression)?;
                match target {
                    AssignTargetIr::Field { object, .. } => {
                        visit(self, object.expression)?;
                    }
                    AssignTargetIr::Index { object, index } => {
                        visit(self, object.expression)?;
                        visit(self, index.expression)?;
                    }
                    AssignTargetIr::Slot { .. } | AssignTargetIr::ActorSelfField { .. } => {}
                }
            }
            StmtIr::Assert { condition, message } => {
                visit(self, condition.expression)?;
                if let Some(message) = message {
                    visit(self, message.expression)?;
                }
            }
            StmtIr::Dispatch { call } => visit(self, call.expression)?,
            StmtIr::Emit { value, .. } => visit(self, value.expression)?,
            StmtIr::Expr { value } => visit(self, value.expression)?,
            StmtIr::Return { value } => {
                if let Some(value) = value {
                    visit(self, value.expression)?;
                }
            }
            StmtIr::Throw { value, .. } => visit(self, value.expression)?,
            StmtIr::TestEffectRegister {
                expect,
                step_expect,
                outcome,
                ..
            } => {
                for expected in expect.iter().chain(step_expect.iter()) {
                    visit(self, expected.value.expression)?;
                }
                match outcome {
                    skiff_artifact_model::TestEffectOutcomeIr::Respond { value, .. } => {
                        visit(self, value.expression)?;
                    }
                    skiff_artifact_model::TestEffectOutcomeIr::Throw { value, .. } => {
                        visit(self, value.expression)?;
                    }
                    skiff_artifact_model::TestEffectOutcomeIr::Stream { values, .. } => {
                        for value in values {
                            visit(self, value.expression)?;
                        }
                    }
                }
            }
            StmtIr::If { condition, .. } => visit(self, condition.expression)?,
            StmtIr::ForIn { iterable, .. } => visit(self, iterable.expression)?,
            StmtIr::While { condition, .. } => visit(self, condition.expression)?,
            StmtIr::Match { value, .. } => visit(self, value.expression)?,
            StmtIr::Timeout { .. }
            | StmtIr::Concurrent { .. }
            | StmtIr::Rethrow { .. }
            | StmtIr::Break
            | StmtIr::Continue => {}
        }
        Ok(())
    }

    /// DFS over the File IR expression DAG: records exception regions and
    /// expression-referenced blocks, recursing through every child expression.
    fn walk_expression(
        &mut self,
        expression: u32,
        catch_depth: u32,
        loop_context: Option<(u32, u32)>,
        statement_continuation: Option<u32>,
        seen: &mut BTreeSet<u32>,
    ) -> Result<(), String> {
        if !seen.insert(expression) {
            return Ok(());
        }
        let expr = self
            .executable
            .body
            .expressions
            .get(expression as usize)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "MIR build in {module}:{symbol} references missing expression index {expression}",
                    module = self.unit.module_path,
                    symbol = self.executable.symbol
                )
            })?;
        let mut visit = |cfg: &mut Self, child: u32| -> Result<(), String> {
            cfg.walk_expression(
                child,
                catch_depth,
                loop_context,
                statement_continuation,
                seen,
            )
        };
        match expr {
            ExprIr::Literal { .. }
            | ExprIr::LoadSlot { .. }
            | ExprIr::LoadConst { .. }
            | ExprIr::LoadPackageConst { .. }
            | ExprIr::ActorSelfField { .. }
            | ExprIr::Rethrow { .. } => {}
            ExprIr::Field { object, .. } => visit(self, object.expression)?,
            ExprIr::Construct { fields, .. } => {
                for value in fields.values() {
                    visit(self, value.expression)?;
                }
            }
            ExprIr::RepresentationWrap { value, .. } => visit(self, value.expression)?,
            ExprIr::InterfaceBox { value, .. } => visit(self, value.expression)?,
            ExprIr::MapLiteral { entries } => {
                for value in entries.values() {
                    visit(self, value.expression)?;
                }
            }
            ExprIr::ArrayLiteral { items } => {
                for item in items {
                    visit(self, item.expression)?;
                }
            }
            ExprIr::Unary { value, .. } => visit(self, value.expression)?,
            ExprIr::Binary { left, right, .. } => {
                visit(self, left.expression)?;
                visit(self, right.expression)?;
            }
            ExprIr::Call { call } => {
                for argument in call.args {
                    visit(self, argument.expression)?;
                }
            }
            ExprIr::Throw { value, .. } => visit(self, value.expression)?,
            ExprIr::Catch {
                try_expression,
                catch_slot,
                catch_type,
                body,
            } => {
                let region_id = self.next_region_id;
                self.next_region_id += 1;
                self.regions.push(MirRegion {
                    id: region_id,
                    catch_expr: expression,
                    catch_slot,
                    catch_type: catch_type.clone(),
                    cleanup_depth: catch_depth,
                });
                let nested_depth = catch_depth + 1;
                self.walk_expression(
                    try_expression.expression,
                    nested_depth,
                    loop_context,
                    statement_continuation,
                    seen,
                )?;
                self.walk_expression(
                    body.expression,
                    nested_depth,
                    loop_context,
                    statement_continuation,
                    seen,
                )?;
            }
            ExprIr::Timeout { value, .. } => visit(self, value.expression)?,
            ExprIr::ValueBlock { block, result } => {
                self.record_expression_block(&block, loop_context, statement_continuation)?;
                visit(self, result.expression)?;
            }
            ExprIr::ConcurrentValue { plan } => {
                self.record_concurrent_lanes(&plan, loop_context, statement_continuation)?;
                for lane in &plan.lanes {
                    if let ConcurrentLaneIr::Tail { tail, .. } = lane {
                        visit(self, tail.expression)?;
                    }
                }
            }
            ExprIr::DbOperation { operation } => {
                for child in db_operation_child_expressions(&operation) {
                    visit(self, child)?;
                }
            }
            ExprIr::DbQuery { query } => {
                for child in db_query_child_expressions(&query.query) {
                    visit(self, child)?;
                }
            }
            ExprIr::DbTransaction { transaction } => {
                self.record_expression_block(
                    &transaction.body,
                    loop_context,
                    statement_continuation,
                )?;
                visit(self, transaction.result.expression)?;
            }
            ExprIr::DbLeaseClaim { claim } => {
                self.record_expression_block(&claim.body, loop_context, statement_continuation)?;
                visit(self, claim.key.expression)?;
            }
            ExprIr::DbLeaseRead { read } => {
                visit(self, read.key.expression)?;
            }
        }
        Ok(())
    }

    fn record_expression_block(
        &mut self,
        label: &str,
        loop_context: Option<(u32, u32)>,
        statement_continuation: Option<u32>,
    ) -> Result<(), String> {
        if let Some(context) = loop_context {
            if self
                .loop_contexts
                .insert(label.to_string(), context)
                .is_some()
            {
                return Err(format!(
                    "MIR build: block label `{label}` referenced more than once"
                ));
            }
        }
        if let Some(continuation) = statement_continuation {
            if self
                .continuations
                .insert(label.to_string(), continuation)
                .is_some()
            {
                return Err(format!(
                    "MIR build: block label `{label}` continuation recorded more than once"
                ));
            }
        }
        Ok(())
    }

    fn record_concurrent_lanes(
        &mut self,
        plan: &ConcurrentPlanIr,
        loop_context: Option<(u32, u32)>,
        statement_continuation: Option<u32>,
    ) -> Result<(), String> {
        for lane in &plan.lanes {
            if let ConcurrentLaneIr::Statement { body, .. }
            | ConcurrentLaneIr::Serial { body, .. } = lane
            {
                self.record_expression_block(body, loop_context, statement_continuation)?;
            }
        }
        Ok(())
    }
}
