use serde_json::Value;
use skiff_artifact_identity::{validate_file_ir_identity, validate_package_unit_identities};
use skiff_artifact_model::{
    FileIrRef, FileIrUnit, PackageUnit, ServiceUnit, FILE_IR_FORMAT_VERSION,
    FILE_IR_OPCODE_TABLE_VERSION, FILE_IR_SCHEMA_VERSION, PACKAGE_UNIT_SCHEMA_VERSION,
    SERVICE_UNIT_SCHEMA_VERSION,
};

use crate::paths::ArtifactRootRelativePath;

pub(super) fn deserialize_service_unit(
    value: Value,
    relative_path: &ArtifactRootRelativePath,
) -> anyhow::Result<ServiceUnit> {
    let unit: ServiceUnit = serde_json::from_value(value).map_err(|error| {
        anyhow::anyhow!("failed to deserialize canonical service unit: {error}")
    })?;
    if unit.schema_version != SERVICE_UNIT_SCHEMA_VERSION {
        anyhow::bail!(
            "service unit {} schemaVersion must be {}, got {}",
            relative_path.display(),
            SERVICE_UNIT_SCHEMA_VERSION,
            unit.schema_version
        );
    }
    Ok(unit)
}

pub(super) fn deserialize_file_ir_unit(
    value: Value,
    relative_path: &ArtifactRootRelativePath,
) -> anyhow::Result<FileIrUnit> {
    let unit: FileIrUnit = serde_json::from_value(value)
        .map_err(|error| anyhow::anyhow!("failed to deserialize File IR unit: {error}"))?;
    if unit.schema_version != FILE_IR_SCHEMA_VERSION {
        anyhow::bail!(
            "File IR unit {} schemaVersion must be {}, got {}",
            relative_path.display(),
            FILE_IR_SCHEMA_VERSION,
            unit.schema_version
        );
    }
    if unit.ir_format_version != FILE_IR_FORMAT_VERSION {
        anyhow::bail!(
            "File IR unit {} irFormatVersion must be {}, got {}",
            relative_path.display(),
            FILE_IR_FORMAT_VERSION,
            unit.ir_format_version
        );
    }
    if unit.opcode_table_version != FILE_IR_OPCODE_TABLE_VERSION {
        anyhow::bail!(
            "File IR unit {} opcodeTableVersion must be {}, got {}",
            relative_path.display(),
            FILE_IR_OPCODE_TABLE_VERSION,
            unit.opcode_table_version
        );
    }
    validate_file_ir_identity(&unit).map_err(|error| {
        anyhow::anyhow!(
            "File IR unit {} content identity validation failed: {error}",
            relative_path.display()
        )
    })?;
    Ok(unit)
}

pub(super) fn deserialize_package_unit(
    value: Value,
    relative_path: &ArtifactRootRelativePath,
) -> anyhow::Result<PackageUnit> {
    let unit: PackageUnit = serde_json::from_value(value).map_err(|error| {
        anyhow::anyhow!("failed to deserialize canonical package unit: {error}")
    })?;
    if unit.schema_version != PACKAGE_UNIT_SCHEMA_VERSION {
        anyhow::bail!(
            "package unit {} schemaVersion must be {}, got {}",
            relative_path.display(),
            PACKAGE_UNIT_SCHEMA_VERSION,
            unit.schema_version
        );
    }
    validate_package_unit_identities(&unit).map_err(|error| {
        anyhow::anyhow!(
            "package unit {} content identity validation failed: {error}",
            relative_path.display()
        )
    })?;
    Ok(unit)
}

pub(super) fn validate_loaded_file_ref(
    unit: &FileIrUnit,
    file_ref: &FileIrRef,
    relative_path: &ArtifactRootRelativePath,
    label: &str,
) -> anyhow::Result<()> {
    if unit.file_ir_identity != file_ref.file_ir_identity {
        anyhow::bail!(
            "{label} {} loaded from {} has fileIrIdentity {}",
            file_ref.file_ir_identity,
            relative_path.display(),
            unit.file_ir_identity
        );
    }
    if unit.module_path != file_ref.module_path {
        anyhow::bail!(
            "{label} {} loaded from {} modulePath mismatch: expected {}, got {}",
            file_ref.file_ir_identity,
            relative_path.display(),
            file_ref.module_path,
            unit.module_path
        );
    }
    Ok(())
}
