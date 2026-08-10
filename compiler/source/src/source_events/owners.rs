use std::collections::BTreeMap;

use crate::{
    semantic::impl_method_declaration_name,
    shared::ast::{
        Block, BlockSourceSpans, DbIndexWhereSourceSpans, Expr, ExprSourceSpans, SourceFile,
    },
    ExpressionKey, ExpressionOwnerKey, ExpressionSourceFact,
};

use super::{collector::OwnerCollector, SourceEventFact, SourceEventKey};

pub(super) fn collect_source(
    module_path: &str,
    ast: &SourceFile,
    events: &mut BTreeMap<SourceEventKey, SourceEventFact>,
    expressions: &mut BTreeMap<ExpressionKey, ExpressionSourceFact>,
) -> Result<(), String> {
    collect_functions(module_path, ast, events, expressions)?;
    collect_impl_methods(module_path, ast, events, expressions)?;

    for (constant, spans) in ast.consts.iter().zip(&ast.source_spans.consts) {
        collect_owner_expression(
            module_path,
            ExpressionOwnerKey::Const(constant.name.clone()),
            &constant.value,
            spans,
            events,
            expressions,
        )?;
    }
    assert_count(
        ast.consts.len(),
        ast.source_spans.consts.len(),
        module_path,
        "const initializer spans",
    )?;

    for (test, spans) in ast.tests.iter().zip(&ast.source_spans.tests) {
        collect_owner_block(
            module_path,
            ExpressionOwnerKey::Test(test.name.clone()),
            &test.body,
            &spans.body,
            events,
            expressions,
        )?;
    }
    assert_count(
        ast.tests.len(),
        ast.source_spans.tests.len(),
        module_path,
        "test body spans",
    )?;

    for where_spans in &ast.source_spans.db_index_wheres {
        collect_owner_expression(
            module_path,
            ExpressionOwnerKey::DbIndexWhere {
                db: where_spans.db_name.clone(),
                index: where_spans.index_name.clone(),
            },
            db_index_where_expression(ast, where_spans, module_path)?,
            &where_spans.expression,
            events,
            expressions,
        )?;
    }
    Ok(())
}

fn collect_functions(
    module_path: &str,
    ast: &SourceFile,
    events: &mut BTreeMap<SourceEventKey, SourceEventFact>,
    expressions: &mut BTreeMap<ExpressionKey, ExpressionSourceFact>,
) -> Result<(), String> {
    let mut span_index = 0;
    for function in &ast.functions {
        if function.is_native || function.is_provider {
            continue;
        }
        let spans = ast.source_spans.functions.get(span_index).ok_or_else(|| {
            format!(
                "source event model mismatch in module {module_path}: missing function body span for {}",
                function.name
            )
        })?;
        collect_owner_block(
            module_path,
            ExpressionOwnerKey::Function(function.name.clone()),
            &function.body,
            &spans.body,
            events,
            expressions,
        )?;
        span_index += 1;
    }
    assert_count(
        span_index,
        ast.source_spans.functions.len(),
        module_path,
        "function body spans",
    )
}

fn collect_impl_methods(
    module_path: &str,
    ast: &SourceFile,
    events: &mut BTreeMap<SourceEventKey, SourceEventFact>,
    expressions: &mut BTreeMap<ExpressionKey, ExpressionSourceFact>,
) -> Result<(), String> {
    let mut span_index = 0;
    for implementation in &ast.impls {
        for method in &implementation.method_bodies {
            if method.is_native || method.is_provider {
                continue;
            }
            let spans = ast.source_spans.impl_methods.get(span_index).ok_or_else(|| {
                format!(
                    "source event model mismatch in module {module_path}: missing impl method span for {}",
                    impl_method_declaration_name(&implementation.target, &method.name)
                )
            })?;
            collect_owner_block(
                module_path,
                ExpressionOwnerKey::ImplMethod {
                    type_name: implementation.target.clone(),
                    method: method.name.clone(),
                },
                &method.body,
                &spans.body,
                events,
                expressions,
            )?;
            span_index += 1;
        }
    }
    assert_count(
        span_index,
        ast.source_spans.impl_methods.len(),
        module_path,
        "impl method body spans",
    )
}

fn collect_owner_block(
    module_path: &str,
    owner: ExpressionOwnerKey,
    block: &Block,
    spans: &BlockSourceSpans,
    events: &mut BTreeMap<SourceEventKey, SourceEventFact>,
    expressions: &mut BTreeMap<ExpressionKey, ExpressionSourceFact>,
) -> Result<(), String> {
    OwnerCollector::new(module_path, owner, events, expressions).visit_block(block, spans)
}

fn collect_owner_expression(
    module_path: &str,
    owner: ExpressionOwnerKey,
    expression: &Expr,
    spans: &ExprSourceSpans,
    events: &mut BTreeMap<SourceEventKey, SourceEventFact>,
    expressions: &mut BTreeMap<ExpressionKey, ExpressionSourceFact>,
) -> Result<(), String> {
    OwnerCollector::new(module_path, owner, events, expressions).visit_expr(expression, spans)
}

fn db_index_where_expression<'a>(
    ast: &'a SourceFile,
    spans: &DbIndexWhereSourceSpans,
    module_path: &str,
) -> Result<&'a Expr, String> {
    ast.dbs
        .iter()
        .find(|db| db.name == spans.db_name)
        .and_then(|db| {
            db.indexes
                .iter()
                .find(|index| index.name == spans.index_name)
                .and_then(|index| index.where_expr.as_ref())
        })
        .ok_or_else(|| {
            format!(
                "source event model mismatch in module {module_path}: missing db index where expr for {}.{}",
                spans.db_name, spans.index_name
            )
        })
}

fn assert_count(
    ast_len: usize,
    span_len: usize,
    module_path: &str,
    label: &str,
) -> Result<(), String> {
    if ast_len == span_len {
        return Ok(());
    }
    Err(format!(
        "source event model mismatch in module {module_path}: {label} count {span_len} does not match AST count {ast_len}"
    ))
}
