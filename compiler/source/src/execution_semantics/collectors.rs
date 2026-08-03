use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    builtin_receiver_op_spec_by_name, BuiltinReceiverOp, BuiltinReceiverOpSpec,
};

use crate::{
    parsed_sources::ParsedCompilerSource,
    semantic::impl_method_declaration_name,
    shared::{
        ast::{Expr, ForBinding, FunctionDecl, Pattern, SourceFile, Stmt},
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
                | Stmt::Dispatch { .. }
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
                | Expr::Ternary { .. }
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
