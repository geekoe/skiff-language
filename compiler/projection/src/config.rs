use serde::Serialize;
pub use skiff_artifact_model::ConfigShape;
use skiff_artifact_model::{ConfigShapeEntry, ConfigShapeValueType, CONFIG_SHAPE_SCHEMA_VERSION};

use skiff_compiler_projection_input::{
    ConfigRequirementAccessProjection, ConfigRequirementProjection,
    ConfigRequirementProvenanceProjection, ConfigRequirementScopeProjection,
    ConfigRequirementSetProjection, ConfigRequirementsSeed, ConfigSourceSpanProjection,
};

use crate::error::ProjectionError;

const CONFIG_ACTIVATION_SCHEMA_VERSION: &str = "skiff-config-activation-v1";

#[derive(Debug, Clone)]
pub struct ConfigProjection {
    pub shape: ConfigShape,
    pub uses: Vec<ConfigUseEntry>,
    pub activation: ConfigActivation,
    pub requirements: ConfigRequirementsProjection,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigUseEntry {
    pub path: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub required: bool,
    #[serde(rename = "sourcePath")]
    pub source_path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub provenance: Vec<ConfigRequirementProvenanceEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigActivation {
    #[serde(rename = "schemaVersion")]
    pub schema_version: &'static str,
    #[serde(rename = "hasPaths")]
    pub has_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigRequirementsProjection {
    pub own: Vec<ConfigRequirementEntry>,
    pub dependency: Vec<ConfigRequirementEntry>,
    pub effective: Vec<ConfigRequirementEntry>,
}

impl ConfigRequirementsProjection {
    fn empty() -> Self {
        Self {
            own: Vec::new(),
            dependency: Vec::new(),
            effective: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigRequirementEntry {
    pub scope: ConfigRequirementScopeEntry,
    pub path: String,
    pub access: ConfigRequirementAccessEntry,
    #[serde(rename = "sourcePath")]
    pub source_path: String,
    pub provenance: Vec<ConfigRequirementProvenanceEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRequirementScopeEntry {
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRequirementAccessEntry {
    pub kind: &'static str,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRequirementProvenanceEntry {
    pub source_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_span: Option<ConfigSourceSpanProjection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declaring_publication: Option<ConfigRequirementPublicationEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dependency_path: Vec<ConfigRequirementDependencyStepEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigRequirementPublicationEntry {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigRequirementDependencyStepEntry {
    pub id: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

pub fn project_config_projection(
    requirements: &ConfigRequirementsSeed,
) -> Result<ConfigProjection, ProjectionError> {
    Ok(config_projection_from_requirements(requirements))
}

fn config_projection_from_requirements(requirements: &ConfigRequirementsSeed) -> ConfigProjection {
    let legacy = requirements.legacy();
    let entries = config_shape_entries(legacy);
    let has_paths = config_activation_has_paths(legacy);
    ConfigProjection {
        shape: ConfigShape {
            schema_version: CONFIG_SHAPE_SCHEMA_VERSION.to_string(),
            entries,
        },
        uses: config_use_entries(legacy),
        activation: ConfigActivation {
            schema_version: CONFIG_ACTIVATION_SCHEMA_VERSION,
            has_paths,
        },
        requirements: ConfigRequirementsProjection {
            own: config_requirement_entries(requirements.own()),
            dependency: config_requirement_entries(requirements.dependency()),
            effective: config_requirement_entries(requirements.effective()),
        },
    }
}

fn config_shape_entries(requirements: &ConfigRequirementSetProjection) -> Vec<ConfigShapeEntry> {
    requirements
        .requirements()
        .iter()
        .filter_map(|requirement| {
            let (ty, required) = config_requirement_typed_access(requirement.access())?;
            Some(ConfigShapeEntry {
                path: requirement.path().to_string(),
                ty: config_shape_value_type(ty),
                required,
            })
        })
        .collect()
}

fn config_use_entries(requirements: &ConfigRequirementSetProjection) -> Vec<ConfigUseEntry> {
    requirements
        .requirements()
        .iter()
        .filter_map(|requirement| {
            let (ty, required) = config_requirement_typed_access(requirement.access())?;
            Some(ConfigUseEntry {
                path: requirement.path().to_string(),
                ty: ty.to_string(),
                required,
                source_path: config_requirement_origin(requirement).to_string(),
                provenance: config_use_provenance_entries(requirement),
            })
        })
        .collect()
}

fn config_activation_has_paths(requirements: &ConfigRequirementSetProjection) -> Vec<String> {
    let mut paths = requirements
        .requirements()
        .iter()
        .filter(|requirement| config_requirement_has_access(requirement.access()))
        .map(|requirement| requirement.path().to_string())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn config_requirement_entries(
    requirements: &ConfigRequirementSetProjection,
) -> Vec<ConfigRequirementEntry> {
    requirements
        .requirements()
        .iter()
        .map(|requirement| ConfigRequirementEntry {
            scope: config_requirement_scope_entry(requirement.scope()),
            path: requirement.path().to_string(),
            access: config_requirement_access_entry(requirement.access()),
            source_path: config_requirement_origin(requirement).to_string(),
            provenance: config_requirement_provenance_entries(requirement),
        })
        .collect()
}

fn config_shape_value_type(ty: &str) -> ConfigShapeValueType {
    ConfigShapeValueType::try_from(ty)
        .expect("config requirement type should have been validated before projection")
}

fn config_requirement_typed_access(
    access: &ConfigRequirementAccessProjection,
) -> Option<(&str, bool)> {
    match access {
        ConfigRequirementAccessProjection::Require { ty } => Some((ty, true)),
        ConfigRequirementAccessProjection::Optional { ty } => Some((ty, false)),
        ConfigRequirementAccessProjection::Has => None,
    }
}

fn config_requirement_has_access(access: &ConfigRequirementAccessProjection) -> bool {
    matches!(access, ConfigRequirementAccessProjection::Has)
}

fn config_requirement_origin(requirement: &ConfigRequirementProjection) -> &str {
    requirement
        .provenances()
        .first()
        .map(ConfigRequirementProvenanceProjection::source_path)
        .unwrap_or("")
}

fn config_requirement_scope_entry(
    scope: &ConfigRequirementScopeProjection,
) -> ConfigRequirementScopeEntry {
    match scope {
        ConfigRequirementScopeProjection::Service => ConfigRequirementScopeEntry {
            kind: "service",
            package_id: None,
        },
        ConfigRequirementScopeProjection::Package { package_id } => ConfigRequirementScopeEntry {
            kind: "package",
            package_id: Some(package_id.clone()),
        },
    }
}

fn config_requirement_access_entry(
    access: &ConfigRequirementAccessProjection,
) -> ConfigRequirementAccessEntry {
    match access {
        ConfigRequirementAccessProjection::Require { ty } => ConfigRequirementAccessEntry {
            kind: "require",
            ty: Some(ty.clone()),
            required: Some(true),
        },
        ConfigRequirementAccessProjection::Optional { ty } => ConfigRequirementAccessEntry {
            kind: "optional",
            ty: Some(ty.clone()),
            required: Some(false),
        },
        ConfigRequirementAccessProjection::Has => ConfigRequirementAccessEntry {
            kind: "has",
            ty: None,
            required: None,
        },
    }
}

fn config_requirement_provenance_entries(
    requirement: &ConfigRequirementProjection,
) -> Vec<ConfigRequirementProvenanceEntry> {
    requirement
        .provenances()
        .iter()
        .map(|provenance| ConfigRequirementProvenanceEntry {
            source_path: provenance.source_path().to_string(),
            source_span: provenance.source_span(),
            declaring_publication: provenance.declaring_publication().map(|publication| {
                ConfigRequirementPublicationEntry {
                    id: publication.id().to_string(),
                    version: publication.version().to_string(),
                }
            }),
            dependency_path: provenance
                .dependency_path()
                .iter()
                .map(|step| ConfigRequirementDependencyStepEntry {
                    id: step.id().to_string(),
                    version: step.version().to_string(),
                    alias: step.alias().map(ToString::to_string),
                })
                .collect(),
        })
        .collect()
}

fn config_use_provenance_entries(
    requirement: &ConfigRequirementProjection,
) -> Vec<ConfigRequirementProvenanceEntry> {
    config_requirement_provenance_entries(requirement)
        .into_iter()
        .filter(|provenance| {
            provenance.declaring_publication.is_some() || !provenance.dependency_path.is_empty()
        })
        .collect()
}
