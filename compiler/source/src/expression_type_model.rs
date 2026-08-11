use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::type_ref_abi_key;
use skiff_artifact_model::{
    builtin_receiver_op_spec_by_name, BoundaryValueCarrier, BoundaryValueEncoding,
    BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan, BuiltinReceiverPublicReturnType,
    LiteralIr, PackageRefIr, PackageSymbolRef, PackageTypeRef, ParamModeIr, TypeRefIr,
};
use skiff_compiler_core::type_ref::{
    catch_result_branches, contains_type_param, debug_text, is_null_type, map_entry,
    normalize_union, package_type_ref_to_ir, record_field_type, single_item,
    substitute_type_params_in_type_ref_ref as substitute_type_params_in_ir, BuiltinShape,
};

use crate::{
    contract_type_resolution::substitute_package_type,
    parsed_sources::ParsedCompilerSource,
    semantic::impl_method_declaration_name,
    shared::ast::{
        BinaryOp, Block, CallArg, DbBlockMode, DbBody, DbChangeOp, DbQueryBlock, DbSelector,
        DbWhereClause, DispatchTiming, Expr, ForBinding, FunctionDecl, LocalBindingKind, Literal, Param,
        SourceFile, Stmt, TypeRef, UnaryOp,
    },
    shared::ast_utils::{dependency_source_address_parts, expr_path},
    shared::error::SourceSpan,
    shared::prelude_registry::prelude_registry,
    shared::type_expr::TypeExpr,
};

use super::{
    ExpressionKey, ExpressionOwnerKey, ExpressionSourceMap, PublicationDbMetadataIndex,
    ResolvedTypeRef, SourceDependencyAnalysisInput, TypeResolutionContext, TypeResolutionModel,
};

mod assignability;
mod call_typing;
mod contract_call_typing;
mod db_projection;
mod db_typing;
mod expression_assignability;
mod indexing;
mod materialization;
mod narrowing;
mod object_materialization;
mod test_effects;

pub use call_typing::runtime_receiver_root_from_type_ref;
pub use contract_call_typing::package_type_ref_from_contract_type;
use contract_call_typing::{
    contract_source_assignability_with_projections, package_type_target_assignable,
    ContractCallOutcome, ContractCallTyping, ContractProjectionState,
};
use db_projection::DbProjectionTypeResolver;
use expression_assignability::ExpressionAssignability;
pub use object_materialization::{
    MaterializedObjectField, MaterializedObjectSourceField, ObjectFieldValueSource,
    ObjectMaterializationKind, TargetTypedObjectMaterialization,
};
use object_materialization::{
    ObjectLiteralSource, ObjectLiteralSourceField, ObjectMaterializationState,
};

#[derive(Clone, Debug, Default)]
pub struct ExpressionTypeModel {
    facts: BTreeMap<ExpressionKey, ExpressionTypeFact>,
    index_segments: BTreeMap<ExpressionKey, SourceIndexSegmentFact>,
    constructor_validations: BTreeMap<ExpressionKey, ConstructorValidation>,
    representation_constructor_validations:
        BTreeMap<ExpressionKey, RepresentationConstructorValidation>,
    object_materializations: BTreeMap<ExpressionKey, TargetTypedObjectMaterialization>,
}

/// Compiler-known collection kind selected for one source bracket segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceIndexReceiverKind {
    Array,
    Map,
    JsonObject,
}

/// Exact failure/store policy of one source bracket segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceIndexPolicy {
    StrictRead,
    IntermediateMustExist,
    TerminalReplace,
    TerminalUpsert,
    LoanMustExist,
}

/// Symbolic, source-owned typing fact for one `object[selector]` segment.
///
/// The expression keys preserve the required receiver-before-selector order;
/// every key is owner-relative and therefore does not contain a pool-local
/// artifact index.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceIndexSegmentFact {
    pub receiver_kind: SourceIndexReceiverKind,
    pub receiver_type: TypeRefIr,
    pub selector_type: TypeRefIr,
    pub result_type: TypeRefIr,
    pub policy: SourceIndexPolicy,
    pub object_expression: ExpressionKey,
    pub selector_expression: ExpressionKey,
    pub source_span: SourceSpan,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ExpressionTypeModelBuildError {
    model: Box<ExpressionTypeModel>,
    diagnostics: Vec<String>,
}

impl ExpressionTypeModelBuildError {
    pub fn message(&self) -> String {
        self.diagnostics.join("\n- ")
    }

    #[allow(dead_code)]
    pub fn model(&self) -> &ExpressionTypeModel {
        &self.model
    }
}

#[derive(Clone, Debug)]
pub struct ExpressionTypeFact {
    pub ty: Option<ResolvedTypeRef>,
    pub span: SourceSpan,
    pub test_effect_throw_payload_type: Option<TypeRefIr>,
    stream_emit_target: Option<ResolvedTypeRef>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ConstructorValidation {
    pub target: ResolvedTypeRef,
    pub provided_fields: Vec<ConstructorProvidedField>,
    pub materialized_fields: Vec<MaterializedConstructorField>,
    pub duplicate_fields: Vec<DuplicateConstructorField>,
    pub missing_required_fields: Vec<MissingConstructorField>,
    pub unknown_fields: Vec<UnknownConstructorField>,
    pub type_mismatches: Vec<ConstructorFieldTypeMismatch>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ConstructorProvidedField {
    pub name: String,
    pub expression: ExpressionKey,
    pub actual: Option<ResolvedTypeRef>,
    pub name_span: SourceSpan,
    pub value_span: SourceSpan,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct DuplicateConstructorField {
    pub name: String,
    pub first_expression: ExpressionKey,
    pub duplicate_expression: ExpressionKey,
    pub name_span: SourceSpan,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct MissingConstructorField {
    pub name: String,
    pub expected: ResolvedTypeRef,
    pub span: SourceSpan,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct UnknownConstructorField {
    pub name: String,
    pub expression: ExpressionKey,
    pub actual: Option<ResolvedTypeRef>,
    pub name_span: SourceSpan,
    pub value_span: SourceSpan,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ConstructorFieldTypeMismatch {
    pub name: String,
    pub expression: ExpressionKey,
    pub expected: ResolvedTypeRef,
    pub actual: ResolvedTypeRef,
    pub value_span: SourceSpan,
}

#[derive(Clone, Debug)]
pub struct MaterializedConstructorField {
    pub name: String,
    pub ty: ResolvedTypeRef,
    pub source: ConstructorFieldValueSource,
}

#[derive(Clone, Debug)]
pub enum ConstructorFieldValueSource {
    Provided {
        field_name: String,
        expression: ExpressionKey,
    },
    SyntheticNull,
}

#[derive(Clone, Debug)]
pub struct RepresentationConstructorValidation {
    pub target: ResolvedTypeRef,
    pub payload: ExpressionKey,
}

#[derive(Clone, Debug)]
struct CallableSignature {
    module_path: String,
    declaration_name: String,
    params: Vec<CallableParam>,
    return_type: TypeRef,
    type_params: Vec<String>,
}

#[derive(Clone, Debug)]
struct CallableParam {
    ty: TypeRef,
}

struct ResolvedCallableParams {
    params: Vec<(String, ResolvedTypeRef)>,
    complete: bool,
}

struct ResolvedTypeArgSubstitutions {
    types: BTreeMap<String, TypeRefIr>,
    complete: bool,
}

#[derive(Clone, Debug, Default)]
struct ExactTypeEnvironment {
    resolved: BTreeMap<String, ResolvedTypeRef>,
    projected: BTreeMap<String, Result<PackageTypeRef, String>>,
}

#[derive(Clone, Debug, Default)]
struct ConditionNarrowings {
    when_true: TypeNarrowing,
    when_false: TypeNarrowing,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ExactTestEffectTarget {
    Package {
        package_build_id: skiff_artifact_model::PackageBuildId,
        callable_id: skiff_artifact_model::PackageCallableId,
    },
    Service {
        protocol_identity: skiff_artifact_model::ServiceProtocolIdentity,
        operation_id: skiff_artifact_model::ContractOperationId,
    },
}

#[derive(Clone, Debug, Default)]
struct TypeNarrowing {
    env: BTreeMap<String, ResolvedTypeRef>,
    paths: BTreeMap<String, ResolvedTypeRef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IndexExpressionContext {
    TerminalRead,
    Intermediate,
    AssignmentTerminal,
    LoanTerminal,
}

impl TypeNarrowing {
    fn combined(mut self, other: TypeNarrowing) -> Self {
        self.env.extend(other.env);
        self.paths.extend(other.paths);
        self
    }
}

struct OwnerChecker<'a> {
    module_path: &'a str,
    owner: ExpressionOwnerKey,
    next_index: u32,
    type_resolution: &'a TypeResolutionModel,
    publication_db_metadata: &'a PublicationDbMetadataIndex,
    expression_sources: &'a ExpressionSourceMap,
    callable_signatures: &'a BTreeMap<String, CallableSignature>,
    dependency_analysis: Option<&'a SourceDependencyAnalysisInput>,
    return_type: Option<TypeRef>,
    stream_chunk: Option<ResolvedTypeRef>,
    type_context: TypeResolutionContext<'a>,
    env: BTreeMap<String, ResolvedTypeRef>,
    contract_projection: ContractProjectionState,
    path_refinements: BTreeMap<String, ResolvedTypeRef>,
    /// The declared/last-assigned type of each binding that currently carries
    /// an env-level narrowing (root bindings narrow in `env`, member paths in
    /// `path_refinements`). Write-side invalidation (assignment and inout
    /// loans, R-196) restores the root from here.
    narrowing_base: BTreeMap<String, ResolvedTypeRef>,
    transparent_value_targets: BTreeMap<ExpressionKey, ExpressionKey>,
    test_effect_declarations: BTreeMap<ExactTestEffectTarget, String>,
    db_transaction_depth: usize,
    outputs: &'a mut CheckOutputs,
}

/// Mutable output collections accumulated while checking one compilation's
/// source files. `ExpressionTypeModel::build` creates one `CheckOutputs` and
/// threads it through every owner checker; the finished collections are then
/// moved into the returned model.
#[derive(Default)]
struct CheckOutputs {
    facts: BTreeMap<ExpressionKey, ExpressionTypeFact>,
    index_segments: BTreeMap<ExpressionKey, SourceIndexSegmentFact>,
    constructor_validations: BTreeMap<ExpressionKey, ConstructorValidation>,
    representation_constructor_validations:
        BTreeMap<ExpressionKey, RepresentationConstructorValidation>,
    object_materialization: ObjectMaterializationState,
    diagnostics: Vec<String>,
}

struct ExpressionCheckContext<'a> {
    expression_sources: &'a ExpressionSourceMap,
    type_resolution: &'a TypeResolutionModel,
    publication_db_metadata: &'a PublicationDbMetadataIndex,
    callable_signatures: &'a BTreeMap<String, CallableSignature>,
    dependency_analysis: Option<&'a SourceDependencyAnalysisInput>,
}

struct ValueAssignmentContext<'a> {
    annotation: Option<&'a TypeRef>,
    exact_expected: Option<&'a PackageTypeRef>,
    diagnostic_context: &'a str,
    fallback_span: SourceSpan,
}

impl ExpressionTypeModel {
    pub fn build(
        parsed_sources: &[ParsedCompilerSource],
        expression_sources: &ExpressionSourceMap,
        type_resolution: &TypeResolutionModel,
        publication_db_metadata: &PublicationDbMetadataIndex,
        dependency_analysis: Option<&SourceDependencyAnalysisInput>,
    ) -> Result<Self, ExpressionTypeModelBuildError> {
        let callable_signatures = callable_signatures(parsed_sources);
        let mut outputs = CheckOutputs::default();
        let context = ExpressionCheckContext {
            expression_sources,
            type_resolution,
            publication_db_metadata,
            callable_signatures: &callable_signatures,
            dependency_analysis,
        };
        for parsed in parsed_sources {
            check_source(
                parsed.source().module_path.as_str(),
                parsed.ast(),
                &context,
                &mut outputs,
            );
        }

        for (key, source) in &outputs.object_materialization.sources {
            if source.allow_targetless {
                continue;
            }
            if outputs.object_materialization.targeted.contains(key) {
                continue;
            }
            outputs.diagnostics.push(format!(
                "{}: object literal at {} requires an explicit target type",
                key.module_path(),
                span_label(source.span)
            ));
        }

        let CheckOutputs {
            facts,
            index_segments,
            constructor_validations,
            representation_constructor_validations,
            object_materialization,
            diagnostics,
        } = outputs;
        let model = Self {
            facts,
            index_segments,
            constructor_validations,
            representation_constructor_validations,
            object_materializations: object_materialization.facts,
        };
        if !diagnostics.is_empty() {
            return Err(ExpressionTypeModelBuildError {
                model: Box::new(model),
                diagnostics,
            });
        }

        Ok(model)
    }

    pub fn fact(&self, key: &ExpressionKey) -> Option<&ExpressionTypeFact> {
        self.facts.get(key)
    }

    pub fn index_segment(&self, key: &ExpressionKey) -> Option<&SourceIndexSegmentFact> {
        self.index_segments.get(key)
    }

    pub fn index_segments(&self) -> &BTreeMap<ExpressionKey, SourceIndexSegmentFact> {
        &self.index_segments
    }

    pub fn constructor_validation(&self, key: &ExpressionKey) -> Option<&ConstructorValidation> {
        self.constructor_validations.get(key)
    }

    pub fn representation_constructor_validation(
        &self,
        key: &ExpressionKey,
    ) -> Option<&RepresentationConstructorValidation> {
        self.representation_constructor_validations.get(key)
    }

    pub fn object_materialization(
        &self,
        key: &ExpressionKey,
    ) -> Option<&TargetTypedObjectMaterialization> {
        self.object_materializations.get(key)
    }

    /// Returns the `Stream<T>` chunk target recorded by the unified expression
    /// checker for a `Stmt::Emit` root expression.
    pub fn stream_emit_target(&self, key: &ExpressionKey) -> Option<&ResolvedTypeRef> {
        self.facts.get(key)?.stream_emit_target.as_ref()
    }
}

fn check_source(
    module_path: &str,
    ast: &SourceFile,
    context: &ExpressionCheckContext<'_>,
    outputs: &mut CheckOutputs,
) {
    let const_env = const_type_env(
        ast,
        context.type_resolution,
        context.dependency_analysis,
        &TypeResolutionContext::source(module_path),
    );
    for function in &ast.functions {
        if function.is_native || function.is_provider {
            continue;
        }
        check_function_owner(
            module_path,
            ExpressionOwnerKey::Function(function.name.clone()),
            function,
            &[],
            context.expression_sources,
            context.type_resolution,
            context.publication_db_metadata,
            context.callable_signatures,
            context.dependency_analysis,
            &const_env,
            outputs,
        );
    }

    for implementation in &ast.impls {
        let inherited =
            crate::shared::type_syntax::generic_type_parameter_names(&implementation.target);
        for method in &implementation.method_bodies {
            if method.is_native || method.is_provider {
                continue;
            }
            check_function_owner(
                module_path,
                ExpressionOwnerKey::ImplMethod {
                    type_name: implementation.target.clone(),
                    method: method.name.clone(),
                },
                method,
                &inherited,
                context.expression_sources,
                context.type_resolution,
                context.publication_db_metadata,
                context.callable_signatures,
                context.dependency_analysis,
                &const_env,
                outputs,
            );
        }
    }

    for constant in &ast.consts {
        let (projected_annotation, projection_failed) = match const_env
            .projected
            .get(&constant.name)
        {
            Some(Ok(projected)) => (Some(projected.clone()), false),
            Some(Err(error)) => {
                outputs.diagnostics.push(format!(
                    "{module_path}: const `{}` annotation exact source type projection failed: {error}",
                    constant.name
                ));
                (None, true)
            }
            None => (None, false),
        };
        let mut checker = OwnerChecker::new(
            module_path,
            ExpressionOwnerKey::Const(constant.name.clone()),
            TypeResolutionContext::source(module_path),
            BTreeMap::new(),
            BTreeMap::new(),
            context.expression_sources,
            context.type_resolution,
            context.publication_db_metadata,
            context.callable_signatures,
            context.dependency_analysis,
            None,
            outputs,
        );
        let value_key = checker.peek_key();
        let actual = checker.check_expr(&constant.value);
        if !projection_failed {
            if let (Some(annotation), Some(actual)) = (&constant.ty, actual) {
                checker.check_assignable(
                    annotation,
                    &actual,
                    projected_annotation.as_ref(),
                    constant.span,
                    "const initializer",
                    Some((&constant.value, &value_key)),
                );
            }
        }
    }

    for test in &ast.tests {
        let mut checker = OwnerChecker::new(
            module_path,
            ExpressionOwnerKey::Test(test.name.clone()),
            TypeResolutionContext::source(module_path),
            const_env.resolved.clone(),
            const_env.projected.clone(),
            context.expression_sources,
            context.type_resolution,
            context.publication_db_metadata,
            context.callable_signatures,
            context.dependency_analysis,
            None,
            outputs,
        );
        checker.check_block(&test.body);
    }

    for db in &ast.dbs {
        for index in &db.indexes {
            if let Some(where_expr) = &index.where_expr {
                let type_context = TypeResolutionContext::source(module_path);
                let env = db_index_where_env(&db.name, context.type_resolution, &type_context);
                let mut checker = OwnerChecker::new(
                    module_path,
                    ExpressionOwnerKey::DbIndexWhere {
                        db: db.name.clone(),
                        index: index.name.clone(),
                    },
                    type_context,
                    env,
                    BTreeMap::new(),
                    context.expression_sources,
                    context.type_resolution,
                    context.publication_db_metadata,
                    context.callable_signatures,
                    context.dependency_analysis,
                    None,
                    outputs,
                );
                checker.check_condition(where_expr, "db index where condition");
            }
        }
    }
}

fn direct_stream_item_type(ty: &PackageTypeRef) -> Option<&PackageTypeRef> {
    match ty {
        PackageTypeRef::Container { name, arguments }
            if name == BuiltinShape::Stream.name() && arguments.len() == 1 =>
        {
            arguments.first()
        }
        _ => None,
    }
}

fn const_type_env(
    ast: &SourceFile,
    type_resolution: &TypeResolutionModel,
    dependency_analysis: Option<&SourceDependencyAnalysisInput>,
    type_context: &TypeResolutionContext<'_>,
) -> ExactTypeEnvironment {
    let mut env = ExactTypeEnvironment::default();
    for constant in &ast.consts {
        let Some(ty) = constant.ty.as_ref() else {
            continue;
        };
        let Ok(resolved) = type_resolution.resolve_type_ref(ty, type_context) else {
            continue;
        };
        if let Some(dependency_analysis) = dependency_analysis {
            env.projected.insert(
                constant.name.clone(),
                ContractProjectionState::project_source_type_ref(
                    ty,
                    type_resolution,
                    dependency_analysis,
                    type_context,
                ),
            );
        }
        env.resolved.insert(constant.name.clone(), resolved);
    }
    env
}

fn db_index_where_env(
    db_name: &str,
    type_resolution: &TypeResolutionModel,
    type_context: &TypeResolutionContext<'_>,
) -> BTreeMap<String, ResolvedTypeRef> {
    type_resolution
        .resolve_constructor_target(db_name, &[], type_context)
        .map(|target| target.fields)
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn check_function_owner(
    module_path: &str,
    owner: ExpressionOwnerKey,
    function: &FunctionDecl,
    inherited_type_params: &[String],
    expression_sources: &ExpressionSourceMap,
    type_resolution: &TypeResolutionModel,
    publication_db_metadata: &PublicationDbMetadataIndex,
    callable_signatures: &BTreeMap<String, CallableSignature>,
    dependency_analysis: Option<&SourceDependencyAnalysisInput>,
    const_env: &ExactTypeEnvironment,
    outputs: &mut CheckOutputs,
) {
    let type_params = inherited_type_params
        .iter()
        .chain(&function.type_params)
        .cloned()
        .collect::<BTreeSet<_>>();
    let type_context = TypeResolutionContext::with_type_params(module_path, type_params);
    let mut env = const_env.resolved.clone();
    let mut projected_env = const_env.projected.clone();
    if let Some(self_type) = &function.implicit_self {
        if let Ok(resolved) = type_resolution.resolve_type_ref(self_type, &type_context) {
            if let Some(dependency_analysis) = dependency_analysis {
                projected_env.insert(
                    "self".to_string(),
                    ContractProjectionState::project_source_type_ref(
                        self_type,
                        type_resolution,
                        dependency_analysis,
                        &type_context,
                    ),
                );
            }
            env.insert("self".to_string(), resolved);
        }
    }
    for param in &function.params {
        if let Ok(resolved) = type_resolution.resolve_type_ref(&param.ty, &type_context) {
            if let Some(dependency_analysis) = dependency_analysis {
                projected_env.insert(
                    param.name.clone(),
                    ContractProjectionState::project_source_type_ref(
                        &param.ty,
                        type_resolution,
                        dependency_analysis,
                        &type_context,
                    ),
                );
            }
            env.insert(param.name.clone(), resolved);
        }
    }
    let mut checker = OwnerChecker::new(
        module_path,
        owner,
        type_context,
        env,
        projected_env,
        expression_sources,
        type_resolution,
        publication_db_metadata,
        callable_signatures,
        dependency_analysis,
        Some(function.return_type.clone()),
        outputs,
    );
    checker.check_block(&function.body);
}

#[allow(clippy::too_many_arguments)]
impl<'a> OwnerChecker<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        module_path: &'a str,
        owner: ExpressionOwnerKey,
        type_context: TypeResolutionContext<'a>,
        env: BTreeMap<String, ResolvedTypeRef>,
        exact_bindings: BTreeMap<String, Result<PackageTypeRef, String>>,
        expression_sources: &'a ExpressionSourceMap,
        type_resolution: &'a TypeResolutionModel,
        publication_db_metadata: &'a PublicationDbMetadataIndex,
        callable_signatures: &'a BTreeMap<String, CallableSignature>,
        dependency_analysis: Option<&'a SourceDependencyAnalysisInput>,
        return_type: Option<TypeRef>,
        outputs: &'a mut CheckOutputs,
    ) -> Self {
        let stream_chunk = return_type.as_ref().and_then(|return_type| {
            type_resolution
                .resolve_type_ref(return_type, &type_context)
                .ok()
                .and_then(|return_type| stream_chunk_type(&return_type))
        });
        let (contract_projection, projection_diagnostics) = ContractProjectionState::new(
            &env,
            &exact_bindings,
            type_resolution,
            dependency_analysis,
            &type_context,
        );
        outputs.diagnostics.extend(
            projection_diagnostics
                .into_iter()
                .map(|diagnostic| format!("{module_path}: {diagnostic}")),
        );
        Self {
            module_path,
            owner,
            next_index: 0,
            type_resolution,
            publication_db_metadata,
            expression_sources,
            callable_signatures,
            dependency_analysis,
            return_type,
            stream_chunk,
            type_context,
            env,
            contract_projection,
            path_refinements: BTreeMap::new(),
            narrowing_base: BTreeMap::new(),
            transparent_value_targets: BTreeMap::new(),
            test_effect_declarations: BTreeMap::new(),
            db_transaction_depth: 0,
            outputs,
        }
    }

    fn check_block_in_db_transaction(&mut self, block: &Block) -> bool {
        self.db_transaction_depth += 1;
        let exits = self.check_block(block);
        self.db_transaction_depth -= 1;
        exits
    }

    fn check_block(&mut self, block: &Block) -> bool {
        let mut exits = false;
        for stmt in &block.statements {
            exits = self.check_stmt(stmt) || exits;
        }
        exits
    }

    fn project_source_binding_type(&self, ty: &TypeRef) -> Result<Option<PackageTypeRef>, String> {
        let Some(dependency_analysis) = self.dependency_analysis else {
            return Ok(None);
        };
        ContractProjectionState::project_source_type_ref(
            ty,
            self.type_resolution,
            dependency_analysis,
            &self.type_context,
        )
        .map(Some)
    }

    fn check_stmt(&mut self, stmt: &Stmt) -> bool {
        match stmt {
            Stmt::CompilerTestEffectRegister {
                target,
                target_probe: _,
                declaration_start,
                expect,
                step_expect,
                outcome,
            } => self.check_compiler_test_effect_register_stmt(
                target,
                *declaration_start,
                expect.as_ref(),
                step_expect.as_ref(),
                outcome,
            ),
            Stmt::Assert { condition, .. } => self.check_assert_stmt(condition),
            Stmt::LocalBinding {
                name, ty, value, ..
            } => self.check_let_stmt(name, ty.as_ref(), value),
            Stmt::Assign { target, value } => self.check_assign_stmt(target, value),
            Stmt::Timeout { body, .. } | Stmt::Serial { body } => {
                self.check_timeout_or_serial_stmt(body)
            }
            Stmt::Concurrent { body } => self.check_concurrent_stmt(body),
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => self.check_if_stmt(condition, then_block, else_block.as_ref()),
            Stmt::For {
                binding,
                iterable,
                body,
            } => self.check_for_stmt(binding, iterable, body),
            Stmt::While { condition, body } => self.check_while_stmt(condition, body),
            Stmt::Match { value, arms } => self.check_match_stmt(value, arms),
            Stmt::DbTransaction { body } => self.check_block_in_db_transaction(body),
            Stmt::Throw { value } => self.check_throw_stmt(value),
            Stmt::Emit(value) => self.check_emit_stmt(value),
            Stmt::Expr(value) => self.check_expr_stmt(value),
            Stmt::Rethrow { exception } => self.check_rethrow_stmt(exception),
            Stmt::Return(value) => self.check_return_stmt(value.as_ref()),
            Stmt::Break | Stmt::Continue => true,
        }
    }

    fn check_compiler_test_effect_register_stmt(
        &mut self,
        target: &str,
        declaration_start: bool,
        expect: Option<&Expr>,
        step_expect: Option<&Expr>,
        outcome: &crate::shared::ast::TestEffectStepOutcome,
    ) -> bool {
        // The synthetic target probe exists solely to obtain the same
        // exact ResolvedCallTarget fact ordinary dependency calls use.
        // It is not an invocation and therefore is not type-checked as
        // a zero-argument call.
        self.next_key();
        self.next_key();
        let Some(dependencies) = self.dependency_analysis else {
            self.outputs.diagnostics.push(format!(
                "{}: compiler test effect `{target}` has no dependency analysis",
                self.module_path
            ));
            return false;
        };
        let (signature, exact_target) = match dependencies.resolve_path(target) {
            crate::dependency_analysis::ResolvedDependencyAnalysisTarget::Package {
                package_build_id,
                callable,
                ..
            } => {
                let Some(signature) = callable.signature().cloned() else {
                    self.outputs.diagnostics.push(format!(
                        "{}: compiler test effect target `{target}` has no exact signature",
                        self.module_path
                    ));
                    return false;
                };
                (
                    signature,
                    ExactTestEffectTarget::Package {
                        package_build_id: package_build_id.clone(),
                        callable_id: callable.callable_id().clone(),
                    },
                )
            }
            crate::dependency_analysis::ResolvedDependencyAnalysisTarget::Contract {
                requirement,
                operation,
            } => {
                let contract = &operation.contract;
                let mut parameters = Vec::with_capacity(contract.parameters.len());
                for parameter in &contract.parameters {
                    if !matches!(
                        &parameter.value_plan,
                        BoundaryValuePlan::Linkable {
                            carrier: BoundaryValueCarrier::DetachedValueGraph,
                            encoding: BoundaryValueEncoding::CanonicalValue,
                            owner: BoundaryValueOwner::Caller,
                            lifetime: BoundaryValueLifetime::Call,
                        }
                    ) {
                        self.outputs.diagnostics.push(format!(
                            "{}: compiler test effect target `{target}` parameter `{}` is not proven to use the ordinary boundary value calling mode",
                            self.module_path, parameter.name
                        ));
                        return false;
                    }
                    parameters.push(skiff_artifact_model::PackageCallableParameter {
                        name: parameter.name.clone(),
                        ty: package_type_ref_from_contract_type(&parameter.ty),
                        mode: ParamModeIr::Value,
                    });
                }
                let return_type = match &contract.stream {
                    skiff_artifact_model::BoundaryStreamContract::Unary => {
                        package_type_ref_from_contract_type(&contract.return_value.ty)
                    }
                    skiff_artifact_model::BoundaryStreamContract::ServerStream {
                        item_type,
                        ..
                    } => PackageTypeRef::Container {
                        name: BuiltinShape::Stream.name().to_string(),
                        arguments: vec![package_type_ref_from_contract_type(item_type)],
                    },
                    skiff_artifact_model::BoundaryStreamContract::Unsupported { .. } => {
                        self.outputs.diagnostics.push(format!(
                                    "{}: compiler test effect target `{target}` has an unsupported stream contract",
                                    self.module_path
                                ));
                        return false;
                    }
                };
                (
                    skiff_artifact_model::PackageCallableSignature {
                        type_params: Vec::new(),
                        parameters,
                        return_type,
                        may_suspend: true,
                    },
                    ExactTestEffectTarget::Service {
                        protocol_identity: requirement.expected_protocol_identity.clone(),
                        operation_id: operation.operation_id.clone(),
                    },
                )
            }
            _ => {
                self.outputs.diagnostics.push(format!(
                    "{}: unresolved compiler test effect target `{target}`",
                    self.module_path
                ));
                return false;
            }
        };
        if declaration_start {
            if let Some(previous) = self
                .test_effect_declarations
                .insert(exact_target.clone(), target.to_string())
            {
                self.outputs.diagnostics.push(format!(
                            "{}: test effect targets `{previous}` and `{target}` resolve to the same exact target {exact_target:?}; use one explicit sequence",
                            self.module_path
                        ));
            }
        }
        if let Some(expect) = expect {
            let [parameter] = signature.parameters.as_slice() else {
                self.outputs.diagnostics.push(format!(
                    "{}: test effect `{target}` expect requires exactly one parameter",
                    self.module_path
                ));
                return false;
            };
            self.check_test_effect_request_subset(expect, &parameter.ty);
        }
        if let Some(step_expect) = step_expect {
            let [parameter] = signature.parameters.as_slice() else {
                self.outputs.diagnostics.push(format!(
                            "{}: test effect `{target}` sequence step expect requires exactly one parameter",
                            self.module_path
                        ));
                return false;
            };
            self.check_test_effect_request_subset(step_expect, &parameter.ty);
        }
        match outcome {
            crate::shared::ast::TestEffectStepOutcome::Respond { value } => {
                self.check_test_effect_value(value, &signature.return_type, "respond");
                if direct_stream_item_type(&signature.return_type).is_some() {
                    self.outputs.diagnostics.push(format!(
                                "{}: test effect `{target}` cannot use respond for a direct Stream<T> target; use stream",
                                self.module_path
                            ));
                }
            }
            crate::shared::ast::TestEffectStepOutcome::Throw { value } => {
                self.check_test_effect_throw(value, target);
            }
            crate::shared::ast::TestEffectStepOutcome::Stream { events } => {
                let Some(item) = direct_stream_item_type(&signature.return_type) else {
                    self.outputs.diagnostics.push(format!(
                        "{}: test effect `{target}` stream requires Stream<T> return",
                        self.module_path
                    ));
                    return false;
                };
                for value in events {
                    self.check_test_effect_value(value, item, "stream event");
                }
            }
        }
        false
    }

    fn check_assert_stmt(&mut self, condition: &Expr) -> bool {
        let narrowings = self.condition_narrowings(condition);
        self.check_condition(condition, "condition");
        self.apply_narrowing(&narrowings.when_true);
        false
    }

    fn check_let_stmt(&mut self, name: &String, ty: Option<&TypeRef>, value: &Expr) -> bool {
        let value_key = self.peek_key();
        let actual = if ty.is_none() {
            let actual = self.check_expr(value);
            self.maybe_upgrade_map_literal_expression_type(&value_key, value, actual)
        } else {
            self.check_expr(value)
        };
        let projected_actual = self
            .contract_projection
            .expression_type(&value_key)
            .cloned();
        let (binding_ty, projected_binding) = if let Some(annotation) = ty {
            match self
                .type_resolution
                .resolve_type_ref(annotation, &self.type_context)
            {
                Ok(expected) => {
                    let (projected_expected, projection_failed) = match self
                        .project_source_binding_type(annotation)
                    {
                        Ok(projected) => (projected, false),
                        Err(error) => {
                            self.outputs.diagnostics.push(format!(
                                        "{}: local binding `{name}` annotation exact source type projection failed: {error}",
                                        self.module_path
                                    ));
                            (None, true)
                        }
                    };
                    if !projection_failed {
                        if let Some(actual) = &actual {
                            self.check_value_assignable_to_expected(
                                value,
                                &value_key,
                                actual,
                                &expected,
                                ValueAssignmentContext {
                                    annotation: Some(annotation),
                                    exact_expected: projected_expected.as_ref(),
                                    diagnostic_context: &format!("local binding {name} annotation"),
                                    fallback_span: self.expression_span(&value_key),
                                },
                            );
                        }
                    }
                    (Some(expected), projected_expected)
                }
                Err(error) => {
                    self.outputs.diagnostics.push(format!(
                        "{}: failed to resolve local binding {name} annotation: {error}",
                        self.module_path
                    ));
                    (actual, projected_actual)
                }
            }
        } else {
            (actual, projected_actual)
        };
        if let Some(binding_ty) = binding_ty {
            self.env.insert(name.clone(), binding_ty);
        }
        self.contract_projection.bind(name, projected_binding);
        false
    }

    fn check_assign_stmt(&mut self, target: &Expr, value: &Expr) -> bool {
        let expected = self.check_expr_with_index_context(
            target,
            true,
            None,
            IndexExpressionContext::AssignmentTerminal,
        );
        let value_key = self.peek_key();
        let actual = self.check_expr(value);
        let self_field_assignment = matches!(
            target,
            Expr::Field { object, .. }
                if matches!(object.as_ref(), Expr::Identifier(name) if name == "self")
        );
        if self_field_assignment || place_contains_index(target) {
            if let (Some(actual), Some(expected)) = (actual.as_ref(), expected.as_ref()) {
                self.check_value_assignable_to_expected(
                    value,
                    &value_key,
                    actual,
                    expected,
                    ValueAssignmentContext {
                        annotation: None,
                        exact_expected: None,
                        diagnostic_context: if self_field_assignment {
                            "self field assignment"
                        } else {
                            "indexed assignment"
                        },
                        fallback_span: self.expression_span(&value_key),
                    },
                );
            }
        }
        if let (Expr::Identifier(name), Some(actual)) = (target, actual) {
            self.env.insert(name.clone(), actual);
            // The assignment establishes the binding's new base type; a later
            // write-side invalidation must not revert to the pre-assignment
            // narrowing base.
            self.narrowing_base.remove(name);
            let projected = self
                .contract_projection
                .expression_type(&value_key)
                .cloned();
            self.contract_projection.bind(name, projected);
        }
        self.invalidate_path_refinements_for_write(target);
        false
    }

    fn check_timeout_or_serial_stmt(&mut self, body: &Block) -> bool {
        self.check_block_scoped(body, &TypeNarrowing::default());
        false
    }

    fn check_concurrent_stmt(&mut self, body: &Block) -> bool {
        self.check_concurrent_block(body, None);
        false
    }

    fn check_if_stmt(
        &mut self,
        condition: &Expr,
        then_block: &Block,
        else_block: Option<&Block>,
    ) -> bool {
        let narrowings = self.condition_narrowings(condition);
        self.check_condition(condition, "if condition");
        let then_exits = self.check_block_scoped(then_block, &narrowings.when_true);
        let else_exits = else_block
            .is_some_and(|else_block| self.check_block_scoped(else_block, &narrowings.when_false));
        match else_block {
            Some(_) if then_exits && else_exits => true,
            Some(_) if then_exits => {
                self.apply_narrowing(&narrowings.when_false);
                false
            }
            Some(_) if else_exits => {
                self.apply_narrowing(&narrowings.when_true);
                false
            }
            None if then_exits => {
                self.apply_narrowing(&narrowings.when_false);
                false
            }
            None => {
                if let Some(narrowing) = self.null_guard_assignment_narrowing(condition, then_block)
                {
                    self.apply_narrowing(&narrowing);
                }
                false
            }
            _ => false,
        }
    }

    fn check_for_stmt(&mut self, binding: &ForBinding, iterable: &Expr, body: &Block) -> bool {
        let iterable_key = self.peek_key();
        let iterable_ty = self.check_expr(iterable);
        let iterable_projection = self
            .contract_projection
            .expression_type(&iterable_key)
            .cloned();
        let saved_projected_env = self.contract_projection.binding_snapshot();
        let mut previous = Vec::new();
        let mut previous_projected = Vec::new();
        match binding {
            ForBinding::Item { item } => {
                match iterable_ty.as_ref().and_then(single_for_item_type) {
                    Some(item_ty) => {
                        previous.push((item.clone(), self.env.insert(item.clone(), item_ty)));
                        previous_projected
                            .push((item.clone(), saved_projected_env.get(item).cloned()));
                        self.contract_projection.bind(
                            item,
                            iterable_projection
                                .as_ref()
                                .and_then(single_for_item_projection),
                        );
                    }
                    None => self.outputs.diagnostics.push(format!(
                        "{}: for iterable must be Array, Stream, or Map at {}",
                        self.module_path,
                        self.expression_span_label(&iterable_key)
                    )),
                }
            }
            ForBinding::Entry { key, value } => {
                match iterable_ty.as_ref().and_then(map_entry_types) {
                    Some((key_ty, value_ty)) => {
                        previous.push((key.clone(), self.env.insert(key.clone(), key_ty)));
                        previous.push((value.clone(), self.env.insert(value.clone(), value_ty)));
                        previous_projected
                            .push((key.clone(), saved_projected_env.get(key).cloned()));
                        previous_projected
                            .push((value.clone(), saved_projected_env.get(value).cloned()));
                        let (key_projection, value_projection) = iterable_projection
                            .as_ref()
                            .and_then(map_entry_projections)
                            .map(|(key, value)| (Some(key), Some(value)))
                            .unwrap_or((None, None));
                        self.contract_projection.bind(key, key_projection);
                        self.contract_projection.bind(value, value_projection);
                    }
                    None => self.outputs.diagnostics.push(format!(
                        "{}: for entry binding requires Map at {}",
                        self.module_path,
                        self.expression_span_label(&iterable_key)
                    )),
                }
            }
        }
        self.check_block(body);
        for (name, previous) in previous {
            if let Some(previous) = previous {
                self.env.insert(name, previous);
            } else {
                self.env.remove(&name);
            }
        }
        for (name, previous) in previous_projected {
            self.contract_projection.bind(&name, previous);
        }
        false
    }

    fn check_while_stmt(&mut self, condition: &Expr, body: &Block) -> bool {
        self.check_condition(condition, "while condition");
        self.check_block_scoped(body, &TypeNarrowing::default());
        false
    }

    fn check_match_stmt(&mut self, value: &Expr, arms: &[crate::shared::ast::MatchArm]) -> bool {
        self.check_expr(value);
        for arm in arms {
            self.check_block(&arm.body);
        }
        false
    }

    fn check_throw_stmt(&mut self, value: &Expr) -> bool {
        let key = self.peek_key();
        if let Some(actual) = self.check_expr(value) {
            self.validate_throw_payload(&key, &actual, "throw");
        }
        true
    }

    fn check_emit_stmt(&mut self, value: &Expr) -> bool {
        let value_key = self.peek_key();
        let actual = self.check_expr(value);
        let Some(expected) = self.stream_chunk.clone() else {
            return false;
        };
        self.record_stream_emit_target(&value_key, expected.clone());
        if let Some(actual) = actual {
            self.check_value_assignable_to_expected(
                value,
                &value_key,
                &actual,
                &expected,
                ValueAssignmentContext {
                    annotation: None,
                    exact_expected: None,
                    diagnostic_context: "emit chunk",
                    fallback_span: self.expression_span(&value_key),
                },
            );
        }
        false
    }

    fn check_expr_stmt(&mut self, value: &Expr) -> bool {
        let ty = self.check_expr(value);
        ty.as_ref().is_some_and(|ty| type_ir_is_never(&ty.ir))
    }

    fn check_dispatch_expr(
        &mut self,
        key: &ExpressionKey,
        call: &Expr,
        timing: &Option<DispatchTiming>,
    ) -> Option<ResolvedTypeRef> {
        if self.db_transaction_depth > 0 {
            self.outputs.diagnostics.push(format!(
                "{}: dispatch is not allowed inside a db transaction at {}",
                self.module_path,
                self.expression_span_label(key)
            ));
        }
        let call_key = self.peek_key();
        let actual = self.check_expr(call);
        if let Some(actual) = actual {
            if !type_ir_is_void_or_null(&actual.ir) {
                self.outputs.diagnostics.push(format!(
                    "{}: dispatch target return type mismatch at {}: expected void/null, found {}",
                    self.module_path,
                    self.expression_span_label(&call_key),
                    actual
                ));
            }
        }
        if let Some(timing) = timing {
            let (timing_expr, expected, clause) = match timing {
                DispatchTiming::After(expr) => (expr, "Duration", "after"),
                DispatchTiming::At(expr) => (expr, "Instant", "at"),
            };
            let timing_key = self.peek_key();
            let timing_ty = self.check_expr(timing_expr);
            if let Some(timing_ty) = timing_ty {
                if !dispatch_timing_type_matches(&timing_ty, expected) {
                    self.outputs.diagnostics.push(format!(
                        "{}: dispatch {}(...) expects {}, found {} at {}",
                        self.module_path,
                        clause,
                        expected,
                        timing_ty,
                        self.expression_span_label(&timing_key)
                    ));
                }
            }
        }
        self.resolve_builtin("TaskRef")
    }

    fn check_rethrow_stmt(&mut self, exception: &Expr) -> bool {
        let key = self.peek_key();
        if let Some(actual) = self.check_expr(exception) {
            self.validate_rethrow_operand(&key, &actual);
        }
        true
    }

    fn check_return_stmt(&mut self, value: Option<&Expr>) -> bool {
        if let Some(value) = value {
            self.check_return_value(value);
        }
        true
    }

    fn check_value_block_expr(
        &mut self,
        root_key: &ExpressionKey,
        value: &crate::shared::ast::ValueBlock,
    ) -> Option<ResolvedTypeRef> {
        let saved_env = self.env.clone();
        let saved_projected_env = self.contract_projection.binding_snapshot();
        let saved_path_refinements = self.path_refinements.clone();
        self.check_block(&value.body);
        let tail_key = self.peek_key();
        let ty = self.check_expr(&value.tail);
        let projected = self.contract_projection.expression_type(&tail_key).cloned();
        self.env = saved_env;
        self.contract_projection
            .restore_bindings(saved_projected_env);
        self.path_refinements = saved_path_refinements;
        self.transparent_value_targets
            .insert(root_key.clone(), tail_key);
        if let Some(projected) = projected {
            self.contract_projection
                .record_expression_type(root_key.clone(), projected);
        }
        ty
    }

    fn check_concurrent_block(
        &mut self,
        body: &Block,
        tail: Option<(&ExpressionKey, &Expr)>,
    ) -> Option<ResolvedTypeRef> {
        let saved_env = self.env.clone();
        let saved_projected_env = self.contract_projection.binding_snapshot();
        let saved_path_refinements = self.path_refinements.clone();
        let mut sibling_env = saved_env.clone();
        let mut sibling_projected_env = saved_projected_env.clone();

        for statement in &body.statements {
            self.env = sibling_env.clone();
            self.contract_projection
                .restore_bindings(sibling_projected_env.clone());
            self.path_refinements = saved_path_refinements.clone();
            self.check_stmt(statement);

            if let Stmt::LocalBinding {
                kind: LocalBindingKind::Final,
                name,
                ..
            } = statement
            {
                if let Some(ty) = self.env.get(name).cloned() {
                    sibling_env.insert(name.clone(), ty);
                }
                match self
                    .contract_projection
                    .binding_snapshot()
                    .get(name)
                    .cloned()
                {
                    Some(projected) => {
                        sibling_projected_env.insert(name.clone(), projected);
                    }
                    None => {
                        sibling_projected_env.remove(name);
                    }
                }
            }
        }

        let result = tail.map(|(root_key, tail)| {
            self.env = sibling_env;
            self.contract_projection
                .restore_bindings(sibling_projected_env);
            self.path_refinements = saved_path_refinements.clone();
            let tail_key = self.peek_key();
            let ty = self.check_expr(tail);
            let projected = self.contract_projection.expression_type(&tail_key).cloned();
            self.transparent_value_targets
                .insert(root_key.clone(), tail_key);
            if let Some(projected) = projected {
                self.contract_projection
                    .record_expression_type(root_key.clone(), projected);
            }
            ty
        });

        self.env = saved_env;
        self.contract_projection
            .restore_bindings(saved_projected_env);
        self.path_refinements = saved_path_refinements;
        result.flatten()
    }

    fn record_field_type(&self, ty: &ResolvedTypeRef, field: &str) -> Option<ResolvedTypeRef> {
        self.type_resolution
            .record_field_type(ty, field, &self.type_context)
            .or_else(|| record_field_type_from_ir(&ty.ir, field))
    }

    fn check_return_value(&mut self, value: &Expr) {
        let value_key = self.peek_key();
        let actual = self.check_expr(value);
        let actual = if self.return_type.is_none() {
            self.maybe_upgrade_map_literal_expression_type(&value_key, value, actual)
        } else {
            actual
        };
        if self.stream_chunk.is_some() {
            match actual.as_ref() {
                Some(actual) if type_ir_is_void_or_null(&actual.ir) => return,
                Some(actual) if stream_chunk_type(actual).is_none() => {
                    self.outputs.diagnostics.push(format!(
                        "{}: stream producer completion type mismatch at {}: expected null, found {}",
                        self.module_path,
                        self.expression_span_label(&value_key),
                        actual
                    ));
                    return;
                }
                None => return,
                Some(_) => {}
            }
        }
        let (Some(annotation), Some(actual)) = (self.return_type.clone(), actual) else {
            return;
        };
        let Ok(expected) = self
            .type_resolution
            .resolve_type_ref(&annotation, &self.type_context)
        else {
            return;
        };
        self.check_value_assignable_to_expected(
            value,
            &value_key,
            &actual,
            &expected,
            ValueAssignmentContext {
                annotation: Some(&annotation),
                exact_expected: None,
                diagnostic_context: "return",
                fallback_span: self.expression_span(&value_key),
            },
        );
    }

    fn check_array_literal_expr(
        &mut self,
        _key: &ExpressionKey,
        items: &[Expr],
    ) -> Option<ResolvedTypeRef> {
        let mut element_types = Vec::new();
        for item in items {
            if let Some(ty) = self.check_expr(item) {
                element_types.push(self.array_literal_element_candidate(ty));
            }
        }
        let element = if element_types.is_empty() {
            TypeRefIr::builtin(BuiltinShape::Unknown.name())
        } else {
            normalize_union(TypeRefIr::Union {
                items: element_types.into_iter().map(|ty| ty.ir).collect(),
            })
        };
        Some(array_type_from_ir(element))
    }

    fn array_literal_element_candidate(&self, ty: ResolvedTypeRef) -> ResolvedTypeRef {
        match &ty.ir {
            TypeRefIr::Literal {
                value: LiteralIr::String { .. },
            } => self
                .resolve_builtin(BuiltinShape::String.name())
                .unwrap_or(ty),
            TypeRefIr::Literal {
                value: LiteralIr::Bool { .. },
            } => self
                .resolve_builtin(BuiltinShape::Bool.name())
                .unwrap_or(ty),
            _ => ty,
        }
    }

    fn check_map_literal_expr(
        &mut self,
        key: &ExpressionKey,
        entries: &[crate::shared::ast::MapLiteralEntry],
    ) -> Option<ResolvedTypeRef> {
        let source_fact = self.expression_sources.fact(key);
        let mut source_fields = Vec::with_capacity(entries.len());
        let mut seen = BTreeSet::new();
        for (index, entry) in entries.iter().enumerate() {
            let value_key = self.peek_key();
            let actual = self.check_expr(&entry.value);
            if !seen.insert(entry.key.clone()) {
                self.outputs.diagnostics.push(format!(
                    "{}: duplicate map literal key `{}` at {}",
                    self.module_path,
                    entry.key,
                    self.expression_span_label(key)
                ));
            }
            source_fields.push(ObjectLiteralSourceField {
                name: entry.key.clone(),
                expression: value_key,
                actual,
                value_span: materialization::record_field_value_source_span(source_fact, index),
            });
        }
        self.outputs.object_materialization.sources.insert(
            key.clone(),
            ObjectLiteralSource {
                span: source_fact
                    .map(|fact| fact.span)
                    .unwrap_or_else(SourceSpan::synthetic),
                fields: source_fields.clone(),
                allow_targetless: true,
            },
        );
        let fields = source_fields
            .iter()
            .filter_map(|field| {
                field
                    .actual
                    .as_ref()
                    .map(|ty| (field.name.clone(), ty.ir.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        Some(ResolvedTypeRef::with_text(
            TypeRefIr::Record { fields },
            "{}".to_string(),
        ))
    }

    fn map_literal_value_candidate(&self, ty: ResolvedTypeRef) -> ResolvedTypeRef {
        match &ty.ir {
            TypeRefIr::Literal {
                value: LiteralIr::String { .. },
            } => self
                .resolve_builtin(BuiltinShape::String.name())
                .unwrap_or(ty),
            TypeRefIr::Literal {
                value: LiteralIr::Bool { .. },
            } => self
                .resolve_builtin(BuiltinShape::Bool.name())
                .unwrap_or(ty),
            _ => ty,
        }
    }

    fn maybe_upgrade_map_literal_expression_type(
        &mut self,
        value_key: &ExpressionKey,
        value: &Expr,
        actual: Option<ResolvedTypeRef>,
    ) -> Option<ResolvedTypeRef> {
        let actual = actual?;
        let Expr::MapLiteral { entries } = value else {
            return Some(actual);
        };
        let TypeRefIr::Record { fields } = &actual.ir else {
            return Some(actual);
        };
        let mut value_types = Vec::new();
        for entry in entries {
            if let Some(ty) = fields.get(&entry.key) {
                value_types.push(self.map_literal_value_candidate(ResolvedTypeRef::new(ty.clone())));
            }
        }
        let value = if value_types.is_empty() {
            TypeRefIr::builtin(BuiltinShape::Unknown.name())
        } else {
            normalize_union(TypeRefIr::Union {
                items: value_types.into_iter().map(|ty| ty.ir).collect(),
            })
        };
        let key_ty = self
            .resolve_builtin(BuiltinShape::String.name())
            .map(|ty| ty.ir)
            .unwrap_or_else(|| TypeRefIr::builtin(BuiltinShape::String.name()));
        let map_ty = map_type_from_ir(key_ty, value);
        if let Some(fact) = self.outputs.facts.get_mut(value_key) {
            fact.ty = Some(map_ty.clone());
        }
        Some(map_ty)
    }

    fn check_expr(&mut self, expr: &Expr) -> Option<ResolvedTypeRef> {
        self.check_expr_with_field_diagnostics(expr, true, None)
    }

    fn check_callee_expr(&mut self, expr: &Expr) -> Option<ResolvedTypeRef> {
        self.check_expr_with_field_diagnostics(expr, false, None)
    }

    fn check_expr_with_field_diagnostics(
        &mut self,
        expr: &Expr,
        diagnose_unknown_field: bool,
        db_predicate_fields: Option<&BTreeMap<String, ResolvedTypeRef>>,
    ) -> Option<ResolvedTypeRef> {
        self.check_expr_with_index_context(
            expr,
            diagnose_unknown_field,
            db_predicate_fields,
            IndexExpressionContext::TerminalRead,
        )
    }

    fn check_expr_with_index_context(
        &mut self,
        expr: &Expr,
        diagnose_unknown_field: bool,
        db_predicate_fields: Option<&BTreeMap<String, ResolvedTypeRef>>,
        index_context: IndexExpressionContext,
    ) -> Option<ResolvedTypeRef> {
        let key = self.next_key();
        let refined_ty = expr_path(expr).and_then(|path| self.path_refinements.get(&path).cloned());
        let package_constant = expr_path(expr).and_then(|path| {
            self.dependency_analysis
                .and_then(|dependencies| dependencies.package_constant_by_source_path(&path))
                .map(|(_, _, constant)| constant.ty().clone())
        });
        let ty = if let Some(package_constant) = package_constant {
            self.consume_static_package_value_descendants(expr);
            self.contract_projection
                .record_expression_type(key.clone(), package_constant.clone());
            Some(resolved_package_type_ref(&package_constant))
        } else {
            match expr {
                Expr::Literal(literal) => self.literal_type(literal),
                Expr::Identifier(name) => {
                    refined_ty.clone().or_else(|| self.env.get(name).cloned())
                }
                Expr::DependencySourceAddress(source) => {
                    self.check_dependency_source_address_expr(&key, source, diagnose_unknown_field)
                }
                Expr::Binary { op, left, right } => {
                    self.check_binary_expr(&key, *op, left, right, db_predicate_fields)
                }
                Expr::Unary { op, expr } => {
                    self.check_unary_expr(&key, *op, expr, db_predicate_fields)
                }
                Expr::Ternary {
                    condition,
                    then_expr,
                    else_expr,
                } => self.check_ternary_expr(&key, condition, then_expr, else_expr),
                Expr::Call { callee, args } => self.check_call_expr(&key, callee, args),
                Expr::Generic { callee, .. } => self.check_expr_with_index_context(
                    callee,
                    diagnose_unknown_field,
                    None,
                    IndexExpressionContext::Intermediate,
                ),
                Expr::InterfaceBox { value, interface } => {
                    self.check_interface_box_expr(&key, value, interface)
                }
                Expr::Field { object, field } => self.check_field_expr(
                    &key,
                    object,
                    field,
                    diagnose_unknown_field,
                    index_context,
                ),
                Expr::Index { object, index } => {
                    self.check_index_expr(&key, object, index, index_context)
                }
                Expr::Record {
                    type_name,
                    type_args,
                    fields,
                } => self.check_record_expr(&key, type_name, type_args, fields),
                Expr::ObjectLiteral { entries } => self.check_object_literal_expr(&key, entries),
                Expr::MapLiteral { entries } => self.check_map_literal_expr(&key, entries),
                Expr::ArrayLiteral { items } => self.check_array_literal_expr(&key, items),
                Expr::Patch { operations, .. } => self.check_patch_expr(operations),
                Expr::ValueBlock(value) => self.check_value_block_expr(&key, value),
                Expr::ConcurrentValue(value) => self.check_concurrent_value_expr(&key, value),
                Expr::Timeout { value, .. } => self.check_timeout_expr(&key, value),
                Expr::Throw { value } => self.check_throw_expr(&key, value),
                Expr::Rethrow { exception } => self.check_rethrow_expr(&key, exception),
                Expr::Catch {
                    catch_type,
                    try_expr,
                } => self.check_catch_expr(&key, catch_type, try_expr),
                Expr::DbOperation(operation) => self.check_db_operation_expr(operation),
                Expr::DbQuery(query) => self.check_db_query_expr(query),
                Expr::DbTransaction(transaction) => self.check_db_transaction_expr(transaction),
                Expr::DbLeaseClaim(claim) => self.check_db_lease_claim_expr(claim),
                Expr::DbLeaseRead(read) => self.check_db_lease_read_expr(read),
                Expr::Dispatch { call, timing } => self.check_dispatch_expr(&key, call, timing),
            }
        };
        let ty = refined_ty.clone().or(ty);
        if let (Some(refined_ty), Some(dependency_analysis)) =
            (refined_ty.as_ref(), self.dependency_analysis)
        {
            match ContractProjectionState::project_resolved_type(
                refined_ty,
                self.type_resolution,
                dependency_analysis,
                &self.type_context,
            ) {
                Ok(projected) => self
                    .contract_projection
                    .record_expression_type(key.clone(), projected),
                Err(error) => self.outputs.diagnostics.push(format!(
                    "{}: refined expression exact type projection failed at {}: {error}",
                    self.module_path,
                    self.expression_span_label(&key)
                )),
            }
        }
        if let Expr::Identifier(name) = expr {
            self.contract_projection.inherit_identifier(&key, name);
        }
        if self.contract_projection.expression_type(&key).is_none() {
            if let (Some(ty), Some(dependency_analysis)) = (&ty, self.dependency_analysis) {
                match ContractProjectionState::project_resolved_type(
                    ty,
                    self.type_resolution,
                    dependency_analysis,
                    &self.type_context,
                ) {
                    Ok(projected) => self
                        .contract_projection
                        .record_expression_type(key.clone(), projected),
                    Err(error) => self.outputs.diagnostics.push(format!(
                        "{}: derived expression exact type projection failed at {}: {error}",
                        self.module_path,
                        self.expression_span_label(&key)
                    )),
                }
            }
        }
        let span = self
            .expression_sources
            .fact(&key)
            .map(|fact| fact.span)
            .unwrap_or_else(SourceSpan::synthetic);
        self.outputs.facts.insert(
            key,
            ExpressionTypeFact {
                ty: ty.clone(),
                span,
                test_effect_throw_payload_type: None,
                stream_emit_target: None,
            },
        );
        ty
    }

    fn check_dependency_source_address_expr(
        &mut self,
        key: &ExpressionKey,
        source: &crate::shared::ast::DependencySourceAddress,
        diagnose_unknown_field: bool,
    ) -> Option<ResolvedTypeRef> {
        if diagnose_unknown_field {
            let message = format!(
                        "{}: dependency source address `{}/{}` is not a value at {}; use `{}/{} as I` to box a public instance or call an exported callable",
                        self.module_path,
                        source.dependency_ref,
                        source.public_path,
                        self.expression_span_label(key),
                        source.dependency_ref,
                        source.public_path
                    );
            self.outputs.diagnostics.push(message);
        }
        None
    }

    fn check_binary_expr(
        &mut self,
        key: &ExpressionKey,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        db_predicate_fields: Option<&BTreeMap<String, ResolvedTypeRef>>,
    ) -> Option<ResolvedTypeRef> {
        let db_relational = db_predicate_fields.is_some()
            && matches!(
                op,
                BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
            );
        let db_logical =
            db_predicate_fields.is_some() && matches!(op, BinaryOp::And | BinaryOp::Or);
        let db_field_relational = db_relational
            && db_predicate_fields.is_some_and(|fields| Self::is_db_field_operand(left, fields));
        let left_ty = if db_field_relational {
            self.check_db_field_operand(left, db_predicate_fields.expect("checked above"))
        } else if db_logical {
            self.check_db_predicate_expr(left, db_predicate_fields.expect("checked above"))
        } else {
            self.check_expr(left)
        };
        let right_ty = if db_logical {
            self.check_db_predicate_expr(right, db_predicate_fields.expect("checked above"))
        } else {
            match op {
                BinaryOp::And => {
                    let narrowing = self.condition_narrowings(left).when_true;
                    self.check_expr_scoped(right, &narrowing)
                }
                BinaryOp::Or => {
                    let narrowing = self.condition_narrowings(left).when_false;
                    self.check_expr_scoped(right, &narrowing)
                }
                _ => self.check_expr(right),
            }
        };
        self.check_binary_operands(
            key,
            op,
            left_ty.as_ref(),
            right_ty.as_ref(),
            db_field_relational,
        );
        self.binary_type(op, left_ty.as_ref(), right_ty.as_ref())
    }

    fn check_unary_expr(
        &mut self,
        key: &ExpressionKey,
        op: UnaryOp,
        expr: &Expr,
        db_predicate_fields: Option<&BTreeMap<String, ResolvedTypeRef>>,
    ) -> Option<ResolvedTypeRef> {
        let operand_ty = match db_predicate_fields {
            Some(fields) if matches!(op, UnaryOp::Not) => {
                self.check_db_predicate_expr(expr, fields)
            }
            _ => self.check_expr(expr),
        };
        self.check_unary_operand(key, op, operand_ty.as_ref());
        self.unary_type(op)
    }

    fn check_call_expr(
        &mut self,
        key: &ExpressionKey,
        callee: &Expr,
        args: &[CallArg],
    ) -> Option<ResolvedTypeRef> {
        self.check_callee_expr(callee);
        let arg_types = args
            .iter()
            .map(|arg| {
                let key = self.peek_key();
                let ty = match arg {
                    CallArg::Value(expr) => self.check_expr(expr),
                    CallArg::InOutPlace { expr } => self.check_expr_with_index_context(
                        expr,
                        true,
                        None,
                        IndexExpressionContext::LoanTerminal,
                    ),
                };
                (key, ty)
            })
            .collect::<Vec<_>>();
        let result = self.call_type(key, callee, args, &arg_types);
        // Passing a path as an inout argument invalidates narrowing for that
        // path and its subpaths (R-196), exactly like an assignment write.
        for arg in args {
            if let CallArg::InOutPlace { expr } = arg {
                self.invalidate_path_refinements_for_write(expr);
            }
        }
        result
    }

    fn check_interface_box_expr(
        &mut self,
        key: &ExpressionKey,
        value: &Expr,
        interface: &TypeRef,
    ) -> Option<ResolvedTypeRef> {
        let value_ty = self.check_expr(value);
        let selector = match self
            .type_resolution
            .resolve_object_safe_interface_selector_type_ref(interface, &self.type_context)
        {
            Ok(selector) => selector,
            Err(error) => {
                self.outputs.diagnostics.push(format!(
                    "{}: interface boxing selector `{}` failed at {}: {error}",
                    self.module_path,
                    interface.name,
                    self.expression_span_label(key)
                ));
                return None;
            }
        };
        let value_ty = value_ty?;
        let Some(receiver) = self
            .type_resolution
            .concrete_nominal_record_symbol(&value_ty, &self.type_context)
        else {
            self.outputs.diagnostics.push(format!(
                "{}: interface boxing source at {} must be a concrete nominal record, found {}",
                self.module_path,
                self.expression_span_label(key),
                value_ty
            ));
            return None;
        };
        let expected_interface = ResolvedTypeRef::with_text(
            TypeRefIr::AnyInterface {
                interface: selector.instantiation_ref.clone(),
            },
            selector.source_text.clone(),
        );
        match self.type_resolution.concrete_type_conforms_to_interface(
            &value_ty,
            &expected_interface,
            &self.type_context,
        ) {
            Ok(Some(_)) => Some(ResolvedTypeRef::with_text(
                TypeRefIr::AnyInterface {
                    interface: selector.instantiation_ref,
                },
                format!("any {}", selector.source_text),
            )),
            Ok(None) => {
                self.outputs.diagnostics.push(format!(
                    "{}: type {} does not explicitly implement interface {} for boxing at {}",
                    self.module_path,
                    receiver,
                    selector.source_text,
                    self.expression_span_label(key)
                ));
                None
            }
            Err(error) => {
                self.outputs.diagnostics.push(format!(
                    "{}: interface boxing conformance check failed at {}: {error}",
                    self.module_path,
                    self.expression_span_label(key)
                ));
                None
            }
        }
    }

    fn check_field_expr(
        &mut self,
        key: &ExpressionKey,
        object: &Expr,
        field: &str,
        diagnose_unknown_field: bool,
        _index_context: IndexExpressionContext,
    ) -> Option<ResolvedTypeRef> {
        let object_key = self.peek_key();
        let object_ty = self.check_expr_with_index_context(
            object,
            diagnose_unknown_field,
            None,
            IndexExpressionContext::Intermediate,
        );
        object_ty.and_then(|object_ty| {
            let field_ty = if matches!(object, Expr::Identifier(name) if name == "self")
                && self
                    .type_resolution
                    .actor_type_resolution(&object_ty, &self.type_context)
                    .is_some()
            {
                self.type_resolution
                    .actor_state_field_type(&object_ty, field, &self.type_context)
            } else {
                self.record_field_type(&object_ty, field)
            };
            if let (
                Some(dependency_analysis),
                Some(PackageTypeRef::PackageSchema {
                    package_id,
                    stable_schema_key,
                    package_schema_type_id,
                }),
            ) = (
                self.dependency_analysis,
                self.contract_projection.expression_type(&object_key),
            ) {
                if let Some(record) = dependency_analysis.exact_package_type(
                    package_id,
                    stable_schema_key,
                    package_schema_type_id,
                ) {
                    if let skiff_artifact_model::ContractTypeDescriptor::Record { fields } =
                        &record.canonical_descriptor.descriptor
                    {
                        if let Some(field_type) = fields.get(field) {
                            self.contract_projection.record_expression_type(
                                key.clone(),
                                contract_call_typing::package_type_ref_from_contract_type(
                                    field_type,
                                ),
                            );
                        }
                    }
                }
            }
            if diagnose_unknown_field && field_ty.is_none() {
                self.outputs.diagnostics.push(format!(
                    "{}: unknown field `{field}` on {} at {}",
                    self.module_path,
                    object_ty,
                    self.expression_span_label(key)
                ));
            }
            field_ty
        })
    }

    fn check_index_expr(
        &mut self,
        key: &ExpressionKey,
        object: &Expr,
        index: &Expr,
        context: IndexExpressionContext,
    ) -> Option<ResolvedTypeRef> {
        let object_key = self.peek_key();
        let object_ty = self.check_expr_with_index_context(
            object,
            true,
            None,
            IndexExpressionContext::Intermediate,
        );
        let selector_key = self.peek_key();
        let selector_ty = self.check_expr(index);
        let object_ty = object_ty?;
        let exact = match indexing::exact_index_receiver(
            self.type_resolution,
            &object_ty,
            &self.type_context,
        ) {
            Ok(exact) => exact,
            Err(error) => {
                self.outputs.diagnostics.push(format!(
                    "{}: invalid bracket receiver at {}: {error}",
                    self.module_path,
                    self.expression_span_label(key)
                ));
                return None;
            }
        };
        let selector_ty = selector_ty?;
        if !indexing::selector_has_exact_type(
            self.type_resolution,
            &selector_ty,
            &exact.selector_type,
            &self.type_context,
        ) {
            self.outputs.diagnostics.push(format!(
                "{}: bracket selector type mismatch at {}: expected {}, found {}",
                self.module_path,
                self.expression_span_label(&selector_key),
                debug_text(&exact.selector_type),
                selector_ty
            ));
            return None;
        }
        let policy = match context {
            IndexExpressionContext::TerminalRead => SourceIndexPolicy::StrictRead,
            IndexExpressionContext::Intermediate => SourceIndexPolicy::IntermediateMustExist,
            IndexExpressionContext::AssignmentTerminal => match exact.kind {
                SourceIndexReceiverKind::Array => SourceIndexPolicy::TerminalReplace,
                SourceIndexReceiverKind::Map | SourceIndexReceiverKind::JsonObject => {
                    SourceIndexPolicy::TerminalUpsert
                }
            },
            IndexExpressionContext::LoanTerminal => SourceIndexPolicy::LoanMustExist,
        };
        self.outputs.index_segments.insert(
            key.clone(),
            SourceIndexSegmentFact {
                receiver_kind: exact.kind,
                receiver_type: exact.receiver_type,
                selector_type: exact.selector_type,
                result_type: exact.result_type.clone(),
                policy,
                object_expression: object_key,
                selector_expression: selector_key,
                source_span: self.expression_span(key),
            },
        );
        Some(resolved_type_from_ir(&exact.result_type))
    }

    fn check_patch_expr(
        &mut self,
        operations: &[crate::shared::ast::PatchOperation],
    ) -> Option<ResolvedTypeRef> {
        for operation in operations {
            match operation {
                crate::shared::ast::PatchOperation::Set { value, .. }
                | crate::shared::ast::PatchOperation::Inc { value, .. } => {
                    self.check_expr(value);
                }
            }
        }
        None
    }

    fn check_concurrent_value_expr(
        &mut self,
        key: &ExpressionKey,
        value: &crate::shared::ast::ValueBlock,
    ) -> Option<ResolvedTypeRef> {
        self.check_concurrent_block(&value.body, Some((key, &value.tail)))
    }

    fn check_timeout_expr(&mut self, key: &ExpressionKey, value: &Expr) -> Option<ResolvedTypeRef> {
        let value_key = self.peek_key();
        let ty = self.check_expr(value);
        self.transparent_value_targets
            .insert(key.clone(), value_key.clone());
        if let Some(projected) = self
            .contract_projection
            .expression_type(&value_key)
            .cloned()
        {
            self.contract_projection
                .record_expression_type(key.clone(), projected);
        }
        ty
    }

    fn check_throw_expr(&mut self, key: &ExpressionKey, value: &Expr) -> Option<ResolvedTypeRef> {
        if let Some(actual) = self.check_expr(value) {
            self.validate_throw_payload(key, &actual, "throw expression");
        }
        None
    }

    fn check_rethrow_expr(
        &mut self,
        key: &ExpressionKey,
        exception: &Expr,
    ) -> Option<ResolvedTypeRef> {
        if let Some(actual) = self.check_expr(exception) {
            self.validate_rethrow_operand(key, &actual);
        }
        None
    }

    fn check_catch_expr(
        &mut self,
        key: &ExpressionKey,
        catch_type: &TypeRef,
        try_expr: &Expr,
    ) -> Option<ResolvedTypeRef> {
        let try_ty = self.check_expr(try_expr)?;
        let catch_ty = match self
            .type_resolution
            .resolve_type_ref(catch_type, &self.type_context)
        {
            Ok(catch_ty) => catch_ty,
            Err(error) => {
                self.outputs.diagnostics.push(format!(
                    "{}: catch type cannot be resolved at {}: {error}",
                    self.module_path,
                    self.expression_span_label(key)
                ));
                return None;
            }
        };
        if let Err(error) = self
            .type_resolution
            .catch_leaves(&catch_ty, &self.type_context)
        {
            self.outputs.diagnostics.push(format!(
                "{}: invalid catch type `{}` at {}: {error}",
                self.module_path,
                catch_ty,
                self.expression_span_label(key)
            ));
        }
        Some(catch_result_type(try_ty, catch_ty))
    }

    fn check_ternary_expr(
        &mut self,
        key: &ExpressionKey,
        condition: &Expr,
        then_expr: &Expr,
        else_expr: &Expr,
    ) -> Option<ResolvedTypeRef> {
        let narrowings = self.condition_narrowings(condition);
        self.check_condition(condition, "ternary condition");
        let then_ty = self.check_expr_scoped(then_expr, &narrowings.when_true);
        let else_ty = self.check_expr_scoped(else_expr, &narrowings.when_false);
        let Some(result) = self.ternary_join_type(then_ty.as_ref(), else_ty.as_ref()) else {
            if let (Some(then_ty), Some(else_ty)) = (then_ty.as_ref(), else_ty.as_ref()) {
                self.outputs.diagnostics.push(format!(
                    "{}: ternary branches have incompatible types at {}: `{}` and `{}`",
                    self.module_path,
                    self.expression_span_label(key),
                    then_ty,
                    else_ty,
                ));
            }
            return None;
        };
        Some(result)
    }

    fn ternary_join_type(
        &self,
        then_ty: Option<&ResolvedTypeRef>,
        else_ty: Option<&ResolvedTypeRef>,
    ) -> Option<ResolvedTypeRef> {
        let (then_ty, else_ty) = (then_ty?, else_ty?);
        if then_ty == else_ty {
            return Some(then_ty.clone());
        }
        if is_never_type(&then_ty.ir) {
            return Some(else_ty.clone());
        }
        if is_never_type(&else_ty.ir) {
            return Some(then_ty.clone());
        }
        if is_string_literal_type(&then_ty.ir) && is_string_literal_type(&else_ty.ir) {
            return self.resolve_builtin(BuiltinShape::String.name());
        }
        if self
            .type_resolution
            .assignable_in_context(then_ty, else_ty, &self.type_context)
        {
            return Some(else_ty.clone());
        }
        if self
            .type_resolution
            .assignable_in_context(else_ty, then_ty, &self.type_context)
        {
            return Some(then_ty.clone());
        }
        if is_null_type(&then_ty.ir) || is_null_type(&else_ty.ir) {
            let items = vec![then_ty.ir.clone(), else_ty.ir.clone()];
            return Some(resolved_type_from_ir(&normalize_union(TypeRefIr::Union {
                items,
            })));
        }
        None
    }

    fn record_stream_emit_target(&mut self, key: &ExpressionKey, target: ResolvedTypeRef) {
        let Some(fact) = self.outputs.facts.get_mut(key) else {
            self.outputs.diagnostics.push(format!(
                "{}: emit target fact could not be recorded at {}",
                self.module_path,
                self.expression_span_label(key)
            ));
            return;
        };
        fact.stream_emit_target = Some(target);
    }

    fn next_key(&mut self) -> ExpressionKey {
        let key = self.peek_key();
        self.next_index += 1;
        key
    }

    fn consume_static_package_value_descendants(&mut self, expr: &Expr) {
        match expr {
            Expr::Field { object, .. } => {
                self.next_key();
                self.consume_static_package_value_descendants(object);
            }
            Expr::Index { .. } => {}
            Expr::Generic { callee, .. } => {
                self.next_key();
                self.consume_static_package_value_descendants(callee);
            }
            Expr::Timeout { value, .. } => {
                self.next_key();
                self.consume_static_package_value_descendants(value);
            }
            Expr::ValueBlock(_) | Expr::ConcurrentValue(_) => {}
            Expr::Literal(_)
            | Expr::Identifier(_)
            | Expr::DependencySourceAddress(_)
            | Expr::Binary { .. }
            | Expr::Unary { .. }
            | Expr::Ternary { .. }
            | Expr::Call { .. }
            | Expr::Record { .. }
            | Expr::ObjectLiteral { .. }
            | Expr::MapLiteral { .. }
            | Expr::ArrayLiteral { .. }
            | Expr::Patch { .. }
            | Expr::InterfaceBox { .. }
            | Expr::Throw { .. }
            | Expr::Rethrow { .. }
            | Expr::Catch { .. }
            | Expr::DbOperation(_)
            | Expr::DbQuery(_)
            | Expr::DbTransaction(_)
            | Expr::DbLeaseClaim(_)
            | Expr::DbLeaseRead(_)
            | Expr::Dispatch { .. } => {}
        }
    }

    fn peek_key(&self) -> ExpressionKey {
        ExpressionKey::new(
            self.module_path.to_string(),
            self.owner.clone(),
            self.next_index,
        )
    }

    fn current_key_index(&self) -> u32 {
        self.next_index.saturating_sub(1)
    }

    fn current_expression_span_label(&self) -> String {
        let key = ExpressionKey::new(
            self.module_path.to_string(),
            self.owner.clone(),
            self.current_key_index(),
        );
        self.expression_span_label(&key)
    }

    fn expression_span_label(&self, key: &ExpressionKey) -> String {
        span_label(self.expression_span(key))
    }

    fn expression_span(&self, key: &ExpressionKey) -> SourceSpan {
        self.expression_sources
            .fact(key)
            .map(|fact| fact.span)
            .unwrap_or_else(SourceSpan::synthetic)
    }

    fn resolve_builtin(&self, name: &str) -> Option<ResolvedTypeRef> {
        self.type_resolution
            .resolve_type_text(name, &self.type_context)
            .ok()
    }

    fn literal_type(&self, literal: &Literal) -> Option<ResolvedTypeRef> {
        match literal {
            Literal::String(value) => Some(ResolvedTypeRef::with_text(
                TypeRefIr::Literal {
                    value: LiteralIr::String {
                        value: value.clone(),
                    },
                },
                serde_json::to_string(value).unwrap_or_else(|_| "\"<string>\"".to_string()),
            )),
            Literal::Number(number) if number.fract() == 0.0 => {
                self.resolve_builtin(BuiltinShape::Integer.name())
            }
            Literal::Number(_) => self.resolve_builtin(BuiltinShape::Number.name()),
            Literal::Bool(_) => self.resolve_builtin(BuiltinShape::Bool.name()),
            Literal::Null => Some(ResolvedTypeRef::with_text(
                TypeRefIr::Literal {
                    value: LiteralIr::Null,
                },
                "null".to_string(),
            )),
        }
    }

    fn expression_type_at_offset(
        &self,
        key: &ExpressionKey,
        offset: u32,
    ) -> Option<ResolvedTypeRef> {
        let preorder_index = key.preorder_index().checked_add(offset)?;
        let key = ExpressionKey::new(
            key.module_path().to_string(),
            key.owner().clone(),
            preorder_index,
        );
        self.outputs.facts.get(&key)?.ty.clone()
    }

    fn expression_projection_at_offset(
        &self,
        key: &ExpressionKey,
        offset: u32,
    ) -> Option<&PackageTypeRef> {
        let preorder_index = key.preorder_index().checked_add(offset)?;
        let key = ExpressionKey::new(
            key.module_path().to_string(),
            key.owner().clone(),
            preorder_index,
        );
        self.contract_projection.expression_type(&key)
    }
}

fn callable_signatures(
    parsed_sources: &[ParsedCompilerSource],
) -> BTreeMap<String, CallableSignature> {
    let mut signatures = BTreeMap::new();
    for parsed in parsed_sources {
        let module_path = parsed.source().module_path.clone();
        for function in &parsed.ast().function_signatures {
            insert_operation_signature(
                &mut signatures,
                &module_path,
                &function.name,
                function,
                &[],
            );
        }
        for function in &parsed.ast().functions {
            insert_function_signature(&mut signatures, &module_path, &function.name, function, &[]);
        }
        for implementation in &parsed.ast().impls {
            let inherited =
                crate::shared::type_syntax::generic_type_parameter_names(&implementation.target);
            for method in &implementation.methods {
                let declaration_name =
                    impl_method_declaration_name(&implementation.target, &method.name);
                insert_operation_signature(
                    &mut signatures,
                    &module_path,
                    &declaration_name,
                    method,
                    &inherited,
                );
                insert_operation_signature(
                    &mut signatures,
                    &module_path,
                    &method.name,
                    method,
                    &inherited,
                );
            }
            for method in &implementation.method_bodies {
                let declaration_name =
                    impl_method_declaration_name(&implementation.target, &method.name);
                insert_function_signature(
                    &mut signatures,
                    &module_path,
                    &declaration_name,
                    method,
                    &inherited,
                );
                insert_function_signature(
                    &mut signatures,
                    &module_path,
                    &method.name,
                    method,
                    &inherited,
                );
            }
        }
    }
    signatures
}

fn insert_callable_signature_from_parts(
    signatures: &mut BTreeMap<String, CallableSignature>,
    module_path: &str,
    declaration_name: &str,
    inherited_type_params: &[String],
    decl_type_params: &[String],
    params: &[Param],
    return_type: &TypeRef,
) {
    let signature = CallableSignature {
        module_path: module_path.to_string(),
        declaration_name: declaration_name.to_string(),
        params: params
            .iter()
            .map(|param| CallableParam {
                ty: param.ty.clone(),
            })
            .collect(),
        return_type: return_type.clone(),
        type_params: inherited_type_params
            .iter()
            .chain(decl_type_params)
            .cloned()
            .collect(),
    };
    insert_callable_signature(signatures, module_path, declaration_name, signature);
}

fn insert_operation_signature(
    signatures: &mut BTreeMap<String, CallableSignature>,
    module_path: &str,
    declaration_name: &str,
    operation: &crate::shared::ast::InterfaceOperation,
    inherited_type_params: &[String],
) {
    insert_callable_signature_from_parts(
        signatures,
        module_path,
        declaration_name,
        inherited_type_params,
        &operation.type_params,
        &operation.params,
        &operation.return_type,
    );
}

fn insert_function_signature(
    signatures: &mut BTreeMap<String, CallableSignature>,
    module_path: &str,
    declaration_name: &str,
    function: &FunctionDecl,
    inherited_type_params: &[String],
) {
    insert_callable_signature_from_parts(
        signatures,
        module_path,
        declaration_name,
        inherited_type_params,
        &function.type_params,
        &function.params,
        &function.return_type,
    );
}

fn insert_callable_signature(
    signatures: &mut BTreeMap<String, CallableSignature>,
    module_path: &str,
    declaration_name: &str,
    signature: CallableSignature,
) {
    signatures
        .entry(declaration_name.to_string())
        .or_insert(signature.clone());
    signatures
        .entry(format!("{module_path}.{declaration_name}"))
        .or_insert(signature);
}

fn single_for_item_type(ty: &ResolvedTypeRef) -> Option<ResolvedTypeRef> {
    single_item(&ty.ir).map(|item| ResolvedTypeRef::new(item.clone()))
}

fn stream_chunk_type(ty: &ResolvedTypeRef) -> Option<ResolvedTypeRef> {
    let TypeRefIr::Builtin { name, args } = &ty.ir else {
        return None;
    };
    matches!(BuiltinShape::of_name(name), Some(BuiltinShape::Stream))
        .then_some(args)
        .filter(|args| args.len() == 1)
        .map(|args| ResolvedTypeRef::new(args[0].clone()))
}

fn map_entry_types(ty: &ResolvedTypeRef) -> Option<(ResolvedTypeRef, ResolvedTypeRef)> {
    // Preserves the pre-existing short-name-only behavior of this wrapper:
    // unlike the other map helpers it does not accept `std.collection.Map`.
    let TypeRefIr::Builtin { name, args } = &ty.ir else {
        return None;
    };
    if name != BuiltinShape::Map.name() || args.len() != 2 {
        return None;
    }
    let (key, value) = map_entry(&ty.ir)?;
    Some((resolved_type_from_ir(key), resolved_type_from_ir(value)))
}

fn single_for_item_projection(ty: &PackageTypeRef) -> Option<PackageTypeRef> {
    let PackageTypeRef::Container { name, arguments } = ty else {
        return None;
    };
    single_item(&TypeRefIr::Builtin {
        name: name.clone(),
        args: arguments.iter().map(package_type_ref_to_ir).collect(),
    })
    .map(|_| arguments[0].clone())
}

fn map_entry_projections(ty: &PackageTypeRef) -> Option<(PackageTypeRef, PackageTypeRef)> {
    let PackageTypeRef::Container { name, arguments } = ty else {
        return None;
    };
    map_entry(&TypeRefIr::Builtin {
        name: name.clone(),
        args: arguments.iter().map(package_type_ref_to_ir).collect(),
    })
    .map(|_| (arguments[0].clone(), arguments[1].clone()))
}

fn catch_result_type(value: ResolvedTypeRef, error: ResolvedTypeRef) -> ResolvedTypeRef {
    let text = format!("CatchResult<{value}, {error}>");
    ResolvedTypeRef::with_text(
        TypeRefIr::Builtin {
            name: BuiltinShape::CatchResult.name().to_string(),
            args: vec![value.ir, error.ir],
        },
        text,
    )
}

fn record_field_type_from_ir(ty: &TypeRefIr, field: &str) -> Option<ResolvedTypeRef> {
    record_field_type(ty, field)
        .as_ref()
        .map(resolved_type_from_ir)
}

fn array_item_type_ir(ty: &TypeRefIr) -> Option<TypeRefIr> {
    let TypeRefIr::Builtin { name, args } = ty else {
        return None;
    };
    (matches!(
        BuiltinShape::of_name(name),
        Some(BuiltinShape::Array | BuiltinShape::Stream)
    ) && args.len() == 1)
        .then(|| args[0].clone())
}

fn map_value_type_ir(ty: &TypeRefIr) -> Option<TypeRefIr> {
    map_entry(ty).map(|(_, value)| value.clone())
}

fn map_key_type_ir(ty: &TypeRefIr) -> Option<TypeRefIr> {
    map_entry(ty).map(|(key, _)| key.clone())
}

fn resolved_type_from_ir(ty: &TypeRefIr) -> ResolvedTypeRef {
    ResolvedTypeRef::new(ty.clone())
}

fn array_type_from_ir(element: TypeRefIr) -> ResolvedTypeRef {
    let ty = TypeRefIr::Builtin {
        name: BuiltinShape::Array.name().to_string(),
        args: vec![element.clone()],
    };
    ResolvedTypeRef::with_text(ty.clone(), format!("Array<{}>", debug_text(&element)))
}

fn map_type_from_ir(key: TypeRefIr, value: TypeRefIr) -> ResolvedTypeRef {
    let ty = TypeRefIr::Builtin {
        name: BuiltinShape::Map.name().to_string(),
        args: vec![key.clone(), value.clone()],
    };
    ResolvedTypeRef::with_text(
        ty.clone(),
        format!("Map<{}, {}>", debug_text(&key), debug_text(&value)),
    )
}

fn nullable_type(inner: ResolvedTypeRef) -> ResolvedTypeRef {
    let text = format!("{inner}?");
    ResolvedTypeRef::with_text(
        TypeRefIr::Nullable {
            inner: Box::new(inner.ir),
        },
        text,
    )
}

fn is_never_type(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Builtin { name, .. } if name == BuiltinShape::Never.name()
    )
}

fn is_string_literal_type(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Literal {
            value: LiteralIr::String { .. }
        }
    )
}

fn object_literal_key_text(key: &crate::shared::ast::ObjectLiteralKey) -> Option<String> {
    match key {
        crate::shared::ast::ObjectLiteralKey::Name(name) => Some(name.clone()),
    }
}

fn transparent_value_target(expression: &Expr) -> &Expr {
    match expression {
        Expr::ValueBlock(value) | Expr::ConcurrentValue(value) => {
            transparent_value_target(&value.tail)
        }
        Expr::Timeout { value, .. } => transparent_value_target(value),
        _ => expression,
    }
}

fn object_literal_field_value<'a>(value: &'a Expr, name: &str) -> Option<&'a Expr> {
    match value {
        Expr::ObjectLiteral { entries } => entries.iter().find_map(|entry| {
            (object_literal_key_text(&entry.key).as_deref() == Some(name))
                .then_some(&entry.value)
        }),
        Expr::MapLiteral { entries } => entries.iter().find_map(|entry| {
            (entry.key.as_str() == name).then_some(&entry.value)
        }),
        _ => None,
    }
}

fn expr_is_null_literal(expr: &Expr) -> bool {
    matches!(expr, Expr::Literal(Literal::Null))
}

fn expr_string_literal(expr: &Expr) -> Option<&str> {
    let Expr::Literal(Literal::String(value)) = expr else {
        return None;
    };
    Some(value)
}

fn block_assigns_obviously_non_null(block: &Block, path: &str) -> bool {
    block.statements.iter().any(|stmt| match stmt {
        Stmt::Assign { target, value } => {
            expr_path(target).as_deref() == Some(path) && expr_obviously_non_null(value)
        }
        _ => false,
    })
}

fn expr_obviously_non_null(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(Literal::Null) => false,
        Expr::Literal(_)
        | Expr::Record { .. }
        | Expr::ObjectLiteral { .. }
        | Expr::MapLiteral { .. }
        | Expr::ArrayLiteral { .. } => true,
        Expr::Binary {
            op: BinaryOp::Add,
            left,
            right,
        } => {
            expr_string_literal(left).is_some()
                || expr_string_literal(right).is_some()
                || expr_obviously_non_null(left)
                    && expr_obviously_non_null(right)
                    && !matches!(left.as_ref(), Expr::Literal(Literal::Null))
                    && !matches!(right.as_ref(), Expr::Literal(Literal::Null))
        }
        _ => false,
    }
}

fn place_contains_index(expr: &Expr) -> bool {
    match expr {
        Expr::Index { .. } => true,
        Expr::Field { object, .. } | Expr::Generic { callee: object, .. } => {
            place_contains_index(object)
        }
        Expr::Literal(_)
        | Expr::Identifier(_)
        | Expr::DependencySourceAddress(_)
        | Expr::Binary { .. }
        | Expr::Unary { .. }
        | Expr::Ternary { .. }
        | Expr::Call { .. }
        | Expr::InterfaceBox { .. }
        | Expr::Record { .. }
        | Expr::ObjectLiteral { .. }
        | Expr::MapLiteral { .. }
        | Expr::ArrayLiteral { .. }
        | Expr::Patch { .. }
        | Expr::ValueBlock(_)
        | Expr::ConcurrentValue(_)
        | Expr::Timeout { .. }
        | Expr::Throw { .. }
        | Expr::Rethrow { .. }
        | Expr::Catch { .. }
        | Expr::DbOperation(_)
        | Expr::DbQuery(_)
        | Expr::DbTransaction(_)
        | Expr::DbLeaseClaim(_)
        | Expr::DbLeaseRead(_)
        | Expr::Dispatch { .. } => false,
    }
}

fn resolved_package_type_ref(ty: &PackageTypeRef) -> ResolvedTypeRef {
    let ir = package_type_ref_ir(ty);
    ResolvedTypeRef::new(ir)
}

fn package_type_ref_ir(ty: &PackageTypeRef) -> TypeRefIr {
    // Core folded projection keeps `Local` internals verbatim; the only
    // remaining `PackageSchema` nodes in its output are inside `Local`
    // subtrees, so the preserved `ordinary_package_local_type_ir` rewrite
    // reproduces the historical etm behavior (Local-internal PackageSchema
    // collapsed to PackageSymbol) without reimplementing the projection.
    ordinary_package_local_type_ir(&package_type_ref_to_ir(ty))
}

fn ordinary_package_local_type_ir(ty: &TypeRefIr) -> TypeRefIr {
    let recurse = ordinary_package_local_type_ir;
    match ty {
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } => TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: package_id.clone(),
                },
                symbol_path: stable_schema_key.clone(),
                abi_expectation: None,
            },
        },
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name: name.clone(),
            args: args.iter().map(recurse).collect(),
        },
        TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
            base: base.clone(),
            arguments: arguments.iter().map(recurse).collect(),
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| (name.clone(), recurse(ty)))
                .collect(),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items.iter().map(recurse).collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(recurse(inner)),
        },
        TypeRefIr::AnyInterface { interface } => {
            let interface_abi_id = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
                .map(|identity| recurse(&identity))
                .map(|identity| type_ref_abi_key(&identity))
                .unwrap_or_else(|_| interface.interface_abi_id.clone());
            TypeRefIr::AnyInterface {
                interface: skiff_artifact_model::InterfaceInstantiationRef {
                    interface_abi_id,
                    canonical_type_args: interface
                        .canonical_type_args
                        .iter()
                        .map(recurse)
                        .collect(),
                },
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|parameter| skiff_artifact_model::FunctionTypeParamIr {
                    name: parameter.name.clone(),
                    ty: recurse(&parameter.ty),
                })
                .collect(),
            return_type: Box::new(recurse(return_type)),
        },
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => ty.clone(),
    }
}

fn builtin_type(name: &str) -> TypeRefIr {
    TypeRefIr::Builtin {
        name: name.to_string(),
        args: Vec::new(),
    }
}

fn type_ir_is_void_or_null(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Builtin { name, args } if args.is_empty() && matches!(BuiltinShape::of_name(name), Some(BuiltinShape::Void | BuiltinShape::Null)))
        || is_null_type(ty)
}

fn type_ir_is_never(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Builtin { name, args } if args.is_empty() && matches!(BuiltinShape::of_name(name), Some(BuiltinShape::Never)))
}

/// `after(...)` requires `Duration` (`std.time.Duration`); `at(...)` requires
/// `Instant` (`std.time.Instant`). The std `Instant` type is not defined yet,
/// so a bare user/package type spelled `Instant` is accepted as well; the
/// canonical spelling is restored when `std.time.Instant` lands.
fn dispatch_timing_type_matches(ty: &ResolvedTypeRef, expected: &str) -> bool {
    let canonical = format!("std.time.{expected}");
    let matches_name = |name: &str| {
        let name = name.trim();
        name == expected || name == canonical
    };
    if let Some(text) = &ty.source_text {
        if matches_name(text) {
            return true;
        }
    }
    match &ty.ir {
        TypeRefIr::Builtin { name, args } if args.is_empty() => {
            matches_name(name)
                || prelude_registry().known_type_symbol(name).as_deref() == Some(canonical.as_str())
        }
        other => {
            let text = debug_text(other);
            matches_name(&text) || text == canonical
        }
    }
}

#[cfg(test)]
mod tests;

fn span_label(span: SourceSpan) -> String {
    format!("{}:{}", span.start.line, span.start.column)
}
