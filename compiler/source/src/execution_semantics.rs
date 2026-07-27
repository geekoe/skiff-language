use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    builtin_receiver_op_spec_by_name, BuiltinReceiverOp, CallableEffectSummary,
};

use crate::{
    parsed_sources::ParsedCompilerSource,
    semantic::impl_method_declaration_name,
    shared::{
        ast::{Block, DbOperationKind, Expr, ForBinding, FunctionDecl, Pattern, Stmt, ValueBlock},
        ast_utils::{expr_path, walk_expr, walk_stmt, AstVisitor},
        error::SourceSpan,
    },
    ExpressionKey, ExpressionOwnerKey, ExpressionSourceMap, ResolvedCallTarget,
    ResolvedCallTargetFacts, SourceCallableEffectFacts, SourceCompileError, SourceSymbolKey,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionSourceSite {
    pub module_path: String,
    pub owner: ExpressionOwnerKey,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeoutSourcePlan {
    pub duration_milliseconds: u64,
    pub produces_value: bool,
    pub source_site: ExecutionSourceSite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConcurrentLaneKind {
    Statement,
    Serial,
    Tail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConcurrentLanePlan {
    pub source_order: u32,
    pub kind: ConcurrentLaneKind,
    pub dependencies: Vec<u32>,
    pub source_site: ExecutionSourceSite,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConcurrentSourcePlan {
    pub produces_value: bool,
    pub lanes: Vec<ConcurrentLanePlan>,
    pub source_site: ExecutionSourceSite,
}

#[derive(Clone, Debug, Default)]
pub struct SourceExecutionSemantics {
    timeout_plans: Vec<TimeoutSourcePlan>,
    concurrent_plans: Vec<ConcurrentSourcePlan>,
}

impl SourceExecutionSemantics {
    pub fn timeout_plans(&self) -> &[TimeoutSourcePlan] {
        &self.timeout_plans
    }

    pub fn concurrent_plans(&self) -> &[ConcurrentSourcePlan] {
        &self.concurrent_plans
    }

    pub fn validate_complete(&self) -> Result<(), SourceCompileError> {
        for plan in &self.timeout_plans {
            if plan.duration_milliseconds == 0 || !valid_source_site(&plan.source_site) {
                return invalid_execution_plan(&plan.source_site, "timeout plan");
            }
        }
        for plan in &self.concurrent_plans {
            if !valid_source_site(&plan.source_site) {
                return invalid_execution_plan(&plan.source_site, "concurrent source site");
            }
            let tail_indexes = plan
                .lanes
                .iter()
                .enumerate()
                .filter_map(|(index, lane)| {
                    (lane.kind == ConcurrentLaneKind::Tail).then_some(index)
                })
                .collect::<Vec<_>>();
            let valid_tail_shape = if plan.produces_value {
                tail_indexes == [plan.lanes.len().saturating_sub(1)]
            } else {
                tail_indexes.is_empty()
            };
            if !valid_tail_shape {
                return invalid_execution_plan(&plan.source_site, "concurrent tail shape");
            }
            for (index, lane) in plan.lanes.iter().enumerate() {
                if lane.source_order as usize != index
                    || !valid_source_site(&lane.source_site)
                    || lane.source_site.module_path != plan.source_site.module_path
                    || lane.source_site.owner != plan.source_site.owner
                    || lane.dependencies.windows(2).any(|pair| pair[0] >= pair[1])
                    || lane
                        .dependencies
                        .iter()
                        .any(|dependency| *dependency >= lane.source_order)
                {
                    return invalid_execution_plan(&plan.source_site, "concurrent lane");
                }
                if lane.kind == ConcurrentLaneKind::Tail
                    && lane.dependencies != (0..lane.source_order).collect::<Vec<_>>()
                {
                    return invalid_execution_plan(
                        &plan.source_site,
                        "concurrent tail dependencies",
                    );
                }
            }
        }
        Ok(())
    }
}

fn valid_source_site(site: &ExecutionSourceSite) -> bool {
    !site.module_path.trim().is_empty()
        && site.span.start.offset <= site.span.end.offset
        && (site.span.start.offset < site.span.end.offset
            || site.span.start.line == site.span.end.line)
}

fn invalid_execution_plan<T>(
    site: &ExecutionSourceSite,
    subject: &str,
) -> Result<T, SourceCompileError> {
    Err(SourceCompileError::ContractValidation {
        message: format!(
            "source execution semantics contains an invalid {subject} in {} {:?}",
            site.module_path, site.owner
        ),
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CallableEffectProfile {
    accesses: BTreeSet<ExternalAccess>,
    opaque: bool,
}

impl CallableEffectProfile {
    fn join(&mut self, other: &Self) {
        self.accesses.extend(other.accesses.iter().cloned());
        self.opaque |= other.opaque;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ExternalAccessMode {
    Read,
    Write,
    Exclusive,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ExternalAccess {
    target: String,
    conflict_key: String,
    mode: ExternalAccessMode,
    cancel_safe: bool,
}

impl ExternalAccess {
    fn database(target: &str, mode: ExternalAccessMode) -> Self {
        Self {
            target: format!("db:{target}"),
            conflict_key: target.to_string(),
            mode,
            // Database operations are compiler-owned and have a defined
            // response-discard/transaction terminal. Unknown host/package
            // operations never reach this constructor.
            cancel_safe: true,
        }
    }
}

struct CallableDefinition<'a> {
    key: SourceSymbolKey,
    owner: ExpressionOwnerKey,
    function: &'a FunctionDecl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingRoot {
    Outer,
    ConcurrentSibling,
    LaneLocalFresh,
    LaneLocalOpaque,
    Scalar,
}

type Scope = BTreeMap<String, BindingRoot>;

pub(crate) fn analyze_source_execution_semantics(
    parsed_sources: &[ParsedCompilerSource],
    expression_sources: &ExpressionSourceMap,
    resolved_targets: &ResolvedCallTargetFacts,
    callable_effects: &SourceCallableEffectFacts,
) -> Result<SourceExecutionSemantics, SourceCompileError> {
    let definitions = callable_definitions(parsed_sources);
    let expression_keys = expression_key_index(&definitions);
    let callable_profiles =
        callable_effect_profiles(&definitions, &expression_keys, resolved_targets);
    let mut semantics = SourceExecutionSemantics::default();
    let mut diagnostics = Vec::new();
    reject_static_execution_scopes(parsed_sources, &mut diagnostics);

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
                &callable_profiles,
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
                    &callable_profiles,
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

fn top_level_value_names(ast: &crate::shared::ast::SourceFile) -> BTreeSet<String> {
    let mut names = ast
        .imports
        .iter()
        .filter_map(|import| {
            import
                .local_binding
                .as_ref()
                .or(import.alias.as_ref())
                .or_else(|| import.path.last())
                .cloned()
        })
        .collect::<BTreeSet<_>>();
    names.extend(
        ast.functions
            .iter()
            .map(|declaration| declaration.name.clone()),
    );
    names.extend(
        ast.function_signatures
            .iter()
            .map(|declaration| declaration.name.clone()),
    );
    names.extend(
        ast.consts
            .iter()
            .map(|declaration| declaration.name.clone()),
    );
    names
}

fn reject_static_execution_scopes(
    parsed_sources: &[ParsedCompilerSource],
    diagnostics: &mut Vec<String>,
) {
    for parsed in parsed_sources
        .iter()
        .filter(|parsed| !parsed.source().is_test_file)
    {
        for declaration in &parsed.ast().consts {
            let mut detector = ExecutionScopeDetector::default();
            detector.visit_expr(&declaration.value);
            if let Some(kind) = detector.first {
                diagnostics.push(format!(
                    "{} Const({:?}): {kind} execution scope is not allowed in a top-level const",
                    parsed.module_path(),
                    declaration.name
                ));
            }
        }
        for db in &parsed.ast().dbs {
            for index in &db.indexes {
                let Some(predicate) = &index.where_expr else {
                    continue;
                };
                let mut detector = ExecutionScopeDetector::default();
                detector.visit_expr(predicate);
                if let Some(kind) = detector.first {
                    diagnostics.push(format!(
                        "{} db index `{}`: {kind} execution scope is not allowed in a static DB index predicate",
                        parsed.module_path(),
                        index.name
                    ));
                }
            }
        }
    }
}

#[derive(Default)]
struct ExecutionScopeDetector {
    first: Option<&'static str>,
}

impl AstVisitor for ExecutionScopeDetector {
    fn visit_stmt(&mut self, statement: &Stmt) {
        if self.first.is_none() {
            self.first = match statement {
                Stmt::Timeout { .. } => Some("timeout"),
                Stmt::Concurrent { .. } => Some("concurrent"),
                Stmt::Serial { .. } => Some("serial"),
                Stmt::CompilerTestEffectRegister { .. }
                | Stmt::Assert { .. }
                | Stmt::Let { .. }
                | Stmt::Assign { .. }
                | Stmt::If { .. }
                | Stmt::For { .. }
                | Stmt::Match { .. }
                | Stmt::DbTransaction { .. }
                | Stmt::Throw { .. }
                | Stmt::Rethrow { .. }
                | Stmt::Emit(_)
                | Stmt::Return(_)
                | Stmt::Spawn { .. }
                | Stmt::Break
                | Stmt::Continue
                | Stmt::Expr(_) => None,
            };
        }
        walk_stmt(self, statement);
    }

    fn visit_expr(&mut self, expression: &Expr) {
        if self.first.is_none() {
            self.first = match expression {
                Expr::ValueBlock(_) => Some("value"),
                Expr::ConcurrentValue(_) => Some("concurrent value"),
                Expr::Timeout { .. } => Some("timeout"),
                Expr::Literal(_)
                | Expr::Identifier(_)
                | Expr::DependencySourceAddress(_)
                | Expr::Binary { .. }
                | Expr::Unary { .. }
                | Expr::Call { .. }
                | Expr::Generic { .. }
                | Expr::InterfaceBox { .. }
                | Expr::Field { .. }
                | Expr::Record { .. }
                | Expr::ObjectLiteral { .. }
                | Expr::Patch { .. }
                | Expr::Throw { .. }
                | Expr::Rethrow { .. }
                | Expr::Catch { .. }
                | Expr::DbOperation(_)
                | Expr::DbQuery(_)
                | Expr::DbTransaction(_)
                | Expr::DbLeaseClaim(_)
                | Expr::DbLeaseRead(_) => None,
            };
        }
        walk_expr(self, expression);
    }
}

fn callable_definitions(parsed_sources: &[ParsedCompilerSource]) -> Vec<CallableDefinition<'_>> {
    let mut definitions = Vec::new();
    for parsed in parsed_sources
        .iter()
        .filter(|parsed| !parsed.source().is_test_file)
    {
        for function in &parsed.ast().functions {
            if !function.is_native && !function.is_provider {
                definitions.push(CallableDefinition {
                    key: SourceSymbolKey::new(parsed.module_path(), &function.name),
                    owner: ExpressionOwnerKey::Function(function.name.clone()),
                    function,
                });
            }
        }
        for implementation in &parsed.ast().impls {
            for method in &implementation.method_bodies {
                if !method.is_native && !method.is_provider {
                    definitions.push(CallableDefinition {
                        key: SourceSymbolKey::new(
                            parsed.module_path(),
                            impl_method_declaration_name(&implementation.target, &method.name),
                        ),
                        owner: ExpressionOwnerKey::ImplMethod {
                            type_name: implementation.target.clone(),
                            method: method.name.clone(),
                        },
                        function: method,
                    });
                }
            }
        }
    }
    definitions
}

fn expression_key_index(definitions: &[CallableDefinition<'_>]) -> BTreeMap<usize, ExpressionKey> {
    let mut keys = BTreeMap::new();
    for definition in definitions {
        let mut indexer = ExpressionKeyIndexer {
            module_path: definition.key.module_path(),
            owner: definition.owner.clone(),
            next_index: 0,
            keys: &mut keys,
        };
        indexer.visit_block(&definition.function.body);
    }
    keys
}

struct ExpressionKeyIndexer<'a> {
    module_path: &'a str,
    owner: ExpressionOwnerKey,
    next_index: u32,
    keys: &'a mut BTreeMap<usize, ExpressionKey>,
}

impl AstVisitor for ExpressionKeyIndexer<'_> {
    fn visit_expr(&mut self, expr: &Expr) {
        let key = ExpressionKey::new(self.module_path, self.owner.clone(), self.next_index);
        self.next_index = self
            .next_index
            .checked_add(1)
            .expect("ExpressionSourceMap rejects expression preorder overflow");
        self.keys.insert(expr_address(expr), key);
        walk_expr(self, expr);
    }
}

fn callable_effect_profiles(
    definitions: &[CallableDefinition<'_>],
    expression_keys: &BTreeMap<usize, ExpressionKey>,
    resolved_targets: &ResolvedCallTargetFacts,
) -> BTreeMap<SourceSymbolKey, CallableEffectProfile> {
    let mut profiles = definitions
        .iter()
        .map(|definition| (definition.key.clone(), CallableEffectProfile::default()))
        .collect::<BTreeMap<_, _>>();
    for _ in 0..=definitions.len() {
        let next = definitions
            .iter()
            .map(|definition| {
                let mut collector = EffectCollector {
                    expression_keys,
                    resolved_targets,
                    callable_profiles: &profiles,
                    profile: CallableEffectProfile::default(),
                };
                collector.visit_block(&definition.function.body);
                (definition.key.clone(), collector.profile)
            })
            .collect::<BTreeMap<_, _>>();
        if next == profiles {
            return profiles;
        }
        profiles = next;
    }
    profiles
        .values_mut()
        .for_each(|profile| profile.opaque = true);
    profiles
}

struct EffectCollector<'a> {
    expression_keys: &'a BTreeMap<usize, ExpressionKey>,
    resolved_targets: &'a ResolvedCallTargetFacts,
    callable_profiles: &'a BTreeMap<SourceSymbolKey, CallableEffectProfile>,
    profile: CallableEffectProfile,
}

impl AstVisitor for EffectCollector<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if matches!(stmt, Stmt::DbTransaction { .. }) {
            self.profile.accesses.insert(ExternalAccess::database(
                "transaction",
                ExternalAccessMode::Exclusive,
            ));
        }
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::DbOperation(operation) => {
                let mode = match operation.op {
                    DbOperationKind::Find
                    | DbOperationKind::Optional
                    | DbOperationKind::Require
                    | DbOperationKind::Count
                    | DbOperationKind::Exists => ExternalAccessMode::Read,
                    DbOperationKind::Insert
                    | DbOperationKind::Update
                    | DbOperationKind::Upsert
                    | DbOperationKind::Replace
                    | DbOperationKind::Delete => ExternalAccessMode::Write,
                };
                self.profile
                    .accesses
                    .insert(ExternalAccess::database(&operation.target.name, mode));
            }
            Expr::DbQuery(query) => {
                self.profile.accesses.insert(ExternalAccess::database(
                    &query.target.name,
                    ExternalAccessMode::Read,
                ));
            }
            Expr::DbTransaction(_) => {
                self.profile.accesses.insert(ExternalAccess::database(
                    "transaction",
                    ExternalAccessMode::Exclusive,
                ));
            }
            Expr::DbLeaseClaim(claim) => {
                self.profile.accesses.insert(ExternalAccess::database(
                    &claim.target.name,
                    ExternalAccessMode::Exclusive,
                ));
            }
            Expr::DbLeaseRead(read) => {
                self.profile.accesses.insert(ExternalAccess::database(
                    &read.target.name,
                    ExternalAccessMode::Read,
                ));
            }
            Expr::Call { .. } => self.collect_call(expr),
            Expr::Literal(_)
            | Expr::Identifier(_)
            | Expr::DependencySourceAddress(_)
            | Expr::Binary { .. }
            | Expr::Unary { .. }
            | Expr::Generic { .. }
            | Expr::InterfaceBox { .. }
            | Expr::Field { .. }
            | Expr::Record { .. }
            | Expr::ObjectLiteral { .. }
            | Expr::Patch { .. }
            | Expr::ValueBlock(_)
            | Expr::ConcurrentValue(_)
            | Expr::Timeout { .. }
            | Expr::Throw { .. }
            | Expr::Rethrow { .. }
            | Expr::Catch { .. } => {}
        }
        walk_expr(self, expr);
    }
}

impl EffectCollector<'_> {
    fn collect_call(&mut self, expr: &Expr) {
        let Some(key) = self.expression_keys.get(&expr_address(expr)) else {
            self.profile.opaque = true;
            return;
        };
        let Some(target) = self.resolved_targets.target(key) else {
            self.profile.opaque = true;
            return;
        };
        match target {
            ResolvedCallTarget::ConfigIntrinsic { .. }
            | ResolvedCallTarget::ReceiverBuiltin { .. } => {}
            ResolvedCallTarget::LocalFunction { source_callable }
            | ResolvedCallTarget::LocalImplMethod { source_callable } => {
                if let Some(profile) = self.callable_profiles.get(source_callable) {
                    self.profile.join(profile);
                } else {
                    self.profile.opaque = true;
                }
            }
            ResolvedCallTarget::ActorMethod { .. }
            | ResolvedCallTarget::NativeFunction { .. }
            | ResolvedCallTarget::DependencyPackageFunction { .. }
            | ResolvedCallTarget::InterfaceMethod { .. }
            | ResolvedCallTarget::ContractOperation { .. }
            | ResolvedCallTarget::Unknown { .. } => {
                // Current artifacts do not publish the target/conflict/cancel
                // tuple required to prove these calls concurrent-safe. The
                // source boundary therefore rejects rather than inferring from
                // a target name or a maySuspend bit.
                self.profile.opaque = true;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
struct OwnerAnalyzer<'a> {
    module_path: &'a str,
    owner: ExpressionOwnerKey,
    function: &'a FunctionDecl,
    expression_sources: &'a ExpressionSourceMap,
    expression_keys: &'a BTreeMap<usize, ExpressionKey>,
    resolved_targets: &'a ResolvedCallTargetFacts,
    callable_effects: &'a SourceCallableEffectFacts,
    callable_profiles: &'a BTreeMap<SourceSymbolKey, CallableEffectProfile>,
    semantics: &'a mut SourceExecutionSemantics,
    diagnostics: &'a mut Vec<String>,
    all_local_names: BTreeSet<String>,
    top_level_value_names: BTreeSet<String>,
}

impl<'a> OwnerAnalyzer<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        module_path: &'a str,
        owner: ExpressionOwnerKey,
        function: &'a FunctionDecl,
        expression_sources: &'a ExpressionSourceMap,
        expression_keys: &'a BTreeMap<usize, ExpressionKey>,
        resolved_targets: &'a ResolvedCallTargetFacts,
        callable_effects: &'a SourceCallableEffectFacts,
        callable_profiles: &'a BTreeMap<SourceSymbolKey, CallableEffectProfile>,
        top_level_value_names: BTreeSet<String>,
        semantics: &'a mut SourceExecutionSemantics,
        diagnostics: &'a mut Vec<String>,
    ) -> Self {
        let mut local_names = LocalNameCollector::default();
        local_names.visit_block(&function.body);
        Self {
            module_path,
            owner,
            function,
            expression_sources,
            expression_keys,
            resolved_targets,
            callable_effects,
            callable_profiles,
            semantics,
            diagnostics,
            all_local_names: local_names.names,
            top_level_value_names,
        }
    }

    fn analyze(&mut self) {
        let mut scope = self
            .function
            .params
            .iter()
            .map(|parameter| (parameter.name.clone(), BindingRoot::Outer))
            .collect::<Scope>();
        if self.function.implicit_self.is_some() {
            scope.insert("self".to_string(), BindingRoot::Outer);
        }
        self.validate_block(
            &self.function.body,
            &mut scope,
            ValidationContext::default(),
        );
    }

    fn validate_block(&mut self, block: &Block, scope: &mut Scope, context: ValidationContext) {
        for statement in &block.statements {
            self.validate_stmt(statement, scope, context);
        }
    }

    fn validate_stmt(&mut self, statement: &Stmt, scope: &mut Scope, context: ValidationContext) {
        match statement {
            Stmt::CompilerTestEffectRegister {
                target_probe,
                expect,
                step_expect,
                outcome,
                ..
            } => {
                self.validate_expr(target_probe, scope, context);
                if let Some(expect) = expect {
                    self.validate_expr(expect, scope, context);
                }
                if let Some(step_expect) = step_expect {
                    self.validate_expr(step_expect, scope, context);
                }
                match outcome {
                    crate::shared::ast::TestEffectStepOutcome::Respond { value }
                    | crate::shared::ast::TestEffectStepOutcome::Throw { value } => {
                        self.validate_expr(value, scope, context);
                    }
                    crate::shared::ast::TestEffectStepOutcome::Stream { events } => {
                        for event in events {
                            self.validate_expr(event, scope, context);
                        }
                    }
                }
            }
            Stmt::Assert { condition, .. } => self.validate_expr(condition, scope, context),
            Stmt::Let {
                mutable,
                name,
                value,
                ..
            } => {
                self.validate_expr(value, scope, context);
                scope.insert(
                    name.clone(),
                    binding_root_for_value(value, scope, context.in_lane, *mutable),
                );
            }
            Stmt::Assign { target, value } => {
                self.validate_expr(target, scope, context);
                self.validate_expr(value, scope, context);
                self.validate_mutation_target(target, scope, context);
                self.taint_lane_local_root_from_payloads(target, [value], scope, context);
            }
            Stmt::Timeout { duration, body } => {
                self.record_timeout(duration, false, duration.span);
                let mut nested = scope.clone();
                self.validate_block(body, &mut nested, context);
            }
            Stmt::Concurrent { body } => {
                self.validate_concurrent(body, None, scope, context, false);
            }
            Stmt::Serial { body } => {
                self.diagnostic("serial is only legal as a direct concurrent lane");
                let mut nested = scope.clone();
                self.validate_block(body, &mut nested, context);
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                self.validate_expr(condition, scope, context);
                let mut then_scope = scope.clone();
                self.validate_block(then_block, &mut then_scope, context);
                if let Some(else_block) = else_block {
                    let mut else_scope = scope.clone();
                    self.validate_block(else_block, &mut else_scope, context);
                }
            }
            Stmt::For {
                binding,
                iterable,
                body,
            } => {
                self.validate_expr(iterable, scope, context);
                let mut nested = scope.clone();
                match binding {
                    ForBinding::Item { item } => {
                        nested.insert(item.clone(), BindingRoot::LaneLocalOpaque);
                    }
                    ForBinding::Entry { key, value } => {
                        nested.insert(key.clone(), BindingRoot::LaneLocalOpaque);
                        nested.insert(value.clone(), BindingRoot::LaneLocalOpaque);
                    }
                }
                self.validate_block(body, &mut nested, context);
            }
            Stmt::Match { value, arms } => {
                self.validate_expr(value, scope, context);
                for arm in arms {
                    let mut nested = scope.clone();
                    let mut bindings = BTreeSet::new();
                    pattern_bindings(&arm.pattern, &mut bindings);
                    nested.extend(
                        bindings
                            .into_iter()
                            .map(|name| (name, BindingRoot::LaneLocalOpaque)),
                    );
                    self.validate_block(&arm.body, &mut nested, context);
                }
            }
            Stmt::DbTransaction { body } => {
                let mut nested = scope.clone();
                self.validate_block(body, &mut nested, context);
            }
            Stmt::Throw { value } => self.validate_expr(value, scope, context),
            Stmt::Rethrow { exception } => self.validate_expr(exception, scope, context),
            Stmt::Emit(value) | Stmt::Expr(value) => {
                self.validate_expr(value, scope, context);
            }
            Stmt::Return(value) => {
                if context.value_boundary {
                    self.diagnostic("value block control flow cannot cross the value boundary");
                }
                if let Some(value) = value {
                    self.validate_expr(value, scope, context);
                }
            }
            Stmt::Spawn { call } => self.validate_expr(call, scope, context),
            Stmt::Break | Stmt::Continue => {
                if context.value_boundary {
                    self.diagnostic("value block control flow cannot cross the value boundary");
                }
            }
        }
    }

    fn validate_expr(&mut self, expression: &Expr, scope: &mut Scope, context: ValidationContext) {
        match expression {
            Expr::Literal(_) | Expr::DependencySourceAddress(_) => {}
            Expr::Identifier(name) => {
                if !scope.contains_key(name)
                    && self.all_local_names.contains(name)
                    && !self.top_level_value_names.contains(name)
                {
                    self.diagnostic(format!(
                        "unresolved local name `{name}` outside lexical scope"
                    ));
                }
            }
            Expr::Binary { left, right, .. } => {
                self.validate_expr(left, scope, context);
                self.validate_expr(right, scope, context);
            }
            Expr::Unary { expr, .. } | Expr::Generic { callee: expr, .. } => {
                self.validate_expr(expr, scope, context);
            }
            Expr::Call { callee, args } => {
                self.validate_expr(callee, scope, context);
                for argument in args {
                    self.validate_expr(argument, scope, context);
                }
                self.validate_mutating_call(expression, scope, context);
            }
            Expr::InterfaceBox { value, .. } => self.validate_expr(value, scope, context),
            Expr::Field { object, .. } => self.validate_expr(object, scope, context),
            Expr::Record { fields, .. } => {
                for (_, value) in fields {
                    self.validate_expr(value, scope, context);
                }
            }
            Expr::ObjectLiteral { entries } => {
                for entry in entries {
                    self.validate_expr(&entry.value, scope, context);
                }
            }
            Expr::Patch { operations, .. } => {
                for operation in operations {
                    match operation {
                        crate::shared::ast::PatchOperation::Set { value, .. }
                        | crate::shared::ast::PatchOperation::Inc { value, .. } => {
                            self.validate_expr(value, scope, context);
                        }
                    }
                }
            }
            Expr::ValueBlock(value) => self.validate_value_block(value, scope, context, false),
            Expr::ConcurrentValue(value) => {
                self.validate_concurrent(&value.body, Some(&value.tail), scope, context, true);
            }
            Expr::Timeout { duration, value } => {
                self.record_timeout(duration, true, self.expr_span(expression));
                self.validate_expr(value, scope, context);
            }
            Expr::Throw { value } => self.validate_expr(value, scope, context),
            Expr::Rethrow { exception } => self.validate_expr(exception, scope, context),
            Expr::Catch { try_expr, .. } => self.validate_expr(try_expr, scope, context),
            Expr::DbOperation(operation) => {
                let mut visitor = NestedExpressionValidator {
                    analyzer: self,
                    scope,
                    context,
                };
                walk_expr(&mut visitor, expression);
                let _ = operation;
            }
            Expr::DbQuery(_) | Expr::DbLeaseRead(_) => {
                let mut visitor = NestedExpressionValidator {
                    analyzer: self,
                    scope,
                    context,
                };
                walk_expr(&mut visitor, expression);
            }
            Expr::DbTransaction(transaction) => {
                let mut nested = scope.clone();
                if context.in_lane {
                    for statement in &transaction.body.statements {
                        self.validate_lane_stmt(
                            statement,
                            &mut nested,
                            context,
                            ConcurrentLaneKind::Statement,
                        );
                    }
                } else {
                    self.validate_block(&transaction.body, &mut nested, context);
                }
            }
            Expr::DbLeaseClaim(claim) => {
                self.validate_expr(&claim.key, scope, context);
                let mut nested = scope.clone();
                if let Some(binding) = &claim.binding {
                    nested.insert(binding.clone(), BindingRoot::LaneLocalOpaque);
                }
                self.validate_block(&claim.body, &mut nested, context);
            }
        }
    }

    fn validate_value_block(
        &mut self,
        value: &ValueBlock,
        scope: &Scope,
        context: ValidationContext,
        concurrent: bool,
    ) {
        if concurrent {
            self.validate_concurrent(&value.body, Some(&value.tail), scope, context, true);
            return;
        }
        let mut nested = scope.clone();
        let value_context = ValidationContext {
            value_boundary: true,
            ..context
        };
        self.validate_block(&value.body, &mut nested, value_context);
        self.validate_expr(&value.tail, &mut nested, value_context);
    }

    fn validate_concurrent(
        &mut self,
        body: &Block,
        tail: Option<&Expr>,
        scope: &Scope,
        context: ValidationContext,
        produces_value: bool,
    ) {
        if context.in_lane {
            self.diagnostic("illegal concurrent surface: nested concurrent");
        }
        let mut direct_consts = BTreeMap::<String, Vec<u32>>::new();
        for (index, statement) in body.statements.iter().enumerate() {
            if let Stmt::Let {
                mutable: false,
                name,
                ..
            } = statement
            {
                direct_consts
                    .entry(name.clone())
                    .or_default()
                    .push(index as u32);
            }
        }
        let mut nested_names = BTreeSet::new();
        for statement in &body.statements {
            let mut collector = LocalNameCollector::default();
            collector.visit_stmt(statement);
            if let Stmt::Let { name, .. } = statement {
                collector.names.remove(name);
            }
            nested_names.extend(collector.names);
        }
        let plan_site = tail
            .map(|tail| self.expr_site(tail))
            .or_else(|| {
                body.statements
                    .first()
                    .map(|statement| self.stmt_site(statement))
            })
            .unwrap_or_else(|| self.owner_site());
        let mut lanes = Vec::with_capacity(body.statements.len() + usize::from(tail.is_some()));
        let mut sibling_scope = scope.clone();
        let mut lane_profiles = Vec::new();

        for (index, statement) in body.statements.iter().enumerate() {
            let source_order = index as u32;
            let references = referenced_names_in_stmt(statement);
            let dependencies = references
                .iter()
                .filter_map(|name| {
                    direct_consts.get(name).and_then(|declarations| {
                        declarations
                            .iter()
                            .rev()
                            .find(|declaration| **declaration < source_order)
                            .copied()
                    })
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            self.validate_concurrent_references(
                &references,
                source_order,
                &direct_consts,
                &nested_names,
                &sibling_scope,
            );
            let kind = if matches!(statement, Stmt::Serial { .. }) {
                ConcurrentLaneKind::Serial
            } else {
                ConcurrentLaneKind::Statement
            };
            let lane_context = ValidationContext {
                value_boundary: context.value_boundary || produces_value,
                in_lane: true,
            };
            self.validate_lane_stmt(statement, &mut sibling_scope.clone(), lane_context, kind);
            lane_profiles.push(self.effect_profile_for_stmt(statement));
            lanes.push(ConcurrentLanePlan {
                source_order,
                kind,
                dependencies,
                source_site: self.stmt_site(statement),
            });
            if let Stmt::Let {
                mutable: false,
                name,
                value,
                ..
            } = statement
            {
                let _ = value;
                sibling_scope.insert(name.clone(), BindingRoot::ConcurrentSibling);
            }
        }

        if let Some(tail) = tail {
            let source_order = lanes.len() as u32;
            let references = referenced_names_in_expr(tail);
            self.validate_concurrent_references(
                &references,
                source_order,
                &direct_consts,
                &nested_names,
                &sibling_scope,
            );
            let mut dependencies = (0..source_order).collect::<Vec<_>>();
            dependencies.sort_unstable();
            self.validate_lane_expr(
                tail,
                &mut sibling_scope.clone(),
                ValidationContext {
                    value_boundary: true,
                    in_lane: true,
                },
            );
            lane_profiles.push(self.effect_profile_for_expr(tail));
            lanes.push(ConcurrentLanePlan {
                source_order,
                kind: ConcurrentLaneKind::Tail,
                dependencies,
                source_site: self.expr_site(tail),
            });
        }

        self.validate_lane_effects(&lane_profiles);
        self.semantics.concurrent_plans.push(ConcurrentSourcePlan {
            produces_value,
            lanes,
            source_site: plan_site,
        });
    }

    fn validate_concurrent_references(
        &mut self,
        references: &BTreeSet<String>,
        source_order: u32,
        direct_consts: &BTreeMap<String, Vec<u32>>,
        nested_names: &BTreeSet<String>,
        scope: &Scope,
    ) {
        for name in references {
            if let Some(declarations) = direct_consts.get(name) {
                if !declarations
                    .iter()
                    .any(|declaration| *declaration < source_order)
                {
                    let declaration = declarations
                        .iter()
                        .copied()
                        .find(|declaration| *declaration >= source_order)
                        .expect("direct const declarations are source ordered");
                    self.diagnostic(format!(
                        "concurrent forward reference `{name}` from lane {source_order} to lane {declaration}"
                    ));
                }
            } else if nested_names.contains(name) && !scope.contains_key(name) {
                self.diagnostic(format!(
                    "concurrent binding `{name}` is not sibling-visible outside its serial lane"
                ));
            }
        }
    }

    fn validate_lane_stmt(
        &mut self,
        statement: &Stmt,
        scope: &mut Scope,
        context: ValidationContext,
        kind: ConcurrentLaneKind,
    ) {
        let mut forbidden_expression = ForbiddenConcurrentExpression::default();
        forbidden_expression.visit_stmt(statement);
        if let Some(kind) = forbidden_expression.first {
            self.diagnostic(format!("illegal concurrent surface: {kind}"));
        }
        match statement {
            Stmt::Let { mutable: true, .. } => {
                self.diagnostic("concurrent surface rejects mutable `let` bindings");
                self.validate_stmt(statement, scope, context);
            }
            Stmt::Let { .. } | Stmt::Assign { .. } | Stmt::Assert { .. } | Stmt::Expr(_) => {
                self.validate_stmt(statement, scope, context)
            }
            Stmt::DbTransaction { body } => {
                let mut transaction_scope = scope.clone();
                for nested in &body.statements {
                    self.validate_lane_stmt(
                        nested,
                        &mut transaction_scope,
                        context,
                        ConcurrentLaneKind::Statement,
                    );
                }
            }
            Stmt::Serial { body } if kind == ConcurrentLaneKind::Serial => {
                let mut serial_scope = scope.clone();
                for nested in &body.statements {
                    if matches!(nested, Stmt::Serial { .. } | Stmt::Concurrent { .. }) {
                        self.diagnostic("illegal concurrent surface: nested serial/concurrent");
                    }
                    self.validate_lane_stmt(
                        nested,
                        &mut serial_scope,
                        context,
                        ConcurrentLaneKind::Statement,
                    );
                }
            }
            Stmt::CompilerTestEffectRegister { .. }
            | Stmt::Timeout { .. }
            | Stmt::Concurrent { .. }
            | Stmt::Serial { .. }
            | Stmt::If { .. }
            | Stmt::For { .. }
            | Stmt::Match { .. }
            | Stmt::Throw { .. }
            | Stmt::Rethrow { .. }
            | Stmt::Emit(_)
            | Stmt::Return(_)
            | Stmt::Spawn { .. }
            | Stmt::Break
            | Stmt::Continue => {
                self.diagnostic(format!(
                    "illegal concurrent surface: {}",
                    statement_kind(statement)
                ));
                self.validate_stmt(statement, scope, context);
            }
        }
    }

    fn validate_lane_expr(
        &mut self,
        expression: &Expr,
        scope: &mut Scope,
        context: ValidationContext,
    ) {
        let mut forbidden = ForbiddenConcurrentExpression::default();
        forbidden.visit_expr(expression);
        if let Some(kind) = forbidden.first {
            self.diagnostic(format!("illegal concurrent surface: {kind}"));
        }
        self.validate_expr(expression, scope, context);
    }

    fn validate_mutation_target(
        &mut self,
        target: &Expr,
        scope: &Scope,
        context: ValidationContext,
    ) {
        if !context.in_lane {
            return;
        }
        let Some(root) =
            expr_path(target).and_then(|path| path.split('.').next().map(str::to_string))
        else {
            self.diagnostic("concurrent mutation target has opaque root provenance");
            return;
        };
        match scope.get(&root) {
            Some(BindingRoot::LaneLocalFresh) => {}
            Some(BindingRoot::Scalar) => {
                self.diagnostic(format!(
                    "concurrent mutation target `{root}` is not a mutable lane-local fresh root"
                ));
            }
            Some(
                BindingRoot::Outer
                | BindingRoot::ConcurrentSibling
                | BindingRoot::LaneLocalOpaque,
            )
            | None => self.diagnostic(format!(
                "concurrent lane writes outer mutable root `{root}`; outer mutable root writes are forbidden"
            )),
        }
    }

    fn validate_mutating_call(
        &mut self,
        call: &Expr,
        scope: &mut Scope,
        context: ValidationContext,
    ) {
        if !context.in_lane {
            return;
        }
        let Expr::Call { callee, args } = call else {
            return;
        };
        let resolved_target = self.resolved_target(call).cloned();
        let receiver_field = match without_generic(callee) {
            Expr::Field { object, field } => Some((object.as_ref(), field.as_str())),
            _ => None,
        };
        let mutates = resolved_target
            .as_ref()
            .and_then(|target| match target {
                ResolvedCallTarget::ReceiverBuiltin { op } => Some(*op),
                _ => None,
            })
            .and_then(receiver_builtin_spec)
            .is_some_and(|spec| spec.mutates_receiver);
        if mutates {
            if let Some((object, _)) = receiver_field {
                self.validate_mutation_target(object, scope, context);
                self.taint_lane_local_root_from_payloads(object, args.iter(), scope, context);
            }
        } else if receiver_field
            .is_some_and(|(_, field)| matches!(field, "push" | "pop" | "set" | "delete"))
        {
            // If target resolution could not prove the exact builtin, the
            // mutating spelling remains fail closed.
            if let Some((object, _)) = receiver_field {
                self.validate_mutation_target(object, scope, context);
                self.taint_lane_local_root_from_payloads(object, args.iter(), scope, context);
            }
        }

        let source_callable = match resolved_target.as_ref() {
            Some(ResolvedCallTarget::LocalFunction { source_callable })
            | Some(ResolvedCallTarget::LocalImplMethod { source_callable }) => source_callable,
            _ => return,
        };
        let writes_caller_reachable = match self.callable_effects.operations().get(source_callable)
        {
            Some(CallableEffectSummary::Analyzed { effects }) => effects.writes_caller_reachable,
            Some(CallableEffectSummary::Unknown { .. }) | None => {
                self.diagnostic(format!(
                    "concurrent local call `{}` has unknown caller-reachable mutation facts",
                    source_callable.symbol()
                ));
                return;
            }
        };
        if !writes_caller_reachable {
            return;
        }

        let mut actuals = Vec::with_capacity(args.len() + usize::from(receiver_field.is_some()));
        if matches!(
            resolved_target,
            Some(ResolvedCallTarget::LocalImplMethod { .. })
        ) {
            if let Some((receiver, _)) = receiver_field {
                actuals.push(receiver);
            }
        }
        actuals.extend(args);
        for actual in actuals {
            match binding_root_for_value(actual, scope, true, true) {
                BindingRoot::LaneLocalFresh | BindingRoot::Scalar => {}
                BindingRoot::Outer => {
                    let root = expr_path(actual)
                        .and_then(|path| path.split('.').next().map(str::to_string))
                        .unwrap_or_else(|| "<opaque>".to_string());
                    self.diagnostic(format!(
                        "concurrent local call `{}` may write outer mutable root `{root}`; outer mutable root writes are forbidden",
                        source_callable.symbol()
                    ));
                }
                BindingRoot::ConcurrentSibling | BindingRoot::LaneLocalOpaque => {
                    self.diagnostic(format!(
                        "concurrent local call `{}` has caller-reachable mutation through a non-fresh lane value",
                        source_callable.symbol()
                    ));
                }
            }
        }
    }

    fn taint_lane_local_root_from_payloads<'expr>(
        &mut self,
        target: &Expr,
        payloads: impl IntoIterator<Item = &'expr Expr>,
        scope: &mut Scope,
        context: ValidationContext,
    ) {
        if !context.in_lane {
            return;
        }
        let Some(root) =
            expr_path(target).and_then(|path| path.split('.').next().map(str::to_string))
        else {
            return;
        };
        if scope.get(&root) != Some(&BindingRoot::LaneLocalFresh) {
            return;
        }
        if payloads
            .into_iter()
            .any(|payload| !lane_local_payload_is_safe(payload, scope))
        {
            scope.insert(root, BindingRoot::LaneLocalOpaque);
        }
    }

    fn effect_profile_for_stmt(&self, statement: &Stmt) -> CallableEffectProfile {
        let mut collector = EffectCollector {
            expression_keys: self.expression_keys,
            resolved_targets: self.resolved_targets,
            callable_profiles: self.callable_profiles,
            profile: CallableEffectProfile::default(),
        };
        collector.visit_stmt(statement);
        collector.profile
    }

    fn effect_profile_for_expr(&self, expression: &Expr) -> CallableEffectProfile {
        let mut collector = EffectCollector {
            expression_keys: self.expression_keys,
            resolved_targets: self.resolved_targets,
            callable_profiles: self.callable_profiles,
            profile: CallableEffectProfile::default(),
        };
        collector.visit_expr(expression);
        collector.profile
    }

    fn validate_lane_effects(&mut self, lanes: &[CallableEffectProfile]) {
        for (lane, profile) in lanes.iter().enumerate() {
            if profile.opaque {
                self.diagnostic(format!(
                    "concurrent lane {lane} has unknown target/conflict-key/cancel-safety metadata"
                ));
            }
            for access in &profile.accesses {
                if !access.cancel_safe {
                    self.diagnostic(format!(
                        "concurrent lane {lane} effect {} has insufficient cancel-safety",
                        access.target
                    ));
                }
            }
        }
        for left in 0..lanes.len() {
            for right in (left + 1)..lanes.len() {
                for left_access in &lanes[left].accesses {
                    for right_access in &lanes[right].accesses {
                        let conflict = match (left_access.mode, right_access.mode) {
                            (ExternalAccessMode::Read, ExternalAccessMode::Read) => None,
                            (ExternalAccessMode::Exclusive, _)
                            | (_, ExternalAccessMode::Exclusive) => Some("exclusive"),
                            (ExternalAccessMode::Write, ExternalAccessMode::Write) => {
                                Some("write/write")
                            }
                            (ExternalAccessMode::Read, ExternalAccessMode::Write)
                            | (ExternalAccessMode::Write, ExternalAccessMode::Read) => {
                                Some("read/write")
                            }
                        };
                        if let Some(conflict) = conflict {
                            self.diagnostic(format!(
                                "concurrent effect conflict {conflict} between lanes {left} and {right} (targets {} / {}, conflict-keys {} / {})",
                                left_access.target,
                                right_access.target,
                                left_access.conflict_key,
                                right_access.conflict_key
                            ));
                        }
                    }
                }
            }
        }
    }

    fn record_timeout(
        &mut self,
        duration: &crate::shared::ast::DurationLiteral,
        produces_value: bool,
        span: SourceSpan,
    ) {
        match duration.checked_milliseconds() {
            Ok(duration_milliseconds) => {
                self.semantics.timeout_plans.push(TimeoutSourcePlan {
                    duration_milliseconds,
                    produces_value,
                    source_site: ExecutionSourceSite {
                        module_path: self.module_path.to_string(),
                        owner: self.owner.clone(),
                        span,
                    },
                });
            }
            Err(error) => self.diagnostic(format!(
                "invalid checked timeout duration {}{}: {error}",
                duration.digits,
                duration.unit.suffix()
            )),
        }
    }

    fn resolved_target(&self, expression: &Expr) -> Option<&ResolvedCallTarget> {
        self.expression_keys
            .get(&expr_address(expression))
            .and_then(|key| self.resolved_targets.target(key))
    }

    fn expr_span(&self, expression: &Expr) -> SourceSpan {
        self.expression_keys
            .get(&expr_address(expression))
            .and_then(|key| self.expression_sources.fact(key))
            .map(|fact| fact.span)
            .unwrap_or(self.function.span)
    }

    fn expr_site(&self, expression: &Expr) -> ExecutionSourceSite {
        ExecutionSourceSite {
            module_path: self.module_path.to_string(),
            owner: self.owner.clone(),
            span: self.expr_span(expression),
        }
    }

    fn stmt_site(&self, statement: &Stmt) -> ExecutionSourceSite {
        let span = match statement {
            Stmt::Timeout { duration, .. } => duration.span,
            _ => first_expression(statement)
                .map(|expression| self.expr_span(expression))
                .unwrap_or(self.function.span),
        };
        ExecutionSourceSite {
            module_path: self.module_path.to_string(),
            owner: self.owner.clone(),
            span,
        }
    }

    fn owner_site(&self) -> ExecutionSourceSite {
        ExecutionSourceSite {
            module_path: self.module_path.to_string(),
            owner: self.owner.clone(),
            span: self.function.span,
        }
    }

    fn diagnostic(&mut self, message: impl Into<String>) {
        self.diagnostics.push(format!(
            "{} {:?}: {}",
            self.module_path,
            self.owner,
            message.into()
        ));
    }
}

struct NestedExpressionValidator<'a, 'b> {
    analyzer: &'a mut OwnerAnalyzer<'b>,
    scope: &'a mut Scope,
    context: ValidationContext,
}

impl AstVisitor for NestedExpressionValidator<'_, '_> {
    fn visit_expr(&mut self, expression: &Expr) {
        self.analyzer
            .validate_expr(expression, self.scope, self.context);
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ValidationContext {
    value_boundary: bool,
    in_lane: bool,
}

#[derive(Default)]
struct LocalNameCollector {
    names: BTreeSet<String>,
}

impl AstVisitor for LocalNameCollector {
    fn visit_stmt(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Let { name, .. } => {
                self.names.insert(name.clone());
            }
            Stmt::For { binding, .. } => match binding {
                ForBinding::Item { item } => {
                    self.names.insert(item.clone());
                }
                ForBinding::Entry { key, value } => {
                    self.names.insert(key.clone());
                    self.names.insert(value.clone());
                }
            },
            _ => {}
        }
        walk_stmt(self, statement);
    }

    fn visit_expr(&mut self, expression: &Expr) {
        if let Expr::DbLeaseClaim(claim) = expression {
            if let Some(binding) = &claim.binding {
                self.names.insert(binding.clone());
            }
        }
        walk_expr(self, expression);
    }

    fn visit_pattern(&mut self, pattern: &Pattern) {
        pattern_bindings(pattern, &mut self.names);
    }
}

#[derive(Default)]
struct ForbiddenConcurrentExpression {
    first: Option<&'static str>,
}

impl AstVisitor for ForbiddenConcurrentExpression {
    fn visit_expr(&mut self, expression: &Expr) {
        if self.first.is_none() {
            self.first = match expression {
                Expr::ValueBlock(_) => Some("value block"),
                Expr::ConcurrentValue(_) => Some("nested concurrent value"),
                Expr::Timeout { .. } => Some("timeout"),
                Expr::Throw { .. } => Some("throw"),
                Expr::Rethrow { .. } => Some("rethrow"),
                Expr::Catch { .. } => Some("catch"),
                Expr::DbLeaseClaim(_) => Some("with/resource scope"),
                Expr::Literal(_)
                | Expr::Identifier(_)
                | Expr::DependencySourceAddress(_)
                | Expr::Binary { .. }
                | Expr::Unary { .. }
                | Expr::Call { .. }
                | Expr::Generic { .. }
                | Expr::InterfaceBox { .. }
                | Expr::Field { .. }
                | Expr::Record { .. }
                | Expr::ObjectLiteral { .. }
                | Expr::Patch { .. }
                | Expr::DbOperation(_)
                | Expr::DbQuery(_)
                | Expr::DbTransaction(_)
                | Expr::DbLeaseRead(_) => None,
            };
        }
        walk_expr(self, expression);
    }
}

fn binding_root_for_value(
    expression: &Expr,
    scope: &Scope,
    in_lane: bool,
    mutable: bool,
) -> BindingRoot {
    if !in_lane {
        return BindingRoot::Outer;
    }
    if definitely_lane_local_fresh(expression, scope) {
        BindingRoot::LaneLocalFresh
    } else if let Some(root) = expr_path(expression)
        .and_then(|path| path.split('.').next().map(str::to_string))
        .and_then(|root| scope.get(&root).copied())
    {
        root
    } else if !mutable && matches!(expression, Expr::Literal(_)) {
        BindingRoot::Scalar
    } else {
        BindingRoot::LaneLocalOpaque
    }
}

fn definitely_lane_local_fresh(expression: &Expr, scope: &Scope) -> bool {
    match expression {
        Expr::Record { fields, .. } => fields
            .iter()
            .all(|(_, value)| lane_local_payload_is_safe(value, scope)),
        Expr::ObjectLiteral { entries } => entries
            .iter()
            .all(|entry| lane_local_payload_is_safe(&entry.value, scope)),
        Expr::Patch { operations, .. } => operations.iter().all(|operation| match operation {
            crate::shared::ast::PatchOperation::Set { value, .. }
            | crate::shared::ast::PatchOperation::Inc { value, .. } => {
                lane_local_payload_is_safe(value, scope)
            }
        }),
        Expr::ValueBlock(value) | Expr::ConcurrentValue(value) => {
            definitely_lane_local_fresh(&value.tail, scope)
        }
        Expr::Timeout { value, .. } => definitely_lane_local_fresh(value, scope),
        Expr::Literal(_)
        | Expr::Identifier(_)
        | Expr::DependencySourceAddress(_)
        | Expr::Binary { .. }
        | Expr::Unary { .. }
        | Expr::Call { .. }
        | Expr::Generic { .. }
        | Expr::InterfaceBox { .. }
        | Expr::Field { .. }
        | Expr::Throw { .. }
        | Expr::Rethrow { .. }
        | Expr::Catch { .. }
        | Expr::DbOperation(_)
        | Expr::DbQuery(_)
        | Expr::DbTransaction(_)
        | Expr::DbLeaseClaim(_)
        | Expr::DbLeaseRead(_) => false,
    }
}

fn lane_local_payload_is_safe(expression: &Expr, scope: &Scope) -> bool {
    if definitely_lane_local_fresh(expression, scope) {
        return true;
    }
    if matches!(expression, Expr::Literal(_)) {
        return true;
    }
    if let Some(root) = expr_path(expression)
        .and_then(|path| path.split('.').next().map(str::to_string))
        .and_then(|root| scope.get(&root).copied())
    {
        return matches!(root, BindingRoot::LaneLocalFresh | BindingRoot::Scalar);
    }
    match expression {
        Expr::Binary { left, right, .. } => {
            lane_local_payload_is_safe(left, scope) && lane_local_payload_is_safe(right, scope)
        }
        Expr::Unary { expr, .. } | Expr::InterfaceBox { value: expr, .. } => {
            lane_local_payload_is_safe(expr, scope)
        }
        Expr::Timeout { value, .. } => lane_local_payload_is_safe(value, scope),
        Expr::ValueBlock(value) | Expr::ConcurrentValue(value) => {
            lane_local_payload_is_safe(&value.tail, scope)
        }
        Expr::Literal(_)
        | Expr::Identifier(_)
        | Expr::DependencySourceAddress(_)
        | Expr::Call { .. }
        | Expr::Generic { .. }
        | Expr::Field { .. }
        | Expr::Record { .. }
        | Expr::ObjectLiteral { .. }
        | Expr::Patch { .. }
        | Expr::Throw { .. }
        | Expr::Rethrow { .. }
        | Expr::Catch { .. }
        | Expr::DbOperation(_)
        | Expr::DbQuery(_)
        | Expr::DbTransaction(_)
        | Expr::DbLeaseClaim(_)
        | Expr::DbLeaseRead(_) => false,
    }
}

fn referenced_names_in_stmt(statement: &Stmt) -> BTreeSet<String> {
    let mut collector = ReferencedNameCollector::default();
    collector.visit_stmt(statement);
    collector.names
}

fn referenced_names_in_expr(expression: &Expr) -> BTreeSet<String> {
    let mut collector = ReferencedNameCollector::default();
    collector.visit_expr(expression);
    collector.names
}

#[derive(Default)]
struct ReferencedNameCollector {
    names: BTreeSet<String>,
    bound: BTreeSet<String>,
}

impl AstVisitor for ReferencedNameCollector {
    fn visit_block(&mut self, block: &Block) {
        let saved_bound = self.bound.clone();
        for statement in &block.statements {
            self.visit_stmt(statement);
        }
        self.bound = saved_bound;
    }

    fn visit_stmt(&mut self, statement: &Stmt) {
        match statement {
            Stmt::CompilerTestEffectRegister { .. } => walk_stmt(self, statement),
            Stmt::Let { name, value, .. } => {
                self.visit_expr(value);
                self.bound.insert(name.clone());
            }
            Stmt::Assign { target, value } => {
                self.visit_expr(target);
                self.visit_expr(value);
            }
            Stmt::Timeout { body, .. }
            | Stmt::Concurrent { body }
            | Stmt::Serial { body }
            | Stmt::DbTransaction { body } => self.visit_block(body),
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
                let saved_bound = self.bound.clone();
                match binding {
                    ForBinding::Item { item } => {
                        self.bound.insert(item.clone());
                    }
                    ForBinding::Entry { key, value } => {
                        self.bound.insert(key.clone());
                        self.bound.insert(value.clone());
                    }
                }
                self.visit_block(body);
                self.bound = saved_bound;
            }
            Stmt::Match { value, arms } => {
                self.visit_expr(value);
                for arm in arms {
                    let saved_bound = self.bound.clone();
                    pattern_bindings(&arm.pattern, &mut self.bound);
                    self.visit_block(&arm.body);
                    self.bound = saved_bound;
                }
            }
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
        if let Expr::Identifier(name) = expression {
            if !self.bound.contains(name) {
                self.names.insert(name.clone());
            }
            return;
        }
        match expression {
            Expr::ValueBlock(value) | Expr::ConcurrentValue(value) => {
                let saved_bound = self.bound.clone();
                for statement in &value.body.statements {
                    self.visit_stmt(statement);
                }
                self.visit_expr(&value.tail);
                self.bound = saved_bound;
            }
            Expr::Timeout { value, .. } => self.visit_expr(value),
            Expr::DbLeaseClaim(claim) => {
                self.visit_expr(&claim.key);
                let saved_bound = self.bound.clone();
                if let Some(binding) = &claim.binding {
                    self.bound.insert(binding.clone());
                }
                self.visit_block(&claim.body);
                self.bound = saved_bound;
            }
            _ => walk_expr(self, expression),
        }
    }
}

fn first_expression(statement: &Stmt) -> Option<&Expr> {
    match statement {
        Stmt::CompilerTestEffectRegister {
            target_probe,
            expect,
            step_expect,
            outcome,
            ..
        } => Some(target_probe)
            .or(expect.as_ref())
            .or(step_expect.as_ref())
            .or_else(|| match outcome {
                crate::shared::ast::TestEffectStepOutcome::Respond { value }
                | crate::shared::ast::TestEffectStepOutcome::Throw { value } => Some(value),
                crate::shared::ast::TestEffectStepOutcome::Stream { events } => events.first(),
            }),
        Stmt::Assert { condition, .. } => Some(condition),
        Stmt::Let { value, .. } => Some(value),
        Stmt::Assign { target, .. } => Some(target),
        Stmt::Timeout { body, .. }
        | Stmt::Concurrent { body }
        | Stmt::Serial { body }
        | Stmt::DbTransaction { body } => first_expression_in_block(body),
        Stmt::If { condition, .. } => Some(condition),
        Stmt::For { iterable, .. } => Some(iterable),
        Stmt::Match { value, .. } => Some(value),
        Stmt::Throw { value }
        | Stmt::Rethrow { exception: value }
        | Stmt::Emit(value)
        | Stmt::Spawn { call: value, .. }
        | Stmt::Expr(value) => Some(value),
        Stmt::Return(value) => value.as_ref(),
        Stmt::Break | Stmt::Continue => None,
    }
}

fn first_expression_in_block(block: &Block) -> Option<&Expr> {
    block.statements.iter().find_map(first_expression)
}

fn pattern_bindings(pattern: &Pattern, names: &mut BTreeSet<String>) {
    match pattern {
        Pattern::Binding(name) => {
            names.insert(name.clone());
        }
        Pattern::Nominal { fields, .. } | Pattern::Record { fields } => {
            for field in fields {
                if let Some(pattern) = &field.pattern {
                    pattern_bindings(pattern, names);
                } else {
                    names.insert(field.name.clone());
                }
            }
        }
        Pattern::Or(patterns) => {
            for pattern in patterns {
                pattern_bindings(pattern, names);
            }
        }
        Pattern::Wildcard | Pattern::Literal(_) => {}
    }
}

fn statement_kind(statement: &Stmt) -> &'static str {
    match statement {
        Stmt::CompilerTestEffectRegister { .. } => "compiler test effect",
        Stmt::Assert { .. } => "assert",
        Stmt::Let { .. } => "binding",
        Stmt::Assign { .. } => "assignment",
        Stmt::Timeout { .. } => "timeout",
        Stmt::Concurrent { .. } => "nested concurrent",
        Stmt::Serial { .. } => "nested serial",
        Stmt::If { .. } => "if",
        Stmt::For { .. } => "for",
        Stmt::Match { .. } => "match",
        Stmt::DbTransaction { .. } => "db transaction",
        Stmt::Throw { .. } => "throw",
        Stmt::Rethrow { .. } => "rethrow",
        Stmt::Emit(_) => "emit",
        Stmt::Return(_) => "return",
        Stmt::Spawn { .. } => "spawn",
        Stmt::Break => "break",
        Stmt::Continue => "continue",
        Stmt::Expr(_) => "expression",
    }
}

fn expr_address(expression: &Expr) -> usize {
    std::ptr::from_ref(expression).addr()
}

fn without_generic(expression: &Expr) -> &Expr {
    match expression {
        Expr::Generic { callee, .. } => callee,
        _ => expression,
    }
}

fn receiver_builtin_spec(
    op: BuiltinReceiverOp,
) -> Option<&'static skiff_artifact_model::BuiltinReceiverOpSpec> {
    builtin_receiver_op_spec_by_name(op.receiver.as_str(), op.method.as_str())
}
