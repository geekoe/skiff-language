use std::collections::BTreeMap;

use super::config_usage::{ConfigPresenceUse, ConfigSourceSpan, ConfigUsageSeed, ConfigUse};
use super::SourceCompilePackageDependencyFact;
use crate::shared::publication_error::PublicationError;
use compiler_input_model::{PackageDependency, PublicationCompilePolicy};

type PackageDependencyPath = Vec<ConfigRequirementDependencyStep>;
type PackageDependencyPathsByKey = BTreeMap<(String, String), Vec<PackageDependencyPath>>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigRequirementSet {
    requirements: Vec<ConfigRequirement>,
}

#[derive(Clone, Debug)]
pub struct DependencyPackageConfigFacts<'a> {
    pub id: &'a str,
    pub version: &'a str,
    pub dependencies: Vec<ConfigRequirementDependencyStep>,
    pub own_config_requirements: &'a ConfigRequirementSet,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigRequirement {
    scope: ConfigRequirementScope,
    path: String,
    access: ConfigRequirementAccess,
    provenances: Vec<ConfigRequirementProvenance>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfigRequirementScope {
    Service,
    Package { package_id: String },
}

impl ConfigRequirementScope {
    pub(crate) fn from_publication_policy(policy: PublicationCompilePolicy<'_>) -> Self {
        match policy {
            PublicationCompilePolicy::Service { .. } => Self::Service,
            PublicationCompilePolicy::Package { package_id } => Self::Package {
                package_id: package_id.to_string(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfigRequirementAccess {
    Require { ty: String },
    Optional { ty: String },
    Has,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigRequirementProvenance {
    source_path: String,
    source_span: Option<ConfigSourceSpan>,
    declaring_publication: Option<ConfigRequirementPublicationRef>,
    dependency_path: Vec<ConfigRequirementDependencyStep>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigRequirementPublicationRef {
    id: String,
    version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigRequirementDependencyStep {
    id: String,
    version: String,
    alias: Option<String>,
}

#[derive(Clone, Debug)]
struct EffectiveTypedRequirement {
    ty: String,
    required: bool,
    provenances: Vec<ConfigRequirementProvenance>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ConfigRequirementMergeKey {
    scope: ConfigRequirementScope,
    path: String,
}

impl ConfigRequirementSet {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_usage_seed(seed: &ConfigUsageSeed, scope: ConfigRequirementScope) -> Self {
        let mut requirements =
            seed.typed
                .iter()
                .map(|config_use| ConfigRequirement::from_config_use(config_use, scope.clone()))
                .chain(seed.presence.iter().map(|config_use| {
                    ConfigRequirement::from_presence_use(config_use, scope.clone())
                }))
                .collect::<Vec<_>>();
        requirements.sort();
        requirements.dedup();
        requirements = merge_equivalent_requirement_provenances(requirements);
        Self { requirements }
    }

    pub fn from_service_package_graph(
        dependencies: &[PackageDependency],
        loaded_packages: &[DependencyPackageConfigFacts<'_>],
    ) -> Result<Self, PublicationError> {
        validate_loaded_package_versions(loaded_packages)?;
        let dependency_paths = package_dependency_paths(dependencies, loaded_packages)?;
        let mut sets = Vec::new();
        for package in loaded_packages {
            let declaring_publication =
                ConfigRequirementPublicationRef::new(package.id, package.version);
            let key = package_key(package.id, package.version);
            let dependency_paths = dependency_paths
                .get(&key)
                .cloned()
                .unwrap_or_else(|| vec![Vec::new()]);
            sets.push(
                package
                    .own_config_requirements
                    .with_dependency_contexts(declaring_publication, &dependency_paths),
            );
        }
        Self::merge(&sets)
    }

    pub fn effective(
        own: &ConfigRequirementSet,
        dependency: &ConfigRequirementSet,
    ) -> Result<Self, PublicationError> {
        Self::merge([own, dependency])
    }

    pub fn requirements(&self) -> &[ConfigRequirement] {
        &self.requirements
    }

    pub fn matching_scope(&self, scope: &ConfigRequirementScope) -> Self {
        Self {
            requirements: self
                .requirements
                .iter()
                .filter(|requirement| requirement.scope() == scope)
                .cloned()
                .collect(),
        }
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.requirements.is_empty()
    }

    fn merge<'a>(
        sets: impl IntoIterator<Item = &'a ConfigRequirementSet>,
    ) -> Result<Self, PublicationError> {
        let mut flattened = sets
            .into_iter()
            .flat_map(|set| set.requirements.iter().cloned())
            .collect::<Vec<_>>();
        flattened.sort();
        flattened.dedup();

        let mut typed_by_key =
            BTreeMap::<ConfigRequirementMergeKey, EffectiveTypedRequirement>::new();
        let mut has_by_key = BTreeMap::<ConfigRequirementMergeKey, ConfigRequirement>::new();
        let mut violations = Vec::new();

        for requirement in flattened {
            let key = requirement.merge_key();
            let Some((ty, required)) = requirement.access.typed() else {
                has_by_key
                    .entry(key)
                    .and_modify(|existing| existing.merge_provenances_from(&requirement))
                    .or_insert(requirement);
                continue;
            };
            let ty = ty.to_string();
            let required = required;
            let provenances = requirement.provenances.clone();

            typed_by_key
                .entry(key.clone())
                .and_modify(|existing| {
                    existing.merge(&key, &ty, required, &provenances, &mut violations);
                })
                .or_insert_with(|| EffectiveTypedRequirement {
                    ty,
                    required,
                    provenances,
                });
        }

        if !violations.is_empty() {
            return Err(PublicationError::ContractValidation {
                message: violations
                    .into_iter()
                    .map(|violation| format!("- {violation}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            });
        }

        let mut requirements = typed_by_key
            .into_iter()
            .map(|(key, requirement)| requirement.into_config_requirement(key))
            .chain(has_by_key.into_values())
            .collect::<Vec<_>>();
        requirements.sort();
        requirements.dedup();
        Ok(Self { requirements })
    }

    fn with_dependency_contexts(
        &self,
        declaring_publication: ConfigRequirementPublicationRef,
        dependency_paths: &[PackageDependencyPath],
    ) -> Self {
        let requirements = self
            .requirements
            .iter()
            .cloned()
            .map(|mut requirement| {
                requirement.provenances = normalized_provenances(
                    requirement
                        .provenances
                        .into_iter()
                        .flat_map(|provenance| {
                            dependency_paths.iter().map({
                                let declaring_publication = declaring_publication.clone();
                                move |dependency_path| {
                                    provenance.clone().with_dependency_context(
                                        declaring_publication.clone(),
                                        dependency_path.clone(),
                                    )
                                }
                            })
                        })
                        .collect(),
                );
                requirement
            })
            .collect();
        Self { requirements }
    }
}

impl ConfigRequirement {
    pub fn scope(&self) -> &ConfigRequirementScope {
        &self.scope
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn access(&self) -> &ConfigRequirementAccess {
        &self.access
    }

    pub fn source_path(&self) -> &str {
        self.provenances
            .first()
            .map(ConfigRequirementProvenance::source_path)
            .unwrap_or("")
    }

    pub fn provenances(&self) -> &[ConfigRequirementProvenance] {
        &self.provenances
    }

    #[cfg(test)]
    pub fn provenance_descriptions(&self) -> Vec<String> {
        self.provenances
            .iter()
            .map(ConfigRequirementProvenance::description)
            .collect()
    }

    fn merge_provenances_from(&mut self, other: &Self) {
        self.provenances.extend(other.provenances.iter().cloned());
        self.provenances = normalized_provenances(std::mem::take(&mut self.provenances));
    }

    fn merge_key(&self) -> ConfigRequirementMergeKey {
        ConfigRequirementMergeKey {
            scope: self.scope.clone(),
            path: self.path.clone(),
        }
    }

    fn has_same_requirement_identity(&self, other: &Self) -> bool {
        self.scope == other.scope && self.path == other.path && self.access == other.access
    }

    fn from_config_use(config_use: &ConfigUse, scope: ConfigRequirementScope) -> Self {
        let access = if config_use.required {
            ConfigRequirementAccess::Require {
                ty: config_use.ty.clone(),
            }
        } else {
            ConfigRequirementAccess::Optional {
                ty: config_use.ty.clone(),
            }
        };
        Self {
            scope,
            path: config_use.path.clone(),
            access,
            provenances: vec![ConfigRequirementProvenance::new(
                config_use.source_path.clone(),
                config_use.source_span,
            )],
        }
    }

    fn from_presence_use(config_use: &ConfigPresenceUse, scope: ConfigRequirementScope) -> Self {
        Self {
            scope,
            path: config_use.path.clone(),
            access: ConfigRequirementAccess::Has,
            provenances: vec![ConfigRequirementProvenance::new(
                config_use.source_path.clone(),
                config_use.source_span,
            )],
        }
    }
}

impl ConfigRequirementAccess {
    pub fn typed(&self) -> Option<(&str, bool)> {
        match self {
            Self::Require { ty } => Some((ty, true)),
            Self::Optional { ty } => Some((ty, false)),
            Self::Has => None,
        }
    }

    pub fn is_has(&self) -> bool {
        matches!(self, Self::Has)
    }
}

impl ConfigRequirementProvenance {
    pub fn source_path(&self) -> &str {
        &self.source_path
    }

    pub fn source_span(&self) -> Option<ConfigSourceSpan> {
        self.source_span
    }

    pub fn declaring_publication(&self) -> Option<&ConfigRequirementPublicationRef> {
        self.declaring_publication.as_ref()
    }

    pub fn dependency_path(&self) -> &[ConfigRequirementDependencyStep] {
        &self.dependency_path
    }

    fn new(source_path: String, source_span: Option<ConfigSourceSpan>) -> Self {
        Self {
            source_path,
            source_span,
            declaring_publication: None,
            dependency_path: Vec::new(),
        }
    }

    fn with_dependency_context(
        mut self,
        declaring_publication: ConfigRequirementPublicationRef,
        dependency_path: PackageDependencyPath,
    ) -> Self {
        self.declaring_publication = Some(declaring_publication);
        self.dependency_path = dependency_path;
        self
    }

    fn description(&self) -> String {
        let mut description = self.source_path.clone();
        if let Some(publication) = &self.declaring_publication {
            description.push_str(&format!(
                " in package {} version {}",
                publication.id, publication.version
            ));
        }
        if !self.dependency_path.is_empty() {
            let path = self
                .dependency_path
                .iter()
                .map(ConfigRequirementDependencyStep::description)
                .collect::<Vec<_>>()
                .join(" -> ");
            description.push_str(&format!(" via dependency {path}"));
        }
        description
    }
}

impl ConfigRequirementPublicationRef {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
        }
    }
}

impl ConfigRequirementDependencyStep {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    fn from_dependency(dependency: &PackageDependency) -> Self {
        Self {
            id: dependency.id.clone(),
            version: dependency.version.clone(),
            alias: dependency.alias.clone(),
        }
    }

    pub fn from_package_dependency_fact(dependency: &SourceCompilePackageDependencyFact) -> Self {
        Self {
            id: dependency.id.clone(),
            version: dependency.version.clone(),
            alias: dependency.alias.clone(),
        }
    }

    fn description(&self) -> String {
        match &self.alias {
            Some(alias) => format!("{} version {} alias {}", self.id, self.version, alias),
            None => format!("{} version {}", self.id, self.version),
        }
    }
}

impl ConfigRequirementMergeKey {
    fn description(&self) -> String {
        match &self.scope {
            ConfigRequirementScope::Service => format!("service config path {}", self.path),
            ConfigRequirementScope::Package { package_id } => {
                format!("package {package_id} config path {}", self.path)
            }
        }
    }
}

impl EffectiveTypedRequirement {
    fn merge(
        &mut self,
        key: &ConfigRequirementMergeKey,
        ty: &str,
        required: bool,
        provenances: &[ConfigRequirementProvenance],
        violations: &mut Vec<String>,
    ) {
        if self.ty != ty {
            violations.push(format!(
                "{} has conflicting type requirements: {} from {}, {} from {}",
                key.description(),
                self.ty,
                format_provenance_descriptions(&self.provenances),
                ty,
                format_provenance_descriptions(provenances)
            ));
            return;
        }
        self.required |= required;
        self.provenances.extend(provenances.iter().cloned());
        self.provenances = normalized_provenances(std::mem::take(&mut self.provenances));
    }

    fn into_config_requirement(self, key: ConfigRequirementMergeKey) -> ConfigRequirement {
        let access = if self.required {
            ConfigRequirementAccess::Require { ty: self.ty }
        } else {
            ConfigRequirementAccess::Optional { ty: self.ty }
        };
        ConfigRequirement {
            scope: key.scope,
            path: key.path,
            access,
            provenances: normalized_provenances(self.provenances),
        }
    }
}

fn normalized_provenances(
    mut provenances: Vec<ConfigRequirementProvenance>,
) -> Vec<ConfigRequirementProvenance> {
    provenances.sort();
    provenances.dedup();
    provenances
}

fn merge_equivalent_requirement_provenances(
    requirements: Vec<ConfigRequirement>,
) -> Vec<ConfigRequirement> {
    let mut merged = Vec::<ConfigRequirement>::new();
    for requirement in requirements {
        if let Some(existing) = merged
            .last_mut()
            .filter(|existing| existing.has_same_requirement_identity(&requirement))
        {
            existing.merge_provenances_from(&requirement);
            continue;
        }
        merged.push(requirement);
    }
    merged
}

fn format_provenance_descriptions(provenances: &[ConfigRequirementProvenance]) -> String {
    provenances
        .iter()
        .map(ConfigRequirementProvenance::description)
        .collect::<Vec<_>>()
        .join(", ")
}

fn validate_loaded_package_versions(
    loaded_packages: &[DependencyPackageConfigFacts<'_>],
) -> Result<(), PublicationError> {
    let mut selected_versions = BTreeMap::<String, String>::new();
    let mut violations = Vec::new();
    for package in loaded_packages {
        let id = package.id.to_string();
        let version = package.version.to_string();
        if let Some(selected_version) = selected_versions.get(&id) {
            if selected_version != &version {
                violations.push(format!(
                    "service package graph contains package {id} versions {selected_version} and {version}"
                ));
            }
            continue;
        }
        selected_versions.insert(id, version);
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(PublicationError::ContractValidation {
            message: violations
                .into_iter()
                .map(|violation| format!("- {violation}"))
                .collect::<Vec<_>>()
                .join("\n"),
        })
    }
}

fn package_dependency_paths(
    dependencies: &[PackageDependency],
    loaded_packages: &[DependencyPackageConfigFacts<'_>],
) -> Result<PackageDependencyPathsByKey, PublicationError> {
    let loaded_by_key = loaded_packages
        .iter()
        .map(|package| (package_key(package.id, package.version), package))
        .collect::<BTreeMap<_, _>>();
    let loaded_versions_by_id = loaded_packages
        .iter()
        .map(|package| (package.id.to_string(), package.version.to_string()))
        .collect::<BTreeMap<_, _>>();
    let mut paths = PackageDependencyPathsByKey::new();
    let mut stack = dependencies
        .iter()
        .map(|dependency| {
            (
                package_key(&dependency.id, &dependency.version),
                vec![ConfigRequirementDependencyStep::from_dependency(dependency)],
            )
        })
        .collect::<Vec<_>>();

    while let Some((key, dependency_path)) = stack.pop() {
        let Some(package) = loaded_by_key.get(&key) else {
            return Err(missing_loaded_package_error(
                &key.0,
                &key.1,
                loaded_versions_by_id.get(&key.0),
            ));
        };
        let known_paths = paths.entry(key.clone()).or_default();
        if known_paths.contains(&dependency_path) {
            continue;
        }
        known_paths.push(dependency_path.clone());

        for child_step in &package.dependencies {
            let child_key = package_key(&child_step.id, &child_step.version);
            let child_step = child_step.clone();
            if dependency_path_contains_key(&dependency_path, &child_key) {
                return Err(package_dependency_cycle_error(
                    &dependency_path,
                    &child_step,
                ));
            }
            let mut child_path = dependency_path.clone();
            child_path.push(child_step);
            stack.push((child_key, child_path));
        }
    }

    Ok(paths)
}

fn package_key(id: &str, version: &str) -> (String, String) {
    (id.to_string(), version.to_string())
}

fn missing_loaded_package_error(
    package_id: &str,
    version: &str,
    loaded_version: Option<&String>,
) -> PublicationError {
    let message = match loaded_version {
        Some(loaded_version) => format!(
            "package dependency {package_id} version {version} conflicts with loaded package version {loaded_version}"
        ),
        None => format!("package dependency {package_id} version {version} was not loaded"),
    };
    PublicationError::ContractValidation { message }
}

fn dependency_path_contains_key(
    path: &[ConfigRequirementDependencyStep],
    key: &(String, String),
) -> bool {
    path.iter()
        .any(|step| step.id == key.0 && step.version == key.1)
}

fn package_dependency_cycle_error(
    path: &[ConfigRequirementDependencyStep],
    repeated_step: &ConfigRequirementDependencyStep,
) -> PublicationError {
    let mut cycle = path
        .iter()
        .map(ConfigRequirementDependencyStep::description)
        .collect::<Vec<_>>();
    cycle.push(repeated_step.description());
    PublicationError::ContractValidation {
        message: format!("package dependency cycle detected: {}", cycle.join(" -> ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_merges_required_and_optional_typed_access_to_required() {
        let own = ConfigRequirementSet::from_usage_seed(
            &ConfigUsageSeed {
                typed: vec![
                    typed_use("app.secret", "string", false, "source/b.skiff"),
                    typed_use("app.secret", "string", true, "source/a.skiff"),
                ],
                presence: Vec::new(),
            },
            ConfigRequirementScope::Service,
        );

        let effective = ConfigRequirementSet::effective(&own, &ConfigRequirementSet::empty())
            .expect("required and optional of the same type should merge");

        assert_eq!(
            requirement_tuples(&effective),
            vec![(
                "app.secret".to_string(),
                "require:string".to_string(),
                "source/a.skiff".to_string()
            )]
        );
        assert_eq!(
            provenance_descriptions(&effective, &ConfigRequirementScope::Service, "app.secret"),
            vec!["source/a.skiff".to_string(), "source/b.skiff".to_string()]
        );
    }

    #[test]
    fn effective_keeps_service_and_package_scoped_paths_separate() {
        let service = ConfigRequirementSet::from_usage_seed(
            &ConfigUsageSeed {
                typed: vec![typed_use("shared.secret", "string", true, "service.skiff")],
                presence: Vec::new(),
            },
            ConfigRequirementScope::Service,
        );
        let package = ConfigRequirementSet::from_usage_seed(
            &ConfigUsageSeed {
                typed: vec![typed_use("shared.secret", "number", true, "package.skiff")],
                presence: Vec::new(),
            },
            package_scope("example.com/session"),
        );

        let effective = ConfigRequirementSet::effective(&service, &package)
            .expect("same local path in different scopes should not conflict");

        assert_eq!(
            scoped_requirement_tuples(&effective),
            vec![
                (
                    "service".to_string(),
                    "shared.secret".to_string(),
                    "require:string".to_string(),
                    "service.skiff".to_string()
                ),
                (
                    "package:example.com/session".to_string(),
                    "shared.secret".to_string(),
                    "require:number".to_string(),
                    "package.skiff".to_string()
                ),
            ]
        );
    }

    #[test]
    fn effective_merges_same_package_scoped_typed_access_and_retains_provenance() {
        let package = ConfigRequirementSet::from_usage_seed(
            &ConfigUsageSeed {
                typed: vec![
                    typed_use("session.cookie", "string", false, "source/b.skiff"),
                    typed_use("session.cookie", "string", true, "source/a.skiff"),
                    typed_use("session.cookie", "string", true, "source/a.skiff"),
                ],
                presence: Vec::new(),
            },
            package_scope("example.com/session"),
        );

        let effective = ConfigRequirementSet::effective(&package, &ConfigRequirementSet::empty())
            .expect("same package id and local path with compatible type should merge");

        assert_eq!(
            scoped_requirement_tuples(&effective),
            vec![(
                "package:example.com/session".to_string(),
                "session.cookie".to_string(),
                "require:string".to_string(),
                "source/a.skiff".to_string()
            )]
        );
        assert_eq!(
            provenance_descriptions(
                &effective,
                &package_scope("example.com/session"),
                "session.cookie"
            ),
            vec!["source/a.skiff".to_string(), "source/b.skiff".to_string()]
        );
    }

    #[test]
    fn effective_rejects_incompatible_typed_config_types_with_provenance() {
        let own = ConfigRequirementSet::from_usage_seed(
            &ConfigUsageSeed {
                typed: vec![
                    typed_use("app.secret", "string", true, "source/a.skiff"),
                    typed_use("app.secret", "number", true, "source/b.skiff"),
                ],
                presence: Vec::new(),
            },
            ConfigRequirementScope::Service,
        );

        let error = ConfigRequirementSet::effective(&own, &ConfigRequirementSet::empty())
            .expect_err("different typed requirements for the same path should fail");
        let message = error.to_string();

        assert!(message.contains("app.secret"), "{message}");
        assert!(
            message.contains("conflicting type requirements"),
            "{message}"
        );
        assert!(message.contains("source/a.skiff"), "{message}");
        assert!(message.contains("source/b.skiff"), "{message}");
    }

    #[test]
    fn effective_rejects_incompatible_package_scoped_typed_config_types() {
        let package = ConfigRequirementSet::from_usage_seed(
            &ConfigUsageSeed {
                typed: vec![
                    typed_use("session.cookie", "string", true, "source/a.skiff"),
                    typed_use("session.cookie", "number", true, "source/b.skiff"),
                ],
                presence: Vec::new(),
            },
            package_scope("example.com/session"),
        );

        let error = ConfigRequirementSet::effective(&package, &ConfigRequirementSet::empty())
            .expect_err("same package id and local path with different types should fail");
        let message = error.to_string();

        assert!(message.contains("example.com/session"), "{message}");
        assert!(message.contains("session.cookie"), "{message}");
        assert!(message.contains("source/a.skiff"), "{message}");
        assert!(message.contains("source/b.skiff"), "{message}");
    }

    #[test]
    fn effective_dedupes_has_requirements_by_path_without_typed_requirement() {
        let own = ConfigRequirementSet::from_usage_seed(
            &ConfigUsageSeed {
                typed: Vec::new(),
                presence: vec![
                    presence_use("app.enabled", "source/b.skiff"),
                    presence_use("app.enabled", "source/a.skiff"),
                    presence_use("app.enabled", "source/a.skiff"),
                ],
            },
            ConfigRequirementScope::Service,
        );

        let effective = ConfigRequirementSet::effective(&own, &ConfigRequirementSet::empty())
            .expect("has requirements should dedupe by path");

        assert_eq!(
            requirement_tuples(&effective),
            vec![(
                "app.enabled".to_string(),
                "has".to_string(),
                "source/a.skiff".to_string()
            )]
        );
        assert_eq!(
            provenance_descriptions(&effective, &ConfigRequirementScope::Service, "app.enabled"),
            vec!["source/a.skiff".to_string(), "source/b.skiff".to_string()]
        );
    }

    #[test]
    fn effective_retains_config_source_spans_in_provenance() {
        let source_span = config_source_span(3, 29, 81, 3, 67, 119);
        let later_source_span = config_source_span(4, 29, 149, 4, 67, 187);
        let own = ConfigRequirementSet::from_usage_seed(
            &ConfigUsageSeed {
                typed: vec![
                    ConfigUse {
                        path: "app.secret".to_string(),
                        ty: "string".to_string(),
                        required: true,
                        source_path: "source/a.skiff".to_string(),
                        source_span: Some(source_span),
                    },
                    ConfigUse {
                        path: "app.secret".to_string(),
                        ty: "string".to_string(),
                        required: true,
                        source_path: "source/a.skiff".to_string(),
                        source_span: Some(later_source_span),
                    },
                ],
                presence: vec![ConfigPresenceUse {
                    path: "app.enabled".to_string(),
                    source_path: "source/a.skiff".to_string(),
                    source_span: Some(source_span),
                }],
            },
            ConfigRequirementScope::Service,
        );

        let effective = ConfigRequirementSet::effective(&own, &ConfigRequirementSet::empty())
            .expect("source spans should not affect requirement compatibility");

        assert_eq!(
            provenance_source_spans(&effective, &ConfigRequirementScope::Service, "app.secret"),
            vec![Some(source_span), Some(later_source_span)]
        );
        assert_eq!(
            provenance_source_spans(&effective, &ConfigRequirementScope::Service, "app.enabled"),
            vec![Some(source_span)]
        );
    }

    fn typed_use(path: &str, ty: &str, required: bool, source_path: &str) -> ConfigUse {
        ConfigUse {
            path: path.to_string(),
            ty: ty.to_string(),
            required,
            source_path: source_path.to_string(),
            source_span: None,
        }
    }

    fn presence_use(path: &str, source_path: &str) -> ConfigPresenceUse {
        ConfigPresenceUse {
            path: path.to_string(),
            source_path: source_path.to_string(),
            source_span: None,
        }
    }

    fn package_scope(package_id: &str) -> ConfigRequirementScope {
        ConfigRequirementScope::Package {
            package_id: package_id.to_string(),
        }
    }

    fn requirement_tuples(requirements: &ConfigRequirementSet) -> Vec<(String, String, String)> {
        requirements
            .requirements()
            .iter()
            .map(|requirement| {
                (
                    requirement.path().to_string(),
                    access_label(requirement.access()),
                    requirement.source_path().to_string(),
                )
            })
            .collect()
    }

    fn scoped_requirement_tuples(
        requirements: &ConfigRequirementSet,
    ) -> Vec<(String, String, String, String)> {
        requirements
            .requirements()
            .iter()
            .map(|requirement| {
                (
                    scope_label(requirement.scope()),
                    requirement.path().to_string(),
                    access_label(requirement.access()),
                    requirement.source_path().to_string(),
                )
            })
            .collect()
    }

    fn provenance_descriptions(
        requirements: &ConfigRequirementSet,
        scope: &ConfigRequirementScope,
        path: &str,
    ) -> Vec<String> {
        requirements
            .requirements()
            .iter()
            .find(|requirement| requirement.scope() == scope && requirement.path() == path)
            .expect("requirement by path")
            .provenance_descriptions()
    }

    fn provenance_source_spans(
        requirements: &ConfigRequirementSet,
        scope: &ConfigRequirementScope,
        path: &str,
    ) -> Vec<Option<ConfigSourceSpan>> {
        requirements
            .requirements()
            .iter()
            .find(|requirement| requirement.scope() == scope && requirement.path() == path)
            .expect("requirement by path")
            .provenances()
            .iter()
            .map(ConfigRequirementProvenance::source_span)
            .collect()
    }

    fn config_source_span(
        start_line: usize,
        start_column: usize,
        start_offset: usize,
        end_line: usize,
        end_column: usize,
        end_offset: usize,
    ) -> ConfigSourceSpan {
        ConfigSourceSpan::from(crate::shared::error::SourceSpan {
            start: crate::shared::error::SourceLocation {
                line: start_line,
                column: start_column,
                offset: start_offset,
            },
            end: crate::shared::error::SourceLocation {
                line: end_line,
                column: end_column,
                offset: end_offset,
            },
        })
    }

    fn scope_label(scope: &ConfigRequirementScope) -> String {
        match scope {
            ConfigRequirementScope::Service => "service".to_string(),
            ConfigRequirementScope::Package { package_id } => format!("package:{package_id}"),
        }
    }

    fn access_label(access: &ConfigRequirementAccess) -> String {
        match access {
            ConfigRequirementAccess::Require { ty } => format!("require:{ty}"),
            ConfigRequirementAccess::Optional { ty } => format!("optional:{ty}"),
            ConfigRequirementAccess::Has => "has".to_string(),
        }
    }
}
