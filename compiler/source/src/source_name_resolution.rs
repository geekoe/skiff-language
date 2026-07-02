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
    shared::ast_utils::{expr_contains, expr_path},
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
        collect_resolved_paths_in_expr(&constant.value, &env, &mut resolved_paths);
    }
    for function in &input.ast.functions {
        let mut names = top_level_expression_names.clone();
        names.extend(function.params.iter().map(|param| param.name.clone()));
        collect_local_bindings_in_block(&function.body, &mut names);
        let env = build_top_level_env(
            input.module_roots,
            input.package_aliases,
            input.service_aliases,
            &names,
            top_level_names.type_names(),
            input.current_publication_entities,
        );
        collect_resolved_paths_in_block(&function.body, &env, &mut resolved_paths);
    }
    for implementation in &input.ast.impls {
        for method in &implementation.method_bodies {
            let mut names = top_level_expression_names.clone();
            names.extend(method.params.iter().map(|param| param.name.clone()));
            if method.implicit_self.is_some() {
                names.insert("self".to_string());
            }
            collect_local_bindings_in_block(&method.body, &mut names);
            let env = build_top_level_env(
                input.module_roots,
                input.package_aliases,
                input.service_aliases,
                &names,
                top_level_names.type_names(),
                input.current_publication_entities,
            );
            collect_resolved_paths_in_block(&method.body, &env, &mut resolved_paths);
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

/// Table-aware variant used by `SourceCompileModel`.
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

fn collect_local_bindings_in_block(block: &Block, names: &mut BTreeSet<String>) {
    for stmt in &block.statements {
        match stmt {
            Stmt::Let { name, value, .. } => {
                names.insert(name.clone());
                collect_local_bindings_in_expr(value, names);
            }
            Stmt::Assign { target, value } => {
                collect_local_bindings_in_expr(target, names);
                collect_local_bindings_in_expr(value, names);
            }
            Stmt::For {
                binding,
                iterable,
                body,
            } => {
                match binding {
                    ForBinding::Item { item } => {
                        names.insert(item.clone());
                    }
                    ForBinding::Entry { key, value } => {
                        names.insert(key.clone());
                        names.insert(value.clone());
                    }
                }
                collect_local_bindings_in_expr(iterable, names);
                collect_local_bindings_in_block(body, names);
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                collect_local_bindings_in_expr(condition, names);
                collect_local_bindings_in_block(then_block, names);
                if let Some(else_block) = else_block {
                    collect_local_bindings_in_block(else_block, names);
                }
            }
            Stmt::Match { value, arms } => {
                collect_local_bindings_in_expr(value, names);
                for arm in arms {
                    collect_local_bindings_in_pattern(&arm.pattern, names);
                    collect_local_bindings_in_block(&arm.body, names);
                }
            }
            Stmt::DbTransaction { body } => collect_local_bindings_in_block(body, names),
            Stmt::Assert { condition, .. } => collect_local_bindings_in_expr(condition, names),
            Stmt::Emit(value) | Stmt::Expr(value) => collect_local_bindings_in_expr(value, names),
            Stmt::Return(value) => {
                if let Some(value) = value {
                    collect_local_bindings_in_expr(value, names);
                }
            }
            Stmt::Spawn { call } => collect_local_bindings_in_expr(call, names),
            Stmt::Throw { value } => collect_local_bindings_in_expr(value, names),
            Stmt::Rethrow { exception } => collect_local_bindings_in_expr(exception, names),
            Stmt::Break | Stmt::Continue => {}
        }
    }
}

fn collect_local_bindings_in_expr(expr: &Expr, names: &mut BTreeSet<String>) {
    expr_contains(expr, |candidate| {
        match candidate {
            Expr::DbTransaction(transaction) => {
                collect_local_bindings_in_block(&transaction.body, names);
            }
            Expr::DbLeaseClaim(claim) => {
                if let Some(binding) = &claim.binding {
                    names.insert(binding.clone());
                }
                collect_local_bindings_in_block(&claim.body, names);
            }
            _ => {}
        }
        false
    });
}

fn collect_local_bindings_in_pattern(pattern: &Pattern, names: &mut BTreeSet<String>) {
    match pattern {
        Pattern::Binding(name) => {
            names.insert(name.clone());
        }
        Pattern::Nominal { fields, .. } | Pattern::Record { fields } => {
            for field in fields {
                if let Some(pattern) = &field.pattern {
                    collect_local_bindings_in_pattern(pattern, names);
                } else {
                    names.insert(field.name.clone());
                }
            }
        }
        Pattern::Or(patterns) => {
            for pattern in patterns {
                collect_local_bindings_in_pattern(pattern, names);
            }
        }
        Pattern::Wildcard | Pattern::Literal(_) => {}
    }
}

fn collect_resolved_paths_in_block(
    block: &Block,
    env: &ResolutionEnv<'_>,
    resolved_paths: &mut Vec<SourceResolvedPathFact>,
) {
    for stmt in &block.statements {
        match stmt {
            Stmt::Let { value, .. } => collect_resolved_paths_in_expr(value, env, resolved_paths),
            Stmt::Assign { target, value } => {
                collect_resolved_paths_in_expr(target, env, resolved_paths);
                collect_resolved_paths_in_expr(value, env, resolved_paths);
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                collect_resolved_paths_in_expr(condition, env, resolved_paths);
                collect_resolved_paths_in_block(then_block, env, resolved_paths);
                if let Some(else_block) = else_block {
                    collect_resolved_paths_in_block(else_block, env, resolved_paths);
                }
            }
            Stmt::For { iterable, body, .. } => {
                collect_resolved_paths_in_expr(iterable, env, resolved_paths);
                collect_resolved_paths_in_block(body, env, resolved_paths);
            }
            Stmt::Match { value, arms } => {
                collect_resolved_paths_in_expr(value, env, resolved_paths);
                for arm in arms {
                    collect_resolved_paths_in_block(&arm.body, env, resolved_paths);
                }
            }
            Stmt::DbTransaction { body } => {
                collect_resolved_paths_in_block(body, env, resolved_paths);
            }
            Stmt::Assert { condition, .. } => {
                collect_resolved_paths_in_expr(condition, env, resolved_paths);
            }
            Stmt::Emit(value) | Stmt::Expr(value) => {
                collect_resolved_paths_in_expr(value, env, resolved_paths);
            }
            Stmt::Return(value) => {
                if let Some(value) = value {
                    collect_resolved_paths_in_expr(value, env, resolved_paths);
                }
            }
            Stmt::Throw { value } => {
                collect_resolved_paths_in_expr(value, env, resolved_paths);
            }
            Stmt::Rethrow { exception } => {
                collect_resolved_paths_in_expr(exception, env, resolved_paths);
            }
            Stmt::Spawn { call } => {
                collect_resolved_paths_in_expr(call, env, resolved_paths);
            }
            Stmt::Break | Stmt::Continue => {}
        }
    }
}

fn collect_resolved_paths_in_expr(
    expr: &Expr,
    env: &ResolutionEnv<'_>,
    resolved_paths: &mut Vec<SourceResolvedPathFact>,
) {
    expr_contains(expr, |candidate| {
        let Some(expr_path) = expr_path(candidate).filter(|p| p.contains('.') && !p.contains('/'))
        else {
            return false;
        };
        let resolved_path =
            crate::entity::resolve::resolve_dotted_path(env, &expr_path, EntityNamespace::Value);
        let unresolved = resolved_path.is_none();
        resolved_paths.push(SourceResolvedPathFact::new(
            expr_path,
            EntityNamespace::Value,
            resolved_path,
        ));
        unresolved
    });
}

/// 判断 root 是否为 builtin 点号表达式 root(兼容旧 public API)。
///
/// P1a 改为查新 resolver 的 `intrinsic_resolver_root_id` + prelude_registry,
/// 与旧 `matches!("std"|"ext"|...)` 语义等价但基于 registry 而非硬编码。
pub fn is_builtin_dotted_expr_root(root: &str) -> bool {
    intrinsic_resolver_root_id(root).is_some()
        || crate::shared::prelude_registry::prelude_registry().is_native_symbol_root(root)
}
