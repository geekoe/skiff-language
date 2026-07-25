use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{builtin_receiver_op_by_name, TypeRefIr};

use crate::{
    dependency_analysis::ResolvedDependencyAnalysisTarget,
    parsed_sources::ParsedCompilerSource,
    prelude_registry::prelude_registry,
    shared::{
        ast::{Expr, ForBinding, FunctionDecl, Pattern, Stmt},
        ast_utils::{
            dependency_source_address_parts, expr_path, walk_expr, walk_pattern, walk_stmt,
            AstVisitor,
        },
        type_syntax::generic_parts,
    },
    ExpressionKey, ExpressionOwnerKey, ExpressionSourceMap, ExpressionTypeModel,
    SourceDependencyAnalysisInput, SourceSymbolKey, TypeResolutionContext, TypeResolutionModel,
};

use super::{
    dependency_diagnostics::{
        contract_member_error, dependency_member_error, dotted_dependency_call_error,
        unknown_dependency_alias_error,
    },
    ConfigIntrinsic, ResolvedCallTarget, ResolvedCallTargetFacts, UnknownCallTargetReason,
};

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
    ActorMethod {
        actor: SourceSymbolKey,
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
    let contract_operations = targets
        .iter()
        .filter_map(|(key, target)| {
            let ResolvedCallTarget::ContractOperation {
                contract_requirement,
                contract_operation_id,
            } = target
            else {
                return None;
            };
            dependencies
                .exact_contract_operation(contract_requirement, contract_operation_id)
                .cloned()
                .map(|operation| (key.clone(), operation))
        })
        .collect();
    Ok(ResolvedCallTargetFacts::from_targets_and_contract_operations(targets, contract_operations))
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
            let path_root = dependency_source_address_parts(&path)
                .map(|(dependency_ref, _)| dependency_ref)
                .unwrap_or_else(|| path.split('.').next().unwrap_or(&path));
            let path_root_is_local = self.local_value_names.contains(path_root);
            if !path_root_is_local {
                if let Some(intrinsic) = exact_config_intrinsic(callee) {
                    return ResolvedCallTarget::ConfigIntrinsic { intrinsic };
                }
                let local_target = self.local_targets.resolve_path(self.module_path, &path);
                if let Some(binding_key) = exact_native_binding_key(&path) {
                    return ResolvedCallTarget::NativeFunction {
                        binding_key: binding_key.to_string(),
                    };
                }
                match self.dependencies.resolve_path(&path) {
                    ResolvedDependencyAnalysisTarget::Package {
                        alias,
                        expected_local_abi,
                        callable,
                        ..
                    } => {
                        if local_target.is_some() {
                            return unknown(UnknownCallTargetReason::UnsupportedDynamicDispatch);
                        }
                        return ResolvedCallTarget::DependencyPackageFunction {
                            package_requirement_alias: alias,
                            package_callable_id: callable.callable_id().clone(),
                            expected_local_abi: expected_local_abi.clone(),
                            exact_signature: callable.signature().cloned(),
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
                        if let Some((alias, public_path)) = dependency_source_address_parts(&path) {
                            self.errors.push(dependency_member_error(
                                self.diagnostic_path,
                                call_key,
                                self.expression_sources,
                                alias,
                                public_path,
                            ));
                        }
                        return unknown(UnknownCallTargetReason::UnresolvedName);
                    }
                    ResolvedDependencyAnalysisTarget::Missing => {
                        if let Some((alias, _)) = dependency_source_address_parts(&path) {
                            self.errors.push(unknown_dependency_alias_error(
                                self.diagnostic_path,
                                call_key,
                                self.expression_sources,
                                alias,
                            ));
                            return unknown(UnknownCallTargetReason::UnresolvedName);
                        }
                        if let Some((alias, public_path)) = path.split_once('.') {
                            let is_dependency_alias = self
                                .dependencies
                                .package_aliases()
                                .chain(self.dependencies.contract_aliases())
                                .any(|candidate| candidate == alias);
                            if is_dependency_alias {
                                self.errors.push(dotted_dependency_call_error(
                                    self.diagnostic_path,
                                    call_key,
                                    self.expression_sources,
                                    alias,
                                    public_path,
                                ));
                                return unknown(UnknownCallTargetReason::UnresolvedName);
                            }
                        }
                    }
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
            let receiver_type = self
                .expression_types
                .fact(&object_key)
                .and_then(|fact| fact.ty.as_ref());
            if let Some(op) = receiver_type
                .and_then(|ty| {
                    crate::expression_type_model::runtime_receiver_root_from_type_ref(&ty.ir)
                })
                .and_then(|root| builtin_receiver_op_by_name(&root, field))
            {
                return ResolvedCallTarget::ReceiverBuiltin { op };
            }
            if let Some(receiver) = receiver_type.and_then(|ty| {
                self.type_resolution
                    .concrete_nominal_record_symbol(ty, &context)
            }) {
                if let Some(target) = self.local_targets.resolve_receiver(&receiver, field) {
                    return target;
                }
            }
            if receiver_type.is_some_and(|ty| matches!(ty.ir, TypeRefIr::AnyInterface { .. })) {
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

fn exact_config_intrinsic(callee: &Expr) -> Option<ConfigIntrinsic> {
    match callee {
        Expr::Generic { callee, .. } => match callee.as_ref() {
            Expr::Field { object, field } if matches!(object.as_ref(), Expr::Identifier(root) if root == "config") => {
                match field.as_str() {
                    "require" => Some(ConfigIntrinsic::Require),
                    "optional" => Some(ConfigIntrinsic::Optional),
                    _ => None,
                }
            }
            _ => None,
        },
        Expr::Field { object, field }
            if matches!(object.as_ref(), Expr::Identifier(root) if root == "config")
                && field == "has" =>
        {
            Some(ConfigIntrinsic::Has)
        }
        _ => None,
    }
}

fn exact_native_binding_key(path: &str) -> Option<&'static str> {
    let registry = prelude_registry();
    registry
        .native_binding_key(path)
        .or_else(|| crate::prelude_registry::shared_native_binding_key(&format!("std.{path}")))
}

impl LocalCallTargetIndex {
    fn build(parsed_sources: &[ParsedCompilerSource]) -> Self {
        let mut index = Self::default();
        let actor_symbols =
            parsed_sources
                .iter()
                .filter(|parsed| !parsed.source().is_test_file)
                .flat_map(|parsed| {
                    parsed.ast().actors.iter().map(|actor| {
                        SourceSymbolKey::new(parsed.module_path(), actor.name.as_str())
                    })
                })
                .collect::<BTreeSet<_>>();
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
                let actor = resolve_actor_symbol(
                    &actor_symbols,
                    module_path,
                    implementation.target.as_str(),
                );
                for method in &implementation.method_bodies {
                    let target = if !method.is_static {
                        actor.clone().map(|actor| LocalCallTarget::ActorMethod {
                            actor,
                            module_path: module_path.to_string(),
                            type_name: implementation.target.clone(),
                            method_name: method.name.clone(),
                        })
                    } else {
                        None
                    }
                    .unwrap_or_else(|| LocalCallTarget::ImplMethod {
                        module_path: module_path.to_string(),
                        type_name: implementation.target.clone(),
                        method_name: method.name.clone(),
                    });
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
                source_callable: SourceSymbolKey::new(module_path, function_name),
            },
            Self::ImplMethod {
                module_path,
                type_name,
                method_name,
            } => ResolvedCallTarget::LocalImplMethod {
                source_callable: SourceSymbolKey::new(
                    module_path,
                    crate::semantic::impl_method_declaration_name(&type_name, &method_name),
                ),
            },
            Self::ActorMethod {
                actor,
                module_path,
                type_name,
                method_name,
            } => ResolvedCallTarget::ActorMethod {
                method_identity: skiff_artifact_identity::actor_method_identity(
                    actor.module_path(),
                    actor.symbol(),
                    &method_name,
                )
                .expect("indexed actor method identity inputs are non-empty"),
                actor,
                source_callable: SourceSymbolKey::new(
                    module_path,
                    crate::semantic::impl_method_declaration_name(&type_name, &method_name),
                ),
                method_name,
            },
        }
    }
}

fn resolve_actor_symbol(
    actors: &BTreeSet<SourceSymbolKey>,
    module_path: &str,
    raw_target: &str,
) -> Option<SourceSymbolKey> {
    let target = nominal_root(raw_target);
    let target = target.strip_prefix("root.").unwrap_or(&target);
    let candidate = target
        .rsplit_once('.')
        .map(|(module, symbol)| SourceSymbolKey::new(module, symbol))
        .unwrap_or_else(|| SourceSymbolKey::new(module_path, target));
    actors.contains(&candidate).then_some(candidate)
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

fn unknown(reason: UnknownCallTargetReason) -> ResolvedCallTarget {
    ResolvedCallTarget::Unknown { reason }
}
