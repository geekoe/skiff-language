use std::{collections::BTreeMap, path::PathBuf};

use crate::{PackageDependency, PublicationApiSpec, PublicationResourceSpec};
use skiff_artifact_model::PackageStateRequirement;
use skiff_compiler_core::id::{PublicationId, PublicationIdError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationManifest {
    pub id: PublicationId,
    pub version: String,
    pub api: PublicationApiSpec,
    pub dependencies: Vec<PackageDependency>,
    pub resources: Vec<PublicationResourceSpec>,
    pub state: BTreeMap<String, PackageStateRequirement>,
    pub provenance: ManifestProvenance,
}

impl PublicationManifest {
    pub fn new(
        id: PublicationId,
        version: String,
        api: PublicationApiSpec,
        dependencies: Vec<PackageDependency>,
        provenance: ManifestProvenance,
    ) -> Self {
        Self {
            id,
            version,
            api,
            dependencies,
            resources: Vec::new(),
            state: BTreeMap::new(),
            provenance,
        }
    }

    pub fn new_with_resources(
        id: PublicationId,
        version: String,
        api: PublicationApiSpec,
        dependencies: Vec<PackageDependency>,
        resources: Vec<PublicationResourceSpec>,
        provenance: ManifestProvenance,
    ) -> Self {
        Self {
            id,
            version,
            api,
            dependencies,
            resources,
            state: BTreeMap::new(),
            provenance,
        }
    }

    pub fn with_state(mut self, state: BTreeMap<String, PackageStateRequirement>) -> Self {
        self.state = state;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestProvenance {
    pub owner: ManifestOwner,
    pub path: PathBuf,
    pub synthetic: bool,
}

impl ManifestProvenance {
    pub fn file(path: impl Into<PathBuf>, owner: ManifestOwner) -> Self {
        Self {
            owner,
            path: path.into(),
            synthetic: false,
        }
    }

    pub fn synthetic(path: impl Into<PathBuf>, owner: ManifestOwner) -> Self {
        Self {
            owner,
            path: path.into(),
            synthetic: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestOwner {
    CompilerStandardPackage,
    UserOrBuiltinPackage,
}

pub fn parse_publication_id_field(
    field: &str,
    value: Option<String>,
    violations: &mut Vec<String>,
) -> Option<PublicationId> {
    let value = required_string(field, value, violations)?;
    match PublicationId::parse(&value) {
        Ok(id) => Some(id),
        Err(PublicationIdError::UnsafePathForm) if value == "std" || value.starts_with("std.") => {
            violations.push(format!(
                "{field} {value} is invalid: official standard package is skiff.run/std"
            ));
            None
        }
        Err(source) => {
            violations.push(format!(
                "{field} {value} must be a publication id: {source}"
            ));
            None
        }
    }
}

pub fn validate_publication_version_field(
    field: &str,
    value: Option<String>,
    violations: &mut Vec<String>,
) -> Option<String> {
    required_string(field, value, violations)
}

fn required_string(
    field: &str,
    value: Option<String>,
    violations: &mut Vec<String>,
) -> Option<String> {
    let Some(value) = value else {
        violations.push(format!("{field} is required"));
        return None;
    };
    if value.trim().is_empty() {
        violations.push(format!("{field} cannot be empty"));
        return None;
    }
    Some(value)
}
