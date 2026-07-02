use std::collections::{BTreeMap, BTreeSet};

use crate::file_ir::DbProjectionIr;
use skiff_compiler_core::package_export_resolver::PackageExportResolver;
use skiff_compiler_source::{
    type_text_with_args, ExpressionKey, ExpressionOwnerKey, ExpressionTypeModel, SourceSymbolKey,
};
use skiff_syntax::{
    ast::{
        DbBlockMode, DbBody, DbChangeOp, DbOperation, DbQueryBlock, DbSelector, DbWhereClause,
        Expr, ForBinding, Literal, PatchOperation, SourceFile, Stmt, TypeRef,
    },
    type_syntax::generic_inner,
};

use super::db_lowering::{
    db_field_path_ir, db_lease_read_result_type_text, db_operation_result_type_text_no_db,
};
use super::function_lowering::{expr_path, is_builtin_call_root};
use super::type_lowering::{bare_type_name, type_root};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ExecutableSuspendKey {
    Function { name: String },
    Method { receiver_type: String, name: String },
}

#[derive(Debug, Clone, Default)]
pub(super) struct SuspendIndex {
    values: BTreeMap<ExecutableSuspendKey, bool>,
}

impl SuspendIndex {
    pub(super) fn function_may_suspend(&self, name: &str) -> bool {
        self.values
            .get(&ExecutableSuspendKey::Function {
                name: name.to_string(),
            })
            .copied()
            .unwrap_or(true)
    }

    pub(super) fn method_may_suspend(&self, receiver_type: &str, name: &str) -> bool {
        self.values
            .get(&ExecutableSuspendKey::Method {
                receiver_type: receiver_type.to_string(),
                name: name.to_string(),
            })
            .copied()
            .unwrap_or(true)
    }
}

#[derive(Debug, Clone)]
struct SuspendExecutable<'a> {
    key: ExecutableSuspendKey,
    params: &'a [skiff_syntax::ast::Param],
    return_type: &'a TypeRef,
    implicit_self: Option<&'a TypeRef>,
    body: Option<&'a skiff_syntax::ast::Block>,
    is_native: bool,
    is_provider: bool,
    method_receiver_type: Option<&'a str>,
    is_static_method: bool,
    owner: ExpressionOwnerKey,
}

#[derive(Debug, Clone)]
struct SuspendMethod {
    key: ExecutableSuspendKey,
    receiver_type: String,
    name: String,
    is_static: bool,
}

struct SuspendAnalyzer<'a> {
    module_path: &'a str,
    package_aliases: &'a BTreeMap<String, Vec<String>>,
    service_dependency_aliases: &'a BTreeSet<String>,
    expression_types: Option<&'a ExpressionTypeModel>,
    executables: Vec<SuspendExecutable<'a>>,
    functions_by_name: BTreeMap<String, ExecutableSuspendKey>,
    functions_by_source_key: BTreeMap<SourceSymbolKey, ExecutableSuspendKey>,
    methods: Vec<SuspendMethod>,
}

pub(super) fn suspend_index_for_source(
    ast: &SourceFile,
    module_path: &str,
    package_aliases: &BTreeMap<String, Vec<String>>,
    service_dependency_aliases: &BTreeSet<String>,
    expression_types: Option<&ExpressionTypeModel>,
) -> SuspendIndex {
    SuspendAnalyzer::new(
        ast,
        module_path,
        package_aliases,
        service_dependency_aliases,
        expression_types,
    )
    .analyze()
}

impl<'a> SuspendAnalyzer<'a> {
    fn new(
        ast: &'a SourceFile,
        module_path: &'a str,
        package_aliases: &'a BTreeMap<String, Vec<String>>,
        service_dependency_aliases: &'a BTreeSet<String>,
        expression_types: Option<&'a ExpressionTypeModel>,
    ) -> Self {
        let mut analyzer = Self {
            module_path,
            package_aliases,
            service_dependency_aliases,
            expression_types,
            executables: Vec::new(),
            functions_by_name: BTreeMap::new(),
            functions_by_source_key: BTreeMap::new(),
            methods: Vec::new(),
        };
        analyzer.collect(ast);
        analyzer
    }

    fn collect(&mut self, ast: &'a SourceFile) {
        for function in &ast.functions {
            let key = ExecutableSuspendKey::Function {
                name: function.name.clone(),
            };
            self.functions_by_name
                .entry(function.name.clone())
                .or_insert_with(|| key.clone());
            self.functions_by_source_key
                .entry(SourceSymbolKey::new(self.module_path, &function.name))
                .or_insert_with(|| key.clone());
            self.executables.push(SuspendExecutable {
                key,
                params: &function.params,
                return_type: &function.return_type,
                implicit_self: function.implicit_self.as_ref(),
                body: (!function.is_native && !function.is_provider).then_some(&function.body),
                is_native: function.is_native,
                is_provider: function.is_provider,
                method_receiver_type: None,
                is_static_method: false,
                owner: ExpressionOwnerKey::Function(function.name.clone()),
            });
        }

        for implementation in &ast.impls {
            for method in &implementation.method_bodies {
                let key = ExecutableSuspendKey::Method {
                    receiver_type: implementation.target.clone(),
                    name: method.name.clone(),
                };
                self.methods.push(SuspendMethod {
                    key: key.clone(),
                    receiver_type: implementation.target.clone(),
                    name: method.name.clone(),
                    is_static: method.is_static,
                });
                self.executables.push(SuspendExecutable {
                    key,
                    params: &method.params,
                    return_type: &method.return_type,
                    implicit_self: method.implicit_self.as_ref(),
                    body: (!method.is_native && !method.is_provider).then_some(&method.body),
                    is_native: method.is_native,
                    is_provider: method.is_provider,
                    method_receiver_type: Some(&implementation.target),
                    is_static_method: method.is_static,
                    owner: ExpressionOwnerKey::ImplMethod {
                        type_name: implementation.target.clone(),
                        method: method.name.clone(),
                    },
                });
            }
        }
    }

    fn analyze(self) -> SuspendIndex {
        let mut values = self
            .executables
            .iter()
            .map(|executable| (executable.key.clone(), false))
            .collect::<BTreeMap<_, _>>();

        for _ in 0..self.executables.len().saturating_add(1) {
            let mut changed = false;
            for executable in &self.executables {
                let may_suspend = self.executable_may_suspend(executable, &values);
                if may_suspend && values.get(&executable.key) != Some(&true) {
                    values.insert(executable.key.clone(), true);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        SuspendIndex { values }
    }

    fn executable_may_suspend(
        &self,
        executable: &SuspendExecutable<'_>,
        values: &BTreeMap<ExecutableSuspendKey, bool>,
    ) -> bool {
        if executable.is_provider
            || executable.is_native
            || generic_inner(executable.return_type.name.trim(), "Stream").is_some()
        {
            return true;
        }
        let Some(body) = executable.body else {
            return true;
        };

        let mut env = SuspendTypeEnv::default();
        if let Some(receiver_type) = executable.method_receiver_type {
            if !executable.is_static_method {
                env.declare("self", receiver_type.to_string());
            }
        }
        if let Some(implicit_self) = executable.implicit_self {
            env.declare("self", implicit_self.name.clone());
        }
        for param in executable.params {
            env.declare(&param.name, param.ty.name.clone());
        }

        SuspendContext {
            analyzer: self,
            values,
            env,
            owner: executable.owner.clone(),
            next_expression_index: 0,
        }
        .block_may_suspend(body)
    }

    fn local_function_key(
        &self,
        path: &str,
        env: &SuspendTypeEnv,
    ) -> Option<&ExecutableSuspendKey> {
        if path_root_is_bound(path, env) {
            return None;
        }
        match self.parse_function_path(path)? {
            ParsedFunctionPath::Name(name) => self.functions_by_name.get(name),
            ParsedFunctionPath::SourceKey(source_key) => {
                self.functions_by_source_key.get(&source_key)
            }
        }
    }

    fn local_static_method_key(
        &self,
        path: &str,
        env: &SuspendTypeEnv,
    ) -> Option<&ExecutableSuspendKey> {
        if path_root_is_bound(path, env) {
            return None;
        }
        let path = path
            .strip_prefix(&format!("{}.", self.module_path))
            .unwrap_or(path);
        let (receiver_type, name) = path.rsplit_once('.')?;
        self.resolve_method_key(receiver_type, name, true)
    }

    fn parse_function_path<'b>(&self, path: &'b str) -> Option<ParsedFunctionPath<'b>> {
        if !path.contains('.') {
            return Some(ParsedFunctionPath::Name(path));
        }
        current_module_source_key(self.module_path, path).map(ParsedFunctionPath::SourceKey)
    }

    fn local_receiver_method_key(
        &self,
        receiver_type: &str,
        name: &str,
    ) -> Option<&ExecutableSuspendKey> {
        self.resolve_method_key(receiver_type, name, false)
            .or_else(|| self.resolve_method_key(receiver_type, name, true))
    }

    fn resolve_method_key(
        &self,
        receiver_type: &str,
        name: &str,
        is_static: bool,
    ) -> Option<&ExecutableSuspendKey> {
        self.methods
            .iter()
            .find(|method| {
                method.name == name
                    && method.is_static == is_static
                    && type_roots_match(&method.receiver_type, receiver_type)
            })
            .map(|method| &method.key)
    }
}

enum ParsedFunctionPath<'a> {
    Name(&'a str),
    SourceKey(SourceSymbolKey),
}

struct SuspendContext<'a, 'b> {
    analyzer: &'a SuspendAnalyzer<'b>,
    values: &'a BTreeMap<ExecutableSuspendKey, bool>,
    env: SuspendTypeEnv,
    owner: ExpressionOwnerKey,
    next_expression_index: u32,
}

impl SuspendContext<'_, '_> {
    fn next_expression_key(&mut self) -> ExpressionKey {
        let key = ExpressionKey::new(
            self.analyzer.module_path.to_string(),
            self.owner.clone(),
            self.next_expression_index,
        );
        self.next_expression_index += 1;
        key
    }

    fn expression_type_at_offset(&self, offset: u32) -> Option<String> {
        let preorder_index = self.next_expression_index.checked_add(offset)?;
        let key = ExpressionKey::new(
            self.analyzer.module_path.to_string(),
            self.owner.clone(),
            preorder_index,
        );
        self.analyzer
            .expression_types?
            .fact(&key)?
            .ty
            .as_ref()
            .map(|ty| ty.source_text.clone())
    }

    fn next_expression_type(&self) -> Option<String> {
        self.expression_type_at_offset(0)
    }

    fn receiver_type_for_call_callee(&self, callee: &Expr) -> Option<String> {
        match callee {
            Expr::Field { object, .. } => self
                .expression_type_at_offset(1)
                .or_else(|| self.legacy_expr_type_for_missing_facts(object)),
            Expr::Generic { callee, .. } => match callee.as_ref() {
                Expr::Field { object, .. } => self
                    .expression_type_at_offset(2)
                    .or_else(|| self.legacy_expr_type_for_missing_facts(object)),
                _ => None,
            },
            _ => None,
        }
    }

    fn block_may_suspend(&mut self, block: &skiff_syntax::ast::Block) -> bool {
        self.env.push_scope();
        let may_suspend = block
            .statements
            .iter()
            .any(|stmt| self.stmt_may_suspend(stmt));
        self.env.pop_scope();
        may_suspend
    }

    fn stmt_may_suspend(&mut self, stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Assert { condition, .. } => self.expr_may_suspend(condition),
            Stmt::Let {
                name, ty, value, ..
            } => {
                let fact_type = self.next_expression_type();
                let may_suspend = self.expr_may_suspend(value);
                let inferred = ty
                    .as_ref()
                    .map(|ty| ty.name.clone())
                    .or(fact_type)
                    .or_else(|| self.legacy_expr_type_for_missing_facts(value));
                if let Some(ty) = inferred {
                    self.env.declare(name, ty);
                }
                may_suspend
            }
            Stmt::Assign { target, value } => {
                self.expr_may_suspend(target) || self.expr_may_suspend(value)
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                self.expr_may_suspend(condition)
                    || self.block_may_suspend(then_block)
                    || else_block
                        .as_ref()
                        .is_some_and(|block| self.block_may_suspend(block))
            }
            Stmt::For {
                binding,
                iterable,
                body,
            } => {
                let binding_types = match binding {
                    ForBinding::Item { item } => self
                        .iterable_item_type(iterable)
                        .map(|item_type| vec![(item.as_str(), item_type)])
                        .unwrap_or_default(),
                    ForBinding::Entry { key, value } => self
                        .map_entry_types(iterable)
                        .map(|(key_type, value_type)| {
                            vec![(key.as_str(), key_type), (value.as_str(), value_type)]
                        })
                        .unwrap_or_default(),
                };
                let _ = self.expr_may_suspend(iterable);
                self.env.push_scope();
                for (name, ty) in binding_types {
                    self.env.declare(name, ty);
                }
                let _ = body
                    .statements
                    .iter()
                    .any(|stmt| self.stmt_may_suspend(stmt));
                self.env.pop_scope();
                true
            }
            Stmt::Match { value, arms } => {
                self.expr_may_suspend(value)
                    || arms.iter().any(|arm| self.block_may_suspend(&arm.body))
            }
            Stmt::DbTransaction { body } => {
                let _ = self.block_may_suspend(body);
                true
            }
            Stmt::Throw { value } | Stmt::Rethrow { exception: value } => {
                self.expr_may_suspend(value)
            }
            Stmt::Return(value) => value
                .as_ref()
                .is_some_and(|value| self.expr_may_suspend(value)),
            Stmt::Emit(value) => {
                let _ = self.expr_may_suspend(value);
                true
            }
            Stmt::Expr(value) => self.expr_may_suspend(value),
            Stmt::Spawn { call } => {
                let _ = self.expr_may_suspend(call);
                true
            }
            Stmt::Break | Stmt::Continue => false,
        }
    }

    fn expr_may_suspend(&mut self, expr: &Expr) -> bool {
        self.next_expression_key();
        match expr {
            Expr::Literal(_) | Expr::Identifier(_) | Expr::RemotePublicInstanceSource(_) => false,
            Expr::Binary { left, right, .. } => {
                self.expr_may_suspend(left) || self.expr_may_suspend(right)
            }
            Expr::Unary { expr, .. } | Expr::Generic { callee: expr, .. } => {
                self.expr_may_suspend(expr)
            }
            Expr::InterfaceBox { value, .. } => self.expr_may_suspend(value),
            Expr::Call { callee, args } => {
                let receiver_type = self.receiver_type_for_call_callee(callee);
                let callee_may_suspend = self.expr_may_suspend(callee);
                let args_may_suspend = args.iter().any(|arg| self.expr_may_suspend(arg));
                let (callee, type_args) = unpack_generic_callee(callee);
                callee_may_suspend
                    || args_may_suspend
                    || self.call_may_suspend(callee, type_args, receiver_type.as_deref())
            }
            Expr::Field { object, .. } => self.expr_may_suspend(object),
            Expr::Record { fields, .. } => {
                fields.iter().any(|(_, value)| self.expr_may_suspend(value))
            }
            Expr::ObjectLiteral { entries } => entries
                .iter()
                .any(|entry| self.expr_may_suspend(&entry.value)),
            Expr::Patch { operations, .. } => operations.iter().any(|operation| match operation {
                PatchOperation::Set { value, .. } | PatchOperation::Inc { value, .. } => {
                    self.expr_may_suspend(value)
                }
            }),
            Expr::Throw { value } => self.expr_may_suspend(value),
            Expr::Rethrow { exception } => self.expr_may_suspend(exception),
            Expr::Catch { try_expr, .. } => self.expr_may_suspend(try_expr),
            Expr::DbOperation(operation) => self.db_operation_may_suspend(operation),
            Expr::DbQuery(query) => {
                self.db_query_may_suspend(&query.query);
                false
            }
            Expr::DbTransaction(transaction) => {
                let _ = self.block_may_suspend(&transaction.body);
                true
            }
            Expr::DbLeaseClaim(claim) => {
                let _ = self.expr_may_suspend(&claim.key);
                let _ = self.block_may_suspend(&claim.body);
                true
            }
            Expr::DbLeaseRead(read) => {
                let _ = self.expr_may_suspend(&read.key);
                true
            }
        }
    }

    fn db_operation_may_suspend(&mut self, operation: &DbOperation) -> bool {
        if let Some(selector) = &operation.selector {
            match selector {
                DbSelector::Key { value } => {
                    let _ = self.expr_may_suspend(value);
                }
                DbSelector::Query { query } => self.db_query_may_suspend(query),
            }
        }
        if let Some(query) = operation.independent_query() {
            self.db_query_may_suspend(query);
        }
        for body in [&operation.body, &operation.insert_body]
            .into_iter()
            .flatten()
        {
            match body {
                DbBody::ObjectFields { fields } => {
                    for field in fields {
                        let _ = self.expr_may_suspend(&field.value);
                    }
                }
                DbBody::Values { value } => {
                    let _ = self.expr_may_suspend(value);
                }
            }
        }
        if let Some(change) = &operation.change {
            for op in &change.ops {
                match op {
                    DbChangeOp::Set { value, .. }
                    | DbChangeOp::Inc { value, .. }
                    | DbChangeOp::AddToSet { value, .. }
                    | DbChangeOp::Remove { value, .. } => {
                        let _ = self.expr_may_suspend(value);
                    }
                    DbChangeOp::Unset { .. } => {}
                }
            }
        }
        true
    }

    fn db_query_may_suspend(&mut self, query: &DbQueryBlock) {
        for clause in &query.where_clauses {
            match clause {
                DbWhereClause::Predicate { predicate } => {
                    let _ = self.expr_may_suspend(predicate);
                }
                DbWhereClause::Conditional {
                    condition,
                    predicate,
                } => {
                    let _ = self.expr_may_suspend(condition);
                    let _ = self.expr_may_suspend(predicate);
                }
            }
        }
        if let Some(limit) = &query.limit {
            let _ = self.expr_may_suspend(limit);
        }
        if let Some(offset) = &query.offset {
            let _ = self.expr_may_suspend(offset);
        }
        if let Some(after) = &query.after {
            let _ = self.expr_may_suspend(after);
        }
    }

    fn call_may_suspend(
        &mut self,
        callee: &Expr,
        type_args: &[TypeRef],
        receiver_type: Option<&str>,
    ) -> bool {
        let Some(path) = expr_path(callee) else {
            return true;
        };
        if let Some(may_suspend) = builtin_static_call_may_suspend(&path) {
            return may_suspend;
        }
        if let Some(key) = self.analyzer.local_function_key(&path, &self.env) {
            return self.values.get(key).copied().unwrap_or(true);
        }
        if let Some(key) = self.analyzer.local_static_method_key(&path, &self.env) {
            return self.values.get(key).copied().unwrap_or(true);
        }

        if let Expr::Field { object, field } = callee {
            let legacy_receiver_type;
            let receiver_type = if let Some(receiver_type) = receiver_type {
                Some(receiver_type)
            } else {
                legacy_receiver_type = self.legacy_expr_type_for_missing_facts(object);
                legacy_receiver_type.as_deref()
            };
            if let Some(receiver_type) = receiver_type {
                if let Some(key) = self
                    .analyzer
                    .local_receiver_method_key(&receiver_type, field)
                {
                    return self.values.get(key).copied().unwrap_or(true);
                }
                if builtin_receiver_call_may_suspend(&receiver_type, field, type_args)
                    == Some(false)
                {
                    return false;
                }
            }
        }

        package_or_service_call_may_suspend(
            &path,
            self.analyzer.package_aliases,
            self.analyzer.service_dependency_aliases,
        )
    }

    fn legacy_expr_type_for_missing_facts(&self, expr: &Expr) -> Option<String> {
        self.analyzer
            .expression_types
            .is_none()
            .then(|| self.legacy_expr_type(expr))
            .flatten()
    }

    fn legacy_expr_type(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Literal(Literal::String(_)) => Some("string".to_string()),
            Expr::Literal(Literal::Number(_)) => Some("number".to_string()),
            Expr::Literal(Literal::Bool(_)) => Some("bool".to_string()),
            Expr::Literal(Literal::Null) => Some("null".to_string()),
            Expr::Identifier(name) => self.env.resolve(name).map(str::to_string),
            Expr::RemotePublicInstanceSource(_) => None,
            Expr::Record {
                type_name,
                type_args,
                ..
            } => Some(type_text_with_args(type_name, type_args)),
            Expr::InterfaceBox { interface, .. } => Some(format!("any {}", interface.name)),
            Expr::Call { callee, .. } => {
                let (callee, type_args) = unpack_generic_callee(callee);
                self.legacy_call_return_type(callee, type_args)
            }
            Expr::Generic { callee, .. } => self.legacy_expr_type(callee),
            Expr::Binary { .. }
            | Expr::Unary { .. }
            | Expr::Field { .. }
            | Expr::ObjectLiteral { .. }
            | Expr::Patch { .. }
            | Expr::Throw { .. }
            | Expr::Rethrow { .. }
            | Expr::Catch { .. }
            | Expr::DbQuery(_) => None,
            Expr::DbOperation(operation) => {
                let projection = operation
                    .projection
                    .as_ref()
                    .map(|projection| DbProjectionIr {
                        fields: projection.fields.iter().map(db_field_path_ir).collect(),
                    });
                Some(db_operation_result_type_text_no_db(
                    operation,
                    projection.as_ref(),
                ))
            }
            Expr::DbTransaction(transaction) => match transaction.mode {
                DbBlockMode::Effect => Some("null".to_string()),
                DbBlockMode::Value => None,
            },
            Expr::DbLeaseClaim(_) => Some("bool".to_string()),
            Expr::DbLeaseRead(_) => Some(db_lease_read_result_type_text()),
        }
    }

    fn legacy_call_return_type(&self, callee: &Expr, type_args: &[TypeRef]) -> Option<String> {
        let path = expr_path(callee)?;
        if let Some(return_type) = builtin_static_call_return_type(&path, type_args) {
            return Some(return_type);
        }
        if let Some(key) = self.analyzer.local_function_key(&path, &self.env) {
            return self.executable_return_type(key);
        }
        if let Some(key) = self.analyzer.local_static_method_key(&path, &self.env) {
            return self.executable_return_type(key);
        }
        if let Expr::Field { object, field } = callee {
            let receiver_type = self.legacy_expr_type(object)?;
            if let Some(key) = self
                .analyzer
                .local_receiver_method_key(&receiver_type, field)
            {
                return self.executable_return_type(key);
            }
        }
        None
    }

    fn executable_return_type(&self, key: &ExecutableSuspendKey) -> Option<String> {
        self.analyzer
            .executables
            .iter()
            .find(|executable| &executable.key == key)
            .map(|executable| executable.return_type.name.clone())
    }

    fn iterable_item_type(&mut self, expr: &Expr) -> Option<String> {
        let ty = self
            .next_expression_type()
            .or_else(|| self.legacy_expr_type_for_missing_facts(expr))?;
        if let Some(item) =
            generic_inner(ty.trim(), "Array").or_else(|| generic_inner(ty.trim(), "Stream"))
        {
            return Some(item.trim().to_string());
        }
        map_entry_types(&ty).map(|(key, _value)| key)
    }

    fn map_entry_types(&mut self, expr: &Expr) -> Option<(String, String)> {
        let ty = self
            .next_expression_type()
            .or_else(|| self.legacy_expr_type_for_missing_facts(expr))?;
        map_entry_types(&ty)
    }
}

fn map_entry_types(ty: &str) -> Option<(String, String)> {
    let parts = skiff_syntax::type_syntax::generic_parts(ty.trim())?;
    (parts.root.trim() == "Map" && parts.args.len() == 2).then(|| {
        (
            parts.args[0].trim().to_string(),
            parts.args[1].trim().to_string(),
        )
    })
}

#[derive(Debug, Clone, Default)]
struct SuspendTypeEnv {
    scopes: Vec<BTreeMap<String, String>>,
}

impl SuspendTypeEnv {
    fn push_scope(&mut self) {
        self.scopes.push(BTreeMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn declare(&mut self, name: &str, ty: String) {
        if self.scopes.is_empty() {
            self.push_scope();
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), ty);
        }
    }

    fn resolve(&self, name: &str) -> Option<&str> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).map(String::as_str))
    }
}

fn unpack_generic_callee(callee: &Expr) -> (&Expr, &[TypeRef]) {
    match callee {
        Expr::Generic { callee, type_args } => (callee.as_ref(), type_args.as_slice()),
        _ => (callee, &[]),
    }
}

fn path_root_is_bound(path: &str, env: &SuspendTypeEnv) -> bool {
    path.split('.')
        .next()
        .is_some_and(|root| env.resolve(root).is_some())
}

fn current_module_source_key(module_path: &str, path: &str) -> Option<SourceSymbolKey> {
    path.strip_prefix(module_path)
        .and_then(|symbol| symbol.strip_prefix('.'))
        .filter(|symbol| !symbol.is_empty())
        .map(|symbol| SourceSymbolKey::new(module_path, symbol))
}

fn builtin_static_call_may_suspend(path: &str) -> Option<bool> {
    if path == "emit" || path.starts_with("db.") {
        return Some(true);
    }
    let root = path.split('.').next().unwrap_or(path);
    match root {
        "Array" | "Map" | "object" | "string" | "number" | "bytes" | "json" | "config" => {
            Some(false)
        }
        _ => None,
    }
}

fn builtin_static_call_return_type(path: &str, type_args: &[TypeRef]) -> Option<String> {
    match path {
        "Array.empty" => type_args.first().map(|ty| format!("Array<{}>", ty.name)),
        "db.get" | "db.require" | "db.create" | "db.append" | "db.upsert" => {
            type_args.first().map(|ty| ty.name.clone())
        }
        "db.findMany" | "db.createMany" | "db.create_many" | "db.appendMany" | "db.append_many" => {
            type_args.first().map(|ty| format!("Array<{}>", ty.name))
        }
        "db.exists" => Some("bool".to_string()),
        "db.count" => Some("number".to_string()),
        _ => None,
    }
}

fn builtin_receiver_call_may_suspend(
    receiver_type: &str,
    method: &str,
    _type_args: &[TypeRef],
) -> Option<bool> {
    let root = type_root(receiver_type);
    match (root, method) {
        ("Array", "concat" | "push" | "length")
        | ("Map", "length" | "get" | "has" | "set" | "delete" | "keys" | "clone")
        | ("JsonObject", "length" | "get" | "has" | "set" | "delete" | "clone")
        | ("string", "length" | "split" | "contains" | "startsWith" | "endsWith")
        | ("number", "toString")
        | ("bytes", "toUtf8") => Some(false),
        _ => None,
    }
}

fn package_or_service_call_may_suspend(
    path: &str,
    package_aliases: &BTreeMap<String, Vec<String>>,
    service_dependency_aliases: &BTreeSet<String>,
) -> bool {
    let root = path.split('.').next().unwrap_or(path);
    let resolver = PackageExportResolver::new(package_aliases);
    if resolver.is_package_dependency_root(root)
        || PackageExportResolver::is_default_package_root(root)
        || service_dependency_aliases.contains(root)
        || root == "root"
    {
        return true;
    }
    if is_builtin_call_root(root) {
        return false;
    }
    true
}

fn type_roots_match(declared: &str, actual: &str) -> bool {
    let declared_root = type_root(declared);
    let actual_root = type_root(actual);
    if declared_root == actual_root {
        return true;
    }
    if declared_root.contains('.') || actual_root.contains('.') {
        return false;
    }
    bare_type_name(declared_root) == bare_type_name(actual_root)
}
