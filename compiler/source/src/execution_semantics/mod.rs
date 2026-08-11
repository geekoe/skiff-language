use std::collections::BTreeMap;

use crate::semantic::impl_method_declaration_name;
use crate::{
    parsed_sources::ParsedCompilerSource, ExpressionKey, ExpressionOwnerKey, ExpressionSourceMap,
    ResolvedCallTarget, ResolvedCallTargetFacts, SourceCallableEffectFacts, SourceCompileError,
    SourceSymbolKey,
};
use skiff_syntax::ast_utils::AstVisitor;

mod collectors;
mod inout;
mod model;
mod mutation;
mod owner;

pub use model::{
    ConcurrentLaneKind, ConcurrentLanePlan, ConcurrentSourcePlan, ExecutionSourceSite,
    SourceExecutionSemantics, TimeoutSourcePlan,
};

use collectors::{
    callable_definitions, expression_key_index, reject_static_execution_scopes,
    top_level_value_names, ConstExpressionKeyIndexer,
};
use owner::OwnerAnalyzer;

pub(crate) fn analyze_source_execution_semantics(
    parsed_sources: &[ParsedCompilerSource],
    expression_sources: &ExpressionSourceMap,
    resolved_targets: &ResolvedCallTargetFacts,
    callable_effects: &SourceCallableEffectFacts,
) -> Result<SourceExecutionSemantics, SourceCompileError> {
    let definitions = callable_definitions(parsed_sources);
    let mut expression_keys = expression_key_index(&definitions);
    for parsed in parsed_sources {
        for constant in &parsed.ast().consts {
            let mut indexer = ConstExpressionKeyIndexer {
                module_path: parsed.module_path(),
                owner: ExpressionOwnerKey::Const(constant.name.clone()),
                next_index: 0,
                keys: &mut expression_keys,
            };
            indexer.visit_expr(&constant.value);
        }
    }
    let inout_param_indices = inout_param_indices(parsed_sources);
    let mut semantics = SourceExecutionSemantics::default();
    let mut diagnostics = Vec::new();
    reject_static_execution_scopes(parsed_sources, &mut diagnostics);
    validate_const_initializer_purity(
        parsed_sources,
        &expression_keys,
        resolved_targets,
        callable_effects,
        &mut diagnostics,
    );

    for parsed in parsed_sources
        .iter()
        .filter(|parsed| !parsed.source().is_test_file)
    {
        let module_path = parsed.module_path();
        let top_level_value_names = top_level_value_names(parsed.ast());
        for function in &parsed.ast().functions {
            if function.is_native || function.is_provider {
                continue;
            }
            let owner = ExpressionOwnerKey::Function(function.name.clone());
            let mut analyzer = OwnerAnalyzer::new(
                module_path,
                owner,
                function,
                expression_sources,
                &expression_keys,
                resolved_targets,
                callable_effects,
                &inout_param_indices,
                top_level_value_names.clone(),
                &mut semantics,
                &mut diagnostics,
            );
            analyzer.analyze();
        }
        for implementation in &parsed.ast().impls {
            for method in &implementation.method_bodies {
                if method.is_native || method.is_provider {
                    continue;
                }
                let owner = ExpressionOwnerKey::ImplMethod {
                    type_name: implementation.target.clone(),
                    method: method.name.clone(),
                };
                let mut analyzer = OwnerAnalyzer::new(
                    module_path,
                    owner,
                    method,
                    expression_sources,
                    &expression_keys,
                    resolved_targets,
                    callable_effects,
                    &inout_param_indices,
                    top_level_value_names.clone(),
                    &mut semantics,
                    &mut diagnostics,
                );
                analyzer.analyze();
            }
        }
    }

    if diagnostics.is_empty() {
        semantics.validate_complete()?;
        Ok(semantics)
    } else {
        diagnostics.sort();
        diagnostics.dedup();
        Err(SourceCompileError::ContractValidation {
            message: format!(
                "source execution semantics failed:\n{}",
                diagnostics
                    .into_iter()
                    .map(|diagnostic| format!("- {diagnostic}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        })
    }
}

/// Inout parameter positions of every local callable, by source symbol. The
/// map value carries the parameter name for actionable diagnostics.
fn inout_param_indices(
    parsed_sources: &[ParsedCompilerSource],
) -> BTreeMap<SourceSymbolKey, BTreeMap<usize, String>> {
    let mut indices = BTreeMap::new();
    for parsed in parsed_sources {
        let module_path = parsed.module_path();
        for function in &parsed.ast().functions {
            let positions = function
                .params
                .iter()
                .enumerate()
                .filter(|(_, parameter)| parameter.mode == crate::shared::ast::ParamMode::InOut)
                .map(|(index, parameter)| (index, parameter.name.clone()))
                .collect::<BTreeMap<_, _>>();
            if !positions.is_empty() {
                indices.insert(SourceSymbolKey::new(module_path, &function.name), positions);
            }
        }
        for implementation in &parsed.ast().impls {
            for method in &implementation.method_bodies {
                let positions = method
                    .params
                    .iter()
                    .enumerate()
                    .filter(|(_, parameter)| parameter.mode == crate::shared::ast::ParamMode::InOut)
                    .map(|(index, parameter)| (index, parameter.name.clone()))
                    .collect::<BTreeMap<_, _>>();
                if !positions.is_empty() {
                    indices.insert(
                        SourceSymbolKey::new(
                            module_path,
                            impl_method_declaration_name(&implementation.target, &method.name),
                        ),
                        positions,
                    );
                }
            }
        }
    }
    indices
}

/// Top-level const purity gate (R-196, design §3.1): a const initializer must
/// be a pure request-independent expression. Service/Actor/DB/stream/native
/// calls, any may_pending local call, and references to request-derived
/// values are rejected. Pure local calls (NoPending callees in the same
/// package) stay allowed.
fn validate_const_initializer_purity(
    parsed_sources: &[ParsedCompilerSource],
    expression_keys: &BTreeMap<usize, ExpressionKey>,
    resolved_targets: &ResolvedCallTargetFacts,
    callable_effects: &SourceCallableEffectFacts,
    diagnostics: &mut Vec<String>,
) {
    for parsed in parsed_sources {
        let module_path = parsed.module_path();
        let const_names = parsed
            .ast()
            .consts
            .iter()
            .map(|constant| constant.name.clone())
            .collect::<std::collections::BTreeSet<_>>();
        for constant in &parsed.ast().consts {
            let mut visitor = ConstPurityVisitor {
                const_names: &const_names,
                expression_keys,
                resolved_targets,
                callable_effects,
                rejections: Vec::new(),
            };
            visitor.visit_expr(&constant.value);
            let rejections = std::mem::take(&mut visitor.rejections);
            if !rejections.is_empty() {
                diagnostics.push(format!(
                    "{module_path} Const({:?}): const initializer must be a pure request-independent expression: {}",
                    constant.name,
                    rejections.join("; ")
                ));
            }
        }
    }
}

struct ConstPurityVisitor<'a> {
    const_names: &'a std::collections::BTreeSet<String>,
    expression_keys: &'a BTreeMap<usize, ExpressionKey>,
    resolved_targets: &'a ResolvedCallTargetFacts,
    callable_effects: &'a SourceCallableEffectFacts,
    rejections: Vec<String>,
}

impl ConstPurityVisitor<'_> {
    fn reject_call(&mut self, description: String) {
        self.rejections.push(description);
    }

    fn reject(&mut self, description: impl Into<String>) {
        self.rejections.push(description.into());
    }

    fn local_call_is_pure(&self, target: &ResolvedCallTarget) -> bool {
        let Some(source_callable) = target.source_callable_key() else {
            return false;
        };
        match self.callable_effects.operations().get(&source_callable) {
            Some(skiff_artifact_model::CallableEffectSummary::Analyzed { effects }) => {
                !effects.may_pending
            }
            Some(skiff_artifact_model::CallableEffectSummary::Unknown { .. }) | None => false,
        }
    }
}

impl<'a> crate::shared::ast_utils::AstVisitor for ConstPurityVisitor<'a> {
    fn visit_expr(&mut self, expression: &crate::shared::ast::Expr) {
        match expression {
            crate::shared::ast::Expr::Literal(_) => {}
            crate::shared::ast::Expr::Unary { expr, .. } => {
                self.visit_expr(expr);
            }
            crate::shared::ast::Expr::Binary { left, right, .. } => {
                self.visit_expr(left);
                self.visit_expr(right);
            }
            crate::shared::ast::Expr::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                self.visit_expr(condition);
                self.visit_expr(then_expr);
                self.visit_expr(else_expr);
            }
            crate::shared::ast::Expr::Identifier(name) => {
                if !self.const_names.contains(name) {
                    self.reject(format!(
                        "references `{name}`, which is not a top-level const (request-derived values are not allowed)"
                    ));
                }
            }
            crate::shared::ast::Expr::Call { callee: _, args } => {
                let key = self
                    .expression_keys
                    .get(&collectors::expr_address(expression));
                let target = key.and_then(|key| self.resolved_targets.target(key));
                let pure_local = target.is_some_and(|target| self.local_call_is_pure(target));
                match target {
                    Some(ResolvedCallTarget::LocalFunction { .. })
                    | Some(ResolvedCallTarget::LocalImplMethod { .. })
                        if pure_local =>
                    {
                        for arg in args {
                            self.visit_expr(arg.expr());
                        }
                    }
                    _ => {
                        self.reject_call(format!(
                            "contains a {} call; const initializers may only call pure local callables",
                            target_kind_label(target.as_ref().copied())
                        ));
                    }
                }
                // The callee of a rejected call contributes nothing more.
            }
            crate::shared::ast::Expr::Generic { callee, .. } => {
                self.visit_expr(callee);
            }
            crate::shared::ast::Expr::Field { object, .. } => {
                self.visit_expr(object);
            }
            crate::shared::ast::Expr::Index { object, index } => {
                self.visit_expr(object);
                self.visit_expr(index);
            }
            crate::shared::ast::Expr::Record { fields, .. } => {
                for (_, value) in fields {
                    self.visit_expr(value);
                }
            }
            crate::shared::ast::Expr::ObjectLiteral { entries } => {
                for entry in entries {
                    self.visit_expr(&entry.value);
                }
            }
            crate::shared::ast::Expr::ArrayLiteral { items } => {
                for item in items {
                    self.visit_expr(item);
                }
            }
            crate::shared::ast::Expr::Patch { operations, .. } => {
                for operation in operations {
                    match operation {
                        crate::shared::ast::PatchOperation::Set { value, .. }
                        | crate::shared::ast::PatchOperation::Inc { value, .. } => {
                            self.visit_expr(value);
                        }
                    }
                }
            }
            crate::shared::ast::Expr::InterfaceBox { value, .. } => {
                self.reject("contains an interface box (callback capability)");
                self.visit_expr(value);
            }
            crate::shared::ast::Expr::DependencySourceAddress(source) => {
                self.reject(format!(
                    "references dependency value `{}/{}`",
                    source.dependency_ref, source.public_path
                ));
            }
            crate::shared::ast::Expr::DbOperation(_)
            | crate::shared::ast::Expr::DbQuery(_)
            | crate::shared::ast::Expr::DbTransaction(_)
            | crate::shared::ast::Expr::DbLeaseClaim(_)
            | crate::shared::ast::Expr::DbLeaseRead(_) => {
                self.reject("contains a database operation");
            }
            crate::shared::ast::Expr::Dispatch { .. } => {
                self.reject("contains a dispatch expression");
            }
            crate::shared::ast::Expr::ValueBlock(_)
            | crate::shared::ast::Expr::ConcurrentValue(_)
            | crate::shared::ast::Expr::Timeout { .. }
            | crate::shared::ast::Expr::Throw { .. }
            | crate::shared::ast::Expr::Rethrow { .. }
            | crate::shared::ast::Expr::Catch { .. } => {
                self.reject("contains an execution-scoped expression");
            }
        }
    }
}

fn target_kind_label(target: Option<&ResolvedCallTarget>) -> &'static str {
    match target {
        Some(ResolvedCallTarget::LocalFunction { .. })
        | Some(ResolvedCallTarget::LocalImplMethod { .. }) => "non-NoPending local",
        Some(ResolvedCallTarget::ActorMethod { .. }) => "actor",
        Some(ResolvedCallTarget::NativeFunction { .. }) => "native",
        Some(ResolvedCallTarget::ReceiverBuiltin { .. }) => "receiver",
        Some(ResolvedCallTarget::InterfaceMethod { .. }) => "interface",
        Some(ResolvedCallTarget::ContractOperation { .. }) => "service",
        Some(ResolvedCallTarget::DependencyPackageFunction { .. }) => "package-direct",
        Some(ResolvedCallTarget::ConfigIntrinsic { .. }) => "config",
        Some(ResolvedCallTarget::Unknown { .. }) | None => "unresolved",
    }
}
