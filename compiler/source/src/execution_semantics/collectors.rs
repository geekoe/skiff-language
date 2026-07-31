use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    builtin_receiver_op_spec_by_name, BuiltinReceiverOp, BuiltinReceiverOpSpec,
};

use crate::{
    parsed_sources::ParsedCompilerSource,
    semantic::impl_method_declaration_name,
    shared::{
        ast::{Block, Expr, ForBinding, FunctionDecl, Pattern, SourceFile, Stmt},
        ast_utils::{walk_expr, walk_stmt, AstVisitor},
    },
    ExpressionKey, ExpressionOwnerKey, SourceSymbolKey,
};

pub(super) struct CallableDefinition<'a> {
    pub(super) key: SourceSymbolKey,
    pub(super) owner: ExpressionOwnerKey,
    pub(super) function: &'a FunctionDecl,
}

pub(super) fn callable_definitions(
    parsed_sources: &[ParsedCompilerSource],
) -> Vec<CallableDefinition<'_>> {
    let mut definitions = Vec::new();
    for parsed in parsed_sources
        .iter()
        .filter(|parsed| !parsed.source().is_test_file)
    {
        for function in &parsed.ast().functions {
            if !function.is_native && !function.is_provider {
                definitions.push(CallableDefinition {
                    key: SourceSymbolKey::new(parsed.module_path(), &function.name),
                    owner: ExpressionOwnerKey::Function(function.name.clone()),
                    function,
                });
            }
        }
        for implementation in &parsed.ast().impls {
            for method in &implementation.method_bodies {
                if !method.is_native && !method.is_provider {
                    definitions.push(CallableDefinition {
                        key: SourceSymbolKey::new(
                            parsed.module_path(),
                            impl_method_declaration_name(&implementation.target, &method.name),
                        ),
                        owner: ExpressionOwnerKey::ImplMethod {
                            type_name: implementation.target.clone(),
                            method: method.name.clone(),
                        },
                        function: method,
                    });
                }
            }
        }
    }
    definitions
}

pub(super) fn expression_key_index(
    definitions: &[CallableDefinition<'_>],
) -> BTreeMap<usize, ExpressionKey> {
    let mut keys = BTreeMap::new();
    for definition in definitions {
        let mut indexer = ExpressionKeyIndexer {
            module_path: definition.key.module_path(),
            owner: definition.owner.clone(),
            next_index: 0,
            keys: &mut keys,
        };
        indexer.visit_block(&definition.function.body);
    }
    keys
}

struct ExpressionKeyIndexer<'a> {
    module_path: &'a str,
    owner: ExpressionOwnerKey,
    next_index: u32,
    keys: &'a mut BTreeMap<usize, ExpressionKey>,
}

impl AstVisitor for ExpressionKeyIndexer<'_> {
    fn visit_expr(&mut self, expression: &Expr) {
        let key = ExpressionKey::new(self.module_path, self.owner.clone(), self.next_index);
        self.next_index = self
            .next_index
            .checked_add(1)
            .expect("ExpressionSourceMap rejects expression preorder overflow");
        self.keys.insert(expr_address(expression), key);
        walk_expr(self, expression);
    }
}

pub(super) fn top_level_value_names(ast: &SourceFile) -> BTreeSet<String> {
    let mut names = ast
        .imports
        .iter()
        .filter_map(|import| {
            import
                .local_binding
                .as_ref()
                .or(import.alias.as_ref())
                .or_else(|| import.path.last())
                .cloned()
        })
        .collect::<BTreeSet<_>>();
    names.extend(
        ast.functions
            .iter()
            .map(|declaration| declaration.name.clone()),
    );
    names.extend(
        ast.function_signatures
            .iter()
            .map(|declaration| declaration.name.clone()),
    );
    names.extend(
        ast.consts
            .iter()
            .map(|declaration| declaration.name.clone()),
    );
    names
}

pub(super) fn reject_static_execution_scopes(
    parsed_sources: &[ParsedCompilerSource],
    diagnostics: &mut Vec<String>,
) {
    for parsed in parsed_sources
        .iter()
        .filter(|parsed| !parsed.source().is_test_file)
    {
        for declaration in &parsed.ast().consts {
            let mut detector = ExecutionScopeDetector::default();
            detector.visit_expr(&declaration.value);
            if let Some(kind) = detector.first {
                diagnostics.push(format!(
                    "{} Const({:?}): {kind} execution scope is not allowed in a top-level const",
                    parsed.module_path(),
                    declaration.name
                ));
            }
        }
        for db in &parsed.ast().dbs {
            for index in &db.indexes {
                let Some(predicate) = &index.where_expr else {
                    continue;
                };
                let mut detector = ExecutionScopeDetector::default();
                detector.visit_expr(predicate);
                if let Some(kind) = detector.first {
                    diagnostics.push(format!(
                        "{} db index `{}`: {kind} execution scope is not allowed in a static DB index predicate",
                        parsed.module_path(),
                        index.name
                    ));
                }
            }
        }
    }
}

#[derive(Default)]
struct ExecutionScopeDetector {
    first: Option<&'static str>,
}

impl AstVisitor for ExecutionScopeDetector {
    fn visit_stmt(&mut self, statement: &Stmt) {
        if self.first.is_none() {
            self.first = match statement {
                Stmt::Timeout { .. } => Some("timeout"),
                Stmt::Concurrent { .. } => Some("concurrent"),
                Stmt::Serial { .. } => Some("serial"),
                Stmt::CompilerTestEffectRegister { .. }
                | Stmt::Assert { .. }
                | Stmt::Let { .. }
                | Stmt::Assign { .. }
                | Stmt::If { .. }
                | Stmt::For { .. }
                | Stmt::While { .. }
                | Stmt::Match { .. }
                | Stmt::DbTransaction { .. }
                | Stmt::Throw { .. }
                | Stmt::Rethrow { .. }
                | Stmt::Emit(_)
                | Stmt::Return(_)
                | Stmt::Spawn { .. }
                | Stmt::Break
                | Stmt::Continue
                | Stmt::Expr(_) => None,
            };
        }
        walk_stmt(self, statement);
    }

    fn visit_expr(&mut self, expression: &Expr) {
        if self.first.is_none() {
            self.first = match expression {
                Expr::ValueBlock(_) => Some("value"),
                Expr::ConcurrentValue(_) => Some("concurrent value"),
                Expr::Timeout { .. } => Some("timeout"),
                Expr::Literal(_)
                | Expr::Identifier(_)
                | Expr::DependencySourceAddress(_)
                | Expr::Binary { .. }
                | Expr::Unary { .. }
                | Expr::Call { .. }
                | Expr::Generic { .. }
                | Expr::InterfaceBox { .. }
                | Expr::Field { .. }
                | Expr::Record { .. }
                | Expr::ObjectLiteral { .. }
                | Expr::Patch { .. }
                | Expr::Throw { .. }
                | Expr::Rethrow { .. }
                | Expr::Catch { .. }
                | Expr::DbOperation(_)
                | Expr::DbQuery(_)
                | Expr::DbTransaction(_)
                | Expr::DbLeaseClaim(_)
                | Expr::DbLeaseRead(_) => None,
            };
        }
        walk_expr(self, expression);
    }
}

#[derive(Default)]
pub(super) struct LocalNameCollector {
    pub(super) names: BTreeSet<String>,
}

impl AstVisitor for LocalNameCollector {
    fn visit_stmt(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Let { name, .. } => {
                self.names.insert(name.clone());
            }
            Stmt::For { binding, .. } => match binding {
                ForBinding::Item { item } => {
                    self.names.insert(item.clone());
                }
                ForBinding::Entry { key, value } => {
                    self.names.insert(key.clone());
                    self.names.insert(value.clone());
                }
            },
            _ => {}
        }
        walk_stmt(self, statement);
    }

    fn visit_expr(&mut self, expression: &Expr) {
        if let Expr::DbLeaseClaim(claim) = expression {
            if let Some(binding) = &claim.binding {
                self.names.insert(binding.clone());
            }
        }
        walk_expr(self, expression);
    }

    fn visit_pattern(&mut self, pattern: &Pattern) {
        pattern_bindings(pattern, &mut self.names);
    }
}

#[derive(Default)]
pub(super) struct ForbiddenConcurrentExpression {
    pub(super) first: Option<&'static str>,
}

impl AstVisitor for ForbiddenConcurrentExpression {
    fn visit_expr(&mut self, expression: &Expr) {
        if self.first.is_none() {
            self.first = match expression {
                Expr::ValueBlock(_) => Some("value block"),
                Expr::ConcurrentValue(_) => Some("nested concurrent value"),
                Expr::Timeout { .. } => Some("timeout"),
                Expr::Throw { .. } => Some("throw"),
                Expr::Rethrow { .. } => Some("rethrow"),
                Expr::Catch { .. } => Some("catch"),
                Expr::DbLeaseClaim(_) => Some("with/resource scope"),
                Expr::Literal(_)
                | Expr::Identifier(_)
                | Expr::DependencySourceAddress(_)
                | Expr::Binary { .. }
                | Expr::Unary { .. }
                | Expr::Call { .. }
                | Expr::Generic { .. }
                | Expr::InterfaceBox { .. }
                | Expr::Field { .. }
                | Expr::Record { .. }
                | Expr::ObjectLiteral { .. }
                | Expr::Patch { .. }
                | Expr::DbOperation(_)
                | Expr::DbQuery(_)
                | Expr::DbTransaction(_)
                | Expr::DbLeaseRead(_) => None,
            };
        }
        walk_expr(self, expression);
    }
}

pub(super) fn referenced_names_in_stmt(statement: &Stmt) -> BTreeSet<String> {
    let mut collector = ReferencedNameCollector::default();
    collector.visit_stmt(statement);
    collector.names
}

pub(super) fn referenced_names_in_expr(expression: &Expr) -> BTreeSet<String> {
    let mut collector = ReferencedNameCollector::default();
    collector.visit_expr(expression);
    collector.names
}

#[derive(Default)]
struct ReferencedNameCollector {
    names: BTreeSet<String>,
    bound: BTreeSet<String>,
}

impl AstVisitor for ReferencedNameCollector {
    fn visit_block(&mut self, block: &Block) {
        let saved_bound = self.bound.clone();
        for statement in &block.statements {
            self.visit_stmt(statement);
        }
        self.bound = saved_bound;
    }

    fn visit_stmt(&mut self, statement: &Stmt) {
        match statement {
            Stmt::CompilerTestEffectRegister { .. } => walk_stmt(self, statement),
            Stmt::Let { name, value, .. } => {
                self.visit_expr(value);
                self.bound.insert(name.clone());
            }
            Stmt::Assign { target, value } => {
                self.visit_expr(target);
                self.visit_expr(value);
            }
            Stmt::Timeout { body, .. }
            | Stmt::Concurrent { body }
            | Stmt::Serial { body }
            | Stmt::DbTransaction { body } => self.visit_block(body),
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                self.visit_expr(condition);
                self.visit_block(then_block);
                if let Some(else_block) = else_block {
                    self.visit_block(else_block);
                }
            }
            Stmt::For {
                binding,
                iterable,
                body,
            } => {
                self.visit_expr(iterable);
                let saved_bound = self.bound.clone();
                match binding {
                    ForBinding::Item { item } => {
                        self.bound.insert(item.clone());
                    }
                    ForBinding::Entry { key, value } => {
                        self.bound.insert(key.clone());
                        self.bound.insert(value.clone());
                    }
                }
                self.visit_block(body);
                self.bound = saved_bound;
            }
            Stmt::While { condition, body } => {
                self.visit_expr(condition);
                self.visit_block(body);
            }
            Stmt::Match { value, arms } => {
                self.visit_expr(value);
                for arm in arms {
                    let saved_bound = self.bound.clone();
                    pattern_bindings(&arm.pattern, &mut self.bound);
                    self.visit_block(&arm.body);
                    self.bound = saved_bound;
                }
            }
            Stmt::Assert { condition, .. } => self.visit_expr(condition),
            Stmt::Throw { value }
            | Stmt::Rethrow { exception: value }
            | Stmt::Emit(value)
            | Stmt::Spawn { call: value, .. }
            | Stmt::Expr(value) => self.visit_expr(value),
            Stmt::Return(value) => {
                if let Some(value) = value {
                    self.visit_expr(value);
                }
            }
            Stmt::Break | Stmt::Continue => {}
        }
    }

    fn visit_expr(&mut self, expression: &Expr) {
        if let Expr::Identifier(name) = expression {
            if !self.bound.contains(name) {
                self.names.insert(name.clone());
            }
            return;
        }
        match expression {
            Expr::ValueBlock(value) | Expr::ConcurrentValue(value) => {
                let saved_bound = self.bound.clone();
                for statement in &value.body.statements {
                    self.visit_stmt(statement);
                }
                self.visit_expr(&value.tail);
                self.bound = saved_bound;
            }
            Expr::Timeout { value, .. } => self.visit_expr(value),
            Expr::DbLeaseClaim(claim) => {
                self.visit_expr(&claim.key);
                let saved_bound = self.bound.clone();
                if let Some(binding) = &claim.binding {
                    self.bound.insert(binding.clone());
                }
                self.visit_block(&claim.body);
                self.bound = saved_bound;
            }
            _ => walk_expr(self, expression),
        }
    }
}

pub(super) fn first_expression(statement: &Stmt) -> Option<&Expr> {
    match statement {
        Stmt::CompilerTestEffectRegister {
            target_probe,
            expect,
            step_expect,
            outcome,
            ..
        } => Some(target_probe)
            .or(expect.as_ref())
            .or(step_expect.as_ref())
            .or_else(|| match outcome {
                crate::shared::ast::TestEffectStepOutcome::Respond { value }
                | crate::shared::ast::TestEffectStepOutcome::Throw { value } => Some(value),
                crate::shared::ast::TestEffectStepOutcome::Stream { events } => events.first(),
            }),
        Stmt::Assert { condition, .. } => Some(condition),
        Stmt::Let { value, .. } => Some(value),
        Stmt::Assign { target, .. } => Some(target),
        Stmt::Timeout { body, .. }
        | Stmt::Concurrent { body }
        | Stmt::Serial { body }
        | Stmt::DbTransaction { body } => first_expression_in_block(body),
        Stmt::If { condition, .. } => Some(condition),
        Stmt::For { iterable, .. } => Some(iterable),
        Stmt::While { condition, .. } => Some(condition),
        Stmt::Match { value, .. } => Some(value),
        Stmt::Throw { value }
        | Stmt::Rethrow { exception: value }
        | Stmt::Emit(value)
        | Stmt::Spawn { call: value, .. }
        | Stmt::Expr(value) => Some(value),
        Stmt::Return(value) => value.as_ref(),
        Stmt::Break | Stmt::Continue => None,
    }
}

fn first_expression_in_block(block: &Block) -> Option<&Expr> {
    block.statements.iter().find_map(first_expression)
}

pub(super) fn pattern_bindings(pattern: &Pattern, names: &mut BTreeSet<String>) {
    match pattern {
        Pattern::Binding(name) => {
            names.insert(name.clone());
        }
        Pattern::Nominal { fields, .. } | Pattern::Record { fields } => {
            for field in fields {
                if let Some(pattern) = &field.pattern {
                    pattern_bindings(pattern, names);
                } else {
                    names.insert(field.name.clone());
                }
            }
        }
        Pattern::Or(patterns) => {
            for pattern in patterns {
                pattern_bindings(pattern, names);
            }
        }
        Pattern::Wildcard | Pattern::Literal(_) => {}
    }
}

pub(super) fn statement_kind(statement: &Stmt) -> &'static str {
    match statement {
        Stmt::CompilerTestEffectRegister { .. } => "compiler test effect",
        Stmt::Assert { .. } => "assert",
        Stmt::Let { .. } => "binding",
        Stmt::Assign { .. } => "assignment",
        Stmt::Timeout { .. } => "timeout",
        Stmt::Concurrent { .. } => "nested concurrent",
        Stmt::Serial { .. } => "nested serial",
        Stmt::If { .. } => "if",
        Stmt::For { .. } => "for",
        Stmt::While { .. } => "while",
        Stmt::Match { .. } => "match",
        Stmt::DbTransaction { .. } => "db transaction",
        Stmt::Throw { .. } => "throw",
        Stmt::Rethrow { .. } => "rethrow",
        Stmt::Emit(_) => "emit",
        Stmt::Return(_) => "return",
        Stmt::Spawn { .. } => "spawn",
        Stmt::Break => "break",
        Stmt::Continue => "continue",
        Stmt::Expr(_) => "expression",
    }
}

pub(super) fn expr_address(expression: &Expr) -> usize {
    std::ptr::from_ref(expression).addr()
}

pub(super) fn without_generic(expression: &Expr) -> &Expr {
    match expression {
        Expr::Generic { callee, .. } => callee,
        _ => expression,
    }
}

pub(super) fn receiver_builtin_spec(
    op: BuiltinReceiverOp,
) -> Option<&'static BuiltinReceiverOpSpec> {
    builtin_receiver_op_spec_by_name(op.receiver.as_str(), op.method.as_str())
}
