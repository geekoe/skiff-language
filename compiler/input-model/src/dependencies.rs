use std::collections::BTreeSet;

use serde::{de, Deserialize, Deserializer};
use skiff_compiler_core::id::{PublicationId, SKIFF_STD_PUBLICATION_ID};

pub use skiff_compiler_core::path_safety::{
    is_safe_publication_artifact_id_component, is_safe_publication_artifact_path_segment,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageDependency {
    pub id: String,
    pub version: String,
    pub alias: Option<String>,
    pub top_level_alias: Option<String>,
}

impl PackageDependency {
    pub fn id(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: "1.0.0".to_string(),
            alias: None,
            top_level_alias: None,
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
            #[serde(rename = "topLevelAlias")]
            top_level_alias: Option<String>,
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
            top_level_alias: dependency.top_level_alias,
        })
    }
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
        collect_dependency_alias_violations(
            dependency,
            field_label,
            "alias",
            alias,
            aliases,
            violations,
        );
    } else {
        let effective_alias = dependency.effective_alias();
        if !aliases.insert(effective_alias.to_string()) {
            violations.push(format!(
                "{field_label} alias {effective_alias} is assigned to more than one dependency name"
            ));
        }
    }
    if let Some(top_level_alias) = &dependency.top_level_alias {
        if field_label != "packages" {
            violations.push(format!(
                "{field_label} entry {} cannot declare topLevelAlias; it is available only for package dependencies of test services",
                dependency.id
            ));
        }
        collect_dependency_alias_violations(
            dependency,
            field_label,
            "topLevelAlias",
            top_level_alias,
            aliases,
            violations,
        );
    }
}

fn collect_dependency_alias_violations(
    dependency: &PackageDependency,
    field_label: &str,
    key: &str,
    alias: &str,
    aliases: &mut BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    if !is_valid_source_import_alias(alias) {
        violations.push(format!(
            "{field_label} entry {} {key} {alias} must match [a-z][A-Za-z0-9_]*",
            dependency.id
        ));
    } else if is_reserved_source_import_alias(alias)
        && !(key == "alias" && alias == "std" && is_standard_package_id(&dependency.id))
    {
        violations.push(format!(
            "{field_label} entry {} {key} {alias} uses a reserved package name",
            dependency.id
        ));
    }
    if !aliases.insert(alias.to_string()) {
        violations.push(format!(
            "{field_label} {key} {alias} is assigned to more than one dependency name"
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
    skiff_artifact_model::is_dependency_alias_lexically_valid(alias)
}

pub fn is_reserved_source_import_alias(alias: &str) -> bool {
    skiff_artifact_model::is_dependency_alias_reserved(alias)
}

pub fn is_complex_package_dependency_id(package_id: &str) -> bool {
    package_id.contains('.') || package_id.contains('/')
}

#[cfg(test)]
mod tests;
