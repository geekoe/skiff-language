use crate::{ExpressionKey, ExpressionOwnerKey, ExpressionSourceMap};

pub(super) fn contract_member_error(
    diagnostic_path: &str,
    call_key: &ExpressionKey,
    expression_sources: &ExpressionSourceMap,
    alias: &str,
    stable_key: Option<&str>,
) -> String {
    let location = call_location(diagnostic_path, call_key, expression_sources);
    match stable_key {
        Some(stable_key) => format!(
            "{location}: contract dependency `{alias}` has no operation stable key `{stable_key}`"
        ),
        None => format!(
            "{location}: contract dependency `{alias}` must be followed by an operation stable key"
        ),
    }
}

pub(super) fn dependency_member_error(
    diagnostic_path: &str,
    call_key: &ExpressionKey,
    expression_sources: &ExpressionSourceMap,
    alias: &str,
    public_path: &str,
) -> String {
    let location = call_location(diagnostic_path, call_key, expression_sources);
    format!("{location}: package dependency `{alias}` has no callable public path `{public_path}`")
}

pub(super) fn unknown_dependency_alias_error(
    diagnostic_path: &str,
    call_key: &ExpressionKey,
    expression_sources: &ExpressionSourceMap,
    alias: &str,
) -> String {
    let location = call_location(diagnostic_path, call_key, expression_sources);
    format!("{location}: dependency source address uses undeclared alias `{alias}`")
}

pub(super) fn dotted_dependency_call_error(
    diagnostic_path: &str,
    call_key: &ExpressionKey,
    expression_sources: &ExpressionSourceMap,
    alias: &str,
    public_path: &str,
) -> String {
    let location = call_location(diagnostic_path, call_key, expression_sources);
    format!(
        "{location}: dependency call `{alias}.{public_path}` must use `{alias}/{public_path}`; `.` is reserved for qualified types and member access after a dependency address"
    )
}

fn call_location(
    diagnostic_path: &str,
    call_key: &ExpressionKey,
    expression_sources: &ExpressionSourceMap,
) -> String {
    let source_location = expression_sources
        .fact(call_key)
        .map(|fact| format!("{}:{}", fact.span.start.line, fact.span.start.column))
        .unwrap_or_else(|| "unknown location".to_string());
    format!(
        "{diagnostic_path}:{source_location}: {}, call expression #{}",
        expression_owner_label(call_key.owner()),
        call_key.preorder_index()
    )
}

fn expression_owner_label(owner: &ExpressionOwnerKey) -> String {
    match owner {
        ExpressionOwnerKey::Function(name) => format!("function `{name}`"),
        ExpressionOwnerKey::ImplMethod { type_name, method } => {
            format!("method `{type_name}.{method}`")
        }
        ExpressionOwnerKey::Const(name) => format!("const `{name}`"),
        ExpressionOwnerKey::Test(name) => format!("test `{name}`"),
        ExpressionOwnerKey::DbIndexWhere { db, index } => {
            format!("db index `{db}.{index}`")
        }
    }
}
