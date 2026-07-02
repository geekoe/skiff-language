use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{
    shared::ast::{Block, Expr, SourceFile, Stmt},
    shared::ast_utils::expr_path,
    shared::type_syntax::generic_parts,
};

/// Identifies a callable node in the cross-source call graph.
///
/// Free functions are keyed by their bare name; impl methods by their bare
/// method name. Method keys deliberately collapse across impl targets: the
/// coverage analysis only ever uses these edges to decide whether an emitting
/// helper is reached exclusively from `Stream<T>` producers, and over-connecting
/// methods can only make the analysis more conservative (it never silently
/// accepts an orphan emit, which is the behaviour we must preserve).
pub(super) type CallKey = String;

/// Whether emit is permitted in a callable and, if so, which producers reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum EmitCoverage {
    /// The callable is itself a `Stream<T>` producer; emit is checked against
    /// the producer's own chunk type.
    Producer,
    /// A non-producer reached transitively only from producer call chains.
    /// `producer_keys` is the set of producers whose chains reach it; the emit
    /// chunk type must be compatible with every one of them.
    Covered { producer_keys: BTreeSet<CallKey> },
    /// A non-producer that is an orphan (no caller) or reachable from a
    /// non-producer entry; emit is rejected here.
    Rejected,
}

/// Computes, for every callable across the given sources, whether emit is
/// permitted there (see [`EmitCoverage`]).
///
/// Algorithm:
/// 1. Build the producer set (callables returning `Stream<T>`) and a call graph
///    over callables defined in these sources (edges to unknown callees, e.g.
///    std/native functions, are dropped — they neither cover nor expose).
/// 2. A non-producer callable is "exposed" iff it is an entry (no in-graph
///    caller) or it is called by an exposed callable. Exposure propagates
///    forward along call edges but never crosses into a producer (producers are
///    valid emit contexts and act as boundaries). This is computed as a
///    forward reachability fixpoint seeded from non-producer entries.
/// 3. For a non-producer callable, emit is `Covered` iff it is not exposed and
///    has at least one caller; the covering producer set is collected by
///    backward traversal that stops at producers.
pub(super) fn compute_emit_coverage<'a>(
    sources: impl IntoIterator<Item = &'a SourceFile>,
) -> BTreeMap<CallKey, EmitCoverage> {
    let mut producers: BTreeSet<CallKey> = BTreeSet::new();
    let mut all_nodes: BTreeSet<CallKey> = BTreeSet::new();
    // callee -> set of callers (reverse edges)
    let mut callers_of: BTreeMap<CallKey, BTreeSet<CallKey>> = BTreeMap::new();
    // caller -> set of callees (forward edges, restricted to in-graph nodes)
    let mut callees_of: BTreeMap<CallKey, BTreeSet<CallKey>> = BTreeMap::new();
    // nodes that contain at least one in-graph call edge as a caller, and nodes
    // that appear as callees, drive the "has caller" test below.

    let mut definitions: Vec<(CallKey, &Block, bool)> = Vec::new();
    for source in sources {
        for function in &source.functions {
            if function.is_native || function.is_provider {
                continue;
            }
            let key = function.name.clone();
            let is_producer = return_type_is_stream(&function.return_type.name);
            all_nodes.insert(key.clone());
            if is_producer {
                producers.insert(key.clone());
            }
            definitions.push((key, &function.body, is_producer));
        }
        for implementation in &source.impls {
            for method in &implementation.method_bodies {
                if method.is_native || method.is_provider {
                    continue;
                }
                let key = method.name.clone();
                let is_producer = return_type_is_stream(&method.return_type.name);
                all_nodes.insert(key.clone());
                if is_producer {
                    producers.insert(key.clone());
                }
                definitions.push((key, &method.body, is_producer));
            }
        }
    }

    for (caller, body, _) in &definitions {
        let mut targets = BTreeSet::new();
        collect_call_targets_in_block(body, &all_nodes, &mut targets);
        for target in targets {
            if &target == caller {
                continue;
            }
            callers_of
                .entry(target.clone())
                .or_default()
                .insert(caller.clone());
            callees_of
                .entry(caller.clone())
                .or_default()
                .insert(target);
        }
    }

    // Step 2: forward reachability of "exposed" from non-producer entries.
    let mut exposed: BTreeSet<CallKey> = BTreeSet::new();
    let mut queue: VecDeque<CallKey> = VecDeque::new();
    for node in &all_nodes {
        if producers.contains(node) {
            continue;
        }
        let has_caller = callers_of
            .get(node)
            .is_some_and(|callers| !callers.is_empty());
        if !has_caller {
            // non-producer entry (no in-graph caller)
            if exposed.insert(node.clone()) {
                queue.push_back(node.clone());
            }
        }
    }
    while let Some(node) = queue.pop_front() {
        if let Some(callees) = callees_of.get(&node) {
            for callee in callees {
                // Producers are emit boundaries: do not propagate exposure into
                // them or beyond. A producer being called from an exposed node
                // is still a valid emit context.
                if producers.contains(callee) {
                    continue;
                }
                if exposed.insert(callee.clone()) {
                    queue.push_back(callee.clone());
                }
            }
        }
    }

    // Step 3: classify each node and, for covered nodes, find covering producers.
    let mut coverage = BTreeMap::new();
    for node in &all_nodes {
        if producers.contains(node) {
            coverage.insert(node.clone(), EmitCoverage::Producer);
            continue;
        }
        let has_caller = callers_of
            .get(node)
            .is_some_and(|callers| !callers.is_empty());
        if exposed.contains(node) || !has_caller {
            coverage.insert(node.clone(), EmitCoverage::Rejected);
            continue;
        }
        let producer_keys = covering_producers(node, &producers, &callers_of);
        coverage.insert(node.clone(), EmitCoverage::Covered { producer_keys });
    }
    coverage
}

/// Backward traversal from `node` through caller edges, collecting the producers
/// whose call chains reach it. Traversal stops once it reaches a producer (its
/// callers do not contribute their own chunk type to `node`).
fn covering_producers(
    node: &CallKey,
    producers: &BTreeSet<CallKey>,
    callers_of: &BTreeMap<CallKey, BTreeSet<CallKey>>,
) -> BTreeSet<CallKey> {
    let mut result = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(node.clone());
    visited.insert(node.clone());
    while let Some(current) = queue.pop_front() {
        let Some(callers) = callers_of.get(&current) else {
            continue;
        };
        for caller in callers {
            if producers.contains(caller) {
                result.insert(caller.clone());
                continue;
            }
            if visited.insert(caller.clone()) {
                queue.push_back(caller.clone());
            }
        }
    }
    result
}

fn return_type_is_stream(return_type: &str) -> bool {
    let Some(parts) = generic_parts(return_type.trim()) else {
        return false;
    };
    matches!(parts.root, "Stream" | "std.stream.Stream") && parts.args.len() == 1
}

fn collect_call_targets_in_block(
    block: &Block,
    known: &BTreeSet<CallKey>,
    targets: &mut BTreeSet<CallKey>,
) {
    for stmt in &block.statements {
        collect_call_targets_in_stmt(stmt, known, targets);
    }
}

fn collect_call_targets_in_stmt(
    stmt: &Stmt,
    known: &BTreeSet<CallKey>,
    targets: &mut BTreeSet<CallKey>,
) {
    match stmt {
        Stmt::Let { value, .. }
        | Stmt::Spawn { call: value }
        | Stmt::Emit(value)
        | Stmt::Expr(value)
        | Stmt::Throw { value }
        | Stmt::Rethrow { exception: value } => {
            collect_call_targets_in_expr(value, known, targets)
        }
        Stmt::Assign { target, value } => {
            collect_call_targets_in_expr(target, known, targets);
            collect_call_targets_in_expr(value, known, targets);
        }
        Stmt::If {
            condition,
            then_block,
            else_block,
        } => {
            collect_call_targets_in_expr(condition, known, targets);
            collect_call_targets_in_block(then_block, known, targets);
            if let Some(else_block) = else_block {
                collect_call_targets_in_block(else_block, known, targets);
            }
        }
        Stmt::For { iterable, body, .. } => {
            collect_call_targets_in_expr(iterable, known, targets);
            collect_call_targets_in_block(body, known, targets);
        }
        Stmt::Match { value, arms } => {
            collect_call_targets_in_expr(value, known, targets);
            for arm in arms {
                collect_call_targets_in_block(&arm.body, known, targets);
            }
        }
        Stmt::DbTransaction { body } => collect_call_targets_in_block(body, known, targets),
        Stmt::Assert { condition, .. } => {
            collect_call_targets_in_expr(condition, known, targets)
        }
        Stmt::Return(value) => {
            if let Some(value) = value {
                collect_call_targets_in_expr(value, known, targets);
            }
        }
        Stmt::Break | Stmt::Continue => {}
    }
}

fn collect_call_targets_in_expr(
    expr: &Expr,
    known: &BTreeSet<CallKey>,
    targets: &mut BTreeSet<CallKey>,
) {
    if let Expr::Call { callee, .. } = expr {
        if let Some(key) = call_target_key(callee, known) {
            targets.insert(key);
        }
    }
    match expr {
        Expr::Call { callee, args } => {
            collect_call_targets_in_expr(callee, known, targets);
            for arg in args {
                collect_call_targets_in_expr(arg, known, targets);
            }
        }
        Expr::Generic { callee, .. } | Expr::Unary { expr: callee, .. } => {
            collect_call_targets_in_expr(callee, known, targets);
        }
        Expr::Binary { left, right, .. } => {
            collect_call_targets_in_expr(left, known, targets);
            collect_call_targets_in_expr(right, known, targets);
        }
        Expr::Field { object, .. } => {
            collect_call_targets_in_expr(object, known, targets);
        }
        Expr::Record { fields, .. } => {
            for (_, value) in fields {
                collect_call_targets_in_expr(value, known, targets);
            }
        }
        Expr::ObjectLiteral { entries } => {
            for entry in entries {
                collect_call_targets_in_expr(&entry.value, known, targets);
            }
        }
        Expr::Patch { operations, .. } => {
            for operation in operations {
                match operation {
                    crate::shared::ast::PatchOperation::Set { value, .. }
                    | crate::shared::ast::PatchOperation::Inc { value, .. } => {
                        collect_call_targets_in_expr(value, known, targets);
                    }
                }
            }
        }
        Expr::Throw { value } => collect_call_targets_in_expr(value, known, targets),
        Expr::Rethrow { exception } => collect_call_targets_in_expr(exception, known, targets),
        Expr::Catch { try_expr, .. } => collect_call_targets_in_expr(try_expr, known, targets),
        Expr::DbOperation(operation) => {
            collect_call_targets_in_db_operation(operation, known, targets)
        }
        Expr::DbQuery(query) => collect_call_targets_in_db_query(&query.query, known, targets),
        Expr::DbTransaction(transaction) => {
            collect_call_targets_in_block(&transaction.body, known, targets)
        }
        Expr::Literal(_) | Expr::Identifier(_) => {}
    }
}

/// Resolves a call's callee expression to a graph node key, if it names a
/// callable defined in these sources. Bare identifiers resolve directly; dotted
/// paths (method/qualified calls) resolve via their last segment so that
/// `obj.method()` connects to an impl method named `method`. Calls to unknown
/// callees (std/native/imports) return `None` and are dropped from the graph.
fn call_target_key(callee: &Expr, known: &BTreeSet<CallKey>) -> Option<CallKey> {
    let path = expr_path(callee)?;
    if known.contains(&path) {
        return Some(path);
    }
    let last = path.rsplit('.').next()?.to_string();
    if known.contains(&last) {
        return Some(last);
    }
    None
}

fn collect_call_targets_in_db_operation(
    operation: &crate::shared::ast::DbOperation,
    known: &BTreeSet<CallKey>,
    targets: &mut BTreeSet<CallKey>,
) {
    if let Some(selector) = &operation.selector {
        match selector {
            crate::shared::ast::DbSelector::Key { value } => {
                collect_call_targets_in_expr(value, known, targets)
            }
            crate::shared::ast::DbSelector::Query { query } => {
                collect_call_targets_in_db_query(query, known, targets)
            }
        }
    }
    if let Some(query) = &operation.query {
        collect_call_targets_in_db_query(query, known, targets);
    }
    for body in [&operation.body, &operation.insert_body]
        .into_iter()
        .flatten()
    {
        match body {
            crate::shared::ast::DbBody::ObjectFields { fields } => {
                for field in fields {
                    collect_call_targets_in_expr(&field.value, known, targets);
                }
            }
            crate::shared::ast::DbBody::Values { value } => {
                collect_call_targets_in_expr(value, known, targets)
            }
        }
    }
    if let Some(change) = &operation.change {
        for op in &change.ops {
            match op {
                crate::shared::ast::DbChangeOp::Set { value, .. }
                | crate::shared::ast::DbChangeOp::Inc { value, .. }
                | crate::shared::ast::DbChangeOp::AddToSet { value, .. }
                | crate::shared::ast::DbChangeOp::Remove { value, .. } => {
                    collect_call_targets_in_expr(value, known, targets)
                }
                crate::shared::ast::DbChangeOp::Unset { .. } => {}
            }
        }
    }
}

fn collect_call_targets_in_db_query(
    query: &crate::shared::ast::DbQueryBlock,
    known: &BTreeSet<CallKey>,
    targets: &mut BTreeSet<CallKey>,
) {
    for clause in &query.where_clauses {
        match clause {
            crate::shared::ast::DbWhereClause::Predicate { predicate } => {
                collect_call_targets_in_expr(predicate, known, targets);
            }
            crate::shared::ast::DbWhereClause::Conditional {
                condition,
                predicate,
            } => {
                collect_call_targets_in_expr(condition, known, targets);
                collect_call_targets_in_expr(predicate, known, targets);
            }
        }
    }
    for expr in [&query.limit, &query.offset, &query.after]
        .into_iter()
        .flatten()
    {
        collect_call_targets_in_expr(expr, known, targets);
    }
}
