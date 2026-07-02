use std::collections::{BTreeMap, BTreeSet};

use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use skiff_artifact_model::{
    InterfaceInstantiationRef, OperationAbiRef, ServiceDependencyConstraint,
};
use skiff_compiler_core::id::{PublicationId, SKIFF_STD_PUBLICATION_ID};

pub use skiff_compiler_core::path_safety::{
    is_safe_publication_artifact_id_component, is_safe_publication_artifact_path_segment,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageDependency {
    pub id: String,
    pub version: String,
    pub alias: Option<String>,
    pub config: Value,
    pub collection_name_mapping: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDependency {
    pub id: String,
    pub version: String,
    pub alias: String,
}

#[derive(Debug, Clone, Default)]
pub struct ResolvedServiceDependencies {
    constraints: Vec<ServiceDependencyConstraint>,
    dependency_lock: Vec<ServiceDependencyLockEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceDependencyLockEntry {
    kind: &'static str,
    id: String,
    version: String,
    alias: String,
    declared_alias: String,
    build_id: String,
    service_protocol_identity: String,
    operations: Vec<String>,
    targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    remote_box_provenance: Vec<ServiceDependencyRemoteBoxProvenance>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceDependencyRemoteBoxProvenance {
    pub interface: InterfaceInstantiationRef,
    pub interface_display: String,
    pub public_instance: String,
    pub method_abi_id: String,
    pub operation: OperationAbiRef,
}

impl PackageDependency {
    pub fn id(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: "1.0.0".to_string(),
            alias: None,
            config: empty_dependency_config(),
            collection_name_mapping: BTreeMap::new(),
        }
    }

    pub fn effective_alias(&self) -> &str {
        self.alias.as_deref().unwrap_or_else(|| {
            if self.id == SKIFF_STD_PUBLICATION_ID {
                "std"
            } else {
                &self.id
            }
        })
    }
}

impl ResolvedServiceDependencies {
    pub fn new(
        constraints: Vec<ServiceDependencyConstraint>,
        dependency_lock: Vec<ServiceDependencyLockEntry>,
    ) -> Self {
        Self {
            constraints,
            dependency_lock,
        }
    }

    pub fn constraints(&self) -> &[ServiceDependencyConstraint] {
        &self.constraints
    }

    pub fn dependency_lock(&self) -> &[ServiceDependencyLockEntry] {
        &self.dependency_lock
    }

    pub fn aliases(&self) -> BTreeSet<String> {
        self.constraints
            .iter()
            .map(|dependency| dependency.alias.clone())
            .collect()
    }
}

impl ServiceDependencyLockEntry {
    pub fn from_resolved_service(
        declared: &ServiceDependency,
        resolved: &ServiceDependencyConstraint,
    ) -> Self {
        Self {
            kind: "service",
            id: declared.id.clone(),
            version: declared.version.clone(),
            alias: declared.alias.clone(),
            declared_alias: declared.alias.clone(),
            build_id: resolved.build_id.clone(),
            service_protocol_identity: resolved.service_protocol_identity.clone(),
            operations: resolved
                .publication_abi
                .operation_exports
                .iter()
                .map(|operation| operation.public_path.clone())
                .collect(),
            targets: resolved
                .publication_abi
                .operation_exports
                .iter()
                .map(|operation| operation.operation_abi_id.clone())
                .collect(),
            remote_box_provenance: Vec::new(),
        }
    }

    pub fn alias(&self) -> &str {
        &self.alias
    }

    pub fn add_remote_box_provenance(&mut self, provenance: ServiceDependencyRemoteBoxProvenance) {
        if !self.remote_box_provenance.contains(&provenance) {
            self.remote_box_provenance.push(provenance);
        }
    }
}

impl<'de> Deserialize<'de> for PackageDependency {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawDetailedPackageDependency {
            id: Option<String>,
            version: Option<String>,
            alias: Option<String>,
            collection_name_mapping: Option<BTreeMap<String, String>>,
        }

        let dependency = RawDetailedPackageDependency::deserialize(deserializer)?;
        let Some(id) = dependency.id else {
            return Err(de::Error::custom("packages entry requires id and version"));
        };
        let Some(version) = dependency.version else {
            return Err(de::Error::custom("packages entry requires id and version"));
        };
        Ok(Self {
            id,
            version,
            alias: dependency.alias,
            config: empty_dependency_config(),
            collection_name_mapping: dependency.collection_name_mapping.unwrap_or_default(),
        })
    }
}

pub fn empty_dependency_config() -> Value {
    Value::Object(Map::new())
}

pub fn dependency_config_is_empty(value: &Value) -> bool {
    matches!(value, Value::Object(object) if object.is_empty())
}

pub fn collect_package_dependency_violations(
    dependency: &PackageDependency,
    field_label: &str,
    aliases: &mut BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    if dependency.id.trim().is_empty() || dependency.version.trim().is_empty() {
        violations.push(format!("{field_label} entry requires id and version"));
        return;
    }
    if dependency.id == "ext" || dependency.id.starts_with("ext.") {
        violations.push("ext root has been removed".to_string());
        return;
    }
    if dependency.alias.as_deref() == Some("ext") {
        violations.push("ext root has been removed".to_string());
        return;
    }
    if dependency.id == SKIFF_STD_PUBLICATION_ID {
        violations.push(format!(
            "{field_label} entry {} is invalid: platform std is built into the compiler; remove this package dependency",
            dependency.id
        ));
    } else if dependency.id == "std" || dependency.id.starts_with("std.") {
        violations.push(format!(
            "{field_label} entry {} is invalid: official standard package is skiff.run/std",
            dependency.id
        ));
    } else if !is_publication_dependency_id(&dependency.id) {
        violations.push(format!(
            "{field_label} entry {} must be a publication id",
            dependency.id
        ));
    } else if !is_safe_publication_artifact_id_component(&dependency.id) {
        violations.push(format!(
            "{field_label} entry {} must be safe for package artifact paths",
            dependency.id
        ));
    } else if !is_safe_publication_artifact_path_segment(&dependency.version) {
        violations.push(format!(
            "{field_label} entry {} version {} must be safe for package artifact paths",
            dependency.id, dependency.version
        ));
    } else if is_complex_package_dependency_id(&dependency.id)
        && dependency.alias.is_none()
        && !is_standard_package_id(&dependency.id)
    {
        violations.push(format!(
            "{field_label} entry {} requires alias",
            dependency.id
        ));
    } else if !is_complex_package_dependency_id(&dependency.id)
        && dependency.alias.is_none()
        && !is_standard_package_id(&dependency.id)
        && is_reserved_source_import_alias(&dependency.id)
    {
        violations.push(format!(
            "{field_label} entry {} uses a reserved package name",
            dependency.id
        ));
    }
    if let Some(alias) = &dependency.alias {
        if !is_valid_source_import_alias(alias) {
            violations.push(format!(
                "{field_label} entry {} alias {alias} must match [a-z][A-Za-z0-9_]*",
                dependency.id
            ));
        } else if is_reserved_source_import_alias(alias)
            && !(alias == "std" && is_standard_package_id(&dependency.id))
        {
            violations.push(format!(
                "{field_label} entry {} alias {alias} uses a reserved package name",
                dependency.id
            ));
        }
    }
    let effective_alias = dependency.effective_alias();
    if !aliases.insert(effective_alias.to_string()) {
        violations.push(format!(
            "{field_label} alias {effective_alias} is assigned to more than one package"
        ));
    }
}

pub fn canonical_publication_dependency_id(id: &str) -> Option<String> {
    PublicationId::parse(id)
        .map(PublicationId::into_string)
        .ok()
}

pub fn is_publication_dependency_id(id: &str) -> bool {
    canonical_publication_dependency_id(id).is_some()
}

pub fn is_standard_package_id(id: &str) -> bool {
    id == SKIFF_STD_PUBLICATION_ID
}

pub fn is_valid_source_import_alias(alias: &str) -> bool {
    let mut chars = alias.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_lowercase() && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub fn is_reserved_source_import_alias(alias: &str) -> bool {
    matches!(
        alias,
        "package" | "service" | "std" | "ext" | "connect" | "config" | "root"
    )
}

pub fn is_complex_package_dependency_id(package_id: &str) -> bool {
    package_id.contains('.') || package_id.contains('/')
}
