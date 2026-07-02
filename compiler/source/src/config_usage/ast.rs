use std::collections::{BTreeMap, BTreeSet};

use crate::shared::ast::{Block, BlockSourceSpans, Expr, ExprSourceSpans, SourceFile, Stmt};

use super::validation::{
    config_intrinsic_callee, config_intrinsic_field, const_string_expr,
    push_indirect_config_intrinsic_violation, push_legacy_values_violation, validate_config_path,
    ConfigIntrinsic,
};
use super::{ConfigPresenceUse, ConfigSourceSpan, ConfigUsageSeed, ConfigUse};

pub(super) fn collect_config_uses_in_ast(
    diagnostic_path: &str,
    source_path: &str,
    ast: &SourceFile,
    uses: &mut Vec<ConfigUse>,
    presence_uses: &mut Vec<ConfigPresenceUse>,
    violations: &mut Vec<String>,
) {
    collect_common_config_uses_in_ast(
        diagnostic_path,
        source_path,
        ast,
        &BTreeSet::new(),
        uses,
        presence_uses,
        violations,
    );
}

pub(super) fn collect_common_config_uses_in_ast(
    diagnostic_path: &str,
    source_path: &str,
    ast: &SourceFile,
    excluded_function_names: &BTreeSet<String>,
    uses: &mut Vec<ConfigUse>,
    presence_uses: &mut Vec<ConfigPresenceUse>,
    violations: &mut Vec<String>,
) -> BTreeMap<String, String> {
    let mut const_strings = BTreeMap::new();
    for (index, constant) in ast.consts.iter().enumerate() {
        collect_config_uses_in_expr(
            diagnostic_path,
            source_path,
            &constant.value,
            ast.source_spans.consts.get(index),
            &const_strings,
            uses,
            presence_uses,
            violations,
        );
        if let Some(value) = const_string_expr(&constant.value, &const_strings) {
            const_strings.insert(constant.name.clone(), value);
        }
    }
    for (index, function) in ast.functions.iter().enumerate() {
        if excluded_function_names.contains(&function.name) {
            continue;
        }
        collect_config_uses_in_block(
            diagnostic_path,
            source_path,
            &function.body,
            ast.source_spans
                .functions
                .get(index)
                .map(|spans| &spans.body),
            &const_strings,
            uses,
            presence_uses,
            violations,
        );
    }
    let mut impl_method_span_index = 0usize;
    for implementation in &ast.impls {
        for method in &implementation.method_bodies {
            collect_config_uses_in_block(
                diagnostic_path,
                source_path,
                &method.body,
                ast.source_spans
                    .impl_methods
                    .get(impl_method_span_index)
                    .map(|spans| &spans.body),
                &const_strings,
                uses,
                presence_uses,
                violations,
            );
            impl_method_span_index += 1;
        }
    }
    const_strings
}

pub(super) fn collect_config_uses_in_ast_functions(
    diagnostic_path: &str,
    source_path: &str,
    ast: &SourceFile,
    function_indexes: &BTreeMap<String, usize>,
    inherited_const_strings: &BTreeMap<String, String>,
    seeds: &mut [ConfigUsageSeed],
    violations: &mut Vec<String>,
) {
    for (index, function) in ast.functions.iter().enumerate() {
        let Some(seed_index) = function_indexes.get(&function.name) else {
            continue;
        };
        let Some(seed) = seeds.get_mut(*seed_index) else {
            continue;
        };
        collect_config_uses_in_block(
            diagnostic_path,
            source_path,
            &function.body,
            ast.source_spans
                .functions
                .get(index)
                .map(|spans| &spans.body),
            inherited_const_strings,
            &mut seed.typed,
            &mut seed.presence,
            violations,
        );
    }
}

fn collect_config_uses_in_block(
    diagnostic_path: &str,
    source_path: &str,
    block: &Block,
    block_spans: Option<&BlockSourceSpans>,
    inherited_const_strings: &BTreeMap<String, String>,
    uses: &mut Vec<ConfigUse>,
    presence_uses: &mut Vec<ConfigPresenceUse>,
    violations: &mut Vec<String>,
) {
    let mut const_strings = inherited_const_strings.clone();
    for (statement_index, statement) in block.statements.iter().enumerate() {
        let statement_spans = block_spans.and_then(|spans| spans.statements.get(statement_index));
        match statement {
            Stmt::Let {
                mutable,
                name,
                value,
                ..
            } => {
                collect_config_uses_in_expr(
                    diagnostic_path,
                    source_path,
                    value,
                    statement_spans.and_then(|spans| spans.expressions.first()),
                    &const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
                if !*mutable {
                    if let Some(const_value) = const_string_expr(value, &const_strings) {
                        const_strings.insert(name.clone(), const_value);
                    } else {
                        const_strings.remove(name);
                    }
                } else {
                    const_strings.remove(name);
                }
            }
            Stmt::Spawn { call: value } | Stmt::Emit(value) | Stmt::Expr(value) => {
                collect_config_uses_in_expr(
                    diagnostic_path,
                    source_path,
                    value,
                    statement_spans.and_then(|spans| spans.expressions.first()),
                    &const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
            }
            Stmt::Return(value) => {
                if let Some(value) = value {
                    collect_config_uses_in_expr(
                        diagnostic_path,
                        source_path,
                        value,
                        statement_spans.and_then(|spans| spans.expressions.first()),
                        &const_strings,
                        uses,
                        presence_uses,
                        violations,
                    );
                }
            }
            Stmt::Throw { value } => {
                collect_config_uses_in_expr(
                    diagnostic_path,
                    source_path,
                    value,
                    statement_spans.and_then(|spans| spans.expressions.first()),
                    &const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
            }
            Stmt::Rethrow { exception } => {
                collect_config_uses_in_expr(
                    diagnostic_path,
                    source_path,
                    exception,
                    statement_spans.and_then(|spans| spans.expressions.first()),
                    &const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
            }
            Stmt::Assign { target, value } => {
                collect_config_uses_in_expr(
                    diagnostic_path,
                    source_path,
                    target,
                    statement_spans.and_then(|spans| spans.expressions.first()),
                    &const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
                collect_config_uses_in_expr(
                    diagnostic_path,
                    source_path,
                    value,
                    statement_spans.and_then(|spans| spans.expressions.get(1)),
                    &const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                collect_config_uses_in_expr(
                    diagnostic_path,
                    source_path,
                    condition,
                    statement_spans.and_then(|spans| spans.expressions.first()),
                    &const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
                collect_config_uses_in_block(
                    diagnostic_path,
                    source_path,
                    then_block,
                    statement_spans.and_then(|spans| spans.blocks.first()),
                    &const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
                if let Some(else_block) = else_block {
                    collect_config_uses_in_block(
                        diagnostic_path,
                        source_path,
                        else_block,
                        statement_spans.and_then(|spans| spans.blocks.get(1)),
                        &const_strings,
                        uses,
                        presence_uses,
                        violations,
                    );
                }
            }
            Stmt::For { iterable, body, .. } => {
                collect_config_uses_in_expr(
                    diagnostic_path,
                    source_path,
                    iterable,
                    statement_spans.and_then(|spans| spans.expressions.first()),
                    &const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
                collect_config_uses_in_block(
                    diagnostic_path,
                    source_path,
                    body,
                    statement_spans.and_then(|spans| spans.blocks.first()),
                    &const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
            }
            Stmt::Match { value, arms } => {
                collect_config_uses_in_expr(
                    diagnostic_path,
                    source_path,
                    value,
                    statement_spans.and_then(|spans| spans.expressions.first()),
                    &const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
                for (arm_index, arm) in arms.iter().enumerate() {
                    collect_config_uses_in_block(
                        diagnostic_path,
                        source_path,
                        &arm.body,
                        statement_spans.and_then(|spans| spans.blocks.get(arm_index)),
                        &const_strings,
                        uses,
                        presence_uses,
                        violations,
                    );
                }
            }
            Stmt::Assert { condition, .. } => {
                collect_config_uses_in_expr(
                    diagnostic_path,
                    source_path,
                    condition,
                    statement_spans.and_then(|spans| spans.expressions.first()),
                    &const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
            }
            Stmt::DbTransaction { body } => {
                collect_config_uses_in_block(
                    diagnostic_path,
                    source_path,
                    body,
                    statement_spans.and_then(|spans| spans.blocks.first()),
                    &const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
            }
            Stmt::Break | Stmt::Continue => {}
        }
    }
}

fn collect_config_uses_in_expr(
    diagnostic_path: &str,
    source_path: &str,
    expr: &Expr,
    expr_spans: Option<&ExprSourceSpans>,
    const_strings: &BTreeMap<String, String>,
    uses: &mut Vec<ConfigUse>,
    presence_uses: &mut Vec<ConfigPresenceUse>,
    violations: &mut Vec<String>,
) {
    match expr {
        Expr::Call { callee, args } => {
            if let Some((intrinsic, type_args)) = config_intrinsic_callee(callee) {
                collect_direct_config_intrinsic_call(
                    diagnostic_path,
                    source_path,
                    intrinsic,
                    type_args,
                    args,
                    expr_spans.map(|spans| ConfigSourceSpan::from(spans.span)),
                    const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
                for (arg_index, arg) in args.iter().enumerate() {
                    collect_config_uses_in_expr(
                        diagnostic_path,
                        source_path,
                        arg,
                        child_span(expr_spans, arg_index + 1),
                        const_strings,
                        uses,
                        presence_uses,
                        violations,
                    );
                }
                return;
            }
            if expression_root_is_values(callee) {
                push_legacy_values_violation(diagnostic_path, violations);
                for (arg_index, arg) in args.iter().enumerate() {
                    collect_config_uses_in_expr(
                        diagnostic_path,
                        source_path,
                        arg,
                        child_span(expr_spans, arg_index + 1),
                        const_strings,
                        uses,
                        presence_uses,
                        violations,
                    );
                }
                return;
            }
            collect_config_uses_in_expr(
                diagnostic_path,
                source_path,
                callee,
                child_span(expr_spans, 0),
                const_strings,
                uses,
                presence_uses,
                violations,
            );
            for (arg_index, arg) in args.iter().enumerate() {
                collect_config_uses_in_expr(
                    diagnostic_path,
                    source_path,
                    arg,
                    child_span(expr_spans, arg_index + 1),
                    const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
            }
        }
        Expr::Generic { callee, .. } => collect_config_uses_in_expr(
            diagnostic_path,
            source_path,
            callee,
            child_span(expr_spans, 0),
            const_strings,
            uses,
            presence_uses,
            violations,
        ),
        Expr::InterfaceBox { value, .. } => collect_config_uses_in_expr(
            diagnostic_path,
            source_path,
            value,
            child_span(expr_spans, 0),
            const_strings,
            uses,
            presence_uses,
            violations,
        ),
        Expr::Field { object, field } => {
            if matches!(object.as_ref(), Expr::Identifier(root) if root == "config") {
                if field == "get" {
                    push_removed_config_get_violation(diagnostic_path, violations);
                } else if config_intrinsic_field(expr).is_some() {
                    push_indirect_config_intrinsic_violation(diagnostic_path, violations);
                } else {
                    violations.push(format!(
                        "{diagnostic_path}: config only exposes direct config.require<T>(const-path), config.optional<T>(const-path), or config.has(const-path)"
                    ));
                }
                return;
            }
            if expression_root_is_values(expr) {
                push_legacy_values_violation(diagnostic_path, violations);
                return;
            }
            collect_config_uses_in_expr(
                diagnostic_path,
                source_path,
                object,
                child_span(expr_spans, 0),
                const_strings,
                uses,
                presence_uses,
                violations,
            );
        }
        Expr::Binary { left, right, .. } => {
            collect_config_uses_in_expr(
                diagnostic_path,
                source_path,
                left,
                child_span(expr_spans, 0),
                const_strings,
                uses,
                presence_uses,
                violations,
            );
            collect_config_uses_in_expr(
                diagnostic_path,
                source_path,
                right,
                child_span(expr_spans, 1),
                const_strings,
                uses,
                presence_uses,
                violations,
            );
        }
        Expr::Unary { expr, .. } => collect_config_uses_in_expr(
            diagnostic_path,
            source_path,
            expr,
            child_span(expr_spans, 0),
            const_strings,
            uses,
            presence_uses,
            violations,
        ),
        Expr::Record { fields, .. } => {
            for (field_index, (_, value)) in fields.iter().enumerate() {
                collect_config_uses_in_expr(
                    diagnostic_path,
                    source_path,
                    value,
                    trailing_child_span(expr_spans, fields.len(), field_index),
                    const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
            }
        }
        Expr::ObjectLiteral { entries } => {
            for (entry_index, entry) in entries.iter().enumerate() {
                collect_config_uses_in_expr(
                    diagnostic_path,
                    source_path,
                    &entry.value,
                    child_span(expr_spans, entry_index),
                    const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
            }
        }
        Expr::Patch { operations, .. } => {
            for (operation_index, operation) in operations.iter().enumerate() {
                match operation {
                    crate::shared::ast::PatchOperation::Set { value, .. }
                    | crate::shared::ast::PatchOperation::Inc { value, .. } => {
                        collect_config_uses_in_expr(
                            diagnostic_path,
                            source_path,
                            value,
                            trailing_child_span(expr_spans, operations.len(), operation_index),
                            const_strings,
                            uses,
                            presence_uses,
                            violations,
                        );
                    }
                }
            }
        }
        Expr::Throw { value } => collect_config_uses_in_expr(
            diagnostic_path,
            source_path,
            value,
            child_span(expr_spans, 0),
            const_strings,
            uses,
            presence_uses,
            violations,
        ),
        Expr::Rethrow { exception } => collect_config_uses_in_expr(
            diagnostic_path,
            source_path,
            exception,
            child_span(expr_spans, 0),
            const_strings,
            uses,
            presence_uses,
            violations,
        ),
        Expr::Catch { try_expr, .. } => collect_config_uses_in_expr(
            diagnostic_path,
            source_path,
            try_expr,
            child_span(expr_spans, 0),
            const_strings,
            uses,
            presence_uses,
            violations,
        ),
        Expr::Identifier(name) if name == "config" => {
            push_indirect_config_intrinsic_violation(diagnostic_path, violations);
        }
        Expr::DbOperation(operation) => collect_config_uses_in_db_operation(
            diagnostic_path,
            source_path,
            operation,
            expr_spans,
            const_strings,
            uses,
            presence_uses,
            violations,
        ),
        Expr::DbQuery(query) => collect_config_uses_in_db_query(
            diagnostic_path,
            source_path,
            &query.query,
            &mut ExprSpanCursor::new(expr_spans),
            const_strings,
            uses,
            presence_uses,
            violations,
        ),
        Expr::DbTransaction(transaction) => collect_config_uses_in_block(
            diagnostic_path,
            source_path,
            &transaction.body,
            None,
            const_strings,
            uses,
            presence_uses,
            violations,
        ),
        Expr::DbLeaseClaim(claim) => {
            collect_config_uses_in_expr(
                diagnostic_path,
                source_path,
                &claim.key,
                child_span(expr_spans, 0),
                const_strings,
                uses,
                presence_uses,
                violations,
            );
            collect_config_uses_in_block(
                diagnostic_path,
                source_path,
                &claim.body,
                None,
                const_strings,
                uses,
                presence_uses,
                violations,
            );
        }
        Expr::DbLeaseRead(read) => collect_config_uses_in_expr(
            diagnostic_path,
            source_path,
            &read.key,
            child_span(expr_spans, 0),
            const_strings,
            uses,
            presence_uses,
            violations,
        ),
        Expr::Literal(_) | Expr::Identifier(_) | Expr::RemotePublicInstanceSource(_) => {}
    }
}

fn child_span(expr_spans: Option<&ExprSourceSpans>, index: usize) -> Option<&ExprSourceSpans> {
    expr_spans.and_then(|spans| spans.children.get(index))
}

fn trailing_child_span(
    expr_spans: Option<&ExprSourceSpans>,
    trailing_count: usize,
    trailing_index: usize,
) -> Option<&ExprSourceSpans> {
    expr_spans.and_then(|spans| {
        spans
            .children
            .len()
            .checked_sub(trailing_count)
            .and_then(|offset| spans.children.get(offset + trailing_index))
    })
}

struct ExprSpanCursor<'a> {
    children: &'a [ExprSourceSpans],
    index: usize,
}

impl<'a> ExprSpanCursor<'a> {
    fn new(expr_spans: Option<&'a ExprSourceSpans>) -> Self {
        Self {
            children: expr_spans
                .map(|spans| spans.children.as_slice())
                .unwrap_or(&[]),
            index: 0,
        }
    }

    fn next(&mut self) -> Option<&'a ExprSourceSpans> {
        let span = self.children.get(self.index);
        self.index += 1;
        span
    }
}

fn collect_config_uses_in_db_operation(
    diagnostic_path: &str,
    source_path: &str,
    operation: &crate::shared::ast::DbOperation,
    operation_spans: Option<&ExprSourceSpans>,
    const_strings: &BTreeMap<String, String>,
    uses: &mut Vec<ConfigUse>,
    presence_uses: &mut Vec<ConfigPresenceUse>,
    violations: &mut Vec<String>,
) {
    let mut child_spans = ExprSpanCursor::new(operation_spans);
    if let Some(selector) = &operation.selector {
        match selector {
            crate::shared::ast::DbSelector::Key { value } => collect_config_uses_in_expr(
                diagnostic_path,
                source_path,
                value,
                child_spans.next(),
                const_strings,
                uses,
                presence_uses,
                violations,
            ),
            crate::shared::ast::DbSelector::Query { query } => collect_config_uses_in_db_query(
                diagnostic_path,
                source_path,
                query,
                &mut child_spans,
                const_strings,
                uses,
                presence_uses,
                violations,
            ),
        }
    }
    if let Some(query) = &operation.query {
        let query_already_visited_as_selector = matches!(
            operation.selector.as_ref(),
            Some(crate::shared::ast::DbSelector::Query { .. })
        );
        if !query_already_visited_as_selector {
            collect_config_uses_in_db_query(
                diagnostic_path,
                source_path,
                query,
                &mut child_spans,
                const_strings,
                uses,
                presence_uses,
                violations,
            );
        }
    }
    for body in [&operation.body, &operation.insert_body]
        .into_iter()
        .flatten()
    {
        match body {
            crate::shared::ast::DbBody::ObjectFields { fields } => {
                for field in fields {
                    collect_config_uses_in_expr(
                        diagnostic_path,
                        source_path,
                        &field.value,
                        child_spans.next(),
                        const_strings,
                        uses,
                        presence_uses,
                        violations,
                    );
                }
            }
            crate::shared::ast::DbBody::Values { value } => collect_config_uses_in_expr(
                diagnostic_path,
                source_path,
                value,
                child_spans.next(),
                const_strings,
                uses,
                presence_uses,
                violations,
            ),
        }
    }
    if let Some(change) = &operation.change {
        for op in &change.ops {
            match op {
                crate::shared::ast::DbChangeOp::Set { value, .. }
                | crate::shared::ast::DbChangeOp::Inc { value, .. }
                | crate::shared::ast::DbChangeOp::AddToSet { value, .. }
                | crate::shared::ast::DbChangeOp::Remove { value, .. } => {
                    collect_config_uses_in_expr(
                        diagnostic_path,
                        source_path,
                        value,
                        child_spans.next(),
                        const_strings,
                        uses,
                        presence_uses,
                        violations,
                    )
                }
                crate::shared::ast::DbChangeOp::Unset { .. } => {}
            }
        }
    }
}

fn collect_config_uses_in_db_query(
    diagnostic_path: &str,
    source_path: &str,
    query: &crate::shared::ast::DbQueryBlock,
    query_spans: &mut ExprSpanCursor<'_>,
    const_strings: &BTreeMap<String, String>,
    uses: &mut Vec<ConfigUse>,
    presence_uses: &mut Vec<ConfigPresenceUse>,
    violations: &mut Vec<String>,
) {
    for clause in &query.where_clauses {
        match clause {
            crate::shared::ast::DbWhereClause::Predicate { predicate } => {
                collect_config_uses_in_expr(
                    diagnostic_path,
                    source_path,
                    predicate,
                    query_spans.next(),
                    const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
            }
            crate::shared::ast::DbWhereClause::Conditional {
                condition,
                predicate,
            } => {
                collect_config_uses_in_expr(
                    diagnostic_path,
                    source_path,
                    condition,
                    query_spans.next(),
                    const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
                collect_config_uses_in_expr(
                    diagnostic_path,
                    source_path,
                    predicate,
                    query_spans.next(),
                    const_strings,
                    uses,
                    presence_uses,
                    violations,
                );
            }
        }
    }
    if let Some(limit) = &query.limit {
        collect_config_uses_in_expr(
            diagnostic_path,
            source_path,
            limit,
            query_spans.next(),
            const_strings,
            uses,
            presence_uses,
            violations,
        );
    }
    if let Some(offset) = &query.offset {
        collect_config_uses_in_expr(
            diagnostic_path,
            source_path,
            offset,
            query_spans.next(),
            const_strings,
            uses,
            presence_uses,
            violations,
        );
    }
    if let Some(after) = &query.after {
        collect_config_uses_in_expr(
            diagnostic_path,
            source_path,
            after,
            query_spans.next(),
            const_strings,
            uses,
            presence_uses,
            violations,
        );
    }
}

fn collect_direct_config_intrinsic_call(
    diagnostic_path: &str,
    source_path: &str,
    intrinsic: ConfigIntrinsic,
    type_args: &[crate::shared::ast::TypeRef],
    args: &[Expr],
    source_span: Option<ConfigSourceSpan>,
    const_strings: &BTreeMap<String, String>,
    uses: &mut Vec<ConfigUse>,
    presence_uses: &mut Vec<ConfigPresenceUse>,
    violations: &mut Vec<String>,
) {
    if intrinsic == ConfigIntrinsic::Get {
        push_removed_config_get_violation(diagnostic_path, violations);
        return;
    }

    let ty = match intrinsic {
        ConfigIntrinsic::Require | ConfigIntrinsic::Optional => {
            let config_type = match type_args {
                [ty] => canonical_config_type(&ty.name),
                _ => {
                    violations.push(format!(
                        "{diagnostic_path}: {} requires exactly one non-null type argument; use {}<T>(const-path)",
                        intrinsic.call_name(),
                        intrinsic.call_name()
                    ));
                    return;
                }
            };
            let Some(ty) = config_type else {
                let ty = type_args
                    .first()
                    .map(|ty| ty.name.as_str())
                    .unwrap_or("<missing>");
                violations.push(format!(
                    "{diagnostic_path}: {} type {ty} is unsupported; expected non-null string, number, bool, Json, or JsonObject",
                    intrinsic.call_name()
                ));
                return;
            };
            Some(ty)
        }
        ConfigIntrinsic::Has => {
            if !type_args.is_empty() {
                violations.push(format!(
                    "{diagnostic_path}: config.has does not take type arguments; use config.has(const-path)"
                ));
                return;
            }
            None
        }
        ConfigIntrinsic::Get => unreachable!("config.get is rejected above"),
    };

    let [path_arg] = args else {
        violations.push(format!(
            "{diagnostic_path}: {} requires one const-foldable string path",
            intrinsic.call_name()
        ));
        return;
    };
    let Some(read_path) = const_string_expr(path_arg, const_strings) else {
        violations.push(format!(
            "{diagnostic_path}: {} requires a const-foldable string path; dynamic config paths are not supported",
            intrinsic.call_name()
        ));
        return;
    };
    if read_path.is_empty() {
        violations.push(format!(
            "{diagnostic_path}: {} path cannot be empty",
            intrinsic.call_name()
        ));
        return;
    }
    if let Some(violation) = validate_config_path(&read_path, intrinsic.call_name()) {
        violations.push(format!("{diagnostic_path}: {violation}"));
        return;
    }

    match intrinsic {
        ConfigIntrinsic::Require | ConfigIntrinsic::Optional => {
            uses.push(ConfigUse {
                path: read_path,
                ty: ty
                    .expect("typed config intrinsic has canonical type")
                    .to_string(),
                required: intrinsic == ConfigIntrinsic::Require,
                source_path: source_path.to_string(),
                source_span,
            });
        }
        ConfigIntrinsic::Has => presence_uses.push(ConfigPresenceUse {
            path: read_path,
            source_path: source_path.to_string(),
            source_span,
        }),
        ConfigIntrinsic::Get => unreachable!("config.get is rejected above"),
    }
}

fn canonical_config_type(raw: &str) -> Option<&'static str> {
    let trimmed = raw.trim();
    if trimmed.ends_with('?') {
        return None;
    }
    match trimmed {
        "string" => Some("string"),
        "number" => Some("number"),
        "bool" => Some("bool"),
        "Json" => Some("Json"),
        "JsonObject" => Some("JsonObject"),
        _ => None,
    }
}

fn push_removed_config_get_violation(diagnostic_path: &str, violations: &mut Vec<String>) {
    violations.push(format!(
        "{diagnostic_path}: config.get<T>(path) has been removed; use config.require<T>(path) for required values or config.optional<T>(path) for optional values"
    ));
}

fn expression_root_is_values(expr: &Expr) -> bool {
    match expr {
        Expr::Field { object, .. } => expression_has_values_root(object),
        Expr::Generic { callee, .. } => expression_root_is_values(callee),
        _ => false,
    }
}

fn expression_has_values_root(expr: &Expr) -> bool {
    match expr {
        Expr::Identifier(name) => name == "values",
        Expr::Field { object, .. } => expression_has_values_root(object),
        Expr::Generic { callee, .. } => expression_has_values_root(callee),
        _ => false,
    }
}
