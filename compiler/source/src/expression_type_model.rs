use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    builtin_receiver_op_spec_by_name, BuiltinReceiverPublicReturnType, LiteralIr, PackageRefIr,
    PackageSymbolRef, PackageTypeRef, TypeRefIr,
};
use skiff_compiler_core::type_ref::substitute_type_params_in_type_ref_ref as substitute_type_params_in_ir;

use crate::{
    contract_type_resolution::substitute_package_type,
    parsed_sources::ParsedCompilerSource,
    semantic::impl_method_declaration_name,
    shared::ast::{
        BinaryOp, Block, DbBlockMode, DbBody, DbChangeOp, DbQueryBlock, DbSelector, DbWhereClause,
        Expr, ForBinding, FunctionDecl, Literal, SourceFile, Stmt, TypeRef, UnaryOp,
    },
    shared::ast_utils::expr_path,
    shared::error::SourceSpan,
    shared::prelude_registry::prelude_registry,
    shared::type_expr::TypeExpr,
};

use super::{
    ExpressionKey, ExpressionOwnerKey, ExpressionSourceMap, PublicationDbMetadataIndex,
    ResolvedTypeRef, SourceDependencyAnalysisInput, TypeResolutionContext, TypeResolutionModel,
};

mod contract_call_typing;
mod db_projection;
mod expression_assignability;
mod object_materialization;

use contract_call_typing::{
    contract_source_assignability_with_projections, ContractCallOutcome, ContractCallTyping,
    ContractProjectionState,
};
use db_projection::DbProjectionTypeResolver;
use expression_assignability::{record_type_fields, ExpressionAssignability};
pub use object_materialization::{
    MaterializedObjectField, ObjectFieldValueSource, ObjectMaterializationKind,
    TargetTypedObjectMaterialization,
};
use object_materialization::{
    ObjectLiteralSource, ObjectLiteralSourceField, ObjectMaterializationState,
};

#[derive(Clone, Debug, Default)]
pub struct ExpressionTypeModel {
    facts: BTreeMap<ExpressionKey, ExpressionTypeFact>,
    constructor_validations: BTreeMap<ExpressionKey, ConstructorValidation>,
    representation_constructor_validations:
        BTreeMap<ExpressionKey, RepresentationConstructorValidation>,
    object_materializations: BTreeMap<ExpressionKey, TargetTypedObjectMaterialization>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ExpressionTypeModelBuildError {
    model: ExpressionTypeModel,
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

#[derive(Clone, Debug, Default)]
struct TypeNarrowing {
    env: BTreeMap<String, ResolvedTypeRef>,
    paths: BTreeMap<String, ResolvedTypeRef>,
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
    facts: &'a mut BTreeMap<ExpressionKey, ExpressionTypeFact>,
    constructor_validations: &'a mut BTreeMap<ExpressionKey, ConstructorValidation>,
    representation_constructor_validations:
        &'a mut BTreeMap<ExpressionKey, RepresentationConstructorValidation>,
    object_materialization: &'a mut ObjectMaterializationState,
    diagnostics: &'a mut Vec<String>,
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
        let mut facts = BTreeMap::new();
        let mut constructor_validations = BTreeMap::new();
        let mut representation_constructor_validations = BTreeMap::new();
        let mut object_materialization = ObjectMaterializationState::default();
        let mut diagnostics = Vec::new();
        for parsed in parsed_sources {
            check_source(
                parsed.source().module_path.as_str(),
                parsed.ast(),
                expression_sources,
                type_resolution,
                publication_db_metadata,
                &callable_signatures,
                dependency_analysis,
                &mut facts,
                &mut constructor_validations,
                &mut representation_constructor_validations,
                &mut object_materialization,
                &mut diagnostics,
            );
        }

        for (key, source) in &object_materialization.sources {
            if object_materialization.targeted.contains(key) {
                continue;
            }
            diagnostics.push(format!(
                "{}: object literal at {} requires an explicit target type",
                key.module_path(),
                span_label(source.span)
            ));
        }

        let model = Self {
            facts,
            constructor_validations,
            representation_constructor_validations,
            object_materializations: object_materialization.facts,
        };
        if !diagnostics.is_empty() {
            return Err(ExpressionTypeModelBuildError { model, diagnostics });
        }

        Ok(model)
    }

    pub fn fact(&self, key: &ExpressionKey) -> Option<&ExpressionTypeFact> {
        self.facts.get(key)
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
    expression_sources: &ExpressionSourceMap,
    type_resolution: &TypeResolutionModel,
    publication_db_metadata: &PublicationDbMetadataIndex,
    callable_signatures: &BTreeMap<String, CallableSignature>,
    dependency_analysis: Option<&SourceDependencyAnalysisInput>,
    facts: &mut BTreeMap<ExpressionKey, ExpressionTypeFact>,
    constructor_validations: &mut BTreeMap<ExpressionKey, ConstructorValidation>,
    representation_constructor_validations: &mut BTreeMap<
        ExpressionKey,
        RepresentationConstructorValidation,
    >,
    object_materialization: &mut ObjectMaterializationState,
    diagnostics: &mut Vec<String>,
) {
    let const_env = const_type_env(
        ast,
        type_resolution,
        dependency_analysis,
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
            expression_sources,
            type_resolution,
            publication_db_metadata,
            callable_signatures,
            dependency_analysis,
            &const_env,
            facts,
            constructor_validations,
            representation_constructor_validations,
            object_materialization,
            diagnostics,
        );
    }

    for implementation in &ast.impls {
        let inherited = generic_type_params(&implementation.target);
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
                expression_sources,
                type_resolution,
                publication_db_metadata,
                callable_signatures,
                dependency_analysis,
                &const_env,
                facts,
                constructor_validations,
                representation_constructor_validations,
                object_materialization,
                diagnostics,
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
                diagnostics.push(format!(
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
            expression_sources,
            type_resolution,
            publication_db_metadata,
            callable_signatures,
            dependency_analysis,
            None,
            facts,
            constructor_validations,
            representation_constructor_validations,
            object_materialization,
            diagnostics,
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
            expression_sources,
            type_resolution,
            publication_db_metadata,
            callable_signatures,
            dependency_analysis,
            None,
            facts,
            constructor_validations,
            representation_constructor_validations,
            object_materialization,
            diagnostics,
        );
        checker.check_block(&test.body);
    }

    for db in &ast.dbs {
        for index in &db.indexes {
            if let Some(where_expr) = &index.where_expr {
                let type_context = TypeResolutionContext::source(module_path);
                let env = db_index_where_env(&db.name, type_resolution, &type_context);
                let mut checker = OwnerChecker::new(
                    module_path,
                    ExpressionOwnerKey::DbIndexWhere {
                        db: db.name.clone(),
                        index: index.name.clone(),
                    },
                    type_context,
                    env,
                    BTreeMap::new(),
                    expression_sources,
                    type_resolution,
                    publication_db_metadata,
                    callable_signatures,
                    dependency_analysis,
                    None,
                    facts,
                    constructor_validations,
                    representation_constructor_validations,
                    object_materialization,
                    diagnostics,
                );
                checker.check_condition(where_expr, "db index where condition");
            }
        }
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
    facts: &mut BTreeMap<ExpressionKey, ExpressionTypeFact>,
    constructor_validations: &mut BTreeMap<ExpressionKey, ConstructorValidation>,
    representation_constructor_validations: &mut BTreeMap<
        ExpressionKey,
        RepresentationConstructorValidation,
    >,
    object_materialization: &mut ObjectMaterializationState,
    diagnostics: &mut Vec<String>,
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
        facts,
        constructor_validations,
        representation_constructor_validations,
        object_materialization,
        diagnostics,
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
        facts: &'a mut BTreeMap<ExpressionKey, ExpressionTypeFact>,
        constructor_validations: &'a mut BTreeMap<ExpressionKey, ConstructorValidation>,
        representation_constructor_validations: &'a mut BTreeMap<
            ExpressionKey,
            RepresentationConstructorValidation,
        >,
        object_materialization: &'a mut ObjectMaterializationState,
        diagnostics: &'a mut Vec<String>,
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
        diagnostics.extend(
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
            facts,
            constructor_validations,
            representation_constructor_validations,
            object_materialization,
            diagnostics,
        }
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
            Stmt::Assert { condition, .. } => {
                let narrowings = self.condition_narrowings(condition);
                self.check_condition(condition, "condition");
                self.apply_narrowing(&narrowings.when_true);
                false
            }
            Stmt::Let {
                name, ty, value, ..
            } => {
                let value_key = self.peek_key();
                let actual = self.check_expr(value);
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
                                    self.diagnostics.push(format!(
                                        "{}: local binding `{name}` annotation exact source type projection failed: {error}",
                                        self.module_path
                                    ));
                                    (None, true)
                                }
                            };
                            if !projection_failed {
                                if let Some(actual) = &actual {
                                    self.check_value_assignable_to_expected(
                                        Some(annotation),
                                        value,
                                        &value_key,
                                        actual,
                                        &expected,
                                        projected_expected.as_ref(),
                                        &format!("local binding {name} annotation"),
                                        self.expression_span(&value_key),
                                    );
                                }
                            }
                            (Some(expected), projected_expected)
                        }
                        Err(error) => {
                            self.diagnostics.push(format!(
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
            Stmt::Assign { target, value } => {
                let expected = self.check_expr(target);
                let value_key = self.peek_key();
                let actual = self.check_expr(value);
                if matches!(
                    target,
                    Expr::Field { object, .. }
                        if matches!(object.as_ref(), Expr::Identifier(name) if name == "self")
                ) {
                    if let (Some(actual), Some(expected)) = (actual.as_ref(), expected.as_ref()) {
                        self.check_value_assignable_to_expected(
                            None,
                            value,
                            &value_key,
                            actual,
                            expected,
                            None,
                            "self field assignment",
                            self.expression_span(&value_key),
                        );
                    }
                }
                if let (Expr::Identifier(name), Some(actual)) = (target, actual) {
                    self.env.insert(name.clone(), actual);
                    let projected = self
                        .contract_projection
                        .expression_type(&value_key)
                        .cloned();
                    self.contract_projection.bind(name, projected);
                }
                false
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                let narrowings = self.condition_narrowings(condition);
                self.check_condition(condition, "if condition");
                let then_exits = self.check_block_scoped(then_block, &narrowings.when_true);
                let else_exits = else_block.as_ref().is_some_and(|else_block| {
                    self.check_block_scoped(else_block, &narrowings.when_false)
                });
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
                        if let Some(narrowing) =
                            self.null_guard_assignment_narrowing(condition, then_block)
                        {
                            self.apply_narrowing(&narrowing);
                        }
                        false
                    }
                    _ => false,
                }
            }
            Stmt::For {
                binding,
                iterable,
                body,
            } => {
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
                                previous
                                    .push((item.clone(), self.env.insert(item.clone(), item_ty)));
                                previous_projected
                                    .push((item.clone(), saved_projected_env.get(item).cloned()));
                                self.contract_projection.bind(
                                    item,
                                    iterable_projection
                                        .as_ref()
                                        .and_then(single_for_item_projection),
                                );
                            }
                            None => self.diagnostics.push(format!(
                                "{}: for iterable must be Array, Stream, or Map at {}",
                                self.module_path,
                                self.expression_span_label(&iterable_key)
                            )),
                        }
                    }
                    ForBinding::Entry { key, value } => match iterable_ty
                        .as_ref()
                        .and_then(map_entry_types)
                    {
                        Some((key_ty, value_ty)) => {
                            previous.push((key.clone(), self.env.insert(key.clone(), key_ty)));
                            previous
                                .push((value.clone(), self.env.insert(value.clone(), value_ty)));
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
                        None => self.diagnostics.push(format!(
                            "{}: for entry binding requires Map at {}",
                            self.module_path,
                            self.expression_span_label(&iterable_key)
                        )),
                    },
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
            Stmt::Match { value, arms } => {
                self.check_expr(value);
                for arm in arms {
                    self.check_block(&arm.body);
                }
                false
            }
            Stmt::DbTransaction { body } => self.check_block(body),
            Stmt::Throw { value } => {
                self.check_expr(value);
                true
            }
            Stmt::Emit(value) => {
                let value_key = self.peek_key();
                let actual = self.check_expr(value);
                let Some(expected) = self.stream_chunk.clone() else {
                    return false;
                };
                self.record_stream_emit_target(&value_key, expected.clone());
                if let Some(actual) = actual {
                    self.check_value_assignable_to_expected(
                        None,
                        value,
                        &value_key,
                        &actual,
                        &expected,
                        None,
                        "emit chunk",
                        self.expression_span(&value_key),
                    );
                }
                false
            }
            Stmt::Expr(value) => {
                let ty = self.check_expr(value);
                ty.as_ref().is_some_and(|ty| type_ir_is_never(&ty.ir))
            }
            Stmt::Spawn { call } => {
                let call_key = self.peek_key();
                let actual = self.check_expr(call);
                if let Some(actual) = actual {
                    if !type_ir_is_void_or_null(&actual.ir) {
                        self.diagnostics.push(format!(
                            "{}: spawn target return type mismatch at {}: expected void/null, found {}",
                            self.module_path,
                            self.expression_span_label(&call_key),
                            actual.source_text
                        ));
                    }
                }
                false
            }
            Stmt::Rethrow { exception } => {
                self.check_expr(exception);
                true
            }
            Stmt::Return(value) => {
                if let Some(value) = value {
                    self.check_return_value(value);
                }
                true
            }
            Stmt::Break | Stmt::Continue => true,
        }
    }

    fn check_block_scoped(&mut self, block: &Block, narrowing: &TypeNarrowing) -> bool {
        let saved_env = self.env.clone();
        let saved_projected_env = self.contract_projection.binding_snapshot();
        let saved_path_refinements = self.path_refinements.clone();
        self.apply_narrowing(narrowing);
        let exits = self.check_block(block);
        self.env = saved_env;
        self.contract_projection
            .restore_bindings(saved_projected_env);
        self.path_refinements = saved_path_refinements;
        exits
    }

    fn check_expr_scoped(
        &mut self,
        expr: &Expr,
        narrowing: &TypeNarrowing,
    ) -> Option<ResolvedTypeRef> {
        let saved_env = self.env.clone();
        let saved_projected_env = self.contract_projection.binding_snapshot();
        let saved_path_refinements = self.path_refinements.clone();
        self.apply_narrowing(narrowing);
        let ty = self.check_expr(expr);
        self.env = saved_env;
        self.contract_projection
            .restore_bindings(saved_projected_env);
        self.path_refinements = saved_path_refinements;
        ty
    }

    fn apply_narrowing(&mut self, narrowing: &TypeNarrowing) {
        let projected_bindings = self.contract_projection.binding_snapshot();
        for (name, ty) in &narrowing.env {
            self.env.insert(name.clone(), ty.clone());
            let projected = match projected_bindings.get(name) {
                Some(PackageTypeRef::Nullable { inner })
                    if !matches!(ty.ir, TypeRefIr::Nullable { .. }) =>
                {
                    Some((**inner).clone())
                }
                _ => self
                    .dependency_analysis
                    .and_then(|dependency_analysis| {
                        ContractProjectionState::project_resolved_type(
                            ty,
                            self.type_resolution,
                            dependency_analysis,
                            &self.type_context,
                        )
                        .ok()
                    })
                    .or_else(|| projected_bindings.get(name).cloned()),
            };
            self.contract_projection.bind(name, projected);
        }
        for (path, ty) in &narrowing.paths {
            self.path_refinements.insert(path.clone(), ty.clone());
        }
    }

    fn condition_narrowings(&self, condition: &Expr) -> ConditionNarrowings {
        match condition {
            Expr::Unary {
                op: UnaryOp::Not,
                expr,
            } => {
                let nested = self.condition_narrowings(expr);
                ConditionNarrowings {
                    when_true: nested.when_false,
                    when_false: nested.when_true,
                }
            }
            Expr::Binary {
                op: BinaryOp::And,
                left,
                right,
            } => ConditionNarrowings {
                when_true: self
                    .condition_narrowings(left)
                    .when_true
                    .combined(self.condition_narrowings(right).when_true),
                when_false: TypeNarrowing::default(),
            },
            Expr::Binary {
                op: BinaryOp::Or,
                left,
                right,
            } => ConditionNarrowings {
                when_true: TypeNarrowing::default(),
                when_false: self
                    .condition_narrowings(left)
                    .when_false
                    .combined(self.condition_narrowings(right).when_false),
            },
            Expr::Binary { op, left, right } if matches!(op, BinaryOp::Eq | BinaryOp::Ne) => {
                self.equality_narrowings(*op, left, right)
            }
            _ => ConditionNarrowings::default(),
        }
    }

    fn equality_narrowings(&self, op: BinaryOp, left: &Expr, right: &Expr) -> ConditionNarrowings {
        if expr_is_null_literal(right) {
            return self.null_comparison_narrowings(op, left);
        }
        if expr_is_null_literal(left) {
            return self.null_comparison_narrowings(op, right);
        }
        if let Some(value) = expr_string_literal(right) {
            return self.discriminant_narrowings(op, left, value);
        }
        if let Some(value) = expr_string_literal(left) {
            return self.discriminant_narrowings(op, right, value);
        }
        ConditionNarrowings::default()
    }

    fn null_comparison_narrowings(&self, op: BinaryOp, value: &Expr) -> ConditionNarrowings {
        let Some(non_null) = self
            .type_for_path_expr(value)
            .and_then(|ty| non_nullable_type(&ty))
        else {
            return ConditionNarrowings::default();
        };
        let narrowing = self.narrowing_for_path_expr(value, non_null);
        match op {
            BinaryOp::Eq => ConditionNarrowings {
                when_true: TypeNarrowing::default(),
                when_false: narrowing,
            },
            BinaryOp::Ne => ConditionNarrowings {
                when_true: narrowing,
                when_false: TypeNarrowing::default(),
            },
            _ => ConditionNarrowings::default(),
        }
    }

    fn discriminant_narrowings(
        &self,
        op: BinaryOp,
        tag_expr: &Expr,
        tag_value: &str,
    ) -> ConditionNarrowings {
        let Expr::Field { object, field } = tag_expr else {
            return ConditionNarrowings::default();
        };
        if field != "tag" {
            return ConditionNarrowings::default();
        }
        let Some(object_ty) = self.type_for_path_expr(object) else {
            return ConditionNarrowings::default();
        };
        let Some(matching) = self.narrow_type_by_tag(&object_ty, tag_value, true) else {
            return ConditionNarrowings::default();
        };
        let non_matching = self.narrow_type_by_tag(&object_ty, tag_value, false);
        let matching = self.narrowing_for_path_expr(object, matching);
        let non_matching = non_matching
            .map(|ty| self.narrowing_for_path_expr(object, ty))
            .unwrap_or_default();
        match op {
            BinaryOp::Eq => ConditionNarrowings {
                when_true: matching,
                when_false: non_matching,
            },
            BinaryOp::Ne => ConditionNarrowings {
                when_true: non_matching,
                when_false: matching,
            },
            _ => ConditionNarrowings::default(),
        }
    }

    fn null_guard_assignment_narrowing(
        &self,
        condition: &Expr,
        then_block: &Block,
    ) -> Option<TypeNarrowing> {
        let Expr::Binary {
            op: BinaryOp::Eq,
            left,
            right,
        } = condition
        else {
            return None;
        };
        let value = if expr_is_null_literal(right) {
            left.as_ref()
        } else if expr_is_null_literal(left) {
            right.as_ref()
        } else {
            return None;
        };
        let path = expr_path(value)?;
        if !block_assigns_obviously_non_null(then_block, &path) {
            return None;
        }
        self.type_for_path_expr(value)
            .and_then(|ty| non_nullable_type(&ty))
            .map(|ty| self.narrowing_for_path_expr(value, ty))
    }

    fn type_for_path_expr(&self, expr: &Expr) -> Option<ResolvedTypeRef> {
        let path = expr_path(expr)?;
        if let Some(ty) = self.path_refinements.get(&path) {
            return Some(ty.clone());
        }
        match expr {
            Expr::Identifier(name) => self.env.get(name).cloned(),
            Expr::Field { object, field } => self
                .type_for_path_expr(object)
                .and_then(|object_ty| self.record_field_type(&object_ty, field)),
            _ => None,
        }
    }

    fn record_field_type(&self, ty: &ResolvedTypeRef, field: &str) -> Option<ResolvedTypeRef> {
        self.type_resolution
            .record_field_type(ty, field, &self.type_context)
            .or_else(|| record_field_type_from_ir(&ty.ir, field))
    }

    fn narrow_type_by_tag(
        &self,
        ty: &ResolvedTypeRef,
        tag_value: &str,
        include_matching: bool,
    ) -> Option<ResolvedTypeRef> {
        narrow_type_by_tag(ty, tag_value, include_matching).or_else(|| {
            let shape = self.type_resolution.type_shape_ir(ty, &self.type_context)?;
            narrow_type_by_tag(&resolved_type_from_ir(&shape), tag_value, include_matching)
        })
    }

    fn narrowing_for_path_expr(&self, expr: &Expr, ty: ResolvedTypeRef) -> TypeNarrowing {
        match expr {
            Expr::Identifier(name) => TypeNarrowing {
                env: BTreeMap::from([(name.clone(), ty)]),
                paths: BTreeMap::new(),
            },
            _ => expr_path(expr)
                .map(|path| TypeNarrowing {
                    env: BTreeMap::new(),
                    paths: BTreeMap::from([(path, ty)]),
                })
                .unwrap_or_default(),
        }
    }

    fn check_condition(&mut self, condition: &Expr, context: &str) {
        let actual = self.check_expr(condition);
        let Some(actual) = actual else {
            return;
        };
        let Some(expected) = self.resolve_builtin("bool") else {
            return;
        };
        if !self
            .type_resolution
            .assignable_in_context(&actual, &expected, &self.type_context)
        {
            self.diagnostics.push(format!(
                "{}: {context} type mismatch at {}: expected bool, found {}",
                self.module_path,
                self.current_expression_span_label(),
                actual.source_text
            ));
        }
    }

    fn check_return_value(&mut self, value: &Expr) {
        let value_key = self.peek_key();
        let actual = self.check_expr(value);
        if self.stream_chunk.is_some() {
            match actual.as_ref() {
                Some(actual) if type_ir_is_void_or_null(&actual.ir) => return,
                Some(actual) if stream_chunk_type(actual).is_none() => {
                    self.diagnostics.push(format!(
                        "{}: stream producer completion type mismatch at {}: expected null, found {}",
                        self.module_path,
                        self.expression_span_label(&value_key),
                        actual.source_text
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
            Some(&annotation),
            value,
            &value_key,
            &actual,
            &expected,
            None,
            "return",
            self.expression_span(&value_key),
        );
    }

    fn check_expr(&mut self, expr: &Expr) -> Option<ResolvedTypeRef> {
        self.check_expr_with_field_diagnostics(expr, true, None)
    }

    fn check_callee_expr(&mut self, expr: &Expr) -> Option<ResolvedTypeRef> {
        self.check_expr_with_field_diagnostics(expr, false, None)
    }

    fn check_db_predicate_expr(
        &mut self,
        expr: &Expr,
        fields: &BTreeMap<String, ResolvedTypeRef>,
    ) -> Option<ResolvedTypeRef> {
        self.check_expr_with_field_diagnostics(expr, true, Some(fields))
    }

    fn check_expr_with_field_diagnostics(
        &mut self,
        expr: &Expr,
        diagnose_unknown_field: bool,
        db_predicate_fields: Option<&BTreeMap<String, ResolvedTypeRef>>,
    ) -> Option<ResolvedTypeRef> {
        let key = self.next_key();
        let refined_ty = expr_path(expr).and_then(|path| self.path_refinements.get(&path).cloned());
        let ty = match expr {
            Expr::Literal(literal) => self.literal_type(literal),
            Expr::Identifier(name) => refined_ty.clone().or_else(|| self.env.get(name).cloned()),
            Expr::DependencySourceAddress(source) => {
                if diagnose_unknown_field {
                    let message = format!(
                        "{}: dependency source address `{}/{}` is not a value at {}; use `{}/{} as I` to box a public instance or call an exported callable",
                        self.module_path,
                        source.dependency_ref,
                        source.public_path,
                        self.expression_span_label(&key),
                        source.dependency_ref,
                        source.public_path
                    );
                    self.diagnostics.push(message);
                }
                None
            }
            Expr::Binary { op, left, right } => {
                let db_relational = db_predicate_fields.is_some()
                    && matches!(
                        op,
                        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
                    );
                let db_logical =
                    db_predicate_fields.is_some() && matches!(op, BinaryOp::And | BinaryOp::Or);
                let db_field_relational = db_relational
                    && db_predicate_fields
                        .is_some_and(|fields| Self::is_db_field_operand(left, fields));
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
                    &key,
                    *op,
                    left_ty.as_ref(),
                    right_ty.as_ref(),
                    db_field_relational,
                );
                self.binary_type(*op, left_ty.as_ref(), right_ty.as_ref())
            }
            Expr::Unary { op, expr } => {
                let operand_ty = if db_predicate_fields.is_some() && matches!(op, UnaryOp::Not) {
                    self.check_db_predicate_expr(expr, db_predicate_fields.expect("checked above"))
                } else {
                    self.check_expr(expr)
                };
                self.check_unary_operand(&key, *op, operand_ty.as_ref());
                self.unary_type(*op)
            }
            Expr::Call { callee, args } => {
                self.check_callee_expr(callee);
                let arg_types = args
                    .iter()
                    .map(|arg| {
                        let key = self.peek_key();
                        (key, self.check_expr(arg))
                    })
                    .collect::<Vec<_>>();
                self.call_type(&key, callee, args, &arg_types)
            }
            Expr::Generic { callee, .. } => {
                if diagnose_unknown_field {
                    self.check_expr(callee)
                } else {
                    self.check_callee_expr(callee)
                }
            }
            Expr::InterfaceBox { value, interface } => {
                let value_ty = self.check_expr(value);
                let selector = match self
                    .type_resolution
                    .resolve_canonical_interface_selector_type_ref(interface, &self.type_context)
                {
                    Ok(selector) => selector,
                    Err(error) => {
                        self.diagnostics.push(format!(
                            "{}: interface boxing selector `{}` failed at {}: {error}",
                            self.module_path,
                            interface.name,
                            self.expression_span_label(&key)
                        ));
                        return None;
                    }
                };
                let Some(value_ty) = value_ty else {
                    return None;
                };
                let Some(receiver) = self
                    .type_resolution
                    .concrete_nominal_record_symbol(&value_ty, &self.type_context)
                else {
                    self.diagnostics.push(format!(
                        "{}: interface boxing source at {} must be a concrete nominal record, found {}",
                        self.module_path,
                        self.expression_span_label(&key),
                        value_ty.source_text
                    ));
                    return None;
                };
                let expected_interface = ResolvedTypeRef {
                    source_text: selector.source_text.clone(),
                    ir: selector.identity.clone(),
                };
                match self.type_resolution.concrete_type_conforms_to_interface(
                    &value_ty,
                    &expected_interface,
                    &self.type_context,
                ) {
                    Ok(Some(_)) => Some(ResolvedTypeRef {
                        source_text: format!("any {}", selector.source_text),
                        ir: TypeRefIr::AnyInterface {
                            interface: selector.instantiation_ref,
                        },
                    }),
                    Ok(None) => {
                        self.diagnostics.push(format!(
                            "{}: type {} does not explicitly implement interface {} for boxing at {}",
                            self.module_path,
                            receiver,
                            selector.source_text,
                            self.expression_span_label(&key)
                        ));
                        None
                    }
                    Err(error) => {
                        self.diagnostics.push(format!(
                            "{}: interface boxing conformance check failed at {}: {error}",
                            self.module_path,
                            self.expression_span_label(&key)
                        ));
                        None
                    }
                }
            }
            Expr::Field { object, field } => {
                let object_key = self.peek_key();
                let object_ty = if diagnose_unknown_field {
                    self.check_expr(object)
                } else {
                    self.check_callee_expr(object)
                };
                object_ty.and_then(|object_ty| {
                    let field_ty =
                        if matches!(object.as_ref(), Expr::Identifier(name) if name == "self")
                            && self
                                .type_resolution
                                .actor_type_resolution(&object_ty, &self.type_context)
                                .is_some()
                        {
                            self.type_resolution.actor_state_field_type(
                                &object_ty,
                                field,
                                &self.type_context,
                            )
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
                            if let skiff_artifact_model::ContractTypeDescriptor::Record {
                                fields,
                            } = &record.canonical_descriptor.descriptor
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
                        self.diagnostics.push(format!(
                            "{}: unknown field `{field}` on {} at {}",
                            self.module_path,
                            object_ty.source_text,
                            self.expression_span_label(&key)
                        ));
                    }
                    field_ty
                })
            }
            Expr::Record {
                type_name,
                type_args,
                fields,
            } => {
                let mut field_types = Vec::new();
                let mut provided_field_keys = Vec::new();
                for (name, value) in fields {
                    let value_key = self.peek_key();
                    provided_field_keys.push((name.clone(), value_key));
                    let value_ty = self.check_expr(value);
                    field_types.push(value_ty);
                }
                self.validate_constructor(
                    &key,
                    type_name,
                    type_args,
                    fields,
                    &field_types,
                    &provided_field_keys,
                )
            }
            Expr::ObjectLiteral { entries } => {
                let source_fact = self.expression_sources.fact(&key);
                let mut fields = BTreeMap::new();
                let mut source_fields = Vec::with_capacity(entries.len());
                for (index, entry) in entries.iter().enumerate() {
                    let value_key = self.peek_key();
                    let actual = self.check_expr(&entry.value);
                    let Some(name) = object_literal_key_text(&entry.key) else {
                        continue;
                    };
                    if let Some(actual) = &actual {
                        fields.insert(name.clone(), actual.clone());
                    }
                    source_fields.push(ObjectLiteralSourceField {
                        name,
                        expression: value_key,
                        actual,
                        value_span: record_field_value_source_span(source_fact, index),
                    });
                }
                self.object_materialization.sources.insert(
                    key.clone(),
                    ObjectLiteralSource {
                        span: source_fact
                            .map(|fact| fact.span)
                            .unwrap_or_else(SourceSpan::synthetic),
                        fields: source_fields,
                    },
                );
                Some(ResolvedTypeRef {
                    ir: TypeRefIr::Record {
                        fields: fields
                            .iter()
                            .map(|(name, ty)| (name.clone(), ty.ir.clone()))
                            .collect(),
                    },
                    source_text: "{}".to_string(),
                })
            }
            Expr::Patch { operations, .. } => {
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
            Expr::Throw { value } => {
                self.check_expr(value);
                None
            }
            Expr::Rethrow { exception } => {
                self.check_expr(exception);
                None
            }
            Expr::Catch {
                catch_type,
                try_expr,
            } => {
                let try_ty = self.check_expr(try_expr)?;
                let catch_ty = self
                    .type_resolution
                    .resolve_type_ref(catch_type, &self.type_context)
                    .ok()?;
                Some(catch_result_type(try_ty, catch_ty))
            }
            Expr::DbOperation(operation) => {
                self.check_db_operation_children(operation);
                self.db_operation_type(operation)
            }
            Expr::DbQuery(query) => {
                self.check_db_query_block(&query.query, &query.target);
                self.db_query_type(&query.target)
            }
            Expr::DbTransaction(transaction) => {
                let mut last = None;
                for stmt in &transaction.body.statements {
                    if let Stmt::Expr(value) = stmt {
                        last = self.check_expr(value);
                    } else {
                        self.check_stmt(stmt);
                    }
                }
                match transaction.mode {
                    DbBlockMode::Effect => self.resolve_builtin("null"),
                    DbBlockMode::Value => last,
                }
            }
            Expr::DbLeaseClaim(claim) => {
                self.check_expr(&claim.key);
                if let Some(binding) = &claim.binding {
                    if let Ok(target) = self
                        .type_resolution
                        .resolve_type_ref(&claim.target, &self.type_context)
                    {
                        let previous = self.env.insert(binding.clone(), target);
                        self.check_block(&claim.body);
                        if let Some(previous) = previous {
                            self.env.insert(binding.clone(), previous);
                        } else {
                            self.env.remove(binding);
                        }
                    } else {
                        self.check_block(&claim.body);
                    }
                } else {
                    self.check_block(&claim.body);
                }
                self.resolve_builtin("bool")
            }
            Expr::DbLeaseRead(read) => {
                self.check_expr(&read.key);
                Some(db_lease_read_type())
            }
        };
        let ty = refined_ty.or(ty);
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
                    Err(error) => self.diagnostics.push(format!(
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
        self.facts.insert(
            key,
            ExpressionTypeFact {
                ty: ty.clone(),
                span,
                stream_emit_target: None,
            },
        );
        ty
    }

    fn record_stream_emit_target(&mut self, key: &ExpressionKey, target: ResolvedTypeRef) {
        let Some(fact) = self.facts.get_mut(key) else {
            self.diagnostics.push(format!(
                "{}: emit target fact could not be recorded at {}",
                self.module_path,
                self.expression_span_label(key)
            ));
            return;
        };
        fact.stream_emit_target = Some(target);
    }

    fn validate_constructor(
        &mut self,
        key: &ExpressionKey,
        type_name: &str,
        type_args: &[TypeRef],
        fields: &[(String, Expr)],
        field_types: &[Option<ResolvedTypeRef>],
        provided_field_keys: &[(String, ExpressionKey)],
    ) -> Option<ResolvedTypeRef> {
        let source_fact = self.expression_sources.fact(key);
        let target = match self.type_resolution.resolve_constructor_target(
            type_name,
            type_args,
            &self.type_context,
        ) {
            Ok(target) => target,
            Err(error) => {
                self.diagnostics.push(format!(
                    "{}: constructor target `{type_name}` failed to resolve at {}: {error}",
                    self.module_path,
                    source_fact
                        .map(|fact| span_label(fact.span))
                        .unwrap_or_else(|| "unknown span".to_string())
                ));
                return None;
            }
        };

        let mut provided = BTreeMap::<String, ExpressionKey>::new();
        let mut duplicates = BTreeSet::new();
        let mut provided_fields = Vec::new();
        let mut duplicate_fields = Vec::new();
        let mut unknown_fields = Vec::new();
        let mut type_mismatches = Vec::new();
        let exact_field_types = self
            .dependency_analysis
            .and_then(|dependency_analysis| {
                let direct = type_name.split_once('.').and_then(|(alias, stable_key)| {
                    dependency_analysis.direct_package_type(alias, stable_key)
                });
                let projected = || {
                    let PackageTypeRef::PackageSchema {
                        package_id,
                        stable_schema_key,
                        package_schema_type_id,
                    } = ContractProjectionState::project_resolved_type(
                        &target.ty,
                        self.type_resolution,
                        dependency_analysis,
                        &self.type_context,
                    )
                    .ok()?
                    else {
                        return None;
                    };
                    dependency_analysis.exact_package_type(
                        &package_id,
                        &stable_schema_key,
                        &package_schema_type_id,
                    )
                };
                direct.or_else(projected).and_then(|record| {
                    let skiff_artifact_model::ContractTypeDescriptor::Record { fields } =
                        &record.canonical_descriptor.descriptor
                    else {
                        return None;
                    };
                    Some(
                        fields
                            .iter()
                            .map(|(name, ty)| {
                                (
                                    name.clone(),
                                    contract_call_typing::package_type_ref_from_contract_type(ty),
                                )
                            })
                            .collect::<BTreeMap<_, _>>(),
                    )
                })
            })
            .unwrap_or_default();
        for (index, (field_name, value_key)) in provided_field_keys.iter().enumerate() {
            let name_span = record_field_name_source_span(source_fact, index);
            let value_span = record_field_value_source_span(source_fact, index);
            provided_fields.push(ConstructorProvidedField {
                name: field_name.clone(),
                expression: value_key.clone(),
                actual: field_types.get(index).cloned().flatten(),
                name_span,
                value_span,
            });
            if let Some(first_expression) = provided.insert(field_name.clone(), value_key.clone()) {
                duplicates.insert(field_name.clone());
                duplicate_fields.push(DuplicateConstructorField {
                    name: field_name.clone(),
                    first_expression,
                    duplicate_expression: value_key.clone(),
                    name_span,
                });
                self.diagnostics.push(format!(
                    "{}: duplicate constructor field `{field_name}` at {}",
                    self.module_path,
                    span_label(name_span)
                ));
            }
        }

        for (index, (field_name, value)) in fields.iter().enumerate() {
            if duplicates.contains(field_name) {
                continue;
            }
            let value_key = provided_field_keys
                .get(index)
                .map(|(_, key)| key)
                .expect("constructor field keys must align with AST fields");
            let actual = field_types
                .get(index)
                .and_then(|field_type| field_type.as_ref());
            let Some(expected) = target.fields.get(field_name) else {
                unknown_fields.push(UnknownConstructorField {
                    name: field_name.clone(),
                    expression: value_key.clone(),
                    actual: actual.cloned(),
                    name_span: record_field_name_source_span(source_fact, index),
                    value_span: record_field_value_source_span(source_fact, index),
                });
                self.diagnostics.push(format!(
                    "{}: unknown constructor field `{field_name}` for `{type_name}` at {}",
                    self.module_path,
                    span_label(record_field_name_source_span(source_fact, index))
                ));
                continue;
            };
            if let Some(actual) = actual {
                let context = format!("constructor field `{field_name}`");
                if !self.check_value_assignable_to_expected(
                    None,
                    value,
                    value_key,
                    actual,
                    expected,
                    exact_field_types.get(field_name),
                    &context,
                    record_field_value_source_span(source_fact, index),
                ) {
                    type_mismatches.push(ConstructorFieldTypeMismatch {
                        name: field_name.clone(),
                        expression: value_key.clone(),
                        expected: expected.clone(),
                        actual: actual.clone(),
                        value_span: record_field_value_source_span(source_fact, index),
                    });
                }
            }
        }

        let mut materialized_fields = Vec::new();
        let mut missing_required_fields = Vec::new();
        for (field_name, expected) in &target.fields {
            if let Some(expression) = provided.get(field_name) {
                materialized_fields.push(MaterializedConstructorField {
                    name: field_name.clone(),
                    ty: expected.clone(),
                    source: ConstructorFieldValueSource::Provided {
                        field_name: field_name.clone(),
                        expression: expression.clone(),
                    },
                });
            } else if self.type_resolution.is_nullable(expected) {
                materialized_fields.push(MaterializedConstructorField {
                    name: field_name.clone(),
                    ty: expected.clone(),
                    source: ConstructorFieldValueSource::SyntheticNull,
                });
            } else {
                let span = source_fact
                    .map(|fact| fact.span)
                    .unwrap_or_else(SourceSpan::synthetic);
                missing_required_fields.push(MissingConstructorField {
                    name: field_name.clone(),
                    expected: expected.clone(),
                    span,
                });
                self.diagnostics.push(format!(
                    "{}: missing required constructor field `{field_name}` for `{type_name}` at {}",
                    self.module_path,
                    span_label(span)
                ));
            }
        }

        self.constructor_validations.insert(
            key.clone(),
            ConstructorValidation {
                target: target.ty.clone(),
                provided_fields,
                materialized_fields,
                duplicate_fields,
                missing_required_fields,
                unknown_fields,
                type_mismatches,
            },
        );
        Some(target.ty)
    }

    fn check_db_operation_children(&mut self, operation: &crate::shared::ast::DbOperation) {
        if let Some(selector) = &operation.selector {
            self.check_db_selector(selector, &operation.target);
        }
        if let Some(query) = operation.independent_query() {
            self.check_db_query_block(query, &operation.target);
        }
        if let Some(body) = &operation.body {
            self.check_db_body(body);
        }
        if let Some(body) = &operation.insert_body {
            self.check_db_body(body);
        }
        if let Some(change) = &operation.change {
            for op in &change.ops {
                match op {
                    DbChangeOp::Set { value, .. }
                    | DbChangeOp::Inc { value, .. }
                    | DbChangeOp::AddToSet { value, .. }
                    | DbChangeOp::Remove { value, .. } => {
                        self.check_expr(value);
                    }
                    DbChangeOp::Unset { .. } => {}
                }
            }
        }
    }

    fn check_db_selector(&mut self, selector: &DbSelector, target: &TypeRef) {
        match selector {
            DbSelector::Key { value } => {
                self.check_expr(value);
            }
            DbSelector::Query { query } => self.check_db_query_block(query, target),
        }
    }

    fn check_db_query_block(&mut self, query: &DbQueryBlock, target: &TypeRef) {
        for clause in &query.where_clauses {
            match clause {
                DbWhereClause::Predicate { predicate } => {
                    self.check_db_predicate(predicate, target);
                }
                DbWhereClause::Conditional {
                    condition,
                    predicate,
                } => {
                    self.check_condition(condition, "db where condition");
                    self.check_db_predicate(predicate, target);
                }
            }
        }
        if let Some(limit) = &query.limit {
            self.check_expr(limit);
        }
        if let Some(offset) = &query.offset {
            self.check_expr(offset);
        }
        if let Some(after) = &query.after {
            self.check_expr(after);
        }
    }

    fn check_db_predicate(&mut self, predicate: &Expr, target: &TypeRef) {
        let fields = self
            .type_resolution
            .resolve_constructor_target_text(&target.name, &self.type_context)
            .map(|target| target.fields)
            .unwrap_or_default();
        let actual = self.check_db_predicate_expr(predicate, &fields);
        let (Some(actual), Some(expected)) = (actual, self.resolve_builtin("bool")) else {
            return;
        };
        if !self
            .type_resolution
            .assignable_in_context(&actual, &expected, &self.type_context)
        {
            self.diagnostics.push(format!(
                "{}: db where predicate type mismatch at {}: expected bool, found {}",
                self.module_path,
                self.current_expression_span_label(),
                actual.source_text
            ));
        }
    }

    fn check_db_field_operand(
        &mut self,
        expr: &Expr,
        fields: &BTreeMap<String, ResolvedTypeRef>,
    ) -> Option<ResolvedTypeRef> {
        let Some(root) = Self::db_field_operand_root(expr, fields) else {
            return self.check_expr(expr);
        };
        let target_type = fields
            .get(&root)
            .expect("DB field operand root must come from target fields")
            .clone();
        let previous_env = self.env.insert(root.clone(), target_type);
        let descendant_prefix = format!("{root}.");
        let conflicting_paths = self
            .path_refinements
            .keys()
            .filter(|path| *path == &root || path.starts_with(&descendant_prefix))
            .cloned()
            .collect::<Vec<_>>();
        let previous_refinements = conflicting_paths
            .into_iter()
            .filter_map(|path| self.path_refinements.remove(&path).map(|ty| (path, ty)))
            .collect::<Vec<_>>();
        let ty = self.check_expr(expr);
        if let Some(previous_env) = previous_env {
            self.env.insert(root.clone(), previous_env);
        } else {
            self.env.remove(&root);
        }
        for (path, ty) in previous_refinements {
            self.path_refinements.insert(path, ty);
        }
        ty
    }

    fn is_db_field_operand(expr: &Expr, fields: &BTreeMap<String, ResolvedTypeRef>) -> bool {
        Self::db_field_operand_root(expr, fields).is_some()
    }

    fn db_field_operand_root(
        expr: &Expr,
        fields: &BTreeMap<String, ResolvedTypeRef>,
    ) -> Option<String> {
        expr_path(expr)
            .and_then(|path| path.split('.').next().map(str::to_string))
            .filter(|root| fields.contains_key(root))
    }

    fn check_db_body(&mut self, body: &DbBody) {
        match body {
            DbBody::ObjectFields { fields } => {
                for field in fields {
                    self.check_expr(&field.value);
                }
            }
            DbBody::Values { value } => {
                self.check_expr(value);
            }
        }
    }

    fn next_key(&mut self) -> ExpressionKey {
        let key = self.peek_key();
        self.next_index += 1;
        key
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
            Literal::String(value) => Some(ResolvedTypeRef {
                ir: TypeRefIr::Literal {
                    value: LiteralIr::String {
                        value: value.clone(),
                    },
                },
                source_text: serde_json::to_string(value)
                    .unwrap_or_else(|_| "\"<string>\"".to_string()),
            }),
            Literal::Number(number) if number.fract() == 0.0 => self.resolve_builtin("integer"),
            Literal::Number(_) => self.resolve_builtin("number"),
            Literal::Bool(_) => self.resolve_builtin("bool"),
            Literal::Null => Some(ResolvedTypeRef {
                ir: TypeRefIr::Literal {
                    value: LiteralIr::Null,
                },
                source_text: "null".to_string(),
            }),
        }
    }

    fn binary_type(
        &self,
        op: BinaryOp,
        left: Option<&ResolvedTypeRef>,
        right: Option<&ResolvedTypeRef>,
    ) -> Option<ResolvedTypeRef> {
        match op {
            BinaryOp::Add if self.operands_string_concat(left, right) => {
                self.resolve_builtin("string")
            }
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge
            | BinaryOp::And
            | BinaryOp::Or => self.resolve_builtin("bool"),
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                self.resolve_builtin("number")
            }
        }
    }

    fn check_binary_operands(
        &mut self,
        key: &ExpressionKey,
        op: BinaryOp,
        left: Option<&ResolvedTypeRef>,
        right: Option<&ResolvedTypeRef>,
        db_field_relational: bool,
    ) {
        match op {
            BinaryOp::Add if self.operands_string_concat(left, right) => {}
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                self.check_operand_assignable(key, "binary arithmetic operand", left, "number");
                self.check_operand_assignable(key, "binary arithmetic operand", right, "number");
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                if db_field_relational && self.operands_both_assignable_to(left, right, "string") {
                    return;
                }
                self.check_operand_assignable(key, "binary comparison operand", left, "number");
                self.check_operand_assignable(key, "binary comparison operand", right, "number");
            }
            BinaryOp::And | BinaryOp::Or => {
                self.check_operand_assignable(key, "binary boolean operand", left, "bool");
                self.check_operand_assignable(key, "binary boolean operand", right, "bool");
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                let (Some(left), Some(right)) = (left, right) else {
                    return;
                };
                if !self
                    .type_resolution
                    .assignable_in_context(left, right, &self.type_context)
                    && !self
                        .type_resolution
                        .assignable_in_context(right, left, &self.type_context)
                {
                    self.diagnostics.push(format!(
                        "{}: equality operand type mismatch at {}: left {}, right {}",
                        self.module_path,
                        self.expression_span_label(key),
                        left.source_text,
                        right.source_text
                    ));
                }
            }
        }
    }

    fn operands_string_concat(
        &self,
        left: Option<&ResolvedTypeRef>,
        right: Option<&ResolvedTypeRef>,
    ) -> bool {
        let Some(expected) = self.resolve_builtin("string") else {
            return false;
        };
        left.is_some_and(|left| {
            self.type_resolution
                .assignable_in_context(left, &expected, &self.type_context)
        }) || right.is_some_and(|right| {
            self.type_resolution
                .assignable_in_context(right, &expected, &self.type_context)
        })
    }

    fn operands_both_assignable_to(
        &self,
        left: Option<&ResolvedTypeRef>,
        right: Option<&ResolvedTypeRef>,
        expected_builtin: &str,
    ) -> bool {
        let (Some(left), Some(right), Some(expected)) =
            (left, right, self.resolve_builtin(expected_builtin))
        else {
            return false;
        };
        self.type_resolution
            .assignable_in_context(left, &expected, &self.type_context)
            && self
                .type_resolution
                .assignable_in_context(right, &expected, &self.type_context)
    }

    fn check_unary_operand(
        &mut self,
        key: &ExpressionKey,
        op: UnaryOp,
        actual: Option<&ResolvedTypeRef>,
    ) {
        match op {
            UnaryOp::Not => {
                self.check_operand_assignable(key, "unary boolean operand", actual, "bool")
            }
        }
    }

    fn check_operand_assignable(
        &mut self,
        key: &ExpressionKey,
        context: &str,
        actual: Option<&ResolvedTypeRef>,
        expected_builtin: &str,
    ) {
        let Some(actual) = actual else {
            return;
        };
        let Some(expected) = self.resolve_builtin(expected_builtin) else {
            return;
        };
        if !self
            .type_resolution
            .assignable_in_context(actual, &expected, &self.type_context)
        {
            self.diagnostics.push(format!(
                "{}: {context} type mismatch at {}: expected {}, found {}",
                self.module_path,
                self.expression_span_label(key),
                expected.source_text,
                actual.source_text
            ));
        }
    }

    fn unary_type(&self, op: UnaryOp) -> Option<ResolvedTypeRef> {
        match op {
            UnaryOp::Not => self.resolve_builtin("bool"),
        }
    }

    fn call_type(
        &mut self,
        key: &ExpressionKey,
        callee: &Expr,
        args: &[Expr],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Option<ResolvedTypeRef> {
        let (callee, type_args) = match callee {
            Expr::Generic { callee, type_args } => (callee.as_ref(), type_args.as_slice()),
            _ => (callee, &[][..]),
        };
        if let Some(return_type) = self.runtime_receiver_call_type(key, callee, args, arg_types) {
            return Some(return_type);
        }
        if let Some(return_type) =
            self.actor_receiver_call_type(key, callee, type_args, args, arg_types)
        {
            return Some(return_type);
        }
        if let Some(return_type) =
            self.any_interface_receiver_call_type(key, callee, type_args, args, arg_types)
        {
            return Some(return_type);
        }
        if let Some(return_type) =
            self.package_interface_receiver_call_type(key, callee, type_args, args, arg_types)
        {
            return Some(return_type);
        }
        let path = expr_path(callee)?;
        if !path
            .split('.')
            .next()
            .is_some_and(|root| self.env.contains_key(root))
        {
            if let Some(dependency_analysis) = self.dependency_analysis {
                match ContractCallTyping::new(
                    self.type_resolution,
                    dependency_analysis,
                    &self.type_context,
                )
                .check_call(
                    &path,
                    type_args.len(),
                    arg_types,
                    self.contract_projection.expression_types(),
                ) {
                    ContractCallOutcome::NotContract => {}
                    ContractCallOutcome::Typed {
                        return_type,
                        projected_return_type,
                    } => {
                        self.contract_projection
                            .record_expression_type(key.clone(), projected_return_type);
                        return Some(return_type);
                    }
                    ContractCallOutcome::Invalid(diagnostics) => {
                        let location = self.expression_span_label(key);
                        self.diagnostics
                            .extend(diagnostics.into_iter().map(|diagnostic| {
                                format!("{}: {diagnostic} at {location}", self.module_path)
                            }));
                        return None;
                    }
                }
            }
        }
        if type_args.is_empty() {
            if let Some(dependency_analysis) = self.dependency_analysis {
                if let Some(signature) = dependency_analysis
                    .package_callable_by_source_path(&path)
                    .and_then(|callable| callable.signature())
                    .filter(|signature| !package_type_contains_local_slot(&signature.return_type))
                {
                    let resolved_return = resolved_package_type_ref(&signature.return_type);
                    let exact_projection = contract_call_typing::project_resolved_package_type(
                        &resolved_return,
                        self.type_resolution,
                        dependency_analysis,
                        &self.type_context,
                    )
                    .unwrap_or_else(|_| signature.return_type.clone());
                    self.contract_projection
                        .record_expression_type(key.clone(), exact_projection);
                    return Some(resolved_return);
                }
            }
        }
        if let Some(return_type) = self.config_intrinsic_call_type(&path, type_args) {
            return Some(return_type);
        }
        if matches!(
            path.as_str(),
            "std.actor.getOrCreate" | "std.actor.replace" | "std.actor.find" | "std.actor.remove"
        ) {
            return self.actor_registry_intrinsic_call_type(&path, type_args, args, arg_types);
        }
        match self.type_resolution.resolve_representation_constructor(
            &path,
            type_args,
            &self.type_context,
        ) {
            Ok(Some(representation)) => {
                self.validate_resolved_call_params(
                    &path,
                    vec![("value".to_string(), representation.payload.clone())],
                    args,
                    arg_types,
                );
                if let Some((payload, _)) = arg_types.first() {
                    self.representation_constructor_validations.insert(
                        key.clone(),
                        RepresentationConstructorValidation {
                            target: representation.wrapper.clone(),
                            payload: payload.clone(),
                        },
                    );
                }
                return Some(representation.wrapper);
            }
            Ok(None) => {}
            Err(error) => {
                self.diagnostics.push(format!(
                    "{}: representation constructor `{path}` failed to resolve: {error}",
                    self.module_path
                ));
                return None;
            }
        }
        if let Some(return_type) = prelude_registry().native_return_type(&path) {
            let native_context = native_return_type_context(&path, &self.type_context);
            if let Some(params) = prelude_registry().native_params(&path) {
                let mut expected = self.resolve_callable_param_types(
                    &path,
                    params.iter().map(String::as_str),
                    &native_context,
                    prelude_registry().builtin_type_params(&path).unwrap_or(&[]),
                    type_args,
                );
                if native_context.module_path != self.module_path {
                    expected.params = expected
                        .params
                        .into_iter()
                        .map(|(name, ty)| {
                            (
                                name,
                                self.type_resolution
                                    .externalize_local_type_refs(&ty, native_context.module_path),
                            )
                        })
                        .collect();
                }
                if expected.complete {
                    self.validate_resolved_call_params(&path, expected.params, args, arg_types);
                }
            }
            let resolved_return_type = self.resolve_callable_return_type(
                &return_type,
                &native_context,
                prelude_registry().builtin_type_params(&path).unwrap_or(&[]),
                type_args,
            )?;
            return Some(if native_context.module_path == self.module_path {
                resolved_return_type
            } else {
                self.type_resolution
                    .externalize_local_type_refs(&resolved_return_type, native_context.module_path)
            });
        }
        if let Some(signature) = self.local_callable_signature(&path).cloned() {
            let signature_context = TypeResolutionContext::with_type_params(
                &signature.module_path,
                signature.type_params.iter().cloned().collect(),
            );
            let type_params = signature.type_params.clone();
            let params = signature.params.clone();
            let return_type = signature.return_type.clone();
            let declaration_name = signature.declaration_name.clone();
            let projected_params = match params
                .iter()
                .map(|param| {
                    self.project_callable_package_type(
                        &param.ty,
                        &signature_context,
                        &type_params,
                        type_args,
                    )
                })
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(projected) => projected,
                Err(error) => {
                    self.diagnostics.push(format!(
                        "{}: call `{declaration_name}` exact parameter type projection failed: {error}",
                        self.module_path
                    ));
                    return None;
                }
            };
            let mut expected = self.resolve_callable_param_types(
                &declaration_name,
                params.iter().map(|param| param.ty.name.as_str()),
                &signature_context,
                &type_params,
                type_args,
            );
            if signature.module_path != self.module_path {
                expected.params = expected
                    .params
                    .into_iter()
                    .map(|(name, ty)| {
                        (
                            name,
                            self.type_resolution
                                .externalize_local_type_refs(&ty, &signature.module_path),
                        )
                    })
                    .collect();
            }
            if expected.complete {
                self.validate_resolved_call_params_with_projections(
                    &declaration_name,
                    expected.params,
                    &projected_params,
                    args,
                    arg_types,
                );
            }
            let projected_return_type = match self.project_callable_package_type(
                &return_type,
                &signature_context,
                &type_params,
                type_args,
            ) {
                Ok(projected) => projected,
                Err(error) => {
                    self.diagnostics.push(format!(
                        "{}: call `{declaration_name}` exact return type projection failed: {error}",
                        self.module_path
                    ));
                    return None;
                }
            };
            let resolved_return_type = self.resolve_callable_return_type(
                &return_type.name,
                &signature_context,
                &type_params,
                type_args,
            )?;
            let resolved_return_type = if signature.module_path == self.module_path {
                resolved_return_type
            } else {
                self.type_resolution
                    .externalize_local_type_refs(&resolved_return_type, &signature.module_path)
            };
            if let Some(projected_return_type) = projected_return_type {
                self.contract_projection
                    .record_expression_type(key.clone(), projected_return_type);
            }
            return Some(resolved_return_type);
        }
        if let Some(signature) = self
            .type_resolution
            .resolve_package_callable(&path)
            .cloned()
        {
            let package_root = package_callable_public_root(&path, &signature.source_symbol);
            let signature_context = TypeResolutionContext::with_type_params(
                &signature.module_path,
                signature.type_params.iter().cloned().collect(),
            );
            let params = signature
                .params
                .iter()
                .map(|param| {
                    qualify_package_signature_type_text(
                        param,
                        &package_root,
                        &signature.local_type_names,
                    )
                })
                .collect::<Vec<_>>();
            let expected = self.resolve_callable_param_types(
                &path,
                params.iter().map(String::as_str),
                &signature_context,
                &signature.type_params,
                type_args,
            );
            if expected.complete {
                self.validate_resolved_call_params(&path, expected.params, args, arg_types);
            }
            if let Some(exact_signature) = signature.exact_signature {
                let substitutions = signature
                    .type_params
                    .iter()
                    .zip(type_args)
                    .map(|(param, argument)| {
                        self.project_source_binding_type(argument)
                            .and_then(|projected| {
                                projected.ok_or_else(|| {
                                    format!(
                                        "call `{path}` type argument `{param}` has no exact package projection"
                                    )
                                })
                            })
                            .map(|projected| (param.clone(), projected))
                    })
                    .collect::<Result<BTreeMap<_, _>, _>>();
                let projected_return = match substitutions {
                    Ok(substitutions) => {
                        substitute_package_type(&exact_signature.return_type, &substitutions)
                    }
                    Err(error) => Err(error),
                };
                match projected_return {
                    Ok(projected_return) => {
                        let resolved_return = resolved_package_type_ref(&projected_return);
                        self.contract_projection
                            .record_expression_type(key.clone(), projected_return);
                        return Some(resolved_return);
                    }
                    Err(error) => {
                        self.diagnostics.push(format!(
                            "{}: call `{path}` exact return type substitution failed: {error}",
                            self.module_path
                        ));
                        return None;
                    }
                }
            }
            let package_return_type = qualify_package_signature_type_text(
                &signature.return_type,
                &package_root,
                &signature.local_type_names,
            );
            return self.resolve_callable_return_type(
                &package_return_type,
                &signature_context,
                &signature.type_params,
                type_args,
            );
        }
        match path.as_str() {
            "db.get" | "db.require" | "db.create" | "db.append" | "db.upsert" => {
                type_args.first().and_then(|ty| {
                    self.type_resolution
                        .resolve_type_ref(ty, &self.type_context)
                        .ok()
                })
            }
            "db.findMany" | "db.createMany" | "db.create_many" | "db.appendMany"
            | "db.append_many" => type_args.first().and_then(|ty| {
                self.type_resolution
                    .resolve_type_ref(ty, &self.type_context)
                    .ok()
                    .map(|item| ResolvedTypeRef {
                        ir: TypeRefIr::Builtin {
                            name: "Array".to_string(),
                            args: vec![item.ir],
                        },
                        source_text: format!("Array<{}>", item.source_text),
                    })
            }),
            "db.exists" => self.resolve_builtin("bool"),
            "db.count" => self.resolve_builtin("number"),
            _ => None,
        }
    }

    fn local_callable_signature(&self, path: &str) -> Option<&CallableSignature> {
        if !path.contains('.') {
            let module_qualified = format!("{}.{}", self.module_path, path);
            if let Some(signature) = self.callable_signatures.get(&module_qualified) {
                return Some(signature);
            }
        }
        self.callable_signatures.get(path).or_else(|| {
            path.strip_prefix("root.")
                .and_then(|source_path| self.callable_signatures.get(source_path))
        })
    }

    fn resolve_callable_param_types<'b>(
        &mut self,
        callable: &str,
        params: impl Iterator<Item = &'b str>,
        context: &TypeResolutionContext<'_>,
        type_params: &[String],
        type_args: &[TypeRef],
    ) -> ResolvedCallableParams {
        let mut complete = true;
        let params = params
            .enumerate()
            .filter_map(|(index, raw)| {
                if let Some(resolved) =
                    self.exact_type_arg_substitution(raw, type_params, type_args)
                {
                    return Some((format!("arg{index}"), resolved));
                }
                let text = self.substitute_type_params_in_text(raw, type_params, type_args);
                match self.type_resolution.resolve_type_text(&text, context) {
                    Ok(resolved) => Some((format!("arg{index}"), resolved)),
                    Err(error) => {
                        let _ = (callable, error);
                        complete = false;
                        None
                    }
                }
            })
            .collect();
        ResolvedCallableParams { params, complete }
    }

    fn resolve_callable_return_type(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
        type_params: &[String],
        type_args: &[TypeRef],
    ) -> Option<ResolvedTypeRef> {
        if let Some(resolved) = self.exact_type_arg_substitution(raw, type_params, type_args) {
            return Some(resolved);
        }
        if let Some(resolved) =
            self.structured_type_arg_substitution(raw, context, type_params, type_args)
        {
            return Some(resolved);
        }
        let substituted = self.substitute_type_params_in_text(raw, type_params, type_args);
        self.type_resolution
            .resolve_type_text(&substituted, context)
            .ok()
    }

    fn exact_type_arg_substitution(
        &self,
        raw: &str,
        type_params: &[String],
        type_args: &[TypeRef],
    ) -> Option<ResolvedTypeRef> {
        let raw = raw.trim();
        let index = type_params.iter().position(|param| param == raw)?;
        let arg = type_args.get(index)?;
        self.type_resolution
            .resolve_type_ref(arg, &self.type_context)
            .ok()
    }

    fn structured_type_arg_substitution(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
        type_params: &[String],
        type_args: &[TypeRef],
    ) -> Option<ResolvedTypeRef> {
        if type_params.is_empty() || type_params.len() != type_args.len() {
            return None;
        }
        let generic_context = TypeResolutionContext::with_type_params(
            context.module_path,
            type_params.iter().cloned().collect(),
        );
        let generic = self
            .type_resolution
            .resolve_type_text(raw, &generic_context)
            .ok()?;
        let substitutions = type_params
            .iter()
            .zip(type_args)
            .map(|(param, argument)| {
                self.type_resolution
                    .resolve_type_ref(argument, &self.type_context)
                    .map(|resolved| (param.clone(), resolved.ir))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()
            .ok()?;
        Some(resolved_type_from_ir(&substitute_type_params_in_ir(
            &generic.ir,
            &substitutions,
        )))
    }

    fn project_callable_package_type(
        &self,
        raw: &TypeRef,
        context: &TypeResolutionContext<'_>,
        type_params: &[String],
        type_args: &[TypeRef],
    ) -> Result<Option<PackageTypeRef>, String> {
        let Some(dependency_analysis) = self.dependency_analysis else {
            return Ok(None);
        };
        let projected = ContractProjectionState::project_source_type_ref(
            raw,
            self.type_resolution,
            dependency_analysis,
            context,
        )?;
        let substitutions = type_params
            .iter()
            .zip(type_args)
            .map(|(param, argument)| {
                Ok((
                    param.clone(),
                    ContractProjectionState::project_source_type_ref(
                        argument,
                        self.type_resolution,
                        dependency_analysis,
                        &self.type_context,
                    )?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        substitute_package_type(&projected, &substitutions).map(Some)
    }

    fn resolve_type_arg_substitutions(
        &mut self,
        callable: &str,
        type_params: &[String],
        type_args: &[TypeRef],
    ) -> ResolvedTypeArgSubstitutions {
        if type_args.len() > type_params.len() {
            self.diagnostics.push(format!(
                "{}: call `{callable}` type arity mismatch: expected {} type arguments, found {}",
                self.module_path,
                type_params.len(),
                type_args.len()
            ));
        }
        let mut complete = true;
        let mut types = BTreeMap::new();
        for (param, arg) in type_params.iter().zip(type_args) {
            match self
                .type_resolution
                .resolve_type_ref(arg, &self.type_context)
            {
                Ok(resolved) => {
                    types.insert(param.clone(), resolved.ir);
                }
                Err(_) => complete = false,
            }
        }
        ResolvedTypeArgSubstitutions { types, complete }
    }

    fn validate_resolved_call_params(
        &mut self,
        callable: &str,
        expected: Vec<(String, ResolvedTypeRef)>,
        args: &[Expr],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) {
        self.validate_resolved_call_params_with_projections(
            callable,
            expected,
            &[],
            args,
            arg_types,
        );
    }

    fn validate_resolved_call_params_with_projections(
        &mut self,
        callable: &str,
        expected: Vec<(String, ResolvedTypeRef)>,
        exact_expected: &[Option<PackageTypeRef>],
        args: &[Expr],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) {
        if expected.len() != args.len() {
            self.diagnostics.push(format!(
                "{}: call `{callable}` arity mismatch: expected {} arguments, found {}",
                self.module_path,
                expected.len(),
                args.len()
            ));
        }
        for (index, ((_, expected), (key, actual))) in expected.iter().zip(arg_types).enumerate() {
            let Some(actual) = actual else {
                continue;
            };
            if type_contains_type_param(&expected.ir) || type_contains_type_param(&actual.ir) {
                continue;
            }
            let context = format!("call `{callable}` argument {}", index + 1);
            self.check_value_assignable_to_expected(
                None,
                &args[index],
                key,
                actual,
                expected,
                exact_expected.get(index).and_then(Option::as_ref),
                &context,
                self.expression_span(key),
            );
        }
    }

    fn substitute_type_params_in_text(
        &self,
        raw: &str,
        type_params: &[String],
        type_args: &[TypeRef],
    ) -> String {
        if type_params.is_empty() || type_args.is_empty() {
            return raw.to_string();
        }
        let substitutions = type_params
            .iter()
            .zip(type_args)
            .map(|(param, arg)| {
                let resolved = self
                    .type_resolution
                    .resolve_type_ref(arg, &self.type_context)
                    .map(|ty| ty.source_text)
                    .unwrap_or_else(|_| arg.name.clone());
                (param.clone(), resolved)
            })
            .collect::<BTreeMap<_, _>>();
        TypeExpr::parse(raw)
            .map_named_types(|name| {
                substitutions
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.to_string())
            })
            .to_type_string()
    }

    fn config_intrinsic_call_type(
        &self,
        path: &str,
        type_args: &[TypeRef],
    ) -> Option<ResolvedTypeRef> {
        match path {
            "config.require" => type_args.first().and_then(|ty| {
                self.type_resolution
                    .resolve_type_ref(ty, &self.type_context)
                    .ok()
            }),
            "config.optional" => type_args
                .first()
                .and_then(|ty| {
                    self.type_resolution
                        .resolve_type_ref(ty, &self.type_context)
                        .ok()
                })
                .map(nullable_type),
            "config.has" => self.resolve_builtin("bool"),
            _ => None,
        }
    }

    fn runtime_receiver_call_type(
        &mut self,
        key: &ExpressionKey,
        callee: &Expr,
        args: &[Expr],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Option<ResolvedTypeRef> {
        let (_, method_name) = receiver_call_parts(callee)?;
        let offset = 1 + receiver_object_offset_in_callee(callee)?;
        let receiver_ty = self.expression_type_at_offset(key, offset)?;
        let return_type = builtin_receiver_call_return_type(&receiver_ty, method_name)?;
        let receiver_root = runtime_receiver_root_from_type_ref(&receiver_ty.ir);
        if receiver_root.as_deref() == Some("Array") && method_name == "push" {
            self.validate_array_push_args(&receiver_ty, args, arg_types);
        }
        if receiver_root.as_deref() == Some("string") && method_name == "contains" {
            self.validate_resolved_call_params(
                "string.contains",
                vec![(
                    "needle".to_string(),
                    resolved_type_from_ir(&builtin_type("string")),
                )],
                args,
                arg_types,
            );
        }
        if receiver_root.as_deref() == Some("JsonObject") {
            match method_name {
                "get" | "has" | "delete" => self.validate_resolved_call_params(
                    &format!("JsonObject.{method_name}"),
                    vec![(
                        "field".to_string(),
                        resolved_type_from_ir(&builtin_type("string")),
                    )],
                    args,
                    arg_types,
                ),
                "set" => self.validate_resolved_call_params(
                    "JsonObject.set",
                    vec![
                        (
                            "field".to_string(),
                            resolved_type_from_ir(&builtin_type("string")),
                        ),
                        (
                            "value".to_string(),
                            resolved_type_from_ir(&builtin_type("Json")),
                        ),
                    ],
                    args,
                    arg_types,
                ),
                _ => {}
            }
        }
        if receiver_root.as_deref() == Some("Map") && matches!(method_name, "has" | "set") {
            self.validate_map_has_or_set_args(&receiver_ty, method_name, args, arg_types);
        }
        if receiver_root.as_deref() == Some("bytes") && method_name == "toHex" {
            self.validate_resolved_call_params("bytes.toHex", Vec::new(), args, arg_types);
        }
        if let Some(projected) =
            self.expression_projection_at_offset(key, offset)
                .and_then(|receiver| {
                    builtin_receiver_call_return_projection(&receiver_ty, receiver, method_name)
                })
        {
            self.contract_projection
                .record_expression_type(key.clone(), projected);
        }
        Some(return_type)
    }

    fn actor_receiver_call_type(
        &mut self,
        key: &ExpressionKey,
        callee: &Expr,
        type_args: &[TypeRef],
        args: &[Expr],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Option<ResolvedTypeRef> {
        let (_, method_name) = receiver_call_parts(callee)?;
        let offset = 1 + receiver_object_offset_in_callee(callee)?;
        let receiver_ty = self.expression_type_at_offset(key, offset)?;
        let (params, return_type) = self.type_resolution.actor_method_signature(
            &receiver_ty,
            method_name,
            &self.type_context,
        )?;
        let callable = format!("{}.{}", receiver_ty.source_text, method_name);
        if !type_args.is_empty() {
            self.diagnostics.push(format!(
                "{}: actor method `{callable}` does not accept explicit method type arguments",
                self.module_path
            ));
        }
        let params = params
            .iter()
            .skip(usize::from(
                params.first().is_some_and(|param| param.name == "self"),
            ))
            .enumerate()
            .map(|(index, param)| {
                (
                    format!("arg{index}"),
                    ResolvedTypeRef {
                        source_text: type_ref_debug_text(&param.ty),
                        ir: param.ty.clone(),
                    },
                )
            })
            .collect();
        self.validate_resolved_call_params(&callable, params, args, arg_types);
        Some(ResolvedTypeRef {
            source_text: type_ref_debug_text(&return_type),
            ir: return_type,
        })
    }

    fn actor_registry_intrinsic_call_type(
        &mut self,
        path: &str,
        type_args: &[TypeRef],
        args: &[Expr],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Option<ResolvedTypeRef> {
        if type_args.len() != 1 {
            self.diagnostics.push(format!(
                "{}: actor registry intrinsic `{path}` expects exactly one actor type argument, found {}",
                self.module_path,
                type_args.len()
            ));
            return None;
        }
        let actor_ty = match self
            .type_resolution
            .resolve_type_ref(&type_args[0], &self.type_context)
        {
            Ok(actor_ty) => actor_ty,
            Err(error) => {
                self.diagnostics.push(format!(
                    "{}: actor registry intrinsic `{path}` has unresolved actor type: {error}",
                    self.module_path
                ));
                return None;
            }
        };
        let Some(actor) = self
            .type_resolution
            .actor_type_resolution(&actor_ty, &self.type_context)
        else {
            self.diagnostics.push(format!(
                "{}: actor registry intrinsic `{path}` type argument `{}` is not an actor declaration",
                self.module_path, actor_ty.source_text
            ));
            return None;
        };
        let needs_bootstrap = matches!(path, "std.actor.getOrCreate" | "std.actor.replace");
        let expected_arity = if needs_bootstrap { 2 } else { 1 };
        if args.len() != expected_arity {
            self.diagnostics.push(format!(
                "{}: actor registry intrinsic `{path}` expects {expected_arity} arguments, found {}",
                self.module_path,
                args.len()
            ));
        } else {
            let mut params = vec![("id".to_string(), actor.id_type.clone())];
            if needs_bootstrap {
                params.push((
                    "bootstrap".to_string(),
                    ResolvedTypeRef {
                        source_text: "{}".to_string(),
                        ir: TypeRefIr::Record {
                            fields: actor
                                .fields
                                .iter()
                                .map(|(name, ty)| (name.clone(), ty.ir.clone()))
                                .collect(),
                        },
                    },
                ));
            }
            self.validate_resolved_call_params(path, params, args, arg_types);
        }
        match path {
            "std.actor.find" => Some(nullable_type(actor.ty)),
            "std.actor.remove" => self.resolve_builtin("bool"),
            _ => Some(actor.ty),
        }
    }

    fn validate_array_push_args(
        &mut self,
        receiver_ty: &ResolvedTypeRef,
        args: &[Expr],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) {
        let Some(expected) =
            array_item_type_ir(&receiver_ty.ir).map(|ty| resolved_type_from_ir(&ty))
        else {
            return;
        };
        if args.len() != 1 {
            self.diagnostics.push(format!(
                "{}: call `Array.push` arity mismatch: expected 1 arguments, found {}",
                self.module_path,
                args.len()
            ));
            return;
        }
        let Some((key, Some(actual))) = arg_types.first() else {
            return;
        };
        self.check_value_assignable_to_expected(
            None,
            &args[0],
            key,
            actual,
            &expected,
            None,
            "call `Array.push` argument 1",
            self.expression_span(key),
        );
    }

    fn validate_map_has_or_set_args(
        &mut self,
        receiver_ty: &ResolvedTypeRef,
        method_name: &str,
        args: &[Expr],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) {
        let Some(key_ty) = map_key_type_ir(&receiver_ty.ir).map(|ty| resolved_type_from_ir(&ty))
        else {
            return;
        };
        let mut params = vec![("key".to_string(), key_ty)];
        if method_name == "set" {
            let Some(value_ty) =
                map_value_type_ir(&receiver_ty.ir).map(|ty| resolved_type_from_ir(&ty))
            else {
                return;
            };
            params.push(("value".to_string(), value_ty));
        }
        self.validate_resolved_call_params(&format!("Map.{method_name}"), params, args, arg_types);
    }

    fn any_interface_receiver_call_type(
        &mut self,
        key: &ExpressionKey,
        callee: &Expr,
        type_args: &[TypeRef],
        args: &[Expr],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Option<ResolvedTypeRef> {
        let (_, method_name) = receiver_call_parts(callee)?;
        let offset = 1 + receiver_object_offset_in_callee(callee)?;
        let receiver_ty = self.expression_type_at_offset(key, offset)?;
        let operation = self
            .type_resolution
            .any_interface_method_signature(&receiver_ty.ir, method_name)?;
        let callable = format!("{}.{}", receiver_ty.source_text, method_name);
        if !type_args.is_empty() {
            self.diagnostics.push(format!(
                "{}: any interface method `{callable}` does not accept method type arguments",
                self.module_path
            ));
        }
        let params = operation
            .params
            .iter()
            .skip(usize::from(
                operation
                    .params
                    .first()
                    .is_some_and(|param| param.name == "self"),
            ))
            .enumerate()
            .map(|(index, param)| {
                (
                    format!("arg{index}"),
                    ResolvedTypeRef {
                        source_text: type_ref_debug_text(&param.ty),
                        ir: param.ty.clone(),
                    },
                )
            })
            .collect();
        self.validate_resolved_call_params(&callable, params, args, arg_types);
        Some(ResolvedTypeRef {
            source_text: type_ref_debug_text(&operation.return_type),
            ir: operation.return_type,
        })
    }

    fn package_interface_receiver_call_type(
        &mut self,
        key: &ExpressionKey,
        callee: &Expr,
        type_args: &[TypeRef],
        args: &[Expr],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Option<ResolvedTypeRef> {
        let (_, method_name) = receiver_call_parts(callee)?;
        let offset = 1 + receiver_object_offset_in_callee(callee)?;
        let receiver_ty = self.expression_type_at_offset(key, offset)?;
        let interface = self
            .type_resolution
            .package_interface_for_type_ref(&receiver_ty.ir)?;
        let operation = interface
            .methods
            .iter()
            .find(|operation| operation.name == method_name)
            .cloned()?;
        let callable = format!("{}.{}", receiver_ty.source_text, method_name);
        let substitutions =
            self.resolve_type_arg_substitutions(&callable, &operation.type_params, type_args);
        if substitutions.complete {
            let params = operation
                .params
                .iter()
                .skip(usize::from(
                    operation
                        .params
                        .first()
                        .is_some_and(|param| param.name == "self"),
                ))
                .enumerate()
                .map(|(index, param)| {
                    let ty = substitute_type_params_in_ir(&param.ty, &substitutions.types);
                    (
                        format!("arg{index}"),
                        ResolvedTypeRef {
                            source_text: type_ref_debug_text(&ty),
                            ir: ty,
                        },
                    )
                })
                .collect();
            self.validate_resolved_call_params(&callable, params, args, arg_types);
        }
        let return_type =
            substitute_type_params_in_ir(&operation.return_type, &substitutions.types);
        Some(ResolvedTypeRef {
            source_text: type_ref_debug_text(&return_type),
            ir: return_type,
        })
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
        self.facts.get(&key)?.ty.clone()
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

    fn db_operation_type(
        &mut self,
        operation: &crate::shared::ast::DbOperation,
    ) -> Option<ResolvedTypeRef> {
        let target = self
            .type_resolution
            .resolve_type_ref(&operation.target, &self.type_context)
            .ok()?;
        if self
            .type_resolution
            .actor_type_resolution(&target, &self.type_context)
            .is_some()
        {
            self.diagnostics.push(format!(
                "{}: actor handle type `{}` cannot be used as a database object",
                self.module_path, target.source_text
            ));
            return None;
        }
        let read = self.db_read_type(operation, &target)?;
        match operation.op {
            crate::shared::ast::DbOperationKind::Find if operation.many => Some(array_type(read)),
            crate::shared::ast::DbOperationKind::Find
            | crate::shared::ast::DbOperationKind::Optional => Some(nullable_type(read)),
            crate::shared::ast::DbOperationKind::Require => Some(read),
            crate::shared::ast::DbOperationKind::Insert if operation.many => {
                self.resolve_builtin("DbInsertManyResult")
            }
            crate::shared::ast::DbOperationKind::Insert => Some(target.clone()),
            crate::shared::ast::DbOperationKind::Update if operation.many => {
                self.resolve_builtin("DbUpdateManyResult")
            }
            crate::shared::ast::DbOperationKind::Update
            | crate::shared::ast::DbOperationKind::Replace => Some(nullable_type(target.clone())),
            crate::shared::ast::DbOperationKind::Delete if operation.many => {
                self.resolve_builtin("DbDeleteManyResult")
            }
            crate::shared::ast::DbOperationKind::Upsert => {
                Some(projection_record_type("DbUpsertResult", &target))
            }
            crate::shared::ast::DbOperationKind::Delete
            | crate::shared::ast::DbOperationKind::Exists => self.resolve_builtin("bool"),
            crate::shared::ast::DbOperationKind::Count => self.resolve_builtin("number"),
        }
    }

    fn db_read_type(
        &mut self,
        operation: &crate::shared::ast::DbOperation,
        target: &ResolvedTypeRef,
    ) -> Option<ResolvedTypeRef> {
        let Some(projection) = operation.projection.as_ref() else {
            return Some(target.clone());
        };
        let paths = projection
            .fields
            .iter()
            .map(|field| field.segments.clone())
            .collect::<Vec<_>>();
        match DbProjectionTypeResolver::new(
            self.module_path,
            self.type_resolution,
            self.publication_db_metadata,
        )
        .project_read_type(&operation.target.name, target.ir.clone(), &paths)
        {
            Ok(ty) => Some(resolved_type_from_ir(&ty)),
            Err(error) => {
                self.diagnostics
                    .push(format!("{}: {error}", self.module_path));
                None
            }
        }
    }

    fn db_query_type(&self, target: &TypeRef) -> Option<ResolvedTypeRef> {
        let target = self
            .type_resolution
            .resolve_type_ref(target, &self.type_context)
            .ok()?;
        Some(projection_record_type("DbQuery", &target))
    }

    fn check_assignable(
        &mut self,
        annotation: &TypeRef,
        actual: &ResolvedTypeRef,
        exact_expected: Option<&PackageTypeRef>,
        span: SourceSpan,
        context: &str,
        value: Option<(&Expr, &ExpressionKey)>,
    ) {
        match self
            .type_resolution
            .resolve_type_ref(annotation, &self.type_context)
        {
            Ok(expected) => {
                if let Some((value, key)) = value {
                    self.check_value_assignable_to_expected(
                        Some(annotation),
                        value,
                        key,
                        actual,
                        &expected,
                        exact_expected,
                        context,
                        span,
                    );
                } else if !self.type_resolution.assignable_in_context(
                    actual,
                    &expected,
                    &self.type_context,
                ) {
                    self.push_type_mismatch(context, span, &expected, actual);
                }
            }
            Err(error) => self.diagnostics.push(format!(
                "{}: failed to resolve {context} annotation at {}: {error}",
                self.module_path,
                span_label(span)
            )),
        }
    }

    fn check_value_assignable_to_expected(
        &mut self,
        annotation: Option<&TypeRef>,
        value: &Expr,
        value_key: &ExpressionKey,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
        exact_expected: Option<&PackageTypeRef>,
        context: &str,
        fallback_span: SourceSpan,
    ) -> bool {
        if matches!(value, Expr::ObjectLiteral { .. }) {
            return self.materialize_target_typed_object_literal(
                annotation, value, value_key, actual, expected, context,
            );
        }
        let assignability = ExpressionAssignability::new(
            self.module_path,
            self.expression_sources,
            self.type_resolution,
            &self.type_context,
            None,
        );
        let expected_projected = match self.dependency_analysis {
            Some(dependency_analysis) => Some(match exact_expected {
                Some(expected) => Ok(expected.clone()),
                None => match annotation {
                    Some(annotation) => ContractProjectionState::project_source_type_ref(
                        annotation,
                        self.type_resolution,
                        dependency_analysis,
                        &self.type_context,
                    ),
                    None => ContractProjectionState::project_resolved_type(
                        expected,
                        self.type_resolution,
                        dependency_analysis,
                        &self.type_context,
                    ),
                },
            }),
            None => None,
        };
        let expected_projected = match expected_projected.transpose() {
            Ok(expected) => expected,
            Err(error) => {
                self.diagnostics.push(format!(
                    "{}: {context} exact source type projection failed at {}: {error}",
                    self.module_path,
                    span_label(fallback_span)
                ));
                return false;
            }
        };
        let contract_assignable = match contract_source_assignability_with_projections(
            actual,
            self.contract_projection.expression_type(value_key),
            expected,
            expected_projected.as_ref(),
            self.type_resolution,
            self.dependency_analysis,
            &self.type_context,
        ) {
            Ok(assignable) => assignable,
            Err(error) => {
                self.diagnostics.push(format!(
                    "{}: {context} exact source type projection failed at {}: {error}",
                    self.module_path,
                    span_label(fallback_span)
                ));
                return false;
            }
        };
        let assignable = match contract_assignable {
            Some(assignable) => assignable,
            None => match assignability
                .value_assignable_to_expected(annotation, value, actual, expected, None)
            {
                Ok(assignable) => assignable,
                Err(error) => {
                    self.diagnostics.push(format!(
                        "{}: {context} exact source type projection failed at {}: {error}",
                        self.module_path,
                        span_label(fallback_span)
                    ));
                    return false;
                }
            },
        };
        if assignable {
            return true;
        }
        if let Some(diagnostics) = assignability.object_literal_assignability_diagnostics(
            annotation, value, value_key, actual, expected, context,
        ) {
            if !diagnostics.is_empty() {
                self.diagnostics.extend(diagnostics);
                return false;
            }
        }
        if let (Some(expected), Some(actual)) = (
            expected_projected.as_ref(),
            self.contract_projection.expression_type(value_key),
        ) {
            self.diagnostics.push(format!(
                "{}: {context} canonical type identity mismatch at {}: expected {expected:?}, found {actual:?}",
                self.module_path,
                span_label(fallback_span),
            ));
        }
        self.push_type_mismatch(context, fallback_span, expected, actual);
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn materialize_target_typed_object_literal(
        &mut self,
        annotation: Option<&TypeRef>,
        value: &Expr,
        value_key: &ExpressionKey,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
        context: &str,
    ) -> bool {
        self.object_materialization
            .targeted
            .insert(value_key.clone());
        let plan = match ExpressionAssignability::new(
            self.module_path,
            self.expression_sources,
            self.type_resolution,
            &self.type_context,
            None,
        )
        .object_literal_materialization_plan(
            annotation, value, value_key, actual, expected, context,
        ) {
            Ok(plan) => plan,
            Err(diagnostics) => {
                self.diagnostics.extend(diagnostics);
                return false;
            }
        };
        let Some(source) = self.object_materialization.sources.get(value_key).cloned() else {
            self.diagnostics.push(format!(
                "{}: {context} target-typed object literal is missing source facts at {}",
                self.module_path,
                self.expression_span_label(value_key)
            ));
            return false;
        };
        let provided = source
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field))
            .collect::<BTreeMap<_, _>>();
        let mut fields = Vec::with_capacity(plan.fields.len());
        let mut valid = true;
        for (name, ty) in &plan.fields {
            let source = if let Some(provided) = provided.get(name.as_str()) {
                if let Some(actual) = &provided.actual {
                    valid &= self.check_value_assignable_to_expected(
                        None,
                        object_literal_field_value(value, name)
                            .expect("materialization plan field must exist in object literal"),
                        &provided.expression,
                        actual,
                        ty,
                        None,
                        &format!("{context} object literal field `{name}`"),
                        provided.value_span,
                    );
                } else {
                    // Exact contract-derived and flow-assigned bindings can be
                    // name-resolved without carrying a structural
                    // ResolvedTypeRef. Once the enclosing object has selected
                    // a unique target, retain that exact field target for the
                    // identifier instead of abandoning recursive
                    // materialization.
                    if !expression_accepts_contextual_target(
                        object_literal_field_value(value, name)
                            .expect("materialization plan field must exist in object literal"),
                    ) {
                        self.diagnostics.push(format!(
                            "{}: {context} object literal field `{name}` has no resolved expression type at {}",
                            self.module_path,
                            span_label(provided.value_span)
                        ));
                        valid = false;
                    }
                }
                ObjectFieldValueSource::Provided {
                    expression: provided.expression.clone(),
                }
            } else if self.type_resolution.is_nullable(ty) {
                ObjectFieldValueSource::SyntheticNull
            } else {
                self.diagnostics.push(format!(
                    "{}: {context} materialization plan omitted required object literal field `{name}` at {}",
                    self.module_path,
                    span_label(source.span)
                ));
                valid = false;
                continue;
            };
            fields.push(MaterializedObjectField {
                name: name.clone(),
                ty: ty.clone(),
                source,
            });
        }
        if valid {
            self.object_materialization.facts.insert(
                value_key.clone(),
                TargetTypedObjectMaterialization {
                    resolved_target: plan.resolved_target,
                    kind: plan.kind,
                    fields,
                },
            );
        }
        valid
    }

    fn push_type_mismatch(
        &mut self,
        context: &str,
        span: SourceSpan,
        expected: &ResolvedTypeRef,
        actual: &ResolvedTypeRef,
    ) {
        self.diagnostics.push(format!(
            "{}: {context} type mismatch at {}: expected {}, found {}",
            self.module_path,
            span_label(span),
            expected.source_text,
            actual.source_text
        ));
    }
}

fn package_callable_public_root(path: &str, source_symbol: &str) -> String {
    let suffix = format!(".{source_symbol}");
    if let Some(root) = path.strip_suffix(&suffix) {
        return root.to_string();
    }
    path.split('.').next().unwrap_or(path).to_string()
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
            let inherited = generic_type_params(&implementation.target);
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

fn insert_function_signature(
    signatures: &mut BTreeMap<String, CallableSignature>,
    module_path: &str,
    declaration_name: &str,
    function: &FunctionDecl,
    inherited_type_params: &[String],
) {
    let signature = CallableSignature {
        module_path: module_path.to_string(),
        declaration_name: declaration_name.to_string(),
        params: function
            .params
            .iter()
            .map(|param| CallableParam {
                ty: param.ty.clone(),
            })
            .collect(),
        return_type: function.return_type.clone(),
        type_params: inherited_type_params
            .iter()
            .chain(&function.type_params)
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
    let signature = CallableSignature {
        module_path: module_path.to_string(),
        declaration_name: declaration_name.to_string(),
        params: operation
            .params
            .iter()
            .map(|param| CallableParam {
                ty: param.ty.clone(),
            })
            .collect(),
        return_type: operation.return_type.clone(),
        type_params: inherited_type_params
            .iter()
            .chain(&operation.type_params)
            .cloned()
            .collect(),
    };
    insert_callable_signature(signatures, module_path, declaration_name, signature);
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

fn generic_type_params(name: &str) -> Vec<String> {
    crate::shared::type_syntax::generic_parts(name)
        .map(|parts| {
            parts
                .args
                .iter()
                .map(|arg| arg.trim())
                .filter(|arg| {
                    !arg.is_empty()
                        && arg
                            .chars()
                            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
                })
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn single_for_item_type(ty: &ResolvedTypeRef) -> Option<ResolvedTypeRef> {
    let TypeRefIr::Builtin { name, args } = &ty.ir else {
        return None;
    };
    match name.as_str() {
        "Array" | "Stream" | "std.collection.Array" | "std.stream.Stream" if args.len() == 1 => {
            Some(ResolvedTypeRef {
                ir: args[0].clone(),
                source_text: type_ref_debug_text(&args[0]),
            })
        }
        "Map" | "std.collection.Map" if args.len() == 2 => Some(ResolvedTypeRef {
            ir: args[0].clone(),
            source_text: type_ref_debug_text(&args[0]),
        }),
        _ => None,
    }
}

fn stream_chunk_type(ty: &ResolvedTypeRef) -> Option<ResolvedTypeRef> {
    let TypeRefIr::Builtin { name, args } = &ty.ir else {
        return None;
    };
    matches!(name.as_str(), "Stream" | "std.stream.Stream")
        .then_some(args)
        .filter(|args| args.len() == 1)
        .map(|args| ResolvedTypeRef {
            ir: args[0].clone(),
            source_text: type_ref_debug_text(&args[0]),
        })
}

fn map_entry_types(ty: &ResolvedTypeRef) -> Option<(ResolvedTypeRef, ResolvedTypeRef)> {
    let TypeRefIr::Builtin { name, args } = &ty.ir else {
        return None;
    };
    if name != "Map" || args.len() != 2 {
        return None;
    }
    Some((
        ResolvedTypeRef {
            ir: args[0].clone(),
            source_text: type_ref_debug_text(&args[0]),
        },
        ResolvedTypeRef {
            ir: args[1].clone(),
            source_text: type_ref_debug_text(&args[1]),
        },
    ))
}

fn single_for_item_projection(ty: &PackageTypeRef) -> Option<PackageTypeRef> {
    let PackageTypeRef::Container { name, arguments } = ty else {
        return None;
    };
    match name.as_str() {
        "Array" | "Stream" | "std.collection.Array" | "std.stream.Stream"
            if arguments.len() == 1 =>
        {
            Some(arguments[0].clone())
        }
        "Map" | "std.collection.Map" if arguments.len() == 2 => Some(arguments[0].clone()),
        _ => None,
    }
}

fn map_entry_projections(ty: &PackageTypeRef) -> Option<(PackageTypeRef, PackageTypeRef)> {
    let PackageTypeRef::Container { name, arguments } = ty else {
        return None;
    };
    (matches!(name.as_str(), "Map" | "std.collection.Map") && arguments.len() == 2)
        .then(|| (arguments[0].clone(), arguments[1].clone()))
}

fn type_contains_type_param(ty: &TypeRefIr) -> bool {
    match ty {
        TypeRefIr::TypeParam { .. } => true,
        TypeRefIr::Builtin { args, .. } | TypeRefIr::Union { items: args } => {
            args.iter().any(type_contains_type_param)
        }
        TypeRefIr::Nullable { inner } => type_contains_type_param(inner),
        TypeRefIr::AnyInterface { interface } => interface
            .canonical_type_args
            .iter()
            .any(type_contains_type_param),
        TypeRefIr::Record { fields } => fields.values().any(type_contains_type_param),
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            params
                .iter()
                .any(|param| type_contains_type_param(&param.ty))
                || type_contains_type_param(return_type)
        }
        TypeRefIr::Literal { .. }
        | TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::PackageSymbol { .. } => false,
    }
}

fn native_return_type_context<'a>(
    path: &'a str,
    fallback: &TypeResolutionContext<'a>,
) -> TypeResolutionContext<'a> {
    path.rsplit_once('.')
        .and_then(|(owner, _)| {
            prelude_registry()
                .type_decl_module(owner)
                .or_else(|| (!prelude_registry().is_prelude_type_name(owner)).then_some(owner))
        })
        .map(|module_path| {
            TypeResolutionContext::with_type_params(module_path, fallback.type_params.clone())
        })
        .unwrap_or_else(|| {
            TypeResolutionContext::with_type_params(
                fallback.module_path,
                fallback.type_params.clone(),
            )
        })
}

fn projection_record_type(name: &str, target: &ResolvedTypeRef) -> ResolvedTypeRef {
    ResolvedTypeRef {
        ir: TypeRefIr::Builtin {
            name: name.to_string(),
            args: vec![target.ir.clone()],
        },
        source_text: format!("{name}<{}>", target.source_text),
    }
}

fn catch_result_type(value: ResolvedTypeRef, error: ResolvedTypeRef) -> ResolvedTypeRef {
    ResolvedTypeRef {
        ir: TypeRefIr::Builtin {
            name: "CatchResult".to_string(),
            args: vec![value.ir, error.ir],
        },
        source_text: format!("CatchResult<{}, {}>", value.source_text, error.source_text),
    }
}

fn record_field_type_from_ir(ty: &TypeRefIr, field: &str) -> Option<ResolvedTypeRef> {
    match ty {
        TypeRefIr::Record { fields } => fields.get(field).map(resolved_type_from_ir),
        TypeRefIr::Union { items } => {
            let mut field_types = Vec::new();
            for item in items {
                field_types.push(record_field_type_from_ir(item, field)?.ir);
            }
            Some(resolved_type_from_ir(&union_type_ir(field_types)))
        }
        TypeRefIr::Builtin { name, args } if name == "CatchResult" && args.len() == 2 => {
            match field {
                "tag" => Some(resolved_type_from_ir(&union_type_ir(vec![
                    literal_string_type("ok"),
                    literal_string_type("err"),
                ]))),
                _ => None,
            }
        }
        TypeRefIr::Builtin { name, args } if name == "DbUpsertResult" && args.len() == 1 => {
            match field {
                "inserted" => Some(resolved_type_from_ir(&TypeRefIr::Builtin {
                    name: "bool".to_string(),
                    args: Vec::new(),
                })),
                "value" => Some(resolved_type_from_ir(&args[0])),
                _ => None,
            }
        }
        TypeRefIr::Builtin { name, args } if name == "Exception" && args.len() == 1 => {
            match field {
                "error" => Some(resolved_type_from_ir(&args[0])),
                _ => None,
            }
        }
        _ => None,
    }
}

fn receiver_call_parts(expr: &Expr) -> Option<(&Expr, &str)> {
    match expr {
        Expr::Field { object, field } => Some((object, field)),
        Expr::Generic { callee, .. } => receiver_call_parts(callee),
        _ => None,
    }
}

fn receiver_object_offset_in_callee(expr: &Expr) -> Option<u32> {
    match expr {
        Expr::Field { .. } => Some(1),
        Expr::Generic { callee, .. } => receiver_object_offset_in_callee(callee).map(|offset| {
            offset
                .checked_add(1)
                .expect("receiver expression preorder offset should fit in u32")
        }),
        _ => None,
    }
}

fn builtin_receiver_call_return_type(
    receiver_ty: &ResolvedTypeRef,
    method_name: &str,
) -> Option<ResolvedTypeRef> {
    let root = runtime_receiver_root_from_type_ref(&receiver_ty.ir)?;
    let spec = builtin_receiver_op_spec_by_name(&root, method_name)?;
    let ty = match spec.public_return_type {
        BuiltinReceiverPublicReturnType::Fixed(name) => builtin_type(name),
        BuiltinReceiverPublicReturnType::Receiver => receiver_ty.ir.clone(),
        BuiltinReceiverPublicReturnType::ArrayItem => array_item_type_ir(&receiver_ty.ir)?,
        BuiltinReceiverPublicReturnType::MapValue => map_value_type_ir(&receiver_ty.ir)?,
        BuiltinReceiverPublicReturnType::MapKeyArray => TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![map_key_type_ir(&receiver_ty.ir)?],
        },
    };
    Some(resolved_type_from_ir(&ty))
}

fn builtin_receiver_call_return_projection(
    receiver_ty: &ResolvedTypeRef,
    receiver_projection: &PackageTypeRef,
    method_name: &str,
) -> Option<PackageTypeRef> {
    let root = runtime_receiver_root_from_type_ref(&receiver_ty.ir)?;
    let spec = builtin_receiver_op_spec_by_name(&root, method_name)?;
    match spec.public_return_type {
        BuiltinReceiverPublicReturnType::Fixed(name) => Some(PackageTypeRef::Container {
            name: name.to_string(),
            arguments: Vec::new(),
        }),
        BuiltinReceiverPublicReturnType::Receiver => Some(receiver_projection.clone()),
        BuiltinReceiverPublicReturnType::ArrayItem => {
            let PackageTypeRef::Container { arguments, .. } = receiver_projection else {
                return None;
            };
            (arguments.len() == 1).then(|| arguments[0].clone())
        }
        BuiltinReceiverPublicReturnType::MapValue => {
            let PackageTypeRef::Container { arguments, .. } = receiver_projection else {
                return None;
            };
            (arguments.len() == 2).then(|| arguments[1].clone())
        }
        BuiltinReceiverPublicReturnType::MapKeyArray => {
            let PackageTypeRef::Container { arguments, .. } = receiver_projection else {
                return None;
            };
            (arguments.len() == 2).then(|| PackageTypeRef::Container {
                name: "Array".to_string(),
                arguments: vec![arguments[0].clone()],
            })
        }
    }
}

pub fn runtime_receiver_root_from_type_ref(ty: &TypeRefIr) -> Option<String> {
    match ty {
        TypeRefIr::Builtin { name, .. } => Some(canonical_runtime_receiver_root(name).to_string()),
        TypeRefIr::PackageSymbol { symbol } if is_official_std_package_ref(&symbol.package) => {
            Some(canonical_runtime_receiver_root(&symbol.symbol_path).to_string())
        }
        TypeRefIr::ServiceSymbol { symbol }
            if prelude_registry()
                .known_type_symbol(&format!("{}.{}", symbol.module_path, symbol.symbol))
                == Some(format!("{}.{}", symbol.module_path, symbol.symbol)) =>
        {
            Some(
                canonical_runtime_receiver_root(&format!(
                    "{}.{}",
                    symbol.module_path, symbol.symbol
                ))
                .to_string(),
            )
        }
        TypeRefIr::Literal {
            value: LiteralIr::String { .. },
        } => Some("string".to_string()),
        TypeRefIr::Literal {
            value: LiteralIr::Number { .. },
        } => Some("number".to_string()),
        TypeRefIr::Nullable { inner } => runtime_receiver_root_from_type_ref(inner),
        _ => None,
    }
}

fn is_official_std_package_ref(package: &PackageRefIr) -> bool {
    match package {
        PackageRefIr::PackageId { package_id } => {
            package_id == crate::shared::id::SKIFF_STD_PUBLICATION_ID
        }
        PackageRefIr::Dependency { dependency_ref } => dependency_ref == "std",
    }
}

fn canonical_runtime_receiver_root(root: &str) -> &str {
    skiff_artifact_model::canonical_runtime_receiver_root(root)
}

fn array_item_type_ir(ty: &TypeRefIr) -> Option<TypeRefIr> {
    let TypeRefIr::Builtin { name, args } = ty else {
        return None;
    };
    (matches!(
        name.as_str(),
        "Array" | "Stream" | "std.collection.Array" | "std.stream.Stream"
    ) && args.len() == 1)
        .then(|| args[0].clone())
}

fn map_value_type_ir(ty: &TypeRefIr) -> Option<TypeRefIr> {
    let TypeRefIr::Builtin { name, args } = ty else {
        return None;
    };
    (matches!(name.as_str(), "Map" | "std.collection.Map") && args.len() == 2)
        .then(|| args[1].clone())
}

fn map_key_type_ir(ty: &TypeRefIr) -> Option<TypeRefIr> {
    let TypeRefIr::Builtin { name, args } = ty else {
        return None;
    };
    (matches!(name.as_str(), "Map" | "std.collection.Map") && args.len() == 2)
        .then(|| args[0].clone())
}

fn non_nullable_type(ty: &ResolvedTypeRef) -> Option<ResolvedTypeRef> {
    match &ty.ir {
        TypeRefIr::Nullable { inner } => {
            let source_text = ty
                .source_text
                .trim()
                .strip_suffix('?')
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| type_ref_debug_text(inner));
            Some(ResolvedTypeRef {
                ir: inner.as_ref().clone(),
                source_text,
            })
        }
        TypeRefIr::Union { items } => {
            let remaining = items
                .iter()
                .filter(|item| !type_ir_is_null(item))
                .cloned()
                .collect::<Vec<_>>();
            (remaining.len() != items.len())
                .then(|| resolved_type_from_ir(&union_type_ir(remaining)))
        }
        _ => None,
    }
}

fn narrow_type_by_tag(
    ty: &ResolvedTypeRef,
    tag_value: &str,
    include_matching: bool,
) -> Option<ResolvedTypeRef> {
    let branches = discriminated_record_branches(&ty.ir)?;
    let selected = branches
        .into_iter()
        .filter(|branch| {
            record_tag_literal(branch).is_some_and(|value| (value == tag_value) == include_matching)
        })
        .collect::<Vec<_>>();
    (!selected.is_empty()).then(|| resolved_type_from_ir(&union_type_ir(selected)))
}

fn discriminated_record_branches(ty: &TypeRefIr) -> Option<Vec<TypeRefIr>> {
    match ty {
        TypeRefIr::Union { items } => Some(items.clone()),
        TypeRefIr::Builtin { name, args } if name == "CatchResult" && args.len() == 2 => {
            Some(catch_result_branch_types(&args[0], &args[1]))
        }
        TypeRefIr::Record { .. } => Some(vec![ty.clone()]),
        _ => None,
    }
}

fn catch_result_branch_types(value: &TypeRefIr, error: &TypeRefIr) -> Vec<TypeRefIr> {
    vec![
        TypeRefIr::Record {
            fields: record_type_fields([
                ("tag", literal_string_type("ok")),
                ("value", value.clone()),
            ]),
        },
        TypeRefIr::Record {
            fields: record_type_fields([
                ("tag", literal_string_type("err")),
                ("exception", exception_type_ir(error.clone())),
            ]),
        },
    ]
}

fn record_tag_literal(ty: &TypeRefIr) -> Option<&str> {
    let TypeRefIr::Record { fields } = ty else {
        return None;
    };
    let TypeRefIr::Literal {
        value: LiteralIr::String { value },
    } = fields.get("tag")?
    else {
        return None;
    };
    Some(value.as_str())
}

fn resolved_type_from_ir(ty: &TypeRefIr) -> ResolvedTypeRef {
    ResolvedTypeRef {
        ir: ty.clone(),
        source_text: type_ref_debug_text(ty),
    }
}

fn union_type_ir(mut items: Vec<TypeRefIr>) -> TypeRefIr {
    items.sort_by_key(type_ref_debug_text);
    items.dedup();
    match items.as_slice() {
        [only] => only.clone(),
        _ => TypeRefIr::Union { items },
    }
}

fn exception_type_ir(error: TypeRefIr) -> TypeRefIr {
    TypeRefIr::Builtin {
        name: "Exception".to_string(),
        args: vec![error],
    }
}

fn nullable_type(inner: ResolvedTypeRef) -> ResolvedTypeRef {
    ResolvedTypeRef {
        ir: TypeRefIr::Nullable {
            inner: Box::new(inner.ir),
        },
        source_text: format!("{}?", inner.source_text),
    }
}

fn db_lease_read_type() -> ResolvedTypeRef {
    let string = TypeRefIr::Builtin {
        name: "string".to_string(),
        args: Vec::new(),
    };
    nullable_type(ResolvedTypeRef {
        ir: TypeRefIr::Record {
            fields: BTreeMap::from([
                ("expiresAt".to_string(), string.clone()),
                ("owner".to_string(), string.clone()),
                ("requestId".to_string(), string),
            ]),
        },
        source_text: "{ expiresAt: string, owner: string, requestId: string }".to_string(),
    })
}

fn array_type(item: ResolvedTypeRef) -> ResolvedTypeRef {
    ResolvedTypeRef {
        ir: TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![item.ir],
        },
        source_text: format!("Array<{}>", item.source_text),
    }
}

fn object_literal_key_text(key: &crate::shared::ast::ObjectLiteralKey) -> Option<String> {
    match key {
        crate::shared::ast::ObjectLiteralKey::Name(name) => Some(name.clone()),
    }
}

fn object_literal_field_value<'a>(value: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::ObjectLiteral { entries } = value else {
        return None;
    };
    entries.iter().find_map(|entry| {
        (object_literal_key_text(&entry.key).as_deref() == Some(name)).then_some(&entry.value)
    })
}

fn expression_accepts_contextual_target(value: &Expr) -> bool {
    matches!(value, Expr::Identifier(_))
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
        Expr::Literal(_) | Expr::Record { .. } | Expr::ObjectLiteral { .. } => true,
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

fn qualify_package_signature_type_text(
    raw: &str,
    package_root: &str,
    local_type_names: &BTreeSet<String>,
) -> String {
    TypeExpr::parse(raw)
        .map_named_types(|name| {
            if local_type_names.contains(name) {
                format!("{package_root}.{name}")
            } else {
                name.to_string()
            }
        })
        .to_type_string()
}

fn type_ref_debug_text(ty: &TypeRefIr) -> String {
    match ty {
        TypeRefIr::Builtin { name, args } if args.is_empty() => name.clone(),
        TypeRefIr::Builtin { name, args } => format!(
            "{name}<{}>",
            args.iter()
                .map(type_ref_debug_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::Nullable { inner } => format!("{}?", type_ref_debug_text(inner)),
        TypeRefIr::Union { items } => items
            .iter()
            .map(type_ref_debug_text)
            .collect::<Vec<_>>()
            .join(" | "),
        TypeRefIr::Literal {
            value: LiteralIr::String { value },
        } => serde_json::to_string(value).unwrap_or_else(|_| "\"<string>\"".to_string()),
        TypeRefIr::Literal {
            value: LiteralIr::Null,
        } => "null".to_string(),
        TypeRefIr::Literal { .. } => "<literal>".to_string(),
        TypeRefIr::LocalType { type_index } => format!("#{type_index}"),
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => format!("{module_path}#{type_index}"),
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            symbol.symbol_path()
        }
        TypeRefIr::PackageSymbol { symbol } => symbol.symbol_path.clone(),
        TypeRefIr::AnyInterface { interface } => {
            let interface_name = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
                .map_or_else(
                    |_| interface.interface_abi_id.clone(),
                    |identity| type_ref_debug_text(&identity),
                );
            if interface.canonical_type_args.is_empty() {
                format!("any {interface_name}")
            } else {
                format!(
                    "any {}<{}>",
                    interface_name,
                    interface
                        .canonical_type_args
                        .iter()
                        .map(type_ref_debug_text)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        TypeRefIr::Record { .. } => "{}".to_string(),
        TypeRefIr::TypeParam { name } => name.clone(),
        TypeRefIr::Function { .. } => "fn".to_string(),
    }
}

fn resolved_package_type_ref(ty: &PackageTypeRef) -> ResolvedTypeRef {
    let ir = package_type_ref_ir(ty);
    ResolvedTypeRef {
        source_text: type_ref_debug_text(&ir),
        ir,
    }
}

fn package_type_ref_ir(ty: &PackageTypeRef) -> TypeRefIr {
    match ty {
        PackageTypeRef::Local { local_type } => local_type.clone(),
        PackageTypeRef::PackageSchema {
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
        PackageTypeRef::Container { name, arguments } => TypeRefIr::Builtin {
            name: name.clone(),
            args: arguments.iter().map(package_type_ref_ir).collect(),
        },
        PackageTypeRef::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(package_type_ref_ir(inner)),
        },
        PackageTypeRef::AnyInterface {
            interface,
            arguments,
        } => TypeRefIr::AnyInterface {
            interface: skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: serde_json::to_string(&package_type_ref_ir(interface))
                    .expect("PackageTypeRef interface identity must serialize"),
                canonical_type_args: arguments.iter().map(package_type_ref_ir).collect(),
            },
        },
    }
}

fn package_type_contains_local_slot(ty: &PackageTypeRef) -> bool {
    match ty {
        PackageTypeRef::Local { local_type } => type_ir_contains_local_slot(local_type),
        PackageTypeRef::PackageSchema { .. } => false,
        PackageTypeRef::Container { arguments, .. } => {
            arguments.iter().any(package_type_contains_local_slot)
        }
        PackageTypeRef::Nullable { inner } => package_type_contains_local_slot(inner),
        PackageTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            package_type_contains_local_slot(interface)
                || arguments.iter().any(package_type_contains_local_slot)
        }
    }
}

fn type_ir_contains_local_slot(ty: &TypeRefIr) -> bool {
    match ty {
        TypeRefIr::LocalType { .. } | TypeRefIr::ServiceSymbol { .. } => true,
        TypeRefIr::Builtin { args, .. } | TypeRefIr::Union { items: args } => {
            args.iter().any(type_ir_contains_local_slot)
        }
        TypeRefIr::Nullable { inner } => type_ir_contains_local_slot(inner),
        TypeRefIr::Record { fields } => fields.values().any(type_ir_contains_local_slot),
        _ => false,
    }
}

fn builtin_type(name: &str) -> TypeRefIr {
    TypeRefIr::Builtin {
        name: name.to_string(),
        args: Vec::new(),
    }
}

fn literal_string_type(value: &str) -> TypeRefIr {
    TypeRefIr::Literal {
        value: LiteralIr::String {
            value: value.to_string(),
        },
    }
}

fn type_ir_is_void_or_null(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Builtin { name, args } if args.is_empty() && (name == "void" || name == "null"))
        || type_ir_is_null(ty)
}

fn type_ir_is_never(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Builtin { name, args } if args.is_empty() && name == "never")
}

fn type_ir_is_null(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Builtin { name, .. } if name == "null")
        || matches!(
            ty,
            TypeRefIr::Literal {
                value: LiteralIr::Null
            }
        )
}

fn record_field_name_source_span(
    fact: Option<&super::ExpressionSourceFact>,
    index: usize,
) -> SourceSpan {
    fact.and_then(|fact| fact.record_fields.get(index))
        .map(|field| field.name_span)
        .unwrap_or_else(SourceSpan::synthetic)
}

fn record_field_value_source_span(
    fact: Option<&super::ExpressionSourceFact>,
    index: usize,
) -> SourceSpan {
    fact.and_then(|fact| fact.record_fields.get(index))
        .map(|field| field.value_span)
        .unwrap_or_else(SourceSpan::synthetic)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf};

    use crate::{
        parsed_sources::parse_publication_sources, prelude_registry::initialize_prelude_registry,
        publication_db_metadata_index, source_graph::CompilerSourceFile,
        PublicationDbMetadataIndex, PublicationTypeSymbolIndex,
    };
    use skiff_compiler_input::CompilerPlatformSources;

    use super::*;

    const ANY_INTERFACE_MODULE: &str = "internal.any_interface";

    fn expression_type_result(
        source_text: &str,
    ) -> Result<ExpressionTypeModel, ExpressionTypeModelBuildError> {
        expression_type_result_with_source_role(source_text, false)
    }

    fn test_expression_type_result(
        source_text: &str,
    ) -> Result<ExpressionTypeModel, ExpressionTypeModelBuildError> {
        expression_type_result_with_source_role(source_text, true)
    }

    fn expression_type_result_with_source_role(
        source_text: &str,
        is_test_file: bool,
    ) -> Result<ExpressionTypeModel, ExpressionTypeModelBuildError> {
        let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace root resolves");
        let platform_sources =
            CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load");
        initialize_prelude_registry(&platform_sources).expect("prelude registry initializes");
        let relative_path = if is_test_file {
            "internal/any_interface.test.skiff"
        } else {
            "internal/any_interface.skiff"
        };
        let module_path = if is_test_file {
            "internal.any_interface.__test"
        } else {
            ANY_INTERFACE_MODULE
        };
        let source = CompilerSourceFile::parse(
            PathBuf::from(relative_path),
            module_path.to_string(),
            false,
            is_test_file,
            source_text.to_string(),
            relative_path,
        )
        .expect("test source should parse");
        let parsed_sources = parse_publication_sources(&PathBuf::from("/test"), &[source])
            .expect("test source should build parsed source facts");
        let type_resolution = TypeResolutionModel::build(
            &parsed_sources,
            &BTreeMap::new(),
            &[],
            None,
            None,
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type resolution should build");
        let expression_sources = ExpressionSourceMap::build(&parsed_sources)
            .expect("expression source facts should build");
        let db_metadata = publication_db_metadata_index(
            parsed_sources
                .iter()
                .map(|source| (source.module_path(), source.ast())),
            &BTreeMap::new(),
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("DB metadata should build");
        ExpressionTypeModel::build(
            &parsed_sources,
            &expression_sources,
            &type_resolution,
            &db_metadata,
            None,
        )
    }

    fn boxing_source(body: &str) -> String {
        format!(
            r#"
              interface Provider {{
                function name(self: Self) -> string
              }}

              type Host implements Provider {{
                label: string,
              }}

              impl Host {{
                function name() -> string {{ return self.label }}
              }}

              type Other {{
                label: string,
              }}

              {body}
            "#
        )
    }

    #[test]
    fn actor_self_field_assignment_requires_declared_field_type() {
        let error = expression_type_result(
            r#"
              actor Counter id string {
                count: number,
              }

              impl Counter {
                function corrupt() -> void {
                  self.count = "not a number"
                }
              }
            "#,
        )
        .expect_err("Actor self field assignment must be type checked");
        assert!(
            error
                .message()
                .contains("self field assignment type mismatch"),
            "unexpected diagnostic: {}",
            error.message()
        );
    }

    #[test]
    fn explicit_actor_registry_intrinsics_return_nominal_handles() {
        expression_type_result(
            r#"
              actor UserActor id string {
                displayName: string,
                loginCount: number,
              }

              impl UserActor {
                function label() -> string { return self.displayName }
              }

              function load(id: string) -> UserActor {
                const actor: UserActor = std.actor.getOrCreate<UserActor>(
                  id,
                  { displayName: "Ada", loginCount: 1 }
                )
                const label: string = actor.label()
                const found: UserActor? = std.actor.find<UserActor>(id)
                const removed: bool = std.actor.remove<UserActor>(id)
                return actor
              }
            "#,
        )
        .expect("actor declarations should be nominal handle types for registry results");
    }

    #[test]
    fn actor_registry_intrinsics_reject_non_actor_wrong_id_and_bootstrap_shape() {
        let error = expression_type_result(
            r#"
              type User { id: string }
              actor UserActor id string { displayName: string }

              function invalid() -> void {
                std.actor.find<User>("u1")
                std.actor.find<UserActor>(42)
                std.actor.replace<UserActor>("u1", { displayName: 42 })
                const actor = std.actor.getOrCreate<UserActor>("u1", { displayName: "Ada" })
                const leaked = actor.displayName
                const stored = db require UserActor("u1")
              }
            "#,
        )
        .expect_err("invalid actor registry uses must fail");
        let message = error.message();
        assert!(message.contains("is not an actor declaration"), "{message}");
        assert!(message.contains("argument 1"), "{message}");
        assert!(
            message.contains("argument 2 object literal field"),
            "{message}"
        );
        assert!(message.contains("unknown field `displayName`"), "{message}");
        assert!(
            message.contains("cannot be used as a database object"),
            "{message}"
        );
    }

    #[test]
    fn explicit_actor_cannot_be_constructed_as_a_record() {
        let error = expression_type_result(
            r#"
              actor UserActor id string { displayName: string }
              function invalid() -> UserActor {
                return UserActor { displayName: "Ada" }
              }
            "#,
        )
        .expect_err("ordinary actor construction must fail");
        assert!(
            error.message().contains("nominal handle")
                && error.message().contains("cannot be constructed directly"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn typed_catch_value_requires_and_respects_tag_narrowing() {
        expression_type_result(
            r#"
              type Payload { value: string }

              function make() -> Payload {
                return Payload { value: "ok" }
              }

              function equalBranch() -> Payload? {
                const attempted = catch<string>(make())
                if attempted.tag == "ok" { return attempted.value }
                return null
              }

              function reverseComparison() -> Payload? {
                const attempted = catch<string>(make())
                if "ok" != attempted.tag { return null }
                return attempted.value
              }

              function earlyReturn() -> Payload? {
                const attempted = catch<string>(make())
                if attempted.tag != "ok" { return null }
                return attempted.value
              }

              function nestedCatch() -> Payload? {
                const outer = catch<string>(equalBranch())
                if outer.tag != "ok" { return null }
                return outer.value
              }
            "#,
        )
        .expect("ok-tag branches must expose the exact catch success type");

        let unnarrowed = expression_type_result(
            r#"
              type Payload { value: string }
              function make() -> Payload { return Payload { value: "ok" } }
              function invalid() -> Payload {
                const attempted = catch<string>(make())
                return attempted.value
              }
            "#,
        )
        .expect_err("an un-narrowed catch result must not expose value")
        .message();
        assert!(
            unnarrowed.contains("unknown field `value` on CatchResult"),
            "{unnarrowed}"
        );

        let error_branch = expression_type_result(
            r#"
              type Payload { value: string }
              function make() -> Payload { return Payload { value: "ok" } }
              function invalid() -> Payload? {
                const attempted = catch<string>(make())
                if attempted.tag == "err" { return attempted.value }
                return null
              }
            "#,
        )
        .expect_err("the error branch must not expose the success value")
        .message();
        assert!(
            error_branch.contains("unknown field `value`"),
            "{error_branch}"
        );
    }

    #[test]
    fn test_assertion_true_flow_narrows_stable_bindings() {
        test_expression_type_result(
            r#"
              type Payload { value: string }

              function make() -> Payload {
                return Payload { value: "ok" }
              }

              function maybe() -> Payload? {
                return make()
              }

              test "nullable local" {
                const value: Payload? = maybe()
                assert value != null
                assert value.value == "ok"
              }

              test "tagged catch result" {
                const attempted = catch<string>(make())
                assert attempted.tag == "ok"
                assert attempted.value.value == "ok"
              }

              test "conjunction" {
                const value: Payload? = maybe()
                const attempted = catch<string>(make())
                assert value != null && attempted.tag == "ok"
                assert value.value == attempted.value.value
              }

              test "nested test block" {
                const value: Payload? = maybe()
                if true {
                  assert value != null
                  assert value.value == "ok"
                }
              }
            "#,
        )
        .expect("assertions in tests must carry their true-flow narrowing forward");
    }

    #[test]
    fn test_assertion_narrowing_fails_closed_for_invalidated_or_unstable_values() {
        let cases = [
            (
                r#"
                  type Payload { value: string }
                  function maybe() -> Payload? { return Payload { value: "ok" } }
                  test "opposite null assertion" {
                    const value: Payload? = maybe()
                    assert value == null
                    assert value.value == "ok"
                  }
                "#,
                "opposite null assertion",
            ),
            (
                r#"
                  type Payload { value: string }
                  function maybe() -> Payload? { return Payload { value: "ok" } }
                  test "unstable call" {
                    assert maybe() != null
                    assert maybe().value == "ok"
                  }
                "#,
                "unstable call expression",
            ),
            (
                r#"
                  type Payload { value: string }
                  function maybe() -> Payload? { return Payload { value: "ok" } }
                  test "reassignment" {
                    let value: Payload? = maybe()
                    assert value != null
                    value = null
                    assert value.value == "ok"
                  }
                "#,
                "reassignment",
            ),
            (
                r#"
                  type Payload { value: string }
                  function maybe() -> Payload? { return Payload { value: "ok" } }
                  test "branch merge" {
                    const value: Payload? = maybe()
                    if true {
                      assert value != null
                    }
                    assert value.value == "ok"
                  }
                "#,
                "branch merge",
            ),
        ];

        for (source, label) in cases {
            let error = test_expression_type_result(source)
                .expect_err("invalid assert narrowing must fail closed")
                .message();
            assert!(
                error.contains("nullable") || error.contains("unknown field"),
                "{label} should retain the unsafe optional type, got:\n{error}"
            );
        }
    }

    #[test]
    fn self_field_resolution_keeps_actor_and_record_owners_distinct() {
        expression_type_result(
            r#"
              type User { name: string }
              type Box<T> { value: T }
              actor UserActor id string { name: string }

              impl User {
                function name() -> string { return self.name }
              }
              impl Box<T> {
                function get() -> T { return self.value }
              }
              impl UserActor {
                function name() -> string { return self.name }
              }
            "#,
        )
        .expect("ordinary, generic, and actor self fields must use their canonical static owner");

        let error = expression_type_result(
            r#"
              type User { name: string }
              actor UserActor id string { name: string }

              impl User {
                function invalid() -> string { return self.missing }
              }
              impl UserActor {
                function invalid() -> string { return self.missing }
              }
            "#,
        )
        .expect_err("unknown ordinary and actor self fields must both fail closed")
        .message();
        assert!(error.contains("unknown field `missing` on User"), "{error}");
        assert!(
            error.contains("unknown field `missing` on UserActor"),
            "{error}"
        );
    }

    #[test]
    fn db_read_projection_publishes_selected_fields_and_automatic_key() {
        expression_type_result(
            r#"
              type Credential {
                id: string,
                label: string,
                apiKey: string,
              }

              db object Credential {
                primary key(id)
                storage apiKey using encrypted
              }

              function projected(id: string) -> { id: string, apiKey: string } {
                const credential = db require Credential(id) {
                  fields { apiKey }
                }
                return { id: credential.id, apiKey: credential.apiKey }
              }
            "#,
        )
        .expect("projected fields and the automatic key should be available to source typing");
    }

    #[test]
    fn db_read_projection_preserves_nested_nullable_and_many_wrappers() {
        expression_type_result(
            r#"
              type Profile {
                displayName: string,
                ignored: number,
              }

              type User {
                id: string,
                profile: Profile?,
              }

              db object User {
                primary key(id)
              }

              function projectedMany() -> Array<{ id: string, profile: { displayName: string }? }> {
                return db find many User {
                  fields { profile.displayName }
                }
              }

              function projectedOptional(id: string) -> { id: string, profile: { displayName: string }? }? {
                return db optional User(id) {
                  fields { profile.displayName }
                }
              }
            "#,
        )
        .expect("nested projected shape should preserve nullable and many wrappers");
    }

    #[test]
    fn db_read_projection_rejects_unknown_duplicate_and_parent_child_paths() {
        let source = |fields: &str| {
            format!(
                r#"
                  type Profile {{ displayName: string }}
                  type User {{ id: string, profile: Profile, label: string }}
                  db object User {{ primary key(id) }}

                  function projected(id: string) -> void {{
                    db require User(id) {{ fields {{ {fields} }} }}
                  }}
                "#
            )
        };

        for (fields, expected) in [
            (
                "missing",
                "db projection references unknown field `missing`",
            ),
            ("label, label", "duplicate db projection field `label`"),
            (
                "profile, profile.displayName",
                "cannot include both `profile` and child path `profile.displayName`",
            ),
        ] {
            let error = expression_type_result(&source(fields))
                .expect_err("invalid projection should fail source typing")
                .message();
            assert!(
                error.contains(expected),
                "projection {fields:?} should report {expected:?}, got:\n{error}"
            );
        }
    }

    #[test]
    fn relational_comparison_accepts_numbers_and_db_string_cursor() {
        expression_type_result(
            r#"
              type Credential { id: string }
              db object Credential { primary key(id) }

              function scan(lastId: string) -> Array<Credential> {
                return db find many Credential {
                  where id > lastId
                  order id asc
                  limit 100
                }
              }

              function numberOrder(left: number, right: number) -> bool {
                return left < right || left <= right || left > right || left >= right
              }

              function lexicalBindingSurvivesDbPredicate(id: number) -> number {
                const count = db count Credential { where id > "credential-0" }
                return id + count
              }
            "#,
        )
        .expect("DB string cursor and numeric relational comparisons should type-check");
    }

    #[test]
    fn relational_comparison_rejects_runtime_strings_mixed_nullable_and_other_types() {
        for (source, label) in [
            (
                r#"
                  function invalid(left: string, right: string) -> bool {
                    return left > right
                  }
                "#,
                "ordinary runtime string relation",
            ),
            (
                r#"
                  function invalid(left: string, right: number) -> bool {
                    return left > right
                  }
                "#,
                "mixed string/number",
            ),
            (
                r#"
                  function invalid(left: string?, right: string) -> bool {
                    return left > right
                  }
                "#,
                "nullable string",
            ),
            (
                r#"
                  function invalid(left: bool, right: bool) -> bool {
                    return left > right
                  }
                "#,
                "non-orderable bool",
            ),
            (
                r#"
                  type Credential { id: string }
                  db object Credential { primary key(id) }

                  function invalid(id: number) -> Array<Credential> {
                    return db find many Credential { where id > id }
                  }
                "#,
                "DB field and shadowed lexical value",
            ),
            (
                r#"
                  type Credential { id: string }
                  db object Credential { primary key(id) }

                  function invalid(id: number?) -> bool {
                    if id != null {
                      const count = db count Credential { where id > id }
                    }
                    return true
                  }
                "#,
                "DB field and non-null-narrowed lexical root",
            ),
            (
                r#"
                  type StoredProfile { name: number }
                  type Credential { id: string, profile: StoredProfile }
                  db object Credential { primary key(id) }

                  type LexicalProfile { name: string? }

                  function invalid(profile: LexicalProfile, lastString: string) -> bool {
                    if profile.name != null {
                      const count = db count Credential { where profile.name > lastString }
                    }
                    return true
                  }
                "#,
                "nested DB field and narrowed lexical path",
            ),
        ] {
            let error = expression_type_result(source)
                .expect_err("invalid relational comparison should fail")
                .message();
            assert!(
                error.contains("binary comparison operand type mismatch"),
                "{label} should report a comparison mismatch, got:\n{error}"
            );
        }
    }

    #[test]
    fn explicit_interface_boxing_and_any_interface_method_call_type_check() {
        expression_type_result(&boxing_source(
            r#"
              function run() -> string {
                const provider: any Provider = Host { label: "host" } as Provider
                return provider.name()
              }
            "#,
        ))
        .expect("explicit boxing and any-interface method call should type-check");
    }

    #[test]
    fn any_interface_internal_named_record_and_function_type_hosts_type_check() {
        expression_type_result(&boxing_source(
            r#"
              type Holder {
                provider: any Provider,
              }

              function consume(handler: fn(input: any Provider) -> any Provider) -> void {}

              function make() -> Holder {
                const holder: Holder = Holder {
                  provider: Host { label: "host" } as Provider,
                }
                return holder
              }
            "#,
        ))
        .expect("internal named record and function type hosts should type-check");
    }

    #[test]
    fn interface_boxing_const_return_publishes_expression_type_fact() {
        let source_text = boxing_source(
            r#"
              const provider: Host = Host { label: "host" }

              function testProvider() -> any Provider {
                return provider as Provider
              }
            "#,
        );
        let source = CompilerSourceFile::parse(
            PathBuf::from("internal/any_interface.skiff"),
            ANY_INTERFACE_MODULE.to_string(),
            false,
            false,
            source_text.clone(),
            "internal/any_interface.skiff",
        )
        .expect("test source should parse");
        let parsed_sources = parse_publication_sources(&PathBuf::from("/test"), &[source])
            .expect("test source should build parsed source facts");
        let type_resolution = TypeResolutionModel::build(
            &parsed_sources,
            &BTreeMap::new(),
            &[],
            None,
            None,
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type resolution should build");
        let expression_sources = ExpressionSourceMap::build(&parsed_sources)
            .expect("expression source facts should build");
        let model = ExpressionTypeModel::build(
            &parsed_sources,
            &expression_sources,
            &type_resolution,
            &PublicationDbMetadataIndex::default(),
            None,
        )
        .expect("interface boxing const return should type-check");
        let key = ExpressionKey::new(
            ANY_INTERFACE_MODULE.to_string(),
            ExpressionOwnerKey::Function("testProvider".to_string()),
            0,
        );
        let fact = model
            .fact(&key)
            .and_then(|fact| fact.ty.as_ref())
            .expect("interface boxing return expression should publish a type fact");
        assert!(matches!(fact.ir, TypeRefIr::AnyInterface { .. }));
    }

    #[test]
    fn concrete_value_does_not_implicitly_box_to_any_interface_parameter() {
        let error = expression_type_result(&boxing_source(
            r#"
              function accepts(provider: any Provider) -> void {}

              function run() -> void {
                accepts(Host { label: "host" })
              }
            "#,
        ))
        .expect_err("concrete value must not implicitly box to any Provider");
        let message = error.message();
        assert!(
            message.contains("argument") && message.contains("any "),
            "unexpected implicit boxing diagnostic: {message}"
        );

        expression_type_result(&boxing_source(
            r#"
              function accepts(provider: any Provider) -> void {}

              function run() -> void {
                accepts(Host { label: "host" } as Provider)
              }
            "#,
        ))
        .expect("explicit boxing should satisfy any Provider parameter");
    }

    #[test]
    fn interface_boxing_rejects_invalid_selector_source_and_conformance() {
        let selector_error = expression_type_result(&boxing_source(
            r#"
              function run() -> void {
                const provider = Host { label: "host" } as string
              }
            "#,
        ))
        .expect_err("as string should fail in expression type checking")
        .message();
        assert!(
            selector_error.contains("interface boxing selector `string`")
                && selector_error.contains("primitive/builtin"),
            "unexpected selector diagnostic: {selector_error}"
        );

        let source_error = expression_type_result(&boxing_source(
            r#"
              function run() -> void {
                const provider = { label: "host" } as Provider
              }
            "#,
        ))
        .expect_err("anonymous record source should not box")
        .message();
        assert!(
            source_error.contains("must be a concrete nominal record"),
            "unexpected source diagnostic: {source_error}"
        );

        let conformance_error = expression_type_result(&boxing_source(
            r#"
              function run() -> void {
                const provider = Other { label: "host" } as Provider
              }
            "#,
        ))
        .expect_err("non-conforming record should not box")
        .message();
        assert!(
            conformance_error.contains("does not explicitly implement interface Provider"),
            "unexpected conformance diagnostic: {conformance_error}"
        );
    }

    #[test]
    fn interface_boxing_rejects_marker_interface() {
        let error = expression_type_result(
            r#"
              interface Marker {}

              type Host implements Marker {
                label: string,
              }

              function run() -> void {
                const provider = Host { label: "host" } as Marker
              }
            "#,
        )
        .expect_err("marker interface should not be object-safe for boxing")
        .message();
        assert!(
            error.contains("not object-safe") && error.contains("marker interface"),
            "unexpected marker diagnostic: {error}"
        );
    }

    #[test]
    fn constructor_validation_error_carries_structured_field_facts() {
        let source = CompilerSourceFile::parse(
            PathBuf::from("internal/user.skiff"),
            "internal.user".to_string(),
            false,
            false,
            r#"
              type User {
                name: string,
                email: string,
                age: string,
              }

              function build() -> User {
                return User { name: "Ada", name: "Byron", email: 1, extra: "x" }
              }
            "#
            .to_string(),
            "internal/user.skiff",
        )
        .expect("test source should parse");
        let parsed_sources = parse_publication_sources(&PathBuf::from("/test"), &[source])
            .expect("test source should build parsed source facts");
        let package_aliases = BTreeMap::new();
        let type_resolution = TypeResolutionModel::build(
            &parsed_sources,
            &package_aliases,
            &[],
            None,
            None,
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type resolution should build");
        let expression_sources = ExpressionSourceMap::build(&parsed_sources)
            .expect("expression source facts should build");

        let error = ExpressionTypeModel::build(
            &parsed_sources,
            &expression_sources,
            &type_resolution,
            &PublicationDbMetadataIndex::default(),
            None,
        )
        .expect_err("invalid constructor should fail expression type checking");
        let key = ExpressionKey::new(
            "internal.user".to_string(),
            ExpressionOwnerKey::Function("build".to_string()),
            0,
        );
        let validation = error
            .model()
            .constructor_validation(&key)
            .expect("failed constructor should still have structured validation fact");

        assert_eq!(validation.provided_fields.len(), 4);
        assert_eq!(validation.duplicate_fields[0].name, "name");
        assert!(
            validation.duplicate_fields[0].name_span != SourceSpan::synthetic(),
            "duplicate field should retain source name span"
        );
        assert_eq!(validation.unknown_fields[0].name, "extra");
        assert!(
            validation.unknown_fields[0].name_span != SourceSpan::synthetic(),
            "unknown field should retain source name span"
        );
        assert_eq!(validation.missing_required_fields[0].name, "age");
        assert_eq!(validation.type_mismatches[0].name, "email");
        assert_eq!(validation.type_mismatches[0].expected.source_text, "string");
        assert!(
            validation.type_mismatches[0].value_span != SourceSpan::synthetic(),
            "field mismatch should retain source value span"
        );
    }

    #[test]
    fn db_upsert_result_fields_are_static_expression_type_facts() {
        let source = CompilerSourceFile::parse(
            PathBuf::from("internal/db_upsert_result_fields.test.skiff"),
            "internal.db_upsert_result_fields".to_string(),
            false,
            true,
            r#"
              type User {
                id: string,
                name: string,
              }

              db object User {
                name "user"
                primary key(id)
              }

              test "upsert result fields" {
                const r = db upsert User("u1") { name = "Ada" } { name = "Ada" }
                assert r.inserted
                assert r.value.name == "Ada"
              }
            "#
            .to_string(),
            "internal/db_upsert_result_fields.test.skiff",
        )
        .expect("test source should parse");
        let parsed_sources = parse_publication_sources(&PathBuf::from("/test"), &[source])
            .expect("test source should build parsed source facts");
        let package_aliases = BTreeMap::new();
        let type_resolution = TypeResolutionModel::build(
            &parsed_sources,
            &package_aliases,
            &[],
            None,
            None,
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type resolution should build");
        let expression_sources = ExpressionSourceMap::build(&parsed_sources)
            .expect("expression source facts should build");

        ExpressionTypeModel::build(
            &parsed_sources,
            &expression_sources,
            &type_resolution,
            &PublicationDbMetadataIndex::default(),
            None,
        )
        .expect("DbUpsertResult.inserted and .value fields should type-check statically");

        let user_ir = TypeRefIr::Record {
            fields: BTreeMap::from([(
                "name".to_string(),
                TypeRefIr::Builtin {
                    name: "string".to_string(),
                    args: Vec::new(),
                },
            )]),
        };
        let result_ir = TypeRefIr::Builtin {
            name: "DbUpsertResult".to_string(),
            args: vec![user_ir.clone()],
        };
        assert_eq!(
            record_field_type_from_ir(&result_ir, "inserted")
                .expect("inserted field should resolve")
                .ir,
            TypeRefIr::Builtin {
                name: "bool".to_string(),
                args: Vec::new(),
            }
        );
        assert_eq!(
            record_field_type_from_ir(&result_ir, "value")
                .expect("value field should resolve")
                .ir,
            user_ir
        );
    }

    #[test]
    fn runtime_receiver_builtin_calls_publish_static_return_type_facts() {
        let source_text = r#"
              import std

              type RuntimeLiveDoc {
                id: string,
                value: string,
                visits: number,
                rank: number,
              }

              db object RuntimeLiveDoc {
                name "runtime_live_doc"
                primary key(id)
              }

              function run() -> bool {
                const marker = config.require<string>("runtimeLive.db")
                const prefix = "runtime-live-db-".concat(std.crypto.uuidSimple())
                const firstId = prefix.concat("-a")
                const epoch = Date.fromEpochMilliseconds(0)
                const later = epoch.addMilliseconds(5)
                const epochMillis = epoch.toEpochMilliseconds()
                const diffMillis = later.diffMilliseconds(epoch)
                const ordering = epoch.compare(later)
                db insert RuntimeLiveDoc { id = firstId value = marker.concat("-first") visits = 1 rank = 10 }
                return firstId.contains(marker)
              }
            "#;
        let source = CompilerSourceFile::parse(
            PathBuf::from("internal/db_receiver_concat.skiff"),
            "internal.db_receiver_concat".to_string(),
            false,
            false,
            source_text.to_string(),
            "internal/db_receiver_concat.skiff",
        )
        .expect("test source should parse");
        let parsed_sources = parse_publication_sources(&PathBuf::from("/test"), &[source])
            .expect("test source should build parsed source facts");
        let package_aliases = BTreeMap::new();
        let type_resolution = TypeResolutionModel::build(
            &parsed_sources,
            &package_aliases,
            &[],
            None,
            None,
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type resolution should build");
        let expression_sources = ExpressionSourceMap::build(&parsed_sources)
            .expect("expression source facts should build");

        let model = ExpressionTypeModel::build(
            &parsed_sources,
            &expression_sources,
            &type_resolution,
            &PublicationDbMetadataIndex::default(),
            None,
        )
        .expect("config strings and receiver builtin string calls should type-check statically");
        let owner = ExpressionOwnerKey::Function("run".to_string());

        for (snippet, label, expected) in [
            (
                r#"config.require<string>("runtimeLive.db")"#,
                "config.require<string> result",
                "string",
            ),
            (
                r#""runtime-live-db-".concat(std.crypto.uuidSimple())"#,
                "literal concat result",
                "string",
            ),
            (
                r#"prefix.concat("-a")"#,
                "bound prefix concat result",
                "string",
            ),
            (
                r#"marker.concat("-first")"#,
                "db body marker concat result",
                "string",
            ),
            (
                "epoch.toEpochMilliseconds()",
                "Date.toEpochMilliseconds result",
                "integer",
            ),
            (
                "later.diffMilliseconds(epoch)",
                "Date.diffMilliseconds result",
                "integer",
            ),
            ("epoch.compare(later)", "Date.compare result", "integer"),
            ("firstId.contains(marker)", "contains result", "bool"),
        ] {
            assert_eq!(
                expression_fact_source_text(
                    &model,
                    &expression_sources,
                    source_text,
                    "internal.db_receiver_concat",
                    &owner,
                    snippet,
                ),
                expected,
                "{label} should publish a {expected} expression type fact"
            );
        }
    }

    #[test]
    fn native_signature_local_types_are_externalized_from_the_declaring_module() {
        let production = CompilerSourceFile::parse(
            PathBuf::from("time.skiff"),
            "std.time".to_string(),
            false,
            false,
            r#"
              type Duration = integer
              native function sleep(duration: Duration) -> void
            "#
            .to_string(),
            "time.skiff",
        )
        .expect("production source should parse");
        let test_source = CompilerSourceFile::parse(
            PathBuf::from("time.test.skiff"),
            "std.time.__test".to_string(),
            false,
            true,
            r#"
              import std

              test "duration native signature" {
                const duration = Duration.milliseconds(1)
                std.time.sleep(duration)
              }
            "#
            .to_string(),
            "time.test.skiff",
        )
        .expect("test source should parse");
        let parsed_sources =
            parse_publication_sources(&PathBuf::from("/test"), &[production, test_source])
                .expect("production and test source facts should build");
        let type_resolution = TypeResolutionModel::build(
            &parsed_sources,
            &BTreeMap::new(),
            &[],
            None,
            None,
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type resolution should build");
        let expression_sources = ExpressionSourceMap::build(&parsed_sources)
            .expect("expression source facts should build");

        ExpressionTypeModel::build(
            &parsed_sources,
            &expression_sources,
            &type_resolution,
            &PublicationDbMetadataIndex::default(),
            None,
        )
        .expect("native signature types should retain their declaring module identity");
    }

    fn expression_fact_source_text(
        model: &ExpressionTypeModel,
        expression_sources: &ExpressionSourceMap,
        source_text: &str,
        module_path: &str,
        owner: &ExpressionOwnerKey,
        snippet: &str,
    ) -> String {
        expression_sources
            .facts()
            .iter()
            .find_map(|(key, source_fact)| {
                if key.module_path() != module_path || key.owner() != owner {
                    return None;
                }
                let span_text = source_text
                    .get(source_fact.span.start.offset..source_fact.span.end.offset)?
                    .trim();
                if span_text != snippet {
                    return None;
                }
                model
                    .fact(key)
                    .and_then(|fact| fact.ty.as_ref())
                    .map(|ty| ty.source_text.clone())
            })
            .unwrap_or_else(|| panic!("expression `{snippet}` should have a type fact"))
    }
}

fn span_label(span: SourceSpan) -> String {
    format!("{}:{}", span.start.line, span.start.column)
}
