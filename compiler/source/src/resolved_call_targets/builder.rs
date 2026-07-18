use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::TypeRefIr;

use crate::{
    dependency_analysis::ResolvedDependencyAnalysisTarget,
    parsed_sources::ParsedCompilerSource,
    shared::{
        ast::{Expr, ForBinding, FunctionDecl, Pattern, Stmt},
        ast_utils::{expr_path, walk_expr, walk_pattern, walk_stmt, AstVisitor},
        type_syntax::generic_parts,
    },
    ExpressionKey, ExpressionOwnerKey, ExpressionSourceMap, ExpressionTypeModel,
    SourceDependencyAnalysisInput, SourceSymbolKey, TypeResolutionContext, TypeResolutionModel,
};

use super::{ResolvedCallTarget, ResolvedCallTargetFacts, UnknownCallTargetReason};

#[derive(Clone)]
enum LocalCallTarget {
    Function {
        module_path: String,
        function_name: String,
    },
    ImplMethod {
        module_path: String,
        type_name: String,
        method_name: String,
    },
}

#[derive(Default)]
struct LocalCallTargetIndex {
    by_path: BTreeMap<String, Vec<LocalCallTarget>>,
    receiver_methods: BTreeMap<(String, String, String), Vec<LocalCallTarget>>,
}

struct TargetCollector<'a> {
    diagnostic_path: &'a str,
    module_path: &'a str,
    owner: ExpressionOwnerKey,
    next_index: u32,
    local_targets: &'a LocalCallTargetIndex,
    expression_sources: &'a ExpressionSourceMap,
    expression_types: &'a ExpressionTypeModel,
    type_resolution: &'a TypeResolutionModel,
    dependencies: &'a SourceDependencyAnalysisInput,
    local_value_names: BTreeSet<String>,
    targets: &'a mut BTreeMap<ExpressionKey, ResolvedCallTarget>,
    errors: &'a mut Vec<String>,
}

pub(super) fn build_resolved_call_targets(
    parsed_sources: &[ParsedCompilerSource],
    expression_sources: &ExpressionSourceMap,
    expression_types: &ExpressionTypeModel,
    type_resolution: &TypeResolutionModel,
    dependencies: &SourceDependencyAnalysisInput,
) -> Result<ResolvedCallTargetFacts, crate::SourceCompileError> {
    let local_targets = LocalCallTargetIndex::build(parsed_sources);
    let mut targets = BTreeMap::new();
    let mut errors = Vec::new();
    for parsed in parsed_sources
        .iter()
        .filter(|parsed| !parsed.source().is_test_file)
    {
        let module_path = parsed.module_path();
        let diagnostic_path = parsed.source().relative_path.display().to_string();
        for function in &parsed.ast().functions {
            if function.is_native || function.is_provider {
                continue;
            }
            collect_owner(
                &diagnostic_path,
                module_path,
                ExpressionOwnerKey::Function(function.name.clone()),
                function,
                &local_targets,
                expression_sources,
                expression_types,
                type_resolution,
                dependencies,
                &mut targets,
                &mut errors,
            );
        }
        for implementation in &parsed.ast().impls {
            for method in &implementation.method_bodies {
                if method.is_native || method.is_provider {
                    continue;
                }
                collect_owner(
                    &diagnostic_path,
                    module_path,
                    ExpressionOwnerKey::ImplMethod {
                        type_name: implementation.target.clone(),
                        method: method.name.clone(),
                    },
                    method,
                    &local_targets,
                    expression_sources,
                    expression_types,
                    type_resolution,
                    dependencies,
                    &mut targets,
                    &mut errors,
                );
            }
        }
    }
    if !errors.is_empty() {
        return Err(crate::SourceCompileError::ContractValidation {
            message: format!(
                "contract call target resolution failed:\n{}",
                errors
                    .into_iter()
                    .map(|error| format!("- {error}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        });
    }
    Ok(ResolvedCallTargetFacts::from_targets(targets))
}

#[allow(clippy::too_many_arguments)]
fn collect_owner(
    diagnostic_path: &str,
    module_path: &str,
    owner: ExpressionOwnerKey,
    function: &FunctionDecl,
    local_targets: &LocalCallTargetIndex,
    expression_sources: &ExpressionSourceMap,
    expression_types: &ExpressionTypeModel,
    type_resolution: &TypeResolutionModel,
    dependencies: &SourceDependencyAnalysisInput,
    targets: &mut BTreeMap<ExpressionKey, ResolvedCallTarget>,
    errors: &mut Vec<String>,
) {
    let mut collector = TargetCollector {
        diagnostic_path,
        module_path,
        owner,
        next_index: 0,
        local_targets,
        expression_sources,
        expression_types,
        type_resolution,
        dependencies,
        local_value_names: local_value_names(function),
        targets,
        errors,
    };
    collector.visit_block(&function.body);
}

impl AstVisitor for TargetCollector<'_> {
    fn visit_expr(&mut self, expr: &Expr) {
        let key = ExpressionKey::new(self.module_path, self.owner.clone(), self.next_index);
        self.next_index = self
            .next_index
            .checked_add(1)
            .expect("expression preorder index overflow was rejected by ExpressionSourceMap");
        if let Expr::Call { callee, .. } = expr {
            let target = self.resolve_call_target(&key, callee);
            self.targets.insert(key, target);
        }
        walk_expr(self, expr);
    }
}

impl TargetCollector<'_> {
    fn resolve_call_target(
        &mut self,
        call_key: &ExpressionKey,
        callee: &Expr,
    ) -> ResolvedCallTarget {
        let semantic_callee = without_generic(callee);
        if let Some(path) = expr_path(semantic_callee) {
            let path_root_is_local = path
                .split('.')
                .next()
                .is_some_and(|root| self.local_value_names.contains(root));
            if !path_root_is_local {
                let local_target = self.local_targets.resolve_path(self.module_path, &path);
                match self.dependencies.resolve_path(&path) {
                    ResolvedDependencyAnalysisTarget::Package {
                        alias,
                        expected_local_abi,
                        callable,
                    } => {
                        if local_target.is_some() {
                            return unknown(UnknownCallTargetReason::UnsupportedDynamicDispatch);
                        }
                        return ResolvedCallTarget::DependencyPackageFunction {
                            package_requirement_alias: alias,
                            package_callable_id: callable.callable_id().clone(),
                            expected_local_abi: expected_local_abi.clone(),
                        };
                    }
                    ResolvedDependencyAnalysisTarget::Contract {
                        requirement,
                        operation,
                    } => {
                        if local_target.is_some() {
                            return unknown(UnknownCallTargetReason::UnsupportedDynamicDispatch);
                        }
                        return ResolvedCallTarget::ContractOperation {
                            contract_requirement: requirement.clone(),
                            contract_operation_id: operation.operation_id.clone(),
                        };
                    }
                    ResolvedDependencyAnalysisTarget::UnknownContractMember {
                        alias,
                        stable_key,
                    } => {
                        self.errors.push(contract_member_error(
                            self.diagnostic_path,
                            call_key,
                            self.expression_sources,
                            &alias,
                            stable_key.as_deref(),
                        ));
                        return unknown(UnknownCallTargetReason::UnresolvedName);
                    }
                    ResolvedDependencyAnalysisTarget::MissingMember => {
                        return unknown(UnknownCallTargetReason::UnresolvedName);
                    }
                    ResolvedDependencyAnalysisTarget::Missing => {}
                }

                if let Some(target) = local_target {
                    return target;
                }
            }
        }

        if let Expr::Field { object: _, field } = semantic_callee {
            let object_key = ExpressionKey::new(
                self.module_path,
                self.owner.clone(),
                call_key
                    .preorder_index()
                    .saturating_add(receiver_object_offset(callee)),
            );
            let context = TypeResolutionContext::source(self.module_path);
            if let Some(receiver) = self
                .expression_types
                .fact(&object_key)
                .and_then(|fact| fact.ty.as_ref())
                .and_then(|ty| {
                    self.type_resolution
                        .concrete_nominal_record_symbol(ty, &context)
                })
            {
                if let Some(target) = self.local_targets.resolve_receiver(&receiver, field) {
                    return target;
                }
            }
            if self
                .expression_types
                .fact(&object_key)
                .and_then(|fact| fact.ty.as_ref())
                .is_some_and(|ty| matches!(ty.ir, TypeRefIr::AnyInterface { .. }))
            {
                return unknown(UnknownCallTargetReason::UnsupportedDynamicDispatch);
            }
        }

        unknown(if expr_path(semantic_callee).is_some() {
            UnknownCallTargetReason::UnresolvedName
        } else {
            UnknownCallTargetReason::UnsupportedDynamicDispatch
        })
    }
}

impl LocalCallTargetIndex {
    fn build(parsed_sources: &[ParsedCompilerSource]) -> Self {
        let mut index = Self::default();
        for parsed in parsed_sources
            .iter()
            .filter(|parsed| !parsed.source().is_test_file)
        {
            let module_path = parsed.module_path();
            for function in &parsed.ast().functions {
                index.insert_path(
                    format!("{module_path}.{}", function.name),
                    LocalCallTarget::Function {
                        module_path: module_path.to_string(),
                        function_name: function.name.clone(),
                    },
                );
            }
            for implementation in &parsed.ast().impls {
                let receiver_name = nominal_root(&implementation.target);
                for method in &implementation.method_bodies {
                    let target = LocalCallTarget::ImplMethod {
                        module_path: module_path.to_string(),
                        type_name: implementation.target.clone(),
                        method_name: method.name.clone(),
                    };
                    index.insert_path(
                        format!("{module_path}.{}.{}", implementation.target, method.name),
                        target.clone(),
                    );
                    if !method.is_static {
                        index
                            .receiver_methods
                            .entry((
                                module_path.to_string(),
                                receiver_name.clone(),
                                method.name.clone(),
                            ))
                            .or_default()
                            .push(target);
                    }
                }
            }
        }
        index
    }

    fn insert_path(&mut self, path: String, target: LocalCallTarget) {
        self.by_path.entry(path).or_default().push(target);
    }

    fn resolve_path(&self, module_path: &str, raw_path: &str) -> Option<ResolvedCallTarget> {
        let path = raw_path.strip_prefix("root.").unwrap_or(raw_path);
        unique_target(self.by_path.get(path))
            .or_else(|| unique_target(self.by_path.get(&format!("{module_path}.{path}"))))
    }

    fn resolve_receiver(
        &self,
        receiver: &SourceSymbolKey,
        method: &str,
    ) -> Option<ResolvedCallTarget> {
        unique_target(self.receiver_methods.get(&(
            receiver.module_path().to_string(),
            nominal_root(receiver.symbol()),
            method.to_string(),
        )))
    }
}

impl LocalCallTarget {
    fn into_resolved(self) -> ResolvedCallTarget {
        match self {
            Self::Function {
                module_path,
                function_name,
            } => ResolvedCallTarget::LocalFunction {
                module_path,
                function_name,
            },
            Self::ImplMethod {
                module_path,
                type_name,
                method_name,
            } => ResolvedCallTarget::LocalImplMethod {
                module_path,
                type_name,
                method_name,
            },
        }
    }
}

fn unique_target(targets: Option<&Vec<LocalCallTarget>>) -> Option<ResolvedCallTarget> {
    let targets = targets?;
    (targets.len() == 1).then(|| targets[0].clone().into_resolved())
}

fn without_generic(mut expr: &Expr) -> &Expr {
    while let Expr::Generic { callee, .. } = expr {
        expr = callee;
    }
    expr
}

fn receiver_object_offset(mut callee: &Expr) -> u32 {
    let mut offset = 2u32; // callee expression, then field object
    while let Expr::Generic { callee: inner, .. } = callee {
        offset = offset.saturating_add(1);
        callee = inner;
    }
    offset
}

fn nominal_root(name: &str) -> String {
    generic_parts(name)
        .map(|parts| parts.root.trim().to_string())
        .unwrap_or_else(|| name.trim().to_string())
}

fn local_value_names(function: &FunctionDecl) -> BTreeSet<String> {
    struct Collector(BTreeSet<String>);

    impl AstVisitor for Collector {
        fn visit_expr(&mut self, expression: &Expr) {
            if let Expr::DbLeaseClaim(claim) = expression {
                if let Some(binding) = &claim.binding {
                    self.0.insert(binding.clone());
                }
            }
            walk_expr(self, expression);
        }

        fn visit_stmt(&mut self, statement: &Stmt) {
            match statement {
                Stmt::Let { name, .. } => {
                    self.0.insert(name.clone());
                }
                Stmt::For { binding, .. } => match binding {
                    ForBinding::Item { item } => {
                        self.0.insert(item.clone());
                    }
                    ForBinding::Entry { key, value } => {
                        self.0.insert(key.clone());
                        self.0.insert(value.clone());
                    }
                },
                _ => {}
            }
            walk_stmt(self, statement);
        }

        fn visit_pattern(&mut self, pattern: &Pattern) {
            match pattern {
                Pattern::Binding(name) => {
                    self.0.insert(name.clone());
                }
                Pattern::Nominal { fields, .. } | Pattern::Record { fields } => {
                    for field in fields.iter().filter(|field| field.pattern.is_none()) {
                        self.0.insert(field.name.clone());
                    }
                }
                Pattern::Or(_) | Pattern::Wildcard | Pattern::Literal(_) => {}
            }
            walk_pattern(self, pattern);
        }
    }

    let mut collector = Collector(
        function
            .params
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect(),
    );
    if function.implicit_self.is_some() {
        collector.0.insert("self".to_string());
    }
    collector.visit_block(&function.body);
    collector.0
}

fn contract_member_error(
    diagnostic_path: &str,
    call_key: &ExpressionKey,
    expression_sources: &ExpressionSourceMap,
    alias: &str,
    stable_key: Option<&str>,
) -> String {
    let source_location = expression_sources
        .fact(call_key)
        .map(|fact| format!("{}:{}", fact.span.start.line, fact.span.start.column))
        .unwrap_or_else(|| "unknown location".to_string());
    let location = format!(
        "{diagnostic_path}:{source_location}: {}, call expression #{}",
        expression_owner_label(call_key.owner()),
        call_key.preorder_index()
    );
    match stable_key {
        Some(stable_key) => format!(
            "{location}: contract dependency `{alias}` has no operation stable key `{stable_key}`"
        ),
        None => format!(
            "{location}: contract dependency `{alias}` must be followed by an operation stable key"
        ),
    }
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

fn unknown(reason: UnknownCallTargetReason) -> ResolvedCallTarget {
    ResolvedCallTarget::Unknown { reason }
}
