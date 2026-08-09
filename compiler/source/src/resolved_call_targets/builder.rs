use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{builtin_receiver_op_by_name, TypeRefIr};

use crate::{
    dependency_analysis::ResolvedDependencyAnalysisTarget,
    parsed_sources::ParsedCompilerSource,
    prelude_registry::prelude_registry,
    semantic::{ExecutableIndex, SemanticSource},
    shared::{
        ast::{Block, Expr, ForBinding, FunctionDecl, LetKind, Pattern, Stmt},
        ast_utils::{dependency_source_address_parts, expr_path, walk_expr, walk_stmt, AstVisitor},
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
        executable_index: u32,
    },
    ImplMethod {
        module_path: String,
        type_name: String,
        method_name: String,
        executable_index: u32,
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
    value_scope: BTreeSet<String>,
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
    let local_targets = LocalCallTargetIndex::build(parsed_sources)?;
    let mut targets = BTreeMap::new();
    let mut errors = Vec::new();
    for parsed in parsed_sources
        .iter()
        .filter(|parsed| !parsed.source().is_test_file)
    {
        let module_path = parsed.module_path();
        let diagnostic_path = parsed.source().relative_path.display().to_string();
        for constant in &parsed.ast().consts {
            collect_const_owner(
                &diagnostic_path,
                module_path,
                constant,
                &local_targets,
                expression_sources,
                expression_types,
                type_resolution,
                dependencies,
                &mut targets,
                &mut errors,
            );
        }
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
fn collect_const_owner(
    diagnostic_path: &str,
    module_path: &str,
    constant: &crate::shared::ast::ConstDecl,
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
        owner: ExpressionOwnerKey::Const(constant.name.clone()),
        next_index: 0,
        local_targets,
        expression_sources,
        expression_types,
        type_resolution,
        dependencies,
        value_scope: BTreeSet::new(),
        targets,
        errors,
    };
    collector.visit_expr(&constant.value);
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
        value_scope: initial_value_scope(function),
        targets,
        errors,
    };
    collector.visit_block(&function.body);
}

impl AstVisitor for TargetCollector<'_> {
    fn visit_block(&mut self, block: &Block) {
        let saved_scope = self.value_scope.clone();
        for statement in &block.statements {
            self.visit_stmt(statement);
        }
        self.value_scope = saved_scope;
    }

    fn visit_stmt(&mut self, statement: &Stmt) {
        match statement {
            Stmt::CompilerTestEffectRegister { .. } => walk_stmt(self, statement),
            Stmt::Let { name, value, .. } => {
                self.visit_expr(value);
                self.value_scope.insert(name.clone());
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
                let saved_scope = self.value_scope.clone();
                match binding {
                    ForBinding::Item { item } => {
                        self.value_scope.insert(item.clone());
                    }
                    ForBinding::Entry { key, value } => {
                        self.value_scope.insert(key.clone());
                        self.value_scope.insert(value.clone());
                    }
                }
                self.visit_block(body);
                self.value_scope = saved_scope;
            }
            Stmt::While { condition, body } => {
                self.visit_expr(condition);
                self.visit_block(body);
            }
            Stmt::Match { value, arms } => {
                self.visit_expr(value);
                for arm in arms {
                    let saved_scope = self.value_scope.clone();
                    collect_pattern_bindings(&arm.pattern, &mut self.value_scope);
                    self.visit_block(&arm.body);
                    self.value_scope = saved_scope;
                }
            }
            Stmt::DbTransaction { body } => self.visit_block(body),
            Stmt::Assert { condition, .. } => self.visit_expr(condition),
            Stmt::Throw { value }
            | Stmt::Rethrow { exception: value }
            | Stmt::Emit(value)
            | Stmt::Expr(value) => self.visit_expr(value),
            Stmt::Return(value) => {
                if let Some(value) = value {
                    self.visit_expr(value);
                }
            }
            Stmt::Break | Stmt::Continue => {}
        }
    }

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
        match expr {
            Expr::ValueBlock(value) => self.visit_value_block(value),
            Expr::ConcurrentValue(value) => {
                self.visit_concurrent_block(&value.body, Some(&value.tail))
            }
            Expr::Timeout { value, .. } => self.visit_expr(value),
            Expr::DbLeaseClaim(claim) => {
                self.visit_expr(&claim.key);
                let saved_scope = self.value_scope.clone();
                if let Some(binding) = &claim.binding {
                    self.value_scope.insert(binding.clone());
                }
                self.visit_block(&claim.body);
                self.value_scope = saved_scope;
            }
            _ => walk_expr(self, expr),
        }
    }
}

impl TargetCollector<'_> {
    fn visit_value_block(&mut self, value: &crate::shared::ast::ValueBlock) {
        let saved_scope = self.value_scope.clone();
        for statement in &value.body.statements {
            self.visit_stmt(statement);
        }
        self.visit_expr(&value.tail);
        self.value_scope = saved_scope;
    }

    fn visit_concurrent_block(&mut self, body: &Block, tail: Option<&Expr>) {
        let saved_scope = self.value_scope.clone();
        let mut sibling_scope = saved_scope.clone();
        for statement in &body.statements {
            self.value_scope = sibling_scope.clone();
            self.visit_stmt(statement);
            if let Stmt::Let {
                kind: LetKind::Let,
                name,
                ..
            } = statement
            {
                sibling_scope.insert(name.clone());
            }
        }
        if let Some(tail) = tail {
            self.value_scope = sibling_scope;
            self.visit_expr(tail);
        }
        self.value_scope = saved_scope;
    }

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
            let path_root_is_local = self.value_scope.contains(path_root);
            if !path_root_is_local {
                if let Some(intrinsic) = exact_config_intrinsic(callee) {
                    return ResolvedCallTarget::ConfigIntrinsic { intrinsic };
                }
                let local_target = self.local_targets.resolve_path(self.module_path, &path);
                match self.dependencies.resolve_path(&path) {
                    ResolvedDependencyAnalysisTarget::Package {
                        alias,
                        expected_local_abi,
                        compiler_owned,
                        callable,
                        ..
                    } => {
                        if local_target.is_some() {
                            return unknown(UnknownCallTargetReason::UnsupportedDynamicDispatch);
                        }
                        return ResolvedCallTarget::DependencyPackageFunction {
                            package_requirement_alias: alias,
                            compiler_owned,
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
                        if let Some(binding_key) = exact_native_binding_key(&path) {
                            return ResolvedCallTarget::NativeFunction {
                                binding_key: binding_key.to_string(),
                            };
                        }
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

                if let Some(binding_key) = exact_native_binding_key(&path) {
                    return ResolvedCallTarget::NativeFunction {
                        binding_key: binding_key.to_string(),
                    };
                }
                if let Some(target) = local_target {
                    return target;
                }
            }
        }

        if let Expr::Field { object, field } = semantic_callee {
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
            if let Some(target) =
                receiver_type.and_then(|ty| self.exact_impl_self_edge_target(object, field, &ty.ir))
            {
                return target;
            }
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
                    if let ResolvedCallTarget::LocalImplMethod {
                        source_callable,
                        executable_index,
                        receiver_type_arguments: _,
                    } = target
                    {
                        let Some(exact) = receiver_type.and_then(|ty| {
                            self.type_resolution
                                .local_receiver_method_resolution(&ty.ir, field, &context)
                        }) else {
                            return unknown(UnknownCallTargetReason::UnsupportedDynamicDispatch);
                        };
                        if exact.source_callable != source_callable {
                            return unknown(UnknownCallTargetReason::UnsupportedDynamicDispatch);
                        }
                        return ResolvedCallTarget::LocalImplMethod {
                            source_callable,
                            executable_index,
                            receiver_type_arguments: exact.receiver_type_arguments,
                        };
                    }
                    return target;
                }
            }
            if let Some(method) = receiver_type.and_then(|ty| {
                self.type_resolution
                    .any_interface_method_signature(&ty.ir, field)
            }) {
                return ResolvedCallTarget::InterfaceMethod {
                    interface: method.interface,
                    method_abi_id: method.method_abi_id,
                    slot: method.slot,
                };
            }
            if receiver_type.is_some_and(|ty| matches!(ty.ir, TypeRefIr::AnyInterface { .. })) {
                return unknown(UnknownCallTargetReason::UnresolvedName);
            }
            if let Some(receiver_method) = receiver_type.and_then(|ty| {
                self.type_resolution
                    .package_receiver_method_resolution(&ty.ir, field)
            }) {
                let source_path = format!(
                    "{}/{}",
                    receiver_method.dependency_ref, receiver_method.source_method_path
                );
                return match self.dependencies.resolve_path(&source_path) {
                    ResolvedDependencyAnalysisTarget::Package {
                        alias,
                        package_build_id,
                        expected_local_abi,
                        compiler_owned,
                        callable,
                    } if alias == receiver_method.canonical_dependency_ref
                        && package_build_id == &receiver_method.expected_package_build
                        && expected_local_abi == &receiver_method.expected_local_abi =>
                    {
                        ResolvedCallTarget::DependencyPackageFunction {
                            package_requirement_alias: alias,
                            compiler_owned,
                            package_callable_id: callable.callable_id().clone(),
                            expected_local_abi: expected_local_abi.clone(),
                            exact_signature: callable.signature().cloned(),
                        }
                    }
                    ResolvedDependencyAnalysisTarget::MissingMember => {
                        self.errors.push(dependency_member_error(
                            self.diagnostic_path,
                            call_key,
                            self.expression_sources,
                            &receiver_method.dependency_ref,
                            &receiver_method.source_method_path,
                        ));
                        unknown(UnknownCallTargetReason::UnresolvedName)
                    }
                    _ => unknown(UnknownCallTargetReason::UnsupportedDynamicDispatch),
                };
            }
        }

        unknown(if expr_path(semantic_callee).is_some() {
            UnknownCallTargetReason::UnresolvedName
        } else {
            UnknownCallTargetReason::UnsupportedDynamicDispatch
        })
    }

    fn exact_impl_self_edge_target(
        &self,
        object: &Expr,
        called_method: &str,
        receiver_type: &TypeRefIr,
    ) -> Option<ResolvedCallTarget> {
        if !matches!(object, Expr::Identifier(name) if name == "self") {
            return None;
        }
        let ExpressionOwnerKey::ImplMethod { type_name, method } = &self.owner else {
            return None;
        };
        if called_method != method {
            return None;
        }
        let expected_source_callable = SourceSymbolKey::new(
            self.module_path,
            crate::semantic::impl_method_declaration_name(type_name, method),
        );
        let lookup = format!("{type_name}.{method}");
        let target = self.local_targets.resolve_path(self.module_path, &lookup)?;
        let ResolvedCallTarget::LocalImplMethod {
            source_callable,
            executable_index,
            ..
        } = target
        else {
            return None;
        };
        if source_callable != expected_source_callable {
            return None;
        }
        let receiver_type_arguments = match receiver_type {
            TypeRefIr::AppliedNominal { arguments, .. } => arguments.clone(),
            _ => Vec::new(),
        };
        Some(ResolvedCallTarget::LocalImplMethod {
            source_callable,
            executable_index,
            receiver_type_arguments,
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
    fn build(parsed_sources: &[ParsedCompilerSource]) -> Result<Self, crate::SourceCompileError> {
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
            let semantic_source = SemanticSource::new(
                parsed.relative_path().display().to_string(),
                module_path,
                parsed.ast(),
                parsed.alias_targets(),
            );
            let executable_index =
                ExecutableIndex::source_index(&semantic_source).map_err(|error| {
                    crate::SourceCompileError::ContractValidation {
                        message: format!(
                        "call target executable index failed for module `{module_path}`: {error}"
                    ),
                    }
                })?;
            for function in &parsed.ast().functions {
                let exact_index = executable_index
                    .entry(&function.name)
                    .expect("the canonical executable index includes every parsed function")
                    .executable_index;
                index.insert_path(
                    format!("{module_path}.{}", function.name),
                    LocalCallTarget::Function {
                        module_path: module_path.to_string(),
                        function_name: function.name.clone(),
                        executable_index: exact_index,
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
                    let declaration_name = crate::semantic::impl_method_declaration_name(
                        &implementation.target,
                        &method.name,
                    );
                    let exact_index = executable_index
                        .entry(&declaration_name)
                        .expect("the canonical executable index includes every parsed impl method")
                        .executable_index;
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
                        executable_index: exact_index,
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
        Ok(index)
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
                executable_index,
            } => ResolvedCallTarget::LocalFunction {
                source_callable: SourceSymbolKey::new(module_path, function_name),
                executable_index,
            },
            Self::ImplMethod {
                module_path,
                type_name,
                method_name,
                executable_index,
            } => ResolvedCallTarget::LocalImplMethod {
                source_callable: SourceSymbolKey::new(
                    module_path,
                    crate::semantic::impl_method_declaration_name(&type_name, &method_name),
                ),
                executable_index,
                receiver_type_arguments: Vec::new(),
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

fn initial_value_scope(function: &FunctionDecl) -> BTreeSet<String> {
    let mut scope = function
        .params
        .iter()
        .map(|parameter| parameter.name.clone())
        .collect::<BTreeSet<_>>();
    if function.implicit_self.is_some() {
        scope.insert("self".to_string());
    }
    scope
}

fn collect_pattern_bindings(pattern: &Pattern, scope: &mut BTreeSet<String>) {
    match pattern {
        Pattern::Binding(name) => {
            scope.insert(name.clone());
        }
        Pattern::Nominal { fields, .. } | Pattern::Record { fields } => {
            for field in fields {
                if let Some(pattern) = &field.pattern {
                    collect_pattern_bindings(pattern, scope);
                } else {
                    scope.insert(field.name.clone());
                }
            }
        }
        Pattern::Or(patterns) => {
            for pattern in patterns {
                collect_pattern_bindings(pattern, scope);
            }
        }
        Pattern::Wildcard | Pattern::Literal(_) => {}
    }
}

fn unknown(reason: UnknownCallTargetReason) -> ResolvedCallTarget {
    ResolvedCallTarget::Unknown { reason }
}
