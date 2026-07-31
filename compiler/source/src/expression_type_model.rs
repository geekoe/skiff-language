use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    builtin_receiver_op_spec_by_name, BuiltinReceiverPublicReturnType, LiteralIr, PackageRefIr,
    PackageSymbolRef, PackageTypeRef, TypeRefIr,
};
use skiff_compiler_core::type_ref::{
    contains_type_param, debug_text, is_null_type, map_entry, normalize_union,
    package_type_ref_to_ir, record_field_type, single_item,
    substitute_type_params_in_type_ref_ref as substitute_type_params_in_ir,
};

use crate::{
    contract_type_resolution::substitute_package_type,
    parsed_sources::ParsedCompilerSource,
    semantic::impl_method_declaration_name,
    shared::ast::{
        BinaryOp, Block, DbBlockMode, DbBody, DbChangeOp, DbQueryBlock, DbSelector, DbWhereClause,
        Expr, ForBinding, FunctionDecl, Literal, Param, SourceFile, Stmt, TypeRef, UnaryOp,
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

mod contract_call_typing;
mod db_projection;
mod expression_assignability;
mod object_materialization;

pub use contract_call_typing::package_type_ref_from_contract_type;
use contract_call_typing::{
    contract_source_assignability_with_projections, package_type_target_assignable,
    ContractCallOutcome, ContractCallTyping, ContractProjectionState,
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
    transparent_value_targets: BTreeMap<ExpressionKey, ExpressionKey>,
    test_effect_declarations: BTreeMap<ExactTestEffectTarget, String>,
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

fn direct_stream_item_type(ty: &PackageTypeRef) -> Option<&PackageTypeRef> {
    match ty {
        PackageTypeRef::Container { name, arguments }
            if name == "Stream" && arguments.len() == 1 =>
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
            transparent_value_targets: BTreeMap::new(),
            test_effect_declarations: BTreeMap::new(),
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
            Stmt::CompilerTestEffectRegister {
                target,
                target_probe: _,
                declaration_start,
                expect,
                step_expect,
                outcome,
            } => {
                // The synthetic target probe exists solely to obtain the same
                // exact ResolvedCallTarget fact ordinary dependency calls use.
                // It is not an invocation and therefore is not type-checked as
                // a zero-argument call.
                self.next_key();
                self.next_key();
                let Some(dependencies) = self.dependency_analysis else {
                    self.diagnostics.push(format!(
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
                            self.diagnostics.push(format!(
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
                        let return_type = match &contract.stream {
                            skiff_artifact_model::BoundaryStreamContract::Unary => {
                                package_type_ref_from_contract_type(&contract.return_value.ty)
                            }
                            skiff_artifact_model::BoundaryStreamContract::ServerStream {
                                item_type,
                                ..
                            } => PackageTypeRef::Container {
                                name: "Stream".to_string(),
                                arguments: vec![package_type_ref_from_contract_type(item_type)],
                            },
                            skiff_artifact_model::BoundaryStreamContract::Unsupported {
                                ..
                            } => {
                                self.diagnostics.push(format!(
                                    "{}: compiler test effect target `{target}` has an unsupported stream contract",
                                    self.module_path
                                ));
                                return false;
                            }
                        };
                        (
                            skiff_artifact_model::PackageCallableSignature {
                                type_params: Vec::new(),
                                parameters: contract
                                    .parameters
                                    .iter()
                                    .map(|parameter| {
                                        skiff_artifact_model::PackageCallableParameter {
                                            name: parameter.name.clone(),
                                            ty: package_type_ref_from_contract_type(&parameter.ty),
                                        }
                                    })
                                    .collect(),
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
                        self.diagnostics.push(format!(
                            "{}: unresolved compiler test effect target `{target}`",
                            self.module_path
                        ));
                        return false;
                    }
                };
                if *declaration_start {
                    if let Some(previous) = self
                        .test_effect_declarations
                        .insert(exact_target.clone(), target.clone())
                    {
                        self.diagnostics.push(format!(
                            "{}: test effect targets `{previous}` and `{target}` resolve to the same exact target {exact_target:?}; use one explicit sequence",
                            self.module_path
                        ));
                    }
                }
                if let Some(expect) = expect {
                    let [parameter] = signature.parameters.as_slice() else {
                        self.diagnostics.push(format!(
                            "{}: test effect `{target}` expect requires exactly one parameter",
                            self.module_path
                        ));
                        return false;
                    };
                    self.check_test_effect_request_subset(expect, &parameter.ty);
                }
                if let Some(step_expect) = step_expect {
                    let [parameter] = signature.parameters.as_slice() else {
                        self.diagnostics.push(format!(
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
                            self.diagnostics.push(format!(
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
                            self.diagnostics.push(format!(
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
                self.invalidate_path_refinements_for_write(target);
                false
            }
            Stmt::Timeout { body, .. } | Stmt::Serial { body } => {
                self.check_block_scoped(body, &TypeNarrowing::default());
                false
            }
            Stmt::Concurrent { body } => {
                self.check_concurrent_block(body, None);
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
                let key = self.peek_key();
                if let Some(actual) = self.check_expr(value) {
                    self.validate_throw_payload(&key, &actual, "throw");
                }
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
                let key = self.peek_key();
                if let Some(actual) = self.check_expr(exception) {
                    self.validate_rethrow_operand(&key, &actual);
                }
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

    fn check_test_effect_request_subset(&mut self, value: &Expr, expected: &PackageTypeRef) {
        let Expr::ObjectLiteral { entries } = value else {
            self.check_test_effect_value(value, expected, "expect");
            return;
        };
        let resolved = resolved_package_type_ref(expected);
        let Some(TypeRefIr::Record { fields }) = self
            .type_resolution
            .type_shape_ir(&resolved, &self.type_context)
        else {
            self.check_test_effect_value(value, expected, "expect");
            return;
        };
        let mut selected = BTreeMap::new();
        for entry in entries {
            let Some(name) = object_literal_key_text(&entry.key) else {
                self.diagnostics.push(format!(
                    "{}: test effect expect subset keys must name static request fields",
                    self.module_path
                ));
                continue;
            };
            let Some(ty) = fields.get(&name) else {
                self.diagnostics.push(format!(
                    "{}: test effect expect subset contains unknown request field `{name}`",
                    self.module_path
                ));
                continue;
            };
            selected.insert(name, ty.clone());
        }
        let partial = ResolvedTypeRef {
            ir: TypeRefIr::Record { fields: selected },
            source_text: format!("subset<{}>", resolved.source_text),
        };
        let key = self.peek_key();
        let actual = self.check_expr(value);
        if let Some(actual) = actual {
            self.check_value_assignable_to_expected(
                None,
                value,
                &key,
                &actual,
                &partial,
                None,
                "test effect expect subset",
                self.expression_span(&key),
            );
        }
    }

    fn check_test_effect_value(&mut self, value: &Expr, expected: &PackageTypeRef, context: &str) {
        let key = self.peek_key();
        let actual = self.check_expr(value);
        let Some(actual) = actual else {
            return;
        };
        if let (Some(projected), Some(dependencies)) = (
            self.contract_projection.expression_type(&key),
            self.dependency_analysis,
        ) {
            if package_type_target_assignable(projected, expected, dependencies) {
                return;
            }
            self.diagnostics.push(format!(
                "{}: test effect {context} package type is not assignable to the declared target at {}",
                self.module_path,
                self.expression_span_label(&key)
            ));
            return;
        }
        let resolved_expected = resolved_package_type_ref(expected);
        self.check_value_assignable_to_expected(
            None,
            value,
            &key,
            &actual,
            &resolved_expected,
            Some(expected),
            &format!("test effect {context}"),
            self.expression_span(&key),
        );
    }

    fn check_test_effect_throw(&mut self, value: &Expr, target: &str) {
        let key = self.peek_key();
        let Some(actual) = self.check_expr(value) else {
            return;
        };
        if let Err(error) = self
            .type_resolution
            .catch_leaves(&actual, &self.type_context)
        {
            self.diagnostics.push(format!(
                "{}: test effect `{target}` throw has invalid catch payload at {}: {error}",
                self.module_path,
                self.expression_span_label(&key),
            ));
            return;
        }
        if let Some(fact) = self.facts.get_mut(&key) {
            fact.test_effect_throw_payload_type = Some(actual.ir);
        }
    }

    fn validate_throw_payload(
        &mut self,
        key: &ExpressionKey,
        actual: &ResolvedTypeRef,
        construct: &str,
    ) {
        if let Err(error) = self
            .type_resolution
            .catch_leaves(actual, &self.type_context)
        {
            self.diagnostics.push(format!(
                "{}: {construct} payload `{}` has no valid nominal catch identity at {}: {error}",
                self.module_path,
                actual.source_text,
                self.expression_span_label(key)
            ));
        }
    }

    fn validate_rethrow_operand(&mut self, key: &ExpressionKey, actual: &ResolvedTypeRef) {
        if let Err(error) = self
            .type_resolution
            .exception_catch_leaves(actual, &self.type_context)
        {
            self.diagnostics.push(format!(
                "{}: invalid rethrow operand `{}` at {}: {error}",
                self.module_path,
                actual.source_text,
                self.expression_span_label(key)
            ));
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

            if let Stmt::Let {
                mutable: false,
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
                        self.check_db_field_operand(
                            left,
                            db_predicate_fields.expect("checked above"),
                        )
                    } else if db_logical {
                        self.check_db_predicate_expr(
                            left,
                            db_predicate_fields.expect("checked above"),
                        )
                    } else {
                        self.check_expr(left)
                    };
                    let right_ty = if db_logical {
                        self.check_db_predicate_expr(
                            right,
                            db_predicate_fields.expect("checked above"),
                        )
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
                    let operand_ty = if db_predicate_fields.is_some() && matches!(op, UnaryOp::Not)
                    {
                        self.check_db_predicate_expr(
                            expr,
                            db_predicate_fields.expect("checked above"),
                        )
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
                        .resolve_canonical_interface_selector_type_ref(
                            interface,
                            &self.type_context,
                        ) {
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
                        ir: TypeRefIr::AnyInterface {
                            interface: selector.instantiation_ref.clone(),
                        },
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
                Expr::ValueBlock(value) => self.check_value_block_expr(&key, value),
                Expr::ConcurrentValue(value) => {
                    self.check_concurrent_block(&value.body, Some((&key, &value.tail)))
                }
                Expr::Timeout { value, .. } => {
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
                Expr::Throw { value } => {
                    if let Some(actual) = self.check_expr(value) {
                        self.validate_throw_payload(&key, &actual, "throw expression");
                    }
                    None
                }
                Expr::Rethrow { exception } => {
                    if let Some(actual) = self.check_expr(exception) {
                        self.validate_rethrow_operand(&key, &actual);
                    }
                    None
                }
                Expr::Catch {
                    catch_type,
                    try_expr,
                } => {
                    let try_ty = self.check_expr(try_expr)?;
                    let catch_ty = match self
                        .type_resolution
                        .resolve_type_ref(catch_type, &self.type_context)
                    {
                        Ok(catch_ty) => catch_ty,
                        Err(error) => {
                            self.diagnostics.push(format!(
                                "{}: catch type cannot be resolved at {}: {error}",
                                self.module_path,
                                self.expression_span_label(&key)
                            ));
                            return None;
                        }
                    };
                    if let Err(error) = self
                        .type_resolution
                        .catch_leaves(&catch_ty, &self.type_context)
                    {
                        self.diagnostics.push(format!(
                            "{}: invalid catch type `{}` at {}: {error}",
                            self.module_path,
                            catch_ty.source_text,
                            self.expression_span_label(&key)
                        ));
                    }
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
                Err(error) => self.diagnostics.push(format!(
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
                test_effect_throw_payload_type: None,
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
                    DbChangeOp::Set { path, value }
                    | DbChangeOp::Inc { path, value }
                    | DbChangeOp::AddToSet { path, value }
                    | DbChangeOp::Remove { path, value } => {
                        self.validate_db_change_path(&operation.target, &path.segments);
                        self.check_expr(value);
                    }
                    DbChangeOp::Unset { path } => {
                        self.validate_db_change_path(&operation.target, &path.segments);
                    }
                }
            }
        }
    }

    fn validate_db_change_path(&mut self, target: &TypeRef, path: &[String]) {
        let Ok(target_type) = self
            .type_resolution
            .resolve_type_ref(target, &self.type_context)
        else {
            return;
        };
        if let Err(error) = DbProjectionTypeResolver::new(
            self.module_path,
            self.type_resolution,
            self.publication_db_metadata,
        )
        .project_read_type(&target.name, target_type.ir, &[path.to_vec()])
        {
            if error.contains("has no DB metadata") {
                return;
            }
            self.diagnostics.push(format!(
                "{}: db change field path `{}` is invalid: {error}",
                self.module_path,
                path.join(".")
            ));
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

    fn invalidate_path_refinements_for_write(&mut self, target: &Expr) {
        let Some(path) = expr_path(target) else {
            self.path_refinements.clear();
            return;
        };
        let descendant_prefix = format!("{path}.");
        self.path_refinements
            .retain(|refined, _| refined != &path && !refined.starts_with(&descendant_prefix));
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

    fn consume_static_package_value_descendants(&mut self, expr: &Expr) {
        match expr {
            Expr::Field { object, .. } => {
                self.next_key();
                self.consume_static_package_value_descendants(object);
            }
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
            | Expr::Call { .. }
            | Expr::Record { .. }
            | Expr::ObjectLiteral { .. }
            | Expr::Patch { .. }
            | Expr::InterfaceBox { .. }
            | Expr::Throw { .. }
            | Expr::Rethrow { .. }
            | Expr::Catch { .. }
            | Expr::DbOperation(_)
            | Expr::DbQuery(_)
            | Expr::DbTransaction(_)
            | Expr::DbLeaseClaim(_)
            | Expr::DbLeaseRead(_) => {}
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
        if let Some(return_type) =
            self.package_receiver_call_type(key, callee, type_args, args, arg_types)
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
            let signature = self.dependency_analysis.and_then(|dependency_analysis| {
                let (canonical_dependency_ref, callable) =
                    dependency_analysis.package_callable_by_source_path(&path)?;
                let type_dependency_ref = dependency_source_address_parts(&path)
                    .map(|(dependency_ref, _)| dependency_ref)
                    .filter(|dependency_ref| {
                        self.type_resolution
                            .is_top_level_package_dependency_ref(dependency_ref)
                    })
                    .unwrap_or(canonical_dependency_ref);
                Some((
                    type_dependency_ref.to_string(),
                    callable.signature()?.clone(),
                ))
            });
            if let Some((dependency_ref, signature)) = signature {
                let canonical_dependency_ref = self
                    .type_resolution
                    .canonical_package_dependency_ref(&dependency_ref)
                    .to_string();
                // Resolve each parameter independently: an owner/slot diagnostic
                // must fail the compile without erasing an exact return fact.
                let expected = signature
                    .parameters
                    .iter()
                    .map(|parameter| {
                        (
                            parameter.name.clone(),
                            self.type_resolution
                                .rehydrate_package_signature_type_for_dependency(
                                    &canonical_dependency_ref,
                                    &parameter.ty,
                                )
                                .or_else(|_| {
                                    self.type_resolution
                                        .rehydrate_package_signature_type_for_dependency(
                                            &dependency_ref,
                                            &parameter.ty,
                                        )
                                })
                                .map(|exact| {
                                    let ordinary =
                                        self.type_resolution.bind_package_type_refs_to_dependency(
                                            &resolved_package_type_ref(&exact),
                                            &canonical_dependency_ref,
                                        );
                                    (ordinary, exact)
                                }),
                        )
                    })
                    .collect::<Vec<_>>();
                self.validate_dependency_package_call_params(
                    key, &path, &expected, args, arg_types,
                );

                let exact_projection = match self
                    .type_resolution
                    .rehydrate_package_signature_type_for_dependency(
                        &dependency_ref,
                        &signature.return_type,
                    ) {
                    Ok(return_type) => return_type,
                    Err(error) => {
                        self.diagnostics.push(format!(
                            "{}: call `{path}` return dependency type resolution failed at {}: {error}",
                            self.module_path,
                            self.expression_span_label(key),
                        ));
                        return None;
                    }
                };
                let resolved_return = self.type_resolution.bind_package_type_refs_to_dependency(
                    &resolved_package_type_ref(&exact_projection),
                    &dependency_ref,
                );
                let projected_return = self
                    .type_resolution
                    .rehydrate_package_signature_type_for_dependency(
                        &canonical_dependency_ref,
                        &signature.return_type,
                    )
                    .unwrap_or_else(|_| exact_projection.clone());
                self.contract_projection
                    .record_expression_type(key.clone(), projected_return);
                return Some(resolved_return);
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
                match self.resolve_callable_signature_type(raw, context, type_params, type_args) {
                    Some(resolved) => Some((format!("arg{index}"), resolved)),
                    None => {
                        let _ = callable;
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
        self.resolve_callable_signature_type(raw, context, type_params, type_args)
    }

    fn resolve_callable_signature_type(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
        type_params: &[String],
        type_args: &[TypeRef],
    ) -> Option<ResolvedTypeRef> {
        self.exact_type_arg_substitution(raw, type_params, type_args)
            .or_else(|| self.structured_type_arg_substitution(raw, context, type_params, type_args))
            .or_else(|| {
                // Omitted generic arguments can still leave a declaration type
                // concrete when the type does not depend on any type parameter.
                (type_params.is_empty() || type_args.is_empty())
                    .then(|| self.type_resolution.resolve_type_text(raw, context).ok())
                    .flatten()
                    .filter(|resolved| !contains_type_param(&resolved.ir))
            })
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
            if contains_type_param(&expected.ir) || contains_type_param(&actual.ir) {
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

    fn validate_dependency_package_call_params(
        &mut self,
        call_key: &ExpressionKey,
        callable: &str,
        expected: &[(String, Result<(ResolvedTypeRef, PackageTypeRef), String>)],
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
        for (index, (name, expected)) in expected.iter().enumerate() {
            let (expected, exact_expected) = match expected {
                Ok(expected) => expected,
                Err(error) => {
                    self.diagnostics.push(format!(
                        "{}: call `{callable}` parameter {} `{name}` dependency type resolution failed at {}: {error}",
                        self.module_path,
                        index + 1,
                        self.expression_span_label(call_key),
                    ));
                    continue;
                }
            };
            let Some((key, actual)) = arg_types.get(index) else {
                continue;
            };
            let Some(actual) = actual else {
                continue;
            };
            if contains_type_param(&expected.ir) || contains_type_param(&actual.ir) {
                continue;
            }
            let context = format!("call `{callable}` argument {}", index + 1);
            self.check_value_assignable_to_expected(
                None,
                &args[index],
                key,
                actual,
                expected,
                Some(exact_expected),
                &context,
                self.expression_span(key),
            );
        }
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
                        source_text: debug_text(&param.ty),
                        ir: param.ty.clone(),
                    },
                )
            })
            .collect();
        self.validate_resolved_call_params(&callable, params, args, arg_types);
        Some(ResolvedTypeRef {
            source_text: debug_text(&return_type),
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
                        source_text: debug_text(&param.ty),
                        ir: param.ty.clone(),
                    },
                )
            })
            .collect();
        self.validate_resolved_call_params(&callable, params, args, arg_types);
        Some(ResolvedTypeRef {
            source_text: debug_text(&operation.return_type),
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
                            source_text: debug_text(&ty),
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
            source_text: debug_text(&return_type),
            ir: return_type,
        })
    }

    fn package_receiver_call_type(
        &mut self,
        key: &ExpressionKey,
        callee: &Expr,
        type_args: &[TypeRef],
        args: &[Expr],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Option<ResolvedTypeRef> {
        let (_, method_name) = receiver_call_parts(callee)?;
        let offset = 1 + receiver_object_offset_in_callee(callee)?;
        let receiver_key = ExpressionKey::new(
            key.module_path().to_string(),
            key.owner().clone(),
            key.preorder_index().checked_add(offset)?,
        );
        let receiver_ty = self.expression_type_at_offset(key, offset)?;
        let receiver_method = self
            .type_resolution
            .package_receiver_method_resolution(&receiver_ty.ir, method_name)?;
        let source_path = format!(
            "{}/{}",
            receiver_method.dependency_ref, receiver_method.source_method_path
        );
        let dependency_analysis = self.dependency_analysis?;
        let Some((canonical_dependency_ref, callable)) =
            dependency_analysis.package_callable_by_source_path(&source_path)
        else {
            self.diagnostics.push(format!(
                "{}: package receiver method `{source_path}` has no exact callable implementation member at {}",
                self.module_path,
                self.expression_span_label(key)
            ));
            return None;
        };
        if canonical_dependency_ref != receiver_method.canonical_dependency_ref {
            self.diagnostics.push(format!(
                "{}: package receiver method `{source_path}` resolves to dependency `{canonical_dependency_ref}` instead of `{}`",
                self.module_path, receiver_method.canonical_dependency_ref
            ));
            return None;
        }
        let signature = callable.signature()?.clone();
        let receiver_param_count = receiver_method.receiver_type_params.len();
        if signature
            .parameters
            .first()
            .map(|parameter| parameter.name.as_str())
            != Some("self")
            || signature.type_params.len() < receiver_param_count
            || signature.type_params.len() - receiver_param_count != type_args.len()
        {
            self.diagnostics.push(format!(
                "{}: package receiver method `{source_path}` has an invalid receiver/generic signature",
                self.module_path
            ));
            return None;
        }
        let mut substitutions = signature
            .type_params
            .iter()
            .take(receiver_param_count)
            .cloned()
            .zip(
                receiver_method
                    .receiver_type_arguments
                    .iter()
                    .cloned()
                    .map(|local_type| PackageTypeRef::Local { local_type }),
            )
            .collect::<BTreeMap<_, _>>();
        for (type_param, type_arg) in signature
            .type_params
            .iter()
            .skip(receiver_param_count)
            .zip(type_args)
        {
            let projected = match self.project_source_binding_type(type_arg) {
                Ok(Some(projected)) => projected,
                Ok(None) => {
                    self.diagnostics.push(format!(
                        "{}: package receiver method `{source_path}` type argument `{type_param}` has no exact package projection",
                        self.module_path
                    ));
                    return None;
                }
                Err(error) => {
                    self.diagnostics.push(format!(
                        "{}: package receiver method `{source_path}` type argument `{type_param}` projection failed: {error}",
                        self.module_path
                    ));
                    return None;
                }
            };
            substitutions.insert(type_param.clone(), projected);
        }

        let exact_parameters = signature
            .parameters
            .iter()
            .map(|parameter| {
                substitute_package_type(&parameter.ty, &substitutions).and_then(|ty| {
                    self.type_resolution
                        .rehydrate_package_signature_type_for_dependency(
                            &receiver_method.dependency_ref,
                            &ty,
                        )
                        .map(|exact| (parameter.name.clone(), exact))
                })
            })
            .collect::<Result<Vec<_>, _>>();
        let exact_parameters = match exact_parameters {
            Ok(parameters) => parameters,
            Err(error) => {
                self.diagnostics.push(format!(
                    "{}: package receiver method `{source_path}` parameter substitution failed at {}: {error}",
                    self.module_path,
                    self.expression_span_label(key)
                ));
                return None;
            }
        };
        let expected_receiver = self.type_resolution.bind_package_type_refs_to_dependency(
            &resolved_package_type_ref(&exact_parameters[0].1),
            &receiver_method.dependency_ref,
        );
        if !self.type_resolution.assignable_in_context(
            &receiver_ty,
            &expected_receiver,
            &self.type_context,
        ) {
            self.diagnostics.push(format!(
                "{}: package receiver method `{source_path}` receiver type mismatch at {}: expected {}, found {}",
                self.module_path,
                self.expression_span_label(&receiver_key),
                expected_receiver.source_text,
                receiver_ty.source_text
            ));
            return None;
        }
        let expected = exact_parameters
            .iter()
            .skip(1)
            .map(|(name, exact)| {
                (
                    name.clone(),
                    Ok((
                        self.type_resolution.bind_package_type_refs_to_dependency(
                            &resolved_package_type_ref(exact),
                            &receiver_method.dependency_ref,
                        ),
                        exact.clone(),
                    )),
                )
            })
            .collect::<Vec<_>>();
        self.validate_dependency_package_call_params(key, &source_path, &expected, args, arg_types);

        let exact_return = match substitute_package_type(&signature.return_type, &substitutions)
            .and_then(|ty| {
                self.type_resolution
                    .rehydrate_package_signature_type_for_dependency(
                        &receiver_method.dependency_ref,
                        &ty,
                    )
            }) {
            Ok(return_type) => return_type,
            Err(error) => {
                self.diagnostics.push(format!(
                    "{}: package receiver method `{source_path}` return substitution failed at {}: {error}",
                    self.module_path,
                    self.expression_span_label(key)
                ));
                return None;
            }
        };
        let resolved_return = self.type_resolution.bind_package_type_refs_to_dependency(
            &resolved_package_type_ref(&exact_return),
            &receiver_method.dependency_ref,
        );
        self.contract_projection
            .record_expression_type(key.clone(), exact_return);
        Some(resolved_return)
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
        let target_key = self.transparent_value_target_key(value_key);
        let target_value = transparent_value_target(value);
        if matches!(target_value, Expr::ObjectLiteral { .. }) {
            let target_actual = self
                .facts
                .get(&target_key)
                .and_then(|fact| fact.ty.clone())
                .unwrap_or_else(|| actual.clone());
            return self.materialize_target_typed_object_literal(
                annotation,
                target_value,
                &target_key,
                &target_actual,
                expected,
                context,
            );
        }
        let assignability = ExpressionAssignability::new(
            self.module_path,
            self.expression_sources,
            self.type_resolution,
            &self.type_context,
            self.dependency_analysis,
        );
        let package_json_context = context.starts_with("call `std.json.encode` argument ");
        let assignability = if package_json_context {
            assignability.with_package_json_context()
        } else {
            assignability
        };
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
            Some(true) => true,
            Some(false) if !package_json_context => false,
            Some(false) | None => match assignability.value_assignable_to_expected(
                annotation,
                value,
                actual,
                expected,
                self.contract_projection.expression_type(value_key),
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

    fn transparent_value_target_key(&self, key: &ExpressionKey) -> ExpressionKey {
        let mut target = key.clone();
        let mut visited = BTreeSet::new();
        while visited.insert(target.clone()) {
            let Some(next) = self.transparent_value_targets.get(&target) else {
                break;
            };
            target = next.clone();
        }
        target
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
        let assignability = ExpressionAssignability::new(
            self.module_path,
            self.expression_sources,
            self.type_resolution,
            &self.type_context,
            self.dependency_analysis,
        );
        let assignability = if context.starts_with("call `std.json.encode` argument ") {
            assignability.with_package_json_context()
        } else {
            assignability
        };
        let plan = match assignability.object_literal_materialization_plan(
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
    single_item(&ty.ir).map(|item| ResolvedTypeRef {
        ir: item.clone(),
        source_text: debug_text(item),
    })
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
            source_text: debug_text(&args[0]),
        })
}

fn map_entry_types(ty: &ResolvedTypeRef) -> Option<(ResolvedTypeRef, ResolvedTypeRef)> {
    // Preserves the pre-existing short-name-only behavior of this wrapper:
    // unlike the other map helpers it does not accept `std.collection.Map`.
    let TypeRefIr::Builtin { name, args } = &ty.ir else {
        return None;
    };
    if name != "Map" || args.len() != 2 {
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
    record_field_type(ty, field)
        .as_ref()
        .map(resolved_type_from_ir)
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
    map_entry(ty).map(|(_, value)| value.clone())
}

fn map_key_type_ir(ty: &TypeRefIr) -> Option<TypeRefIr> {
    map_entry(ty).map(|(key, _)| key.clone())
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
                .unwrap_or_else(|| debug_text(inner));
            Some(ResolvedTypeRef {
                ir: inner.as_ref().clone(),
                source_text,
            })
        }
        TypeRefIr::Union { items } => {
            let remaining = items
                .iter()
                .filter(|item| !is_null_type(item))
                .cloned()
                .collect::<Vec<_>>();
            (remaining.len() != items.len()).then(|| {
                resolved_type_from_ir(&normalize_union(TypeRefIr::Union { items: remaining }))
            })
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
    (!selected.is_empty())
        .then(|| resolved_type_from_ir(&normalize_union(TypeRefIr::Union { items: selected })))
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
        source_text: debug_text(ty),
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

fn resolved_package_type_ref(ty: &PackageTypeRef) -> ResolvedTypeRef {
    let ir = package_type_ref_ir(ty);
    ResolvedTypeRef {
        source_text: debug_text(&ir),
        ir,
    }
}

fn package_type_ref_ir(ty: &PackageTypeRef) -> TypeRefIr {
    match ty {
        PackageTypeRef::Local { local_type } => ordinary_package_local_type_ir(local_type),
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
                .and_then(|identity| serde_json::to_string(&identity))
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

fn literal_string_type(value: &str) -> TypeRefIr {
    TypeRefIr::Literal {
        value: LiteralIr::String {
            value: value.to_string(),
        },
    }
}

fn type_ir_is_void_or_null(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Builtin { name, args } if args.is_empty() && (name == "void" || name == "null"))
        || is_null_type(ty)
}

fn type_ir_is_never(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Builtin { name, args } if args.is_empty() && name == "never")
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
mod tests;

fn span_label(span: SourceSpan) -> String {
    format!("{}:{}", span.start.line, span.start.column)
}
