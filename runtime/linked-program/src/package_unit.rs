use std::collections::BTreeMap;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use skiff_artifact_model::{ConfigShape, ConfigShapeEntry, ConfigShapeValueType, MetadataValue};

use super::{
    addr::{ExecutableIndex, FileAddr, TypeIndex},
    LinkedTypeRef,
};

pub type PackageUnit = skiff_artifact_model::PackageUnit;
pub type PackageBuildIdentity = String;
pub type PackageAbiIdentity = String;
pub type PackageExportIndex = skiff_artifact_model::PackageExportIndex;
pub type PackageImplementationLinks = skiff_artifact_model::PackageImplementationLinks;
pub type TypeExport = skiff_artifact_model::TypeExport;
pub type ExecutableExport = skiff_artifact_model::ExecutableExport;
pub type ConstExport = skiff_artifact_model::ConstExport;
pub type PackageDependencyConstraint = skiff_artifact_model::PackageDependencyConstraint;
pub type ConfigAndEffectMetadata = skiff_artifact_model::ConfigAndEffectMetadata;

pub fn package_config_shape(package: &PackageUnit) -> anyhow::Result<ConfigShape> {
    config_and_effect_metadata_shape(&package.config_and_effect_metadata)
}

pub fn config_and_effect_metadata_shape(
    metadata: &ConfigAndEffectMetadata,
) -> anyhow::Result<ConfigShape> {
    metadata
        .config
        .get("shape")
        .map(metadata_value_to_config_shape)
        .transpose()
        .map(|shape| shape.unwrap_or_else(ConfigShape::empty))
}

fn metadata_value_to_config_shape(value: &MetadataValue) -> anyhow::Result<ConfigShape> {
    let MetadataValue::Object(object) = value else {
        bail!("package config metadata shape must be an object");
    };
    let schema_version = metadata_string_field(object, "schemaVersion")?.to_string();
    let MetadataValue::Array(entries) = object
        .get("entries")
        .ok_or_else(|| anyhow::anyhow!("package config metadata shape requires entries"))?
    else {
        bail!("package config metadata shape entries must be an array");
    };
    let shape = ConfigShape {
        schema_version,
        entries: entries
            .iter()
            .enumerate()
            .map(|(index, entry)| metadata_value_to_config_shape_entry(entry, index))
            .collect::<anyhow::Result<Vec<_>>>()?,
    };
    shape.validate_schema_version()?;
    Ok(shape)
}

fn metadata_value_to_config_shape_entry(
    value: &MetadataValue,
    index: usize,
) -> anyhow::Result<ConfigShapeEntry> {
    let MetadataValue::Object(object) = value else {
        bail!("package config metadata shape entries[{index}] must be an object");
    };
    let path = metadata_string_field(object, "path")?.to_string();
    let ty = ConfigShapeValueType::try_from(metadata_string_field(object, "type")?)?;
    let required = metadata_bool_field(object, "required")?;
    Ok(ConfigShapeEntry { path, ty, required })
}

fn metadata_string_field<'a>(
    object: &'a BTreeMap<String, MetadataValue>,
    field: &str,
) -> anyhow::Result<&'a str> {
    match object.get(field) {
        Some(MetadataValue::String(value)) => Ok(value),
        Some(_) => bail!("package config metadata shape {field} must be a string"),
        None => bail!("package config metadata shape requires {field}"),
    }
}

fn metadata_bool_field(
    object: &BTreeMap<String, MetadataValue>,
    field: &str,
) -> anyhow::Result<bool> {
    match object.get(field) {
        Some(MetadataValue::Bool(value)) => Ok(*value),
        Some(_) => bail!("package config metadata shape {field} must be a bool"),
        None => bail!("package config metadata shape requires {field}"),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedPackageExportIndex {
    #[serde(default)]
    pub types: BTreeMap<String, LinkedTypeExport>,
    #[serde(default)]
    pub constants: BTreeMap<String, LinkedConstExport>,
    #[serde(default)]
    pub functions: BTreeMap<String, LinkedExecutableExport>,
    #[serde(default)]
    pub impl_methods: BTreeMap<String, LinkedExecutableExport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedTypeExport {
    #[serde(default)]
    pub symbol: String,
    pub file: FileAddr,
    pub type_index: TypeIndex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedExecutableExport {
    #[serde(default)]
    pub symbol: String,
    pub file: FileAddr,
    #[serde(rename = "executableIndex")]
    pub executable: ExecutableIndex,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedConstExport {
    #[serde(default)]
    pub symbol: String,
    pub file: FileAddr,
    #[serde(rename = "constIndex")]
    pub const_index: usize,
    pub ty: LinkedTypeRef,
}

impl LinkedPackageExportIndex {
    pub fn from_canonical(
        exports: &skiff_artifact_model::PackageImplementationLinks,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            types: exports
                .types
                .iter()
                .map(|(symbol, export)| (symbol.clone(), LinkedTypeExport::from(export)))
                .collect(),
            constants: exports
                .constants
                .iter()
                .map(|(symbol, export)| Ok((symbol.clone(), LinkedConstExport::try_from(export)?)))
                .collect::<Result<BTreeMap<_, _>, anyhow::Error>>()?,
            functions: exports
                .functions
                .iter()
                .map(|(symbol, export)| (symbol.clone(), LinkedExecutableExport::from(export)))
                .collect(),
            impl_methods: exports
                .impl_methods
                .iter()
                .map(|(symbol, export)| (symbol.clone(), LinkedExecutableExport::from(export)))
                .collect(),
        })
    }
}

impl From<&skiff_artifact_model::TypeExport> for LinkedTypeExport {
    fn from(export: &skiff_artifact_model::TypeExport) -> Self {
        Self {
            symbol: export.symbol.clone(),
            file: FileAddr::file_ir_identity(export.file.file_ir_identity.as_str()),
            type_index: export.type_index as usize,
        }
    }
}

impl TryFrom<&skiff_artifact_model::ConstExport> for LinkedConstExport {
    type Error = anyhow::Error;

    fn try_from(export: &skiff_artifact_model::ConstExport) -> Result<Self, Self::Error> {
        let ty = serde_json::to_value(&export.ty)
            .context("failed to convert const export type to runtime JSON")?;
        Ok(Self {
            symbol: export.symbol.clone(),
            file: FileAddr::file_ir_identity(export.file.file_ir_identity.as_str()),
            const_index: export.const_index as usize,
            ty: serde_json::from_value(ty)
                .context("failed to convert const export type to runtime LinkedTypeRef")?,
        })
    }
}

impl From<&skiff_artifact_model::ExecutableExport> for LinkedExecutableExport {
    fn from(export: &skiff_artifact_model::ExecutableExport) -> Self {
        Self {
            symbol: export.symbol.clone(),
            file: FileAddr::file_ir_identity(export.file.file_ir_identity.as_str()),
            executable: export.executable_index as usize,
        }
    }
}
