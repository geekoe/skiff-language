use std::{collections::BTreeSet, path::Path};

use skiff_artifact_model::TypeRefIr;

use crate::{
    parsed_sources::ParsedCompilerSource,
    shared::ast::{Block, Expr, FunctionDecl, SourceFile, Stmt, TypeRef},
    shared::type_syntax::generic_parts,
    ExpressionKey, ExpressionOwnerKey, ExpressionSourceMap, ExpressionTypeModel, ResolvedTypeRef,
    TypeResolutionContext, TypeResolutionModel,
};

#[cfg(test)]
mod statements;
mod types;

#[cfg(test)]
use statements::validate_emit_usage_in_stmt;
use types::collect_emit_expression_call_violations_in_block;

#[cfg(test)]
pub fn collect_stream_function_return_types(
    ast: &SourceFile,
    return_types: &mut std::collections::BTreeMap<String, String>,
) {
    for function in &ast.functions {
        return_types.insert(function.name.clone(), function.return_type.name.clone());
    }
    for implementation in &ast.impls {
        for method in &implementation.method_bodies {
            return_types.insert(method.name.clone(), method.return_type.name.clone());
        }
    }
}

pub fn collect_stream_emit_expression_call_violations(
    path: &str,
    ast: &SourceFile,
    violations: &mut Vec<String>,
) {
    for function in &ast.functions {
        collect_emit_expression_call_violations_in_block(path, &function.body, violations);
    }
    for implementation in &ast.impls {
        for method in &implementation.method_bodies {
            collect_emit_expression_call_violations_in_block(path, &method.body, violations);
        }
    }
    for test in &ast.tests {
        collect_emit_expression_call_violations_in_block(path, &test.body, violations);
    }
}

pub fn collect_stream_emit_type_violations(
    diagnostic_root: &Path,
    parsed_sources: &[ParsedCompilerSource],
    expression_sources: &ExpressionSourceMap,
    expression_types: &ExpressionTypeModel,
    type_resolution: &TypeResolutionModel,
    violations: &mut Vec<String>,
) {
    for parsed in parsed_sources {
        let path = parsed.source().diagnostic_path_from_root(diagnostic_root);
        collect_source_stream_emit_type_violations(
            &path,
            parsed.source().module_path.as_str(),
            parsed.ast(),
            expression_sources,
            expression_types,
            type_resolution,
            violations,
        );
    }
}

#[cfg(test)]
pub fn collect_stream_emit_violations(
    path: &str,
    ast: &SourceFile,
    function_return_types: &std::collections::BTreeMap<String, String>,
    violations: &mut Vec<String>,
) {
    for function in &ast.functions {
        collect_function_emit_violations(
            path,
            &function.name,
            &function.params,
            &function.return_type,
            &function.body,
            function_return_types,
            violations,
        );
    }
    for implementation in &ast.impls {
        for method in &implementation.method_bodies {
            collect_function_emit_violations(
                path,
                &method.name,
                &method.params,
                &method.return_type,
                &method.body,
                function_return_types,
                violations,
            );
        }
    }
}

#[cfg(test)]
fn collect_function_emit_violations(
    path: &str,
    function_name: &str,
    params: &[crate::shared::ast::Param],
    return_type: &TypeRef,
    body: &Block,
    function_return_types: &std::collections::BTreeMap<String, String>,
    violations: &mut Vec<String>,
) {
    let stream_chunk = crate::shared::type_syntax::generic_inner(return_type.name.trim(), "Stream")
        .map(str::to_string);
    let mut env = std::collections::BTreeMap::new();
    for param in params {
        env.insert(param.name.clone(), param.ty.name.clone());
    }
    validate_emit_usage_in_block(
        path,
        function_name,
        stream_chunk.as_deref(),
        body,
        function_return_types,
        &mut env,
        violations,
    );
}

#[cfg(test)]
fn validate_emit_usage_in_block(
    path: &str,
    function_name: &str,
    stream_chunk: Option<&str>,
    body: &Block,
    function_return_types: &std::collections::BTreeMap<String, String>,
    env: &mut std::collections::BTreeMap<String, String>,
    violations: &mut Vec<String>,
) {
    for stmt in &body.statements {
        validate_emit_usage_in_stmt(
            path,
            function_name,
            stream_chunk,
            stmt,
            function_return_types,
            env,
            violations,
        );
    }
}

fn collect_source_stream_emit_type_violations(
    path: &str,
    module_path: &str,
    ast: &SourceFile,
    expression_sources: &ExpressionSourceMap,
    expression_types: &ExpressionTypeModel,
    type_resolution: &TypeResolutionModel,
    violations: &mut Vec<String>,
) {
    for function in &ast.functions {
        if function.is_native || function.is_provider {
            continue;
        }
        collect_function_stream_emit_type_violations(
            path,
            module_path,
            ExpressionOwnerKey::Function(function.name.clone()),
            function,
            &[],
            expression_sources,
            expression_types,
            type_resolution,
            violations,
        );
    }
    for implementation in &ast.impls {
        let inherited_type_params = generic_type_params(&implementation.target);
        for method in &implementation.method_bodies {
            if method.is_native || method.is_provider {
                continue;
            }
            collect_function_stream_emit_type_violations(
                path,
                module_path,
                ExpressionOwnerKey::ImplMethod {
                    type_name: implementation.target.clone(),
                    method: method.name.clone(),
                },
                method,
                &inherited_type_params,
                expression_sources,
                expression_types,
                type_resolution,
                violations,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_function_stream_emit_type_violations(
    path: &str,
    module_path: &str,
    owner: ExpressionOwnerKey,
    function: &FunctionDecl,
    inherited_type_params: &[String],
    expression_sources: &ExpressionSourceMap,
    expression_types: &ExpressionTypeModel,
    type_resolution: &TypeResolutionModel,
    violations: &mut Vec<String>,
) {
    let type_params = inherited_type_params
        .iter()
        .chain(&function.type_params)
        .cloned()
        .collect::<BTreeSet<_>>();
    let type_context = TypeResolutionContext::with_type_params(module_path, type_params);
    let stream_chunk = stream_chunk_type(&function.return_type, &type_context, type_resolution);
    let mut checker = StreamEmitTypeChecker {
        path,
        module_path,
        owner,
        function_name: function.name.as_str(),
        stream_chunk,
        type_context,
        next_index: 0,
        expression_sources,
        expression_types,
        type_resolution,
        violations,
    };
    checker.check_block(&function.body);
}

struct StreamEmitTypeChecker<'a> {
    path: &'a str,
    module_path: &'a str,
    owner: ExpressionOwnerKey,
    function_name: &'a str,
    stream_chunk: Option<ResolvedTypeRef>,
    type_context: TypeResolutionContext<'a>,
    next_index: u32,
    expression_sources: &'a ExpressionSourceMap,
    expression_types: &'a ExpressionTypeModel,
    type_resolution: &'a TypeResolutionModel,
    violations: &'a mut Vec<String>,
}

impl StreamEmitTypeChecker<'_> {
    fn check_block(&mut self, body: &Block) {
        for stmt in &body.statements {
            self.check_stmt(stmt);
        }
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assert { condition, .. } => {
                self.check_expr(condition);
            }
            Stmt::Let { value, .. } => {
                self.check_expr(value);
            }
            Stmt::Assign { target, value } => {
                self.check_expr(target);
                self.check_expr(value);
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                self.check_expr(condition);
                self.check_block(then_block);
                if let Some(else_block) = else_block {
                    self.check_block(else_block);
                }
            }
            Stmt::For { iterable, body, .. } => {
                self.check_expr(iterable);
                self.check_block(body);
            }
            Stmt::Match { value, arms } => {
                self.check_expr(value);
                for arm in arms {
                    self.check_block(&arm.body);
                }
            }
            Stmt::DbTransaction { body } => self.check_block(body),
            Stmt::Throw { value } | Stmt::Spawn { call: value } | Stmt::Expr(value) => {
                self.check_expr(value);
            }
            Stmt::Emit(value) => {
                let value_key = self.peek_key();
                let actual = self.check_expr(value);
                self.check_emit(value, &value_key, actual);
            }
            Stmt::Rethrow { exception } => {
                self.check_expr(exception);
            }
            Stmt::Return(value) => {
                if let Some(value) = value {
                    self.check_expr(value);
                }
            }
            Stmt::Break | Stmt::Continue => {}
        }
    }

    fn check_emit(
        &mut self,
        value: &Expr,
        value_key: &ExpressionKey,
        actual: Option<ResolvedTypeRef>,
    ) {
        let Some(expected) = &self.stream_chunk else {
            self.violations.push(format!(
                "{}: emit can only be used in a Stream<T> producer; function {} returns a non-stream type",
                self.path, self.function_name
            ));
            return;
        };
        let Some(actual) = actual else {
            self.violations.push(format!(
                "{}: cannot infer emit chunk type in {}; emit expression must have a known Stream<T> chunk type",
                self.path, self.function_name
            ));
            return;
        };
        if self.expression_types.value_assignable_to_expected(
            self.expression_sources,
            self.type_resolution,
            &self.type_context,
            None,
            value,
            &actual,
            expected,
        ) {
            return;
        }
        if let Some(diagnostics) = self
            .expression_types
            .object_literal_assignability_diagnostics(
                self.path,
                self.expression_sources,
                self.type_resolution,
                &self.type_context,
                None,
                value,
                value_key,
                &actual,
                expected,
                &format!("emit chunk in {}", self.function_name),
            )
        {
            if !diagnostics.is_empty() {
                self.violations.extend(diagnostics);
                return;
            }
        }
        self.violations.push(format!(
            "{}: emit chunk type mismatch in {}: expected {}, found {}",
            self.path, self.function_name, expected.source_text, actual.source_text
        ));
    }

    fn check_expr(&mut self, expr: &Expr) -> Option<ResolvedTypeRef> {
        let key = self.next_key();
        let ty = self
            .expression_types
            .fact(&key)
            .and_then(|fact| fact.ty.clone());
        match expr {
            Expr::Literal(_) | Expr::Identifier(_) | Expr::RemotePublicInstanceSource(_) => {}
            Expr::Binary { left, right, .. } => {
                self.check_expr(left);
                self.check_expr(right);
            }
            Expr::Unary { expr, .. } | Expr::Generic { callee: expr, .. } => {
                self.check_expr(expr);
            }
            Expr::InterfaceBox { value, .. } => {
                self.check_expr(value);
            }
            Expr::Call { callee, args } => {
                self.check_expr(callee);
                for arg in args {
                    self.check_expr(arg);
                }
            }
            Expr::Field { object, .. } => {
                self.check_expr(object);
            }
            Expr::Record { fields, .. } => {
                for (_, value) in fields {
                    self.check_expr(value);
                }
            }
            Expr::ObjectLiteral { entries } => {
                for entry in entries {
                    self.check_expr(&entry.value);
                }
            }
            Expr::Patch { operations, .. } => {
                for operation in operations {
                    match operation {
                        crate::shared::ast::PatchOperation::Set { value, .. }
                        | crate::shared::ast::PatchOperation::Inc { value, .. } => {
                            self.check_expr(value);
                        }
                    }
                }
            }
            Expr::Throw { value } => {
                self.check_expr(value);
            }
            Expr::Rethrow { exception } => {
                self.check_expr(exception);
            }
            Expr::Catch { try_expr, .. } => {
                self.check_expr(try_expr);
            }
            Expr::DbOperation(operation) => {
                if let Some(selector) = &operation.selector {
                    match selector {
                        crate::shared::ast::DbSelector::Key { value } => {
                            self.check_expr(value);
                        }
                        crate::shared::ast::DbSelector::Query { query } => {
                            self.check_db_query(query)
                        }
                    }
                }
                if let Some(query) = &operation.query {
                    self.check_db_query(query);
                }
                for body in [&operation.body, &operation.insert_body]
                    .into_iter()
                    .flatten()
                {
                    match body {
                        crate::shared::ast::DbBody::ObjectFields { fields } => {
                            for field in fields {
                                self.check_expr(&field.value);
                            }
                        }
                        crate::shared::ast::DbBody::Values { value } => {
                            self.check_expr(value);
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
                                self.check_expr(value);
                            }
                            crate::shared::ast::DbChangeOp::Unset { .. } => {}
                        }
                    }
                }
            }
            Expr::DbQuery(query) => self.check_db_query(&query.query),
            Expr::DbTransaction(transaction) => self.check_block(&transaction.body),
            Expr::DbLeaseClaim(claim) => {
                self.check_expr(&claim.key);
                self.check_block(&claim.body);
            }
            Expr::DbLeaseRead(read) => {
                self.check_expr(&read.key);
            }
        }
        ty
    }

    fn next_key(&mut self) -> ExpressionKey {
        let key = self.peek_key();
        self.next_index = self.next_index.saturating_add(1);
        key
    }

    fn peek_key(&self) -> ExpressionKey {
        ExpressionKey::new(
            self.module_path.to_string(),
            self.owner.clone(),
            self.next_index,
        )
    }

    fn check_db_query(&mut self, query: &crate::shared::ast::DbQueryBlock) {
        for clause in &query.where_clauses {
            match clause {
                crate::shared::ast::DbWhereClause::Predicate { predicate } => {
                    self.check_expr(predicate);
                }
                crate::shared::ast::DbWhereClause::Conditional {
                    condition,
                    predicate,
                } => {
                    self.check_expr(condition);
                    self.check_expr(predicate);
                }
            }
        }
        for expr in [&query.limit, &query.offset, &query.after]
            .into_iter()
            .flatten()
        {
            self.check_expr(expr);
        }
    }
}

fn stream_chunk_type(
    return_type: &TypeRef,
    context: &TypeResolutionContext<'_>,
    type_resolution: &TypeResolutionModel,
) -> Option<ResolvedTypeRef> {
    let parts = generic_parts(return_type.name.trim())?;
    let root = type_resolution
        .resolve_type_text(parts.root, context)
        .ok()?;
    let is_stream = matches!(root.source_text.as_str(), "Stream" | "std.stream.Stream")
        || matches!(
            root.ir,
            TypeRefIr::Native { ref name, .. } if name == "Stream"
        );
    if !is_stream || parts.args.len() != 1 {
        return None;
    }
    type_resolution
        .resolve_type_text(parts.args[0], context)
        .ok()
}

fn generic_type_params(target: &str) -> Vec<String> {
    generic_parts(target)
        .map(|parts| {
            parts
                .args
                .iter()
                .map(|arg| arg.trim().to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn collect(source: &str) -> Vec<String> {
        let ast = crate::shared::parser::parse_source(source).unwrap();
        let mut return_types = BTreeMap::new();
        collect_stream_function_return_types(&ast, &mut return_types);
        let mut violations = Vec::new();
        collect_stream_emit_violations("test.skiff", &ast, &return_types, &mut violations);
        violations
    }

    #[test]
    fn rejects_emit_expression_call() {
        let violations = collect(
            r#"
                type Chunk {}

                function makeChunk() -> Chunk {
                    return {}
                }

                function events() -> Stream<Chunk> {
                    const ignored = emit(makeChunk())
                    return {}
                }
            "#,
        );

        assert_eq!(
            violations,
            vec!["test.skiff: emit is a stream statement and cannot be used as an expression"]
        );
    }

    #[test]
    fn rejects_emit_in_non_stream_function() {
        let violations = collect(
            r#"
                type Chunk {}

                function echo(chunk: Chunk) -> Chunk {
                    emit(chunk)
                    return chunk
                }
            "#,
        );

        assert_eq!(
            violations,
            vec![
                "test.skiff: emit can only be used in a Stream<T> producer; function echo returns a non-stream type"
            ]
        );
    }

    #[test]
    fn rejects_local_annotation_and_chunk_type_mismatch() {
        let violations = collect(
            r#"
                type GoodChunk {}
                type BadChunk {}

                function makeWrongChunk() -> BadChunk {
                    return {}
                }

                function events() -> Stream<GoodChunk> {
                    const chunk: GoodChunk = makeWrongChunk()
                    emit(chunk)
                    return {}
                }
            "#,
        );

        assert_eq!(
            violations,
            vec![
                "test.skiff: local binding chunk annotation type mismatch in events: expected GoodChunk, found BadChunk",
                "test.skiff: emit chunk type mismatch in events: expected GoodChunk, found BadChunk",
            ]
        );
    }
}
