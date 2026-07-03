use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use crate::{
    parsed_sources::ParsedCompilerSource,
    semantic::PublicationSemanticContext,
    shared::{
        ast::{AliasDecl, FunctionDecl, InterfaceOperation, TypeDecl, TypeRef},
        source_role::PublicationSourceRole,
    },
};
use compiler_input_model::{
    PackageDependency, PublicationCompilePolicy, ResolvedServiceDependencies, ServiceIngressSeed,
};

use super::config_usage::ConfigUsageSeed;
use super::entity::PublicationEntityModel;
use super::source_file_facts::{publication_db_metadata_index, PublicationDbMetadataIndex};
use super::source_identity::PublicationDeclarationAnchors;
use super::source_rules::collect_stream_emit_type_violations;
#[cfg(any(test, feature = "test-support"))]
use super::SourceSymbolKey;
use super::{
    api::{PublicCallableKind, PublicModuleExport, PublicSymbolKind, PublicTypeKind},
    publication_type_symbols, validate_source_name_resolution_from_model, ConfigRequirementScope,
    ConfigRequirementSet, DependencyPackageOperationFacts, ExpressionSourceMap,
    ExpressionTypeModel, NameResolutionModel, PublicationApiSeed, PublicationCompilePlan,
    PublicationKind, PublicationTypeSymbolIndex, ServiceIngressModel, TypeResolutionContext,
    TypeResolutionModel, TypeResolutionPackageFacts,
};
use crate::shared::publication_error::PublicationError;

#[derive(Debug)]
pub struct PublicationSourceSet {
    parsed_sources: Vec<ParsedCompilerSource>,
    policy: SourceCompilePolicy,
}

#[derive(Debug)]
pub struct SourceIndexes {
    publication_type_symbols: PublicationTypeSymbolIndex,
    publication_db_metadata_index: PublicationDbMetadataIndex,
}

#[derive(Debug)]
pub struct ResolutionModels {
    alias_targets_by_module: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Debug)]
pub struct ResolvedDependencies {
    package_aliases: BTreeMap<String, Vec<String>>,
    package_dependencies: Vec<PackageDependency>,
    dependency_package_operation_facts: Vec<DependencyPackageOperationFacts>,
    service_dependency_aliases: BTreeSet<String>,
    service_dependencies: ResolvedServiceDependencies,
}

#[derive(Debug)]
pub struct PublicationApiModel {
    seed: PublicationApiSeed,
}

#[derive(Debug)]
pub struct ExportBindingModel {
    public_symbols: BTreeMap<String, ExportSymbolBinding>,
    public_callables: BTreeMap<String, ExportCallableBinding>,
    public_schema_types: BTreeMap<String, ExportSchemaBinding>,
    public_instances: BTreeMap<String, ExportPublicInstanceBinding>,
    module_exports: Vec<PublicModuleExport>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportSymbolBinding {
    pub public_path: String,
    pub source_module: String,
    pub source_symbol: String,
    pub kind: PublicSymbolKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportCallableBinding {
    pub public_path: String,
    pub source_module: String,
    pub source_symbol: String,
    pub kind: PublicCallableKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportSchemaBinding {
    pub public_path: String,
    pub source_module: String,
    pub source_symbol: String,
    pub kind: PublicTypeKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportPublicInstanceBinding {
    pub public_path: String,
    pub source_module: String,
    pub source_symbol: String,
    pub interfaces: Vec<ExportPublicInstanceInterfaceBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportPublicInstanceInterfaceBinding {
    pub source_module: String,
    pub source_symbol: String,
}

#[derive(Debug)]
pub struct SourceCompileModel {
    sources: PublicationSourceSet,
    indexes: SourceIndexes,
    dependencies: ResolvedDependencies,
    service_ingress: Option<ServiceIngressModel>,
    resolutions: ResolutionModels,
    /// 当前 publication 的 owner-local entity model。`root.<module>.<symbol>` typed
    /// resolution 必须通过这里的 top-level table 落到 `EntityId::TopLevel`。
    entity_model: PublicationEntityModel,
    /// Pipeline 文档要求的 name resolution 单一事实来源。lowering 必须从这里读依赖别名集,
    /// 不得通过 `dependencies` 字段重新拿原始数据。
    name_resolution: NameResolutionModel,
    type_resolution: TypeResolutionModel,
    expression_sources: ExpressionSourceMap,
    expression_types: ExpressionTypeModel,
    publication_api: PublicationApiModel,
    export_bindings: ExportBindingModel,
    // P1b: source_identity (role b) kept for reference; revision_id now uses descriptor-based
    // input in runtime_manifest.rs.  P2 will introduce AbiTypeId consuming declaration_anchors.
    #[allow(dead_code)]
    source_identity: String,
    #[allow(dead_code)]
    declaration_anchors: PublicationDeclarationAnchors,
    #[allow(dead_code)]
    own_config_requirements: ConfigRequirementSet,
    #[allow(dead_code)]
    dependency_config_requirements: ConfigRequirementSet,
    effective_config_requirements: ConfigRequirementSet,
}

pub struct SourceCompileModelInput<'a> {
    pub parsed_sources: Vec<ParsedCompilerSource>,
    pub diagnostic_root: &'a Path,
    pub package_aliases: &'a BTreeMap<String, Vec<String>>,
    pub package_dependencies: &'a [PackageDependency],
    pub package_db_metadata_index: Option<PublicationDbMetadataIndex>,
    pub type_resolution_package_facts: Option<&'a [TypeResolutionPackageFacts<'a>]>,
    pub dependency_package_operation_facts: Vec<DependencyPackageOperationFacts>,
    pub service_dependencies: ResolvedServiceDependencies,
    pub service_ingress: Option<ServiceIngressSeed>,
    pub entity_model: PublicationEntityModel,
    pub name_resolution: NameResolutionModel,
    pub policy: PublicationCompilePolicy<'a>,
    pub publication_api_seed: PublicationApiSeed,
    pub source_identity: String,
    pub declaration_anchors: PublicationDeclarationAnchors,
    pub config_usage_seed: ConfigUsageSeed,
    pub dependency_config_requirements: ConfigRequirementSet,
}

#[derive(Clone, Debug)]
enum SourceCompilePolicy {
    Package { package_id: String },
    Service { service_id: String },
}

impl SourceCompileModel {
    pub fn build(input: SourceCompileModelInput<'_>) -> Result<Self, PublicationError> {
        let policy = SourceCompilePolicy::from_borrowed(input.policy);
        let plan = PublicationCompilePlan::from_policy(policy.as_borrowed());
        let indexes = SourceIndexes::build(
            &input.parsed_sources,
            input.package_aliases,
            input.package_db_metadata_index,
            plan,
        )?;
        let resolutions = ResolutionModels::build(&input.parsed_sources);
        let entity_model = input.entity_model;
        let name_resolution = input.name_resolution;
        validate_source_name_resolution_from_model(
            input.diagnostic_root,
            &input.parsed_sources,
            &name_resolution,
        )?;
        if matches!(&policy, SourceCompilePolicy::Service { .. }) {
            super::service_rules::validate_service_publication_sources_with_name_resolution(
                input.diagnostic_root,
                &input.parsed_sources,
                &name_resolution,
            )?;
        }
        let service_ingress = service_ingress_from_seed(&policy, input.service_ingress)?;
        let type_resolution = TypeResolutionModel::build(
            &input.parsed_sources,
            input.package_aliases,
            input.package_dependencies,
            input.type_resolution_package_facts,
            indexes.publication_type_symbols(),
        )
        .map_err(|message| PublicationError::ContractValidation {
            message: format!("type resolution model failed:\n- {message}"),
        })?;
        validate_no_plain_interface_value_types(
            input.diagnostic_root,
            &input.parsed_sources,
            &type_resolution,
        )?;
        let expression_sources =
            ExpressionSourceMap::build(&input.parsed_sources).map_err(|message| {
                PublicationError::ContractValidation {
                    message: format!("expression source model failed:\n- {message}"),
                }
            })?;
        let publication_api = PublicationApiModel::new(input.publication_api_seed);
        let export_bindings = ExportBindingModel::from_publication_api(publication_api.seed());
        let dependencies = ResolvedDependencies::new(
            input.package_aliases.clone(),
            input.package_dependencies.to_vec(),
            input.dependency_package_operation_facts,
            input.service_dependencies,
        );
        let expression_types = ExpressionTypeModel::build(
            &input.parsed_sources,
            &expression_sources,
            &type_resolution,
            Some(&dependencies),
        )
        .map_err(|error| PublicationError::ContractValidation {
            message: format!("expression type model failed:\n- {}", error.message()),
        })?;
        if matches!(&policy, SourceCompilePolicy::Service { .. }) {
            let mut violations = Vec::new();
            collect_stream_emit_type_violations(
                input.diagnostic_root,
                &input.parsed_sources,
                &expression_sources,
                &expression_types,
                &type_resolution,
                &mut violations,
            );
            if !violations.is_empty() {
                return Err(PublicationError::ContractValidation {
                    message: violations
                        .into_iter()
                        .map(|violation| format!("- {violation}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                });
            }
        }
        let own_config_requirements = ConfigRequirementSet::from_usage_seed(
            &input.config_usage_seed,
            ConfigRequirementScope::from_publication_policy(policy.as_borrowed()),
        );
        let dependency_config_requirements = input.dependency_config_requirements;
        let effective_config_requirements = ConfigRequirementSet::effective(
            &own_config_requirements,
            &dependency_config_requirements,
        )?;
        Ok(Self {
            sources: PublicationSourceSet::new(input.parsed_sources, policy),
            indexes,
            dependencies,
            service_ingress,
            resolutions,
            entity_model,
            name_resolution,
            type_resolution,
            expression_sources,
            expression_types,
            publication_api,
            export_bindings,
            source_identity: input.source_identity,
            declaration_anchors: input.declaration_anchors,
            own_config_requirements,
            dependency_config_requirements,
            effective_config_requirements,
        })
    }

    pub fn sources(&self) -> &PublicationSourceSet {
        &self.sources
    }

    pub fn indexes(&self) -> &SourceIndexes {
        &self.indexes
    }

    pub fn dependencies(&self) -> &ResolvedDependencies {
        &self.dependencies
    }

    pub fn service_ingress(&self) -> Option<&ServiceIngressModel> {
        self.service_ingress.as_ref()
    }

    pub fn resolutions(&self) -> &ResolutionModels {
        &self.resolutions
    }

    pub fn entity_model(&self) -> &PublicationEntityModel {
        &self.entity_model
    }

    /// Pipeline 文档要求的 name resolution 单一事实来源。
    ///
    /// lowering 必须从这里读依赖别名集(package aliases / service aliases / module roots),
    /// 而不是通过 `dependencies()` 重新取原始数据。这保证了 pipeline 的"不通过另一条路径
    /// 重新计算 name resolution"约束。
    pub fn name_resolution(&self) -> &NameResolutionModel {
        &self.name_resolution
    }

    pub fn type_resolution(&self) -> &TypeResolutionModel {
        &self.type_resolution
    }

    pub fn expression_sources(&self) -> &ExpressionSourceMap {
        &self.expression_sources
    }

    pub fn expression_types(&self) -> &ExpressionTypeModel {
        &self.expression_types
    }

    pub fn publication_api(&self) -> &PublicationApiModel {
        &self.publication_api
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn publication_api_mut(&mut self) -> &mut PublicationApiModel {
        &mut self.publication_api
    }

    pub fn export_bindings(&self) -> &ExportBindingModel {
        &self.export_bindings
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn export_bindings_mut(&mut self) -> &mut ExportBindingModel {
        &mut self.export_bindings
    }

    #[allow(dead_code)]
    pub fn source_identity(&self) -> &str {
        &self.source_identity
    }

    #[allow(dead_code)]
    pub fn declaration_anchors(&self) -> &PublicationDeclarationAnchors {
        &self.declaration_anchors
    }

    #[allow(dead_code)]
    pub fn own_config_requirements(&self) -> &ConfigRequirementSet {
        &self.own_config_requirements
    }

    #[allow(dead_code)]
    pub fn dependency_config_requirements(&self) -> &ConfigRequirementSet {
        &self.dependency_config_requirements
    }

    #[allow(dead_code)]
    pub fn effective_config_requirements(&self) -> &ConfigRequirementSet {
        &self.effective_config_requirements
    }

    pub fn legacy_config_projection_requirements(&self) -> ConfigRequirementSet {
        self.effective_config_requirements.matching_scope(
            &ConfigRequirementScope::from_publication_policy(self.sources.policy.as_borrowed()),
        )
    }

    pub fn with_semantic_context<T, E>(
        &self,
        f: impl for<'context> FnOnce(&PublicationSemanticContext<'context>) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<PublicationError>,
    {
        let plan = self.plan();
        let semantic_publication = plan.semantic_publication(self.sources.parsed_sources());
        let semantic_context = PublicationSemanticContext::build(&semantic_publication)
            .map_err(|error| plan.diagnostics.publication_semantic_context_error(error))?;
        f(&semantic_context)
    }

    pub fn plan(&self) -> PublicationCompilePlan<'_> {
        PublicationCompilePlan::from_policy(self.sources.policy.as_borrowed())
    }

    pub fn publication_kind(&self) -> PublicationKind {
        self.sources.policy.publication_kind()
    }

    pub fn is_service_publication(&self) -> bool {
        matches!(self.publication_kind(), PublicationKind::Service)
    }

    pub fn policy(&self) -> PublicationCompilePolicy<'_> {
        self.sources.policy.as_borrowed()
    }
}

fn service_ingress_from_seed(
    policy: &SourceCompilePolicy,
    service_ingress: Option<ServiceIngressSeed>,
) -> Result<Option<ServiceIngressModel>, PublicationError> {
    match policy {
        SourceCompilePolicy::Service { .. } => {
            ServiceIngressModel::build_from_seed(service_ingress.unwrap_or_default()).map(Some)
        }
        SourceCompilePolicy::Package { .. } => Ok(None),
    }
}

fn validate_no_plain_interface_value_types(
    diagnostic_root: &Path,
    parsed_sources: &[ParsedCompilerSource],
    type_resolution: &TypeResolutionModel,
) -> Result<(), PublicationError> {
    let mut violations = Vec::new();

    for parsed in parsed_sources
        .iter()
        .filter(|parsed| !parsed.source().is_test_file)
    {
        let path = diagnostic_root
            .join(&parsed.source().relative_path)
            .display()
            .to_string();
        let module_path = parsed.source().module_path.as_str();
        for ty in &parsed.ast().types {
            collect_type_decl_interface_value_violations(
                &path,
                module_path,
                ty,
                type_resolution,
                &mut violations,
            );
        }
        for alias in &parsed.ast().aliases {
            collect_alias_decl_interface_value_violations(
                &path,
                module_path,
                alias,
                type_resolution,
                &mut violations,
            );
        }
        for interface in &parsed.ast().interfaces {
            for operation in &interface.operations {
                collect_operation_interface_value_signature_violations(
                    &path,
                    module_path,
                    &format!(
                        "interface requirement `{}.{}`",
                        interface.name, operation.name
                    ),
                    operation,
                    interface.type_params.iter().cloned(),
                    type_resolution,
                    &mut violations,
                );
            }
        }
        for function in &parsed.ast().functions {
            collect_function_interface_value_signature_violations(
                &path,
                module_path,
                &format!("function `{}`", function.name),
                function,
                std::iter::empty(),
                type_resolution,
                &mut violations,
            );
        }
        for signature in &parsed.ast().function_signatures {
            collect_operation_interface_value_signature_violations(
                &path,
                module_path,
                &format!("function `{}`", signature.name),
                signature,
                std::iter::empty(),
                type_resolution,
                &mut violations,
            );
        }
        for implementation in &parsed.ast().impls {
            let impl_type_params =
                crate::shared::type_syntax::generic_parts(&implementation.target)
                    .map(|parts| {
                        parts
                            .args
                            .into_iter()
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
            for method in &implementation.method_bodies {
                collect_function_interface_value_signature_violations(
                    &path,
                    module_path,
                    &format!("function `{}.{}`", implementation.target, method.name),
                    method,
                    impl_type_params.iter().cloned(),
                    type_resolution,
                    &mut violations,
                );
            }
            for method in &implementation.methods {
                collect_operation_interface_value_signature_violations(
                    &path,
                    module_path,
                    &format!("function `{}.{}`", implementation.target, method.name),
                    method,
                    impl_type_params.iter().cloned(),
                    type_resolution,
                    &mut violations,
                );
            }
        }
    }

    if violations.is_empty() {
        return Ok(());
    }

    Err(PublicationError::ContractValidation {
        message: format!(
            "ordinary ABI/value types cannot use interface values:\n{}",
            violations
                .into_iter()
                .map(|violation| format!("- {violation}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    })
}

fn collect_type_decl_interface_value_violations(
    path: &str,
    module_path: &str,
    ty: &TypeDecl,
    type_resolution: &TypeResolutionModel,
    violations: &mut Vec<String>,
) {
    let context = TypeResolutionContext::with_type_params(
        module_path,
        ty.type_params.iter().cloned().collect(),
    );
    if let Some(alias) = &ty.alias {
        collect_type_ref_interface_value_signature_violations(
            path,
            &format!("type `{}`", ty.name),
            "representation target",
            alias,
            &context,
            type_resolution,
            violations,
        );
    }
    for field in &ty.fields {
        collect_type_ref_interface_value_signature_violations(
            path,
            &format!("type `{}`", ty.name),
            &format!("field `{}`", field.name),
            &field.ty,
            &context,
            type_resolution,
            violations,
        );
    }
}

fn collect_alias_decl_interface_value_violations(
    path: &str,
    module_path: &str,
    alias: &AliasDecl,
    type_resolution: &TypeResolutionModel,
    violations: &mut Vec<String>,
) {
    let context = TypeResolutionContext::source(module_path);
    collect_type_ref_interface_value_signature_violations(
        path,
        &format!("alias `{}`", alias.name),
        "target",
        &alias.target_type,
        &context,
        type_resolution,
        violations,
    );
}

fn collect_function_interface_value_signature_violations(
    path: &str,
    module_path: &str,
    subject: &str,
    function: &FunctionDecl,
    inherited_type_params: impl Iterator<Item = String>,
    type_resolution: &TypeResolutionModel,
    violations: &mut Vec<String>,
) {
    let type_params = inherited_type_params
        .chain(function.type_params.iter().cloned())
        .collect::<BTreeSet<_>>();
    let context = TypeResolutionContext::with_type_params(module_path, type_params);
    for param in &function.params {
        collect_type_ref_interface_value_signature_violations(
            path,
            subject,
            &format!("parameter `{}`", param.name),
            &param.ty,
            &context,
            type_resolution,
            violations,
        );
    }
    collect_type_ref_interface_value_signature_violations(
        path,
        subject,
        "return type",
        &function.return_type,
        &context,
        type_resolution,
        violations,
    );
}

fn collect_operation_interface_value_signature_violations(
    path: &str,
    module_path: &str,
    subject: &str,
    operation: &InterfaceOperation,
    inherited_type_params: impl Iterator<Item = String>,
    type_resolution: &TypeResolutionModel,
    violations: &mut Vec<String>,
) {
    let type_params = inherited_type_params
        .chain(operation.type_params.iter().cloned())
        .collect::<BTreeSet<_>>();
    let context = TypeResolutionContext::with_type_params(module_path, type_params);
    for param in &operation.params {
        collect_type_ref_interface_value_signature_violations(
            path,
            subject,
            &format!("parameter `{}`", param.name),
            &param.ty,
            &context,
            type_resolution,
            violations,
        );
    }
    collect_type_ref_interface_value_signature_violations(
        path,
        subject,
        "return type",
        &operation.return_type,
        &context,
        type_resolution,
        violations,
    );
}

fn collect_type_ref_interface_value_signature_violations(
    path: &str,
    subject: &str,
    position: &str,
    ty: &TypeRef,
    context: &TypeResolutionContext<'_>,
    type_resolution: &TypeResolutionModel,
    violations: &mut Vec<String>,
) {
    let Ok(resolved) = type_resolution.resolve_type_ref(ty, context) else {
        return;
    };
    if type_resolution.contains_interface_type(&resolved, context) {
        violations.push(format!(
            "{path}: {subject} {position} uses interface type `{}`; interfaces are binding/ABI declarations and cannot be passed as ordinary values",
            ty.name
        ));
    }
}

impl PublicationSourceSet {
    fn new(parsed_sources: Vec<ParsedCompilerSource>, policy: SourceCompilePolicy) -> Self {
        Self {
            parsed_sources,
            policy,
        }
    }

    pub fn parsed_sources(&self) -> &[ParsedCompilerSource] {
        &self.parsed_sources
    }

    pub fn role_for(&self, source: &ParsedCompilerSource) -> PublicationSourceRole {
        PublicationCompilePlan::from_policy(self.policy.as_borrowed())
            .file_role_policy
            .file_role(source)
    }
}

impl SourceCompilePolicy {
    fn from_borrowed(policy: PublicationCompilePolicy<'_>) -> Self {
        match policy {
            PublicationCompilePolicy::Package { package_id } => Self::Package {
                package_id: package_id.to_string(),
            },
            PublicationCompilePolicy::Service { service_id } => Self::Service {
                service_id: service_id.to_string(),
            },
        }
    }

    fn as_borrowed(&self) -> PublicationCompilePolicy<'_> {
        match self {
            Self::Package { package_id } => PublicationCompilePolicy::Package { package_id },
            Self::Service { service_id } => PublicationCompilePolicy::Service { service_id },
        }
    }

    fn publication_kind(&self) -> PublicationKind {
        match self {
            Self::Package { .. } => PublicationKind::Package,
            Self::Service { .. } => PublicationKind::Service,
        }
    }
}

impl SourceIndexes {
    fn build(
        parsed_sources: &[ParsedCompilerSource],
        package_aliases: &BTreeMap<String, Vec<String>>,
        package_db_metadata_index: Option<PublicationDbMetadataIndex>,
        plan: PublicationCompilePlan<'_>,
    ) -> Result<Self, PublicationError> {
        let publication_type_symbols = publication_type_symbols(parsed_sources);
        let mut publication_db_metadata_index = publication_db_metadata_index(
            parsed_sources
                .iter()
                .map(|parsed| (parsed.source().module_path.as_str(), parsed.ast())),
            package_aliases,
            &publication_type_symbols,
        )
        .map_err(|error| plan.diagnostics.publication_db_metadata_index_error(error))?;
        if let Some(package_db_metadata_index) = package_db_metadata_index {
            publication_db_metadata_index.extend(package_db_metadata_index);
        }
        Ok(Self {
            publication_type_symbols,
            publication_db_metadata_index,
        })
    }

    pub fn publication_type_symbols(&self) -> &PublicationTypeSymbolIndex {
        &self.publication_type_symbols
    }

    pub fn publication_db_metadata_index(&self) -> &PublicationDbMetadataIndex {
        &self.publication_db_metadata_index
    }
}

impl ResolutionModels {
    pub(super) fn build(parsed_sources: &[ParsedCompilerSource]) -> Self {
        let alias_targets_by_module = parsed_sources
            .iter()
            .map(|parsed| {
                (
                    parsed.source().module_path.clone(),
                    parsed.alias_targets().clone(),
                )
            })
            .collect();
        Self {
            alias_targets_by_module,
        }
    }

    pub fn alias_targets_for_module(&self, module_path: &str) -> &BTreeMap<String, String> {
        self.alias_targets_by_module
            .get(module_path)
            .expect("resolution alias target index must include every lowered source")
    }
}

impl ResolvedDependencies {
    fn new(
        package_aliases: BTreeMap<String, Vec<String>>,
        package_dependencies: Vec<PackageDependency>,
        dependency_package_operation_facts: Vec<DependencyPackageOperationFacts>,
        service_dependencies: ResolvedServiceDependencies,
    ) -> Self {
        let service_dependency_aliases = service_dependencies.aliases();
        Self {
            package_aliases,
            package_dependencies,
            dependency_package_operation_facts,
            service_dependency_aliases,
            service_dependencies,
        }
    }

    pub fn package_aliases(&self) -> &BTreeMap<String, Vec<String>> {
        &self.package_aliases
    }

    pub fn package_dependencies(&self) -> &[PackageDependency] {
        &self.package_dependencies
    }

    pub fn package_operation_refs(&self, package_id: &str, version: &str) -> Vec<String> {
        let mut refs = vec![package_id.to_string()];
        if package_id == crate::shared::id::SKIFF_STD_PUBLICATION_ID {
            refs.push("std".to_string());
        }
        for dependency in &self.package_dependencies {
            if dependency.id == package_id && dependency.version == version {
                refs.push(dependency.effective_alias().to_string());
            }
        }
        refs.sort();
        refs.dedup();
        refs
    }

    pub fn dependency_package_operation_facts(&self) -> &[DependencyPackageOperationFacts] {
        &self.dependency_package_operation_facts
    }

    fn service_dependency_aliases(&self) -> &BTreeSet<String> {
        &self.service_dependency_aliases
    }

    pub fn service_dependencies(&self) -> &ResolvedServiceDependencies {
        &self.service_dependencies
    }
}

impl PublicationApiModel {
    fn new(seed: PublicationApiSeed) -> Self {
        Self { seed }
    }

    pub fn seed(&self) -> &PublicationApiSeed {
        &self.seed
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn seed_mut(&mut self) -> &mut PublicationApiSeed {
        &mut self.seed
    }
}

impl ExportBindingModel {
    fn from_publication_api(seed: &PublicationApiSeed) -> Self {
        Self {
            public_symbols: seed
                .public_symbols
                .iter()
                .map(|(public_path, symbol)| {
                    (
                        public_path.clone(),
                        ExportSymbolBinding {
                            public_path: symbol.public_path.clone(),
                            source_module: symbol.source_module.clone(),
                            source_symbol: symbol.source_symbol.clone(),
                            kind: symbol.kind,
                        },
                    )
                })
                .collect(),
            public_callables: seed
                .public_callables
                .iter()
                .map(|(public_path, callable)| {
                    (
                        public_path.clone(),
                        ExportCallableBinding {
                            public_path: callable.public_path.clone(),
                            source_module: callable.source_module.clone(),
                            source_symbol: callable.source_symbol.clone(),
                            kind: callable.kind,
                        },
                    )
                })
                .collect(),
            public_schema_types: seed
                .public_schema_types
                .iter()
                .map(|(public_path, schema)| {
                    (
                        public_path.clone(),
                        ExportSchemaBinding {
                            public_path: schema.public_path.clone(),
                            source_module: schema.source_module.clone(),
                            source_symbol: schema.source_symbol.clone(),
                            kind: schema.kind,
                        },
                    )
                })
                .collect(),
            public_instances: seed
                .public_instances
                .iter()
                .map(|(public_path, instance)| {
                    (
                        public_path.clone(),
                        ExportPublicInstanceBinding {
                            public_path: instance.public_path.clone(),
                            source_module: instance.source_module.clone(),
                            source_symbol: instance.source_symbol.clone(),
                            interfaces: instance
                                .interfaces
                                .iter()
                                .map(|interface| ExportPublicInstanceInterfaceBinding {
                                    source_module: interface.source_module.clone(),
                                    source_symbol: interface.source_symbol.clone(),
                                })
                                .collect(),
                        },
                    )
                })
                .collect(),
            module_exports: seed.module_exports.clone(),
        }
    }

    pub fn public_symbols(&self) -> &BTreeMap<String, ExportSymbolBinding> {
        &self.public_symbols
    }

    pub fn public_callables(&self) -> &BTreeMap<String, ExportCallableBinding> {
        &self.public_callables
    }

    pub fn public_schema_types(&self) -> &BTreeMap<String, ExportSchemaBinding> {
        &self.public_schema_types
    }

    pub fn public_instances(&self) -> &BTreeMap<String, ExportPublicInstanceBinding> {
        &self.public_instances
    }

    pub fn module_exports(&self) -> &[PublicModuleExport] {
        &self.module_exports
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn module_exports_mut(&mut self) -> &mut Vec<PublicModuleExport> {
        &mut self.module_exports
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn insert_public_schema_type(
        &mut self,
        public_path: impl Into<String>,
        source_module: impl Into<String>,
        source_symbol: impl Into<String>,
        kind: PublicTypeKind,
    ) {
        let public_path = public_path.into();
        self.public_schema_types.insert(
            public_path.clone(),
            ExportSchemaBinding {
                public_path,
                source_module: source_module.into(),
                source_symbol: source_symbol.into(),
                kind,
            },
        );
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_public_schema_type_public_path(
        &mut self,
        key: &str,
        public_path: impl Into<String>,
    ) {
        if let Some(binding) = self.public_schema_types.get_mut(key) {
            binding.public_path = public_path.into();
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn remove_public_callable_source_key(&mut self, source_key: &SourceSymbolKey) {
        self.public_callables.retain(|_, callable| {
            source_key.module_path() != callable.source_module
                || source_key.symbol() != callable.source_symbol
        });
    }
}
