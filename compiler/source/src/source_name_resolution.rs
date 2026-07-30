//! Source-level name resolution 诊断 pass。
//!
//! P1a 升级:把 `collect_unresolved_dotted_root_violations` 从纯字符串 `BTreeSet` 校验
//! 改为经由 `entity::resolve` 的 typed resolver 驱动。
//!
//! 变更点:
//! - 接收拆分后的 `module_roots`、`package_aliases`、`service_aliases`,而不是单个摊平
//!   的 `available_roots`——旧摊平方案无法区分 package alias 与 service alias,违反架构
//!   case #8。
//! - 内部用 `ResolutionEnv` + `is_unresolved_root` 替换旧 `names.contains(root) &&
//!   is_builtin_dotted_expr_root(root)` 判断。
//! - `is_builtin_dotted_expr_root` 保留为 `pub` 兼容层,委托给新 resolver 的
//!   `intrinsic_resolver_root_id` + prelude_registry 查询。

use std::collections::BTreeSet;

use crate::{
    entity::{
        resolve::{intrinsic_resolver_root_id, ResolutionEnv},
        EntityNamespace, PublicationEntityTable, ResolvedPath,
    },
    shared::ast::{Block, Expr, ForBinding, Pattern, SourceFile, Stmt},
    shared::ast_utils::{expr_path, walk_expr, walk_stmt, AstVisitor},
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamespaceNameFacts {
    value_names: BTreeSet<String>,
    type_names: BTreeSet<String>,
}

impl NamespaceNameFacts {
    pub fn value_names(&self) -> &BTreeSet<String> {
        &self.value_names
    }

    pub fn type_names(&self) -> &BTreeSet<String> {
        &self.type_names
    }

    fn legacy_expression_names(&self) -> BTreeSet<String> {
        self.value_names.union(&self.type_names).cloned().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceResolvedPathFact {
    path: String,
    namespace: EntityNamespace,
    resolved_path: Option<ResolvedPath>,
}

impl SourceResolvedPathFact {
    fn new(path: String, namespace: EntityNamespace, resolved_path: Option<ResolvedPath>) -> Self {
        Self {
            path,
            namespace,
            resolved_path,
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn namespace(&self) -> EntityNamespace {
        self.namespace
    }

    pub fn resolved_path(&self) -> Option<&ResolvedPath> {
        self.resolved_path.as_ref()
    }

    fn root(&self) -> Option<&str> {
        self.path.split('.').next().filter(|root| !root.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceNameResolutionFileFacts {
    relative_path: String,
    module_path: String,
    top_level_names: NamespaceNameFacts,
    resolved_paths: Vec<SourceResolvedPathFact>,
}

impl SourceNameResolutionFileFacts {
    fn new(
        relative_path: String,
        module_path: String,
        top_level_names: NamespaceNameFacts,
        resolved_paths: Vec<SourceResolvedPathFact>,
    ) -> Self {
        Self {
            relative_path,
            module_path,
            top_level_names,
            resolved_paths,
        }
    }

    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    pub fn module_path(&self) -> &str {
        &self.module_path
    }

    pub fn top_level_names(&self) -> &NamespaceNameFacts {
        &self.top_level_names
    }

    pub fn resolved_paths(&self) -> &[SourceResolvedPathFact] {
        &self.resolved_paths
    }
}

pub struct SourceNameResolutionFileBuildInput<'a> {
    pub relative_path: &'a str,
    pub module_path: &'a str,
    pub ast: &'a SourceFile,
    pub module_roots: &'a BTreeSet<String>,
    pub package_aliases: &'a BTreeSet<String>,
    pub service_aliases: &'a BTreeSet<String>,
    pub additional_value_roots: &'a BTreeSet<String>,
    pub current_publication_entities: Option<&'a PublicationEntityTable>,
}

pub fn build_source_name_resolution_file_facts(
    input: SourceNameResolutionFileBuildInput<'_>,
) -> SourceNameResolutionFileFacts {
    let mut top_level_names = top_level_namespace_facts(input.ast);
    top_level_names
        .value_names
        .extend(input.additional_value_roots.iter().cloned());
    let top_level_expression_names = top_level_names.legacy_expression_names();
    let mut resolved_paths = Vec::new();

    for constant in &input.ast.consts {
        let env = build_top_level_env(
            input.module_roots,
            input.package_aliases,
            input.service_aliases,
            &top_level_expression_names,
            top_level_names.type_names(),
            input.current_publication_entities,
        );
        ResolvedPathCollector::new(&env, BTreeSet::new(), &mut resolved_paths)
            .visit_expr(&constant.value);
    }
    for function in &input.ast.functions {
        let mut local_scope = function
            .params
            .iter()
            .map(|param| param.name.clone())
            .collect::<BTreeSet<_>>();
        if function.implicit_self.is_some() {
            local_scope.insert("self".to_string());
        }
        let env = build_top_level_env(
            input.module_roots,
            input.package_aliases,
            input.service_aliases,
            &top_level_expression_names,
            top_level_names.type_names(),
            input.current_publication_entities,
        );
        ResolvedPathCollector::new(&env, local_scope, &mut resolved_paths)
            .visit_block(&function.body);
    }
    for implementation in &input.ast.impls {
        for method in &implementation.method_bodies {
            let mut local_scope = method
                .params
                .iter()
                .map(|param| param.name.clone())
                .collect::<BTreeSet<_>>();
            if method.implicit_self.is_some() {
                local_scope.insert("self".to_string());
            }
            let env = build_top_level_env(
                input.module_roots,
                input.package_aliases,
                input.service_aliases,
                &top_level_expression_names,
                top_level_names.type_names(),
                input.current_publication_entities,
            );
            ResolvedPathCollector::new(&env, local_scope, &mut resolved_paths)
                .visit_block(&method.body);
        }
    }
    for test in &input.ast.tests {
        let env = build_top_level_env(
            input.module_roots,
            input.package_aliases,
            input.service_aliases,
            &top_level_expression_names,
            top_level_names.type_names(),
            input.current_publication_entities,
        );
        ResolvedPathCollector::new(&env, BTreeSet::new(), &mut resolved_paths)
            .visit_block(&test.body);
    }
    for db in &input.ast.dbs {
        let db_fields = input
            .ast
            .types
            .iter()
            .find(|declaration| declaration.name == db.name)
            .map(|declaration| {
                declaration
                    .fields
                    .iter()
                    .map(|field| field.name.clone())
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let env = build_top_level_env(
            input.module_roots,
            input.package_aliases,
            input.service_aliases,
            &top_level_expression_names,
            top_level_names.type_names(),
            input.current_publication_entities,
        );
        for index in &db.indexes {
            if let Some(where_expr) = &index.where_expr {
                ResolvedPathCollector::new(&env, db_fields.clone(), &mut resolved_paths)
                    .visit_expr(where_expr);
            }
        }
    }

    SourceNameResolutionFileFacts::new(
        input.relative_path.to_string(),
        input.module_path.to_string(),
        top_level_names,
        resolved_paths,
    )
}

/// 遍历 AST,收集"点号路径里未解析的 root"违例(诊断 pass)。
///
/// P1a 变更:签名扩展为拆分别名集,消除旧"把 package alias/service alias 与 module root
/// 摊平成单个 `BTreeSet`"的反模式(架构 case #8)。
///
/// - `module_roots`:当前 publication 内的 module 名第一段(如 `user`、`billing`)。
/// - `package_aliases`:package 依赖别名集(manifest 里的 alias)。
/// - `service_aliases`:service 依赖别名集(service.yml 里的 alias)。
///
/// 若调用方目前没有 package 或 service 依赖,传空 `BTreeSet::new()` 的引用即可。
pub fn collect_unresolved_dotted_root_violations(
    path: &str,
    ast: &SourceFile,
    module_roots: &BTreeSet<String>,
    package_aliases: &BTreeSet<String>,
    service_aliases: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    collect_unresolved_dotted_root_violations_with_entities(
        path,
        ast,
        module_roots,
        package_aliases,
        service_aliases,
        None,
        violations,
    );
}

/// Table-aware variant used by `PackageSourceModel`.
///
/// Early source validation can run before declaration anchors exist and therefore calls the
/// compatibility wrapper above with `None`. The source compile model production path has the
/// current publication top-level table and passes it here so `root.<module>.<symbol>` resolves to
/// `EntityId::TopLevel` instead of remaining a generic prefix.
pub fn collect_unresolved_dotted_root_violations_with_entities(
    path: &str,
    ast: &SourceFile,
    module_roots: &BTreeSet<String>,
    package_aliases: &BTreeSet<String>,
    service_aliases: &BTreeSet<String>,
    current_publication_entities: Option<&PublicationEntityTable>,
    violations: &mut Vec<String>,
) {
    let additional_value_roots = BTreeSet::new();
    let facts = build_source_name_resolution_file_facts(SourceNameResolutionFileBuildInput {
        relative_path: path,
        module_path: "",
        ast,
        module_roots,
        package_aliases,
        service_aliases,
        additional_value_roots: &additional_value_roots,
        current_publication_entities,
    });
    collect_unresolved_dotted_root_violations_from_facts(path, &facts, violations);
}

pub fn collect_unresolved_dotted_root_violations_from_facts(
    path: &str,
    facts: &SourceNameResolutionFileFacts,
    violations: &mut Vec<String>,
) {
    let mut unresolved = BTreeSet::new();
    for fact in facts
        .resolved_paths()
        .iter()
        .filter(|fact| fact.namespace() == EntityNamespace::Value)
        .filter(|fact| fact.resolved_path().is_none())
    {
        if let Some(root) = fact.root() {
            unresolved.insert((root.to_string(), fact.path().to_string()));
        }
    }
    for (root, expr_path) in unresolved {
        violations.push(format!(
            "{path}: unresolved root {root} in expression {expr_path}"
        ));
    }
}

/// 构造诊断 pass 用的 `ResolutionEnv`(顶层位置,无 local/type-param owner)。
///
/// 诊断 pass 不区分 value/type namespace,所以 value_names 和 type_names 都传同一份
/// `scope_names`,即能检出 root 未命中即可。
fn build_top_level_env<'a>(
    module_roots: &'a BTreeSet<String>,
    package_aliases: &'a BTreeSet<String>,
    service_aliases: &'a BTreeSet<String>,
    scope_names: &'a BTreeSet<String>,
    type_names: &'a BTreeSet<String>,
    current_publication_entities: Option<&'a PublicationEntityTable>,
) -> ResolutionEnv<'a> {
    let env = ResolutionEnv::new(
        module_roots,
        package_aliases,
        service_aliases,
        scope_names, // value_names
        type_names,  // type_names(诊断 pass 中空集;不做 type/value 区分)
        None,        // no local_owner(顶层)
        None,        // no type_param_owner
    );
    if let Some(top_level_entities) = current_publication_entities {
        env.with_current_publication_entities(top_level_entities)
    } else {
        env
    }
}

fn top_level_namespace_facts(ast: &SourceFile) -> NamespaceNameFacts {
    let mut facts = NamespaceNameFacts::default();
    for import in &ast.imports {
        if let Some(name) = import
            .local_binding
            .as_deref()
            .or_else(|| import.alias.as_deref())
            .or_else(|| import.path.last().map(String::as_str))
        {
            facts.value_names.insert(name.to_string());
            facts.type_names.insert(name.to_string());
        }
    }
    facts
        .type_names
        .extend(ast.types.iter().map(|decl| decl.name.clone()));
    facts
        .type_names
        .extend(ast.aliases.iter().map(|decl| decl.name.clone()));
    facts
        .type_names
        .extend(ast.interfaces.iter().map(|decl| decl.name.clone()));
    facts
        .type_names
        .extend(ast.dbs.iter().map(|decl| decl.name.clone()));
    facts
        .type_names
        .extend(ast.impls.iter().map(|decl| decl.target.clone()));
    facts
        .value_names
        .extend(ast.functions.iter().map(|decl| decl.name.clone()));
    facts
        .value_names
        .extend(ast.function_signatures.iter().map(|decl| decl.name.clone()));
    facts
        .value_names
        .extend(ast.consts.iter().map(|decl| decl.name.clone()));
    facts
}

fn collect_pattern_bindings(pattern: &Pattern, names: &mut BTreeSet<String>) {
    match pattern {
        Pattern::Binding(name) => {
            names.insert(name.clone());
        }
        Pattern::Nominal { fields, .. } | Pattern::Record { fields } => {
            for field in fields {
                if let Some(pattern) = &field.pattern {
                    collect_pattern_bindings(pattern, names);
                } else {
                    names.insert(field.name.clone());
                }
            }
        }
        Pattern::Or(patterns) => {
            for pattern in patterns {
                collect_pattern_bindings(pattern, names);
            }
        }
        Pattern::Wildcard | Pattern::Literal(_) => {}
    }
}

struct ResolvedPathCollector<'a, 'env> {
    env: &'a ResolutionEnv<'env>,
    scope: BTreeSet<String>,
    resolved_paths: &'a mut Vec<SourceResolvedPathFact>,
}

impl<'a, 'env> ResolvedPathCollector<'a, 'env> {
    fn new(
        env: &'a ResolutionEnv<'env>,
        scope: BTreeSet<String>,
        resolved_paths: &'a mut Vec<SourceResolvedPathFact>,
    ) -> Self {
        Self {
            env,
            scope,
            resolved_paths,
        }
    }

    fn visit_value_block(&mut self, value: &crate::shared::ast::ValueBlock) {
        let saved_scope = self.scope.clone();
        for statement in &value.body.statements {
            self.visit_stmt(statement);
        }
        self.visit_expr(&value.tail);
        self.scope = saved_scope;
    }

    fn visit_concurrent_block(&mut self, body: &Block, tail: Option<&Expr>) {
        let saved_scope = self.scope.clone();
        let mut sibling_scope = saved_scope.clone();
        for statement in &body.statements {
            self.scope = sibling_scope.clone();
            self.visit_stmt(statement);
            if let Stmt::Let {
                mutable: false,
                name,
                ..
            } = statement
            {
                sibling_scope.insert(name.clone());
            }
        }
        if let Some(tail) = tail {
            self.scope = sibling_scope;
            self.visit_expr(tail);
        }
        self.scope = saved_scope;
    }
}

impl AstVisitor for ResolvedPathCollector<'_, '_> {
    fn visit_block(&mut self, block: &Block) {
        let saved_scope = self.scope.clone();
        for statement in &block.statements {
            self.visit_stmt(statement);
        }
        self.scope = saved_scope;
    }

    fn visit_stmt(&mut self, statement: &Stmt) {
        match statement {
            Stmt::CompilerTestEffectRegister { .. } => walk_stmt(self, statement),
            Stmt::Let { name, value, .. } => {
                self.visit_expr(value);
                self.scope.insert(name.clone());
            }
            Stmt::Assign { target, value } => {
                self.visit_expr(target);
                self.visit_expr(value);
            }
            Stmt::Timeout { body, .. } | Stmt::Serial { body } => self.visit_block(body),
            Stmt::Concurrent { body } => self.visit_concurrent_block(body, None),
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
                let saved_scope = self.scope.clone();
                match binding {
                    ForBinding::Item { item } => {
                        self.scope.insert(item.clone());
                    }
                    ForBinding::Entry { key, value } => {
                        self.scope.insert(key.clone());
                        self.scope.insert(value.clone());
                    }
                }
                self.visit_block(body);
                self.scope = saved_scope;
            }
            Stmt::Match { value, arms } => {
                self.visit_expr(value);
                for arm in arms {
                    let saved_scope = self.scope.clone();
                    collect_pattern_bindings(&arm.pattern, &mut self.scope);
                    self.visit_block(&arm.body);
                    self.scope = saved_scope;
                }
            }
            Stmt::DbTransaction { body } => self.visit_block(body),
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
        if let Some(path) =
            expr_path(expression).filter(|path| path.contains('.') && !path.contains('/'))
        {
            let root = path.split('.').next().unwrap_or(path.as_str());
            if !self.scope.contains(root) {
                let resolved_path = crate::entity::resolve::resolve_dotted_path(
                    self.env,
                    &path,
                    EntityNamespace::Value,
                );
                self.resolved_paths.push(SourceResolvedPathFact::new(
                    path,
                    EntityNamespace::Value,
                    resolved_path,
                ));
            }
        }
        match expression {
            Expr::ValueBlock(value) => self.visit_value_block(value),
            Expr::ConcurrentValue(value) => {
                self.visit_concurrent_block(&value.body, Some(&value.tail))
            }
            Expr::Timeout { value, .. } => self.visit_expr(value),
            Expr::DbLeaseClaim(claim) => {
                self.visit_expr(&claim.key);
                let saved_scope = self.scope.clone();
                if let Some(binding) = &claim.binding {
                    self.scope.insert(binding.clone());
                }
                self.visit_block(&claim.body);
                self.scope = saved_scope;
            }
            _ => walk_expr(self, expression),
        }
    }
}

/// 判断 root 是否为 builtin 点号表达式 root(兼容旧 public API)。
///
/// P1a 改为查新 resolver 的 `intrinsic_resolver_root_id` + prelude_registry,
/// 与旧 `matches!("std"|"ext"|...)` 语义等价但基于 registry 而非硬编码。
pub fn is_builtin_dotted_expr_root(root: &str) -> bool {
    intrinsic_resolver_root_id(root).is_some()
        || crate::shared::prelude_registry::prelude_registry().is_native_symbol_root(root)
}
