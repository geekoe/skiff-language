use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    builtin_receiver_op_spec_by_name, BuiltinReceiverPublicReturnType, LiteralIr, TypeRefIr,
};
use skiff_compiler_core::type_ref::substitute_type_params_in_type_ref_ref as substitute_type_params_in_ir;

use crate::{
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
    ExpressionKey, ExpressionOwnerKey, ExpressionSourceMap,
    RemotePublicInstanceOperationProjection, RemotePublicInstanceOperationResolver,
    ResolvedDependencies, ResolvedTypeRef, TypeResolutionContext, TypeResolutionModel,
};

mod expression_assignability;

use expression_assignability::{record_type_fields, ExpressionAssignability};

#[derive(Clone, Debug, Default)]
pub struct ExpressionTypeModel {
    facts: BTreeMap<ExpressionKey, ExpressionTypeFact>,
    remote_interface_boxes: BTreeMap<ExpressionKey, RemotePublicInstanceOperationProjection>,
    constructor_validations: BTreeMap<ExpressionKey, ConstructorValidation>,
    representation_constructor_validations:
        BTreeMap<ExpressionKey, RepresentationConstructorValidation>,
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
    expression_sources: &'a ExpressionSourceMap,
    callable_signatures: &'a BTreeMap<String, CallableSignature>,
    remote_public_instances: Option<RemotePublicInstanceOperationResolver<'a>>,
    return_type: Option<TypeRef>,
    type_context: TypeResolutionContext<'a>,
    env: BTreeMap<String, ResolvedTypeRef>,
    path_refinements: BTreeMap<String, ResolvedTypeRef>,
    facts: &'a mut BTreeMap<ExpressionKey, ExpressionTypeFact>,
    remote_interface_boxes:
        &'a mut BTreeMap<ExpressionKey, RemotePublicInstanceOperationProjection>,
    constructor_validations: &'a mut BTreeMap<ExpressionKey, ConstructorValidation>,
    representation_constructor_validations:
        &'a mut BTreeMap<ExpressionKey, RepresentationConstructorValidation>,
    diagnostics: &'a mut Vec<String>,
}

impl ExpressionTypeModel {
    pub fn build(
        parsed_sources: &[ParsedCompilerSource],
        expression_sources: &ExpressionSourceMap,
        type_resolution: &TypeResolutionModel,
        dependencies: Option<&ResolvedDependencies>,
    ) -> Result<Self, ExpressionTypeModelBuildError> {
        let callable_signatures = callable_signatures(parsed_sources);
        let mut facts = BTreeMap::new();
        let mut remote_interface_boxes = BTreeMap::new();
        let mut constructor_validations = BTreeMap::new();
        let mut representation_constructor_validations = BTreeMap::new();
        let mut diagnostics = Vec::new();
        let remote_public_instances = dependencies.map(|dependencies| {
            RemotePublicInstanceOperationResolver::new(dependencies, type_resolution)
        });

        for parsed in parsed_sources {
            check_source(
                parsed.source().module_path.as_str(),
                parsed.ast(),
                expression_sources,
                type_resolution,
                &callable_signatures,
                remote_public_instances.clone(),
                &mut facts,
                &mut remote_interface_boxes,
                &mut constructor_validations,
                &mut representation_constructor_validations,
                &mut diagnostics,
            );
        }

        let model = Self {
            facts,
            remote_interface_boxes,
            constructor_validations,
            representation_constructor_validations,
        };
        if !diagnostics.is_empty() {
            return Err(ExpressionTypeModelBuildError { model, diagnostics });
        }

        Ok(model)
    }

    pub fn fact(&self, key: &ExpressionKey) -> Option<&ExpressionTypeFact> {
        self.facts.get(key)
    }

    pub fn remote_interface_box(
        &self,
        key: &ExpressionKey,
    ) -> Option<&RemotePublicInstanceOperationProjection> {
        self.remote_interface_boxes.get(key)
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

    #[allow(clippy::too_many_arguments)]
    pub fn value_assignable_to_expected(
        &self,
        expression_sources: &ExpressionSourceMap,
        type_resolution: &TypeResolutionModel,
        type_context: &TypeResolutionContext<'_>,
        annotation: Option<&TypeRef>,
        value: &Expr,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
    ) -> bool {
        ExpressionAssignability::new("", expression_sources, type_resolution, type_context)
            .value_assignable_to_expected(annotation, value, actual, expected)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn object_literal_assignability_diagnostics(
        &self,
        diagnostic_path: &str,
        expression_sources: &ExpressionSourceMap,
        type_resolution: &TypeResolutionModel,
        type_context: &TypeResolutionContext<'_>,
        annotation: Option<&TypeRef>,
        value: &Expr,
        value_key: &ExpressionKey,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
        context: &str,
    ) -> Option<Vec<String>> {
        ExpressionAssignability::new(
            diagnostic_path,
            expression_sources,
            type_resolution,
            type_context,
        )
        .object_literal_assignability_diagnostics(
            annotation, value, value_key, actual, expected, context,
        )
    }
}

fn check_source(
    module_path: &str,
    ast: &SourceFile,
    expression_sources: &ExpressionSourceMap,
    type_resolution: &TypeResolutionModel,
    callable_signatures: &BTreeMap<String, CallableSignature>,
    remote_public_instances: Option<RemotePublicInstanceOperationResolver<'_>>,
    facts: &mut BTreeMap<ExpressionKey, ExpressionTypeFact>,
    remote_interface_boxes: &mut BTreeMap<ExpressionKey, RemotePublicInstanceOperationProjection>,
    constructor_validations: &mut BTreeMap<ExpressionKey, ConstructorValidation>,
    representation_constructor_validations: &mut BTreeMap<
        ExpressionKey,
        RepresentationConstructorValidation,
    >,
    diagnostics: &mut Vec<String>,
) {
    let const_env = const_type_env(
        ast,
        type_resolution,
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
            callable_signatures,
            remote_public_instances,
            &const_env,
            facts,
            remote_interface_boxes,
            constructor_validations,
            representation_constructor_validations,
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
                callable_signatures,
                remote_public_instances,
                &const_env,
                facts,
                remote_interface_boxes,
                constructor_validations,
                representation_constructor_validations,
                diagnostics,
            );
        }
    }

    for constant in &ast.consts {
        let mut checker = OwnerChecker::new(
            module_path,
            ExpressionOwnerKey::Const(constant.name.clone()),
            TypeResolutionContext::source(module_path),
            BTreeMap::new(),
            expression_sources,
            type_resolution,
            callable_signatures,
            remote_public_instances,
            None,
            facts,
            remote_interface_boxes,
            constructor_validations,
            representation_constructor_validations,
            diagnostics,
        );
        let value_key = checker.peek_key();
        let actual = checker.check_expr(&constant.value);
        if let (Some(annotation), Some(actual)) = (&constant.ty, actual) {
            checker.check_assignable(
                annotation,
                &actual,
                constant.span,
                "const initializer",
                Some((&constant.value, &value_key)),
            );
        }
    }

    for test in &ast.tests {
        let mut checker = OwnerChecker::new(
            module_path,
            ExpressionOwnerKey::Test(test.name.clone()),
            TypeResolutionContext::source(module_path),
            const_env.clone(),
            expression_sources,
            type_resolution,
            callable_signatures,
            remote_public_instances,
            None,
            facts,
            remote_interface_boxes,
            constructor_validations,
            representation_constructor_validations,
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
                    expression_sources,
                    type_resolution,
                    callable_signatures,
                    remote_public_instances,
                    None,
                    facts,
                    remote_interface_boxes,
                    constructor_validations,
                    representation_constructor_validations,
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
    type_context: &TypeResolutionContext<'_>,
) -> BTreeMap<String, ResolvedTypeRef> {
    ast.consts
        .iter()
        .filter_map(|constant| {
            let ty = constant.ty.as_ref()?;
            type_resolution
                .resolve_type_ref(ty, type_context)
                .ok()
                .map(|resolved| (constant.name.clone(), resolved))
        })
        .collect()
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
    callable_signatures: &BTreeMap<String, CallableSignature>,
    remote_public_instances: Option<RemotePublicInstanceOperationResolver<'_>>,
    const_env: &BTreeMap<String, ResolvedTypeRef>,
    facts: &mut BTreeMap<ExpressionKey, ExpressionTypeFact>,
    remote_interface_boxes: &mut BTreeMap<ExpressionKey, RemotePublicInstanceOperationProjection>,
    constructor_validations: &mut BTreeMap<ExpressionKey, ConstructorValidation>,
    representation_constructor_validations: &mut BTreeMap<
        ExpressionKey,
        RepresentationConstructorValidation,
    >,
    diagnostics: &mut Vec<String>,
) {
    let type_params = inherited_type_params
        .iter()
        .chain(&function.type_params)
        .cloned()
        .collect::<BTreeSet<_>>();
    let type_context = TypeResolutionContext::with_type_params(module_path, type_params);
    let mut env = const_env.clone();
    if let Some(self_type) = &function.implicit_self {
        if let Ok(resolved) = type_resolution.resolve_type_ref(self_type, &type_context) {
            env.insert("self".to_string(), resolved);
        }
    }
    for param in &function.params {
        if let Ok(resolved) = type_resolution.resolve_type_ref(&param.ty, &type_context) {
            env.insert(param.name.clone(), resolved);
        }
    }
    let mut checker = OwnerChecker::new(
        module_path,
        owner,
        type_context,
        env,
        expression_sources,
        type_resolution,
        callable_signatures,
        remote_public_instances,
        Some(function.return_type.clone()),
        facts,
        remote_interface_boxes,
        constructor_validations,
        representation_constructor_validations,
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
        expression_sources: &'a ExpressionSourceMap,
        type_resolution: &'a TypeResolutionModel,
        callable_signatures: &'a BTreeMap<String, CallableSignature>,
        remote_public_instances: Option<RemotePublicInstanceOperationResolver<'a>>,
        return_type: Option<TypeRef>,
        facts: &'a mut BTreeMap<ExpressionKey, ExpressionTypeFact>,
        remote_interface_boxes: &'a mut BTreeMap<
            ExpressionKey,
            RemotePublicInstanceOperationProjection,
        >,
        constructor_validations: &'a mut BTreeMap<ExpressionKey, ConstructorValidation>,
        representation_constructor_validations: &'a mut BTreeMap<
            ExpressionKey,
            RepresentationConstructorValidation,
        >,
        diagnostics: &'a mut Vec<String>,
    ) -> Self {
        Self {
            module_path,
            owner,
            next_index: 0,
            type_resolution,
            expression_sources,
            callable_signatures,
            remote_public_instances,
            return_type,
            type_context,
            env,
            path_refinements: BTreeMap::new(),
            facts,
            remote_interface_boxes,
            constructor_validations,
            representation_constructor_validations,
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

    fn check_stmt(&mut self, stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Assert { condition, .. } => {
                self.check_condition(condition, "condition");
                false
            }
            Stmt::Let {
                name, ty, value, ..
            } => {
                let value_key = self.peek_key();
                let actual = self.check_expr(value);
                let binding_ty = if let Some(annotation) = ty {
                    match self
                        .type_resolution
                        .resolve_type_ref(annotation, &self.type_context)
                    {
                        Ok(expected) => {
                            if let Some(actual) = &actual {
                                self.check_value_assignable_to_expected(
                                    Some(annotation),
                                    value,
                                    &value_key,
                                    actual,
                                    &expected,
                                    &format!("local binding {name} annotation"),
                                    self.expression_span(&value_key),
                                );
                            }
                            Some(expected)
                        }
                        Err(error) => {
                            self.diagnostics.push(format!(
                                "{}: failed to resolve local binding {name} annotation: {error}",
                                self.module_path
                            ));
                            actual
                        }
                    }
                } else {
                    actual
                };
                if let Some(binding_ty) = binding_ty {
                    self.env.insert(name.clone(), binding_ty);
                }
                false
            }
            Stmt::Assign { target, value } => {
                self.check_expr(target);
                let actual = self.check_expr(value);
                if let (Expr::Identifier(name), Some(actual)) = (target, actual) {
                    self.env.insert(name.clone(), actual);
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
                let mut previous = Vec::new();
                match binding {
                    ForBinding::Item { item } => {
                        match iterable_ty.as_ref().and_then(single_for_item_type) {
                            Some(item_ty) => {
                                previous
                                    .push((item.clone(), self.env.insert(item.clone(), item_ty)));
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
            Stmt::Throw { value } | Stmt::Emit(value) => {
                self.check_expr(value);
                matches!(stmt, Stmt::Throw { .. })
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
        let saved_path_refinements = self.path_refinements.clone();
        self.apply_narrowing(narrowing);
        let exits = self.check_block(block);
        self.env = saved_env;
        self.path_refinements = saved_path_refinements;
        exits
    }

    fn check_expr_scoped(
        &mut self,
        expr: &Expr,
        narrowing: &TypeNarrowing,
    ) -> Option<ResolvedTypeRef> {
        let saved_env = self.env.clone();
        let saved_path_refinements = self.path_refinements.clone();
        self.apply_narrowing(narrowing);
        let ty = self.check_expr(expr);
        self.env = saved_env;
        self.path_refinements = saved_path_refinements;
        ty
    }

    fn apply_narrowing(&mut self, narrowing: &TypeNarrowing) {
        for (name, ty) in &narrowing.env {
            self.env.insert(name.clone(), ty.clone());
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
            "return",
            self.expression_span(&value_key),
        );
    }

    fn check_expr(&mut self, expr: &Expr) -> Option<ResolvedTypeRef> {
        self.check_expr_with_field_diagnostics(expr, true)
    }

    fn check_callee_expr(&mut self, expr: &Expr) -> Option<ResolvedTypeRef> {
        self.check_expr_with_field_diagnostics(expr, false)
    }

    fn check_expr_with_field_diagnostics(
        &mut self,
        expr: &Expr,
        diagnose_unknown_field: bool,
    ) -> Option<ResolvedTypeRef> {
        let key = self.next_key();
        let refined_ty = expr_path(expr).and_then(|path| self.path_refinements.get(&path).cloned());
        let ty = match expr {
            Expr::Literal(literal) => self.literal_type(literal),
            Expr::Identifier(name) => refined_ty.clone().or_else(|| self.env.get(name).cloned()),
            Expr::RemotePublicInstanceSource(source) => {
                if diagnose_unknown_field {
                    let mut message = format!(
                        "{}: remote public instance source `{}/{}` is not a value at {}; use `{}/{} as I` to box it or `{}/{}.method(...)` to call it directly",
                        self.module_path,
                        source.dependency_ref,
                        source.public_instance_key,
                        self.expression_span_label(&key),
                        source.dependency_ref,
                        source.public_instance_key,
                        source.dependency_ref,
                        source.public_instance_key
                    );
                    if let Some(resolver) = self.remote_public_instances {
                        if let Ok(interface_count) = resolver.public_instance_interface_count(
                            &source.dependency_ref,
                            &source.public_instance_key,
                        ) {
                            if interface_count > 1 {
                                message.push_str(&format!(
                                    "; public instance exports {interface_count} interfaces, so the interface projection cannot be inferred without `as I`"
                                ));
                            } else if interface_count == 1 {
                                message.push_str(
                                    "; `as I` is required even though the public instance exports one interface",
                                );
                            }
                        }
                    }
                    self.diagnostics.push(message);
                }
                None
            }
            Expr::Binary { op, left, right } => {
                let left_ty = self.check_expr(left);
                let right_ty = match op {
                    BinaryOp::And => {
                        let narrowing = self.condition_narrowings(left).when_true;
                        self.check_expr_scoped(right, &narrowing)
                    }
                    BinaryOp::Or => {
                        let narrowing = self.condition_narrowings(left).when_false;
                        self.check_expr_scoped(right, &narrowing)
                    }
                    _ => self.check_expr(right),
                };
                self.check_binary_operands(&key, *op, left_ty.as_ref(), right_ty.as_ref());
                self.binary_type(*op, left_ty.as_ref(), right_ty.as_ref())
            }
            Expr::Unary { op, expr } => {
                let operand_ty = self.check_expr(expr);
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
                if let Expr::RemotePublicInstanceSource(source) = value.as_ref() {
                    return self.check_remote_interface_box(&key, source, interface);
                }
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
                let object_ty = if diagnose_unknown_field {
                    self.check_expr(object)
                } else {
                    self.check_callee_expr(object)
                };
                object_ty.and_then(|object_ty| {
                    let field_ty = self.record_field_type(&object_ty, field);
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
                let fields = entries
                    .iter()
                    .map(|entry| {
                        let ty = self.check_expr(&entry.value);
                        object_literal_key_text(&entry.key).and_then(|key| ty.map(|ty| (key, ty)))
                    })
                    .flatten()
                    .collect::<BTreeMap<_, _>>();
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
                self.check_db_query_block(&query.query);
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
            },
        );
        ty
    }

    fn check_remote_interface_box(
        &mut self,
        box_key: &ExpressionKey,
        source: &crate::shared::ast::RemotePublicInstanceSource,
        interface: &TypeRef,
    ) -> Option<ResolvedTypeRef> {
        let source_key = self.next_key();
        self.facts.insert(
            source_key.clone(),
            ExpressionTypeFact {
                ty: None,
                span: self.expression_span(&source_key),
            },
        );
        let selector = match self
            .type_resolution
            .resolve_canonical_interface_selector_type_ref(interface, &self.type_context)
        {
            Ok(selector) => selector,
            Err(error) => {
                self.diagnostics.push(format!(
                    "{}: remote interface boxing selector `{}` failed at {}: {error}",
                    self.module_path,
                    interface.name,
                    self.expression_span_label(box_key)
                ));
                return None;
            }
        };
        let Some(resolver) = self.remote_public_instances else {
            self.diagnostics.push(format!(
                "{}: remote public instance source `{}/{}` cannot be boxed at {} because service dependency metadata is unavailable",
                self.module_path,
                source.dependency_ref,
                source.public_instance_key,
                self.expression_span_label(box_key)
            ));
            return None;
        };
        let projection = match resolver.resolve_projection(
            &source.dependency_ref,
            &source.public_instance_key,
            &selector.instantiation_ref,
        ) {
            Ok(projection) => projection,
            Err(error) => {
                self.diagnostics.push(format!(
                    "{}: remote public instance source `{}/{}` failed interface boxing at {}: {error}",
                    self.module_path,
                    source.dependency_ref,
                    source.public_instance_key,
                    self.expression_span_label(box_key)
                ));
                return None;
            }
        };
        self.remote_interface_boxes
            .insert(box_key.clone(), projection);
        let ty = Some(ResolvedTypeRef {
            source_text: format!("any {}", selector.source_text),
            ir: TypeRefIr::AnyInterface {
                interface: selector.instantiation_ref,
            },
        });
        self.facts.insert(
            box_key.clone(),
            ExpressionTypeFact {
                ty: ty.clone(),
                span: self.expression_span(box_key),
            },
        );
        ty
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
            self.check_db_selector(selector);
        }
        if let Some(query) = operation.independent_query() {
            self.check_db_query_block(query);
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

    fn check_db_selector(&mut self, selector: &DbSelector) {
        match selector {
            DbSelector::Key { value } => {
                self.check_expr(value);
            }
            DbSelector::Query { query } => self.check_db_query_block(query),
        }
    }

    fn check_db_query_block(&mut self, query: &DbQueryBlock) {
        for clause in &query.where_clauses {
            match clause {
                DbWhereClause::Predicate { predicate } => {
                    self.check_condition(predicate, "db where predicate");
                }
                DbWhereClause::Conditional {
                    condition,
                    predicate,
                } => {
                    self.check_condition(condition, "db where condition");
                    self.check_condition(predicate, "db where predicate");
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
    ) {
        match op {
            BinaryOp::Add if self.operands_string_concat(left, right) => {}
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                self.check_operand_assignable(key, "binary arithmetic operand", left, "number");
                self.check_operand_assignable(key, "binary arithmetic operand", right, "number");
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
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
        if let Some(return_type) =
            self.remote_public_instance_direct_call_type(callee, type_args, args, arg_types)
        {
            return Some(return_type);
        }
        if let Some(return_type) = self.runtime_receiver_call_type(key, callee) {
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
        if let Some(return_type) = self.config_intrinsic_call_type(&path, type_args) {
            return Some(return_type);
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
                let expected = self.resolve_callable_param_types(
                    &path,
                    params.iter().map(String::as_str),
                    &native_context,
                    prelude_registry().native_type_params(&path).unwrap_or(&[]),
                    type_args,
                );
                if expected.complete {
                    self.validate_resolved_call_params(&path, expected.params, args, arg_types);
                }
            }
            return self.resolve_callable_return_type(
                &return_type,
                &native_context,
                prelude_registry().native_type_params(&path).unwrap_or(&[]),
                type_args,
            );
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
                self.validate_resolved_call_params(
                    &declaration_name,
                    expected.params,
                    args,
                    arg_types,
                );
            }
            let return_type = self.resolve_callable_return_type(
                &return_type.name,
                &signature_context,
                &type_params,
                type_args,
            )?;
            return Some(if signature.module_path == self.module_path {
                return_type
            } else {
                self.type_resolution
                    .externalize_local_type_refs(&return_type, &signature.module_path)
            });
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
                        ir: TypeRefIr::Native {
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

    fn remote_public_instance_direct_call_type(
        &mut self,
        callee: &Expr,
        type_args: &[TypeRef],
        args: &[Expr],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Option<ResolvedTypeRef> {
        let Expr::Field { object, field } = callee else {
            return None;
        };
        let Expr::RemotePublicInstanceSource(source) = object.as_ref() else {
            return None;
        };
        let Some(resolver) = self.remote_public_instances else {
            self.diagnostics.push(format!(
                "{}: remote public instance direct call `{}/{}` cannot resolve method `{field}` because service dependency metadata is unavailable",
                self.module_path, source.dependency_ref, source.public_instance_key
            ));
            return None;
        };
        if !type_args.is_empty() {
            self.diagnostics.push(format!(
                "{}: remote public instance direct call `{}/{}.{field}` does not accept method type arguments",
                self.module_path, source.dependency_ref, source.public_instance_key
            ));
        }
        let operation = match resolver.resolve_direct_method(
            &source.dependency_ref,
            &source.public_instance_key,
            field,
        ) {
            Ok(operation) => operation,
            Err(error) => {
                self.diagnostics.push(format!(
                    "{}: remote public instance direct call `{}/{}.{field}` failed: {error}",
                    self.module_path, source.dependency_ref, source.public_instance_key
                ));
                return None;
            }
        };
        let expected = operation
            .public_signature
            .params
            .iter()
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
        self.validate_resolved_call_params(
            &format!(
                "{}/{}.{}",
                source.dependency_ref, source.public_instance_key, field
            ),
            expected,
            args,
            arg_types,
        );
        Some(ResolvedTypeRef {
            source_text: type_ref_debug_text(&operation.public_signature.return_type),
            ir: operation.public_signature.return_type,
        })
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
        &self,
        key: &ExpressionKey,
        callee: &Expr,
    ) -> Option<ResolvedTypeRef> {
        let (_, method_name) = receiver_call_parts(callee)?;
        let offset = 1 + receiver_object_offset_in_callee(callee)?;
        let receiver_ty = self.expression_type_at_offset(key, offset)?;
        builtin_receiver_call_return_type(&receiver_ty, method_name)
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

    fn db_operation_type(
        &self,
        operation: &crate::shared::ast::DbOperation,
    ) -> Option<ResolvedTypeRef> {
        let target = self
            .type_resolution
            .resolve_type_ref(&operation.target, &self.type_context)
            .ok()?;
        let read = if operation.projection.is_some() {
            projection_record_type("ReadonlyProjectionRecord", &target)
        } else {
            target.clone()
        };
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
        context: &str,
        fallback_span: SourceSpan,
    ) -> bool {
        let assignability = ExpressionAssignability::new(
            self.module_path,
            self.expression_sources,
            self.type_resolution,
            &self.type_context,
        );
        if assignability.value_assignable_to_expected(annotation, value, actual, expected) {
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
        self.push_type_mismatch(context, fallback_span, expected, actual);
        false
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
    let TypeRefIr::Native { name, args } = &ty.ir else {
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

fn map_entry_types(ty: &ResolvedTypeRef) -> Option<(ResolvedTypeRef, ResolvedTypeRef)> {
    let TypeRefIr::Native { name, args } = &ty.ir else {
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

fn type_contains_type_param(ty: &TypeRefIr) -> bool {
    match ty {
        TypeRefIr::TypeParam { .. } => true,
        TypeRefIr::Native { args, .. } | TypeRefIr::Union { items: args } => {
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
        .map(|(module_path, _)| {
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
        ir: TypeRefIr::Native {
            name: name.to_string(),
            args: vec![target.ir.clone()],
        },
        source_text: format!("{name}<{}>", target.source_text),
    }
}

fn catch_result_type(value: ResolvedTypeRef, error: ResolvedTypeRef) -> ResolvedTypeRef {
    ResolvedTypeRef {
        ir: TypeRefIr::Native {
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
        TypeRefIr::Native { name, args } if name == "CatchResult" && args.len() == 2 => match field
        {
            "tag" => Some(resolved_type_from_ir(&union_type_ir(vec![
                literal_string_type("ok"),
                literal_string_type("err"),
            ]))),
            _ => None,
        },
        TypeRefIr::Native { name, args } if name == "DbUpsertResult" && args.len() == 1 => {
            match field {
                "inserted" => Some(resolved_type_from_ir(&TypeRefIr::Native {
                    name: "bool".to_string(),
                    args: Vec::new(),
                })),
                "value" => Some(resolved_type_from_ir(&args[0])),
                _ => None,
            }
        }
        TypeRefIr::Native { name, args } if name == "Exception" && args.len() == 1 => match field {
            "error" => Some(resolved_type_from_ir(&args[0])),
            _ => None,
        },
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
        BuiltinReceiverPublicReturnType::MapKeyArray => TypeRefIr::Native {
            name: "Array".to_string(),
            args: vec![map_key_type_ir(&receiver_ty.ir)?],
        },
    };
    Some(resolved_type_from_ir(&ty))
}

fn runtime_receiver_root_from_type_ref(ty: &TypeRefIr) -> Option<String> {
    match ty {
        TypeRefIr::Native { name, .. } => Some(canonical_runtime_receiver_root(name).to_string()),
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

fn canonical_runtime_receiver_root(root: &str) -> &str {
    skiff_artifact_model::canonical_runtime_receiver_root(root)
}

fn array_item_type_ir(ty: &TypeRefIr) -> Option<TypeRefIr> {
    let TypeRefIr::Native { name, args } = ty else {
        return None;
    };
    (matches!(
        name.as_str(),
        "Array" | "Stream" | "std.collection.Array" | "std.stream.Stream"
    ) && args.len() == 1)
        .then(|| args[0].clone())
}

fn map_value_type_ir(ty: &TypeRefIr) -> Option<TypeRefIr> {
    let TypeRefIr::Native { name, args } = ty else {
        return None;
    };
    (matches!(name.as_str(), "Map" | "std.collection.Map") && args.len() == 2)
        .then(|| args[1].clone())
}

fn map_key_type_ir(ty: &TypeRefIr) -> Option<TypeRefIr> {
    let TypeRefIr::Native { name, args } = ty else {
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
        TypeRefIr::Native { name, args } if name == "CatchResult" && args.len() == 2 => {
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
    TypeRefIr::Native {
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
    let string = TypeRefIr::Native {
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
        ir: TypeRefIr::Native {
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
        TypeRefIr::Native { name, args } if args.is_empty() => name.clone(),
        TypeRefIr::Native { name, args } => format!(
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
            if interface.canonical_type_args.is_empty() {
                format!("any {}", interface.interface_abi_id)
            } else {
                format!(
                    "any {}<{}>",
                    interface.interface_abi_id,
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

fn builtin_type(name: &str) -> TypeRefIr {
    TypeRefIr::Native {
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
    matches!(ty, TypeRefIr::Native { name, args } if args.is_empty() && (name == "void" || name == "null"))
        || type_ir_is_null(ty)
}

fn type_ir_is_never(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Native { name, args } if args.is_empty() && name == "never")
}

fn type_ir_is_null(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Native { name, .. } if name == "null")
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
        parsed_sources::parse_publication_sources, source_graph::CompilerSourceFile,
        PublicationTypeSymbolIndex,
    };

    use super::*;

    const ANY_INTERFACE_MODULE: &str = "internal.any_interface";

    fn expression_type_result(
        source_text: &str,
    ) -> Result<ExpressionTypeModel, ExpressionTypeModelBuildError> {
        let source = CompilerSourceFile::parse(
            PathBuf::from("internal/any_interface.skiff"),
            ANY_INTERFACE_MODULE.to_string(),
            false,
            false,
            source_text.to_string(),
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
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type resolution should build");
        let expression_sources = ExpressionSourceMap::build(&parsed_sources)
            .expect("expression source facts should build");
        ExpressionTypeModel::build(&parsed_sources, &expression_sources, &type_resolution, None)
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
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type resolution should build");
        let expression_sources = ExpressionSourceMap::build(&parsed_sources)
            .expect("expression source facts should build");
        let model = ExpressionTypeModel::build(
            &parsed_sources,
            &expression_sources,
            &type_resolution,
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
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type resolution should build");
        let expression_sources = ExpressionSourceMap::build(&parsed_sources)
            .expect("expression source facts should build");

        let error = ExpressionTypeModel::build(
            &parsed_sources,
            &expression_sources,
            &type_resolution,
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
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type resolution should build");
        let expression_sources = ExpressionSourceMap::build(&parsed_sources)
            .expect("expression source facts should build");

        ExpressionTypeModel::build(&parsed_sources, &expression_sources, &type_resolution, None)
            .expect("DbUpsertResult.inserted and .value fields should type-check statically");

        let user_ir = TypeRefIr::Record {
            fields: BTreeMap::from([(
                "name".to_string(),
                TypeRefIr::Native {
                    name: "string".to_string(),
                    args: Vec::new(),
                },
            )]),
        };
        let result_ir = TypeRefIr::Native {
            name: "DbUpsertResult".to_string(),
            args: vec![user_ir.clone()],
        };
        assert_eq!(
            record_field_type_from_ir(&result_ir, "inserted")
                .expect("inserted field should resolve")
                .ir,
            TypeRefIr::Native {
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
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type resolution should build");
        let expression_sources = ExpressionSourceMap::build(&parsed_sources)
            .expect("expression source facts should build");

        let model = ExpressionTypeModel::build(
            &parsed_sources,
            &expression_sources,
            &type_resolution,
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
