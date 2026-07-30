use serde::Serialize;
use serde_json::Value;
use skiff_artifact_model::{
    validate_file_ir_db_indexes, validate_file_ir_package_calls, validate_file_ir_service_calls,
    ActorDeclarationIr, ConstIr, ExecutableIr, ExternalRefTable, FileDeclarations, FileIrUnit,
    FileLinkTargets, SourceMapSource, SourceMapSpan, TypeDeclIr, FILE_IR_FORMAT_VERSION,
    FILE_IR_OPCODE_TABLE_VERSION, FILE_IR_SCHEMA_VERSION,
};

use crate::framing::{framed_identity, sha256_hex};
use crate::{ArtifactIdentityError, Result, FILE_IR_IDENTITY_PREFIX};
use skiff_canonical_json::canonical_json_value;

pub fn file_ir_hash(unit: &FileIrUnit) -> Result<String> {
    Ok(sha256_hex(&canonical_file_ir_identity_bytes(unit)?))
}

pub fn file_ir_identity(unit: &FileIrUnit) -> Result<String> {
    Ok(framed_identity(
        FILE_IR_IDENTITY_PREFIX,
        &file_ir_hash(unit)?,
    ))
}

pub fn canonical_file_ir_identity_value(unit: &FileIrUnit) -> Result<Value> {
    validate_file_ir_service_calls(unit)?;
    validate_file_ir_package_calls(unit)?;
    validate_file_ir_db_indexes(unit)?;
    let value = serde_json::to_value(FileIrIdentityPayload::from_unit(unit))
        .map_err(ArtifactIdentityError::SerializeFileIrIdentity)?;
    Ok(canonical_json_value(&value))
}

pub fn canonical_file_ir_identity_bytes(unit: &FileIrUnit) -> Result<Vec<u8>> {
    let value = canonical_file_ir_identity_value(unit)?;
    serde_json::to_vec(&value).map_err(ArtifactIdentityError::SerializeFileIrIdentity)
}

pub fn validate_file_ir_identity(unit: &FileIrUnit) -> Result<()> {
    validate_file_ir_generation(unit)?;
    let computed = file_ir_identity(unit)?;
    if unit.file_ir_identity != computed {
        return Err(ArtifactIdentityError::FileIrIdentityMismatch {
            declared: unit.file_ir_identity.clone(),
            computed,
        });
    }
    Ok(())
}

fn validate_file_ir_generation(unit: &FileIrUnit) -> Result<()> {
    for (field, expected, actual) in [
        (
            "schemaVersion",
            FILE_IR_SCHEMA_VERSION,
            unit.schema_version.as_str(),
        ),
        (
            "irFormatVersion",
            FILE_IR_FORMAT_VERSION,
            unit.ir_format_version.as_str(),
        ),
        (
            "opcodeTableVersion",
            FILE_IR_OPCODE_TABLE_VERSION,
            unit.opcode_table_version.as_str(),
        ),
    ] {
        if actual != expected {
            return Err(ArtifactIdentityError::FileIrGenerationMismatch {
                field,
                expected,
                actual: actual.to_string(),
            });
        }
    }
    Ok(())
}

pub fn assign_file_ir_identity(unit: &mut FileIrUnit) -> Result<String> {
    validate_file_ir_generation(unit)?;
    let computed = file_ir_identity(unit)?;
    unit.file_ir_identity = computed.clone();
    Ok(computed)
}

pub fn file_ir_with_identity(mut unit: FileIrUnit) -> Result<FileIrUnit> {
    assign_file_ir_identity(&mut unit)?;
    Ok(unit)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileIrIdentityPayload<'a> {
    schema_version: &'a str,
    module_path: &'a str,
    ir_format_version: &'a str,
    opcode_table_version: &'a str,
    #[serde(skip_serializing_if = "is_zero_u32")]
    required_receiver_builtin_capability_version: u32,
    source_map: SourceMapIdentityPayload<'a>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    actor_declarations: &'a Vec<ActorDeclarationIr>,
    declarations: &'a FileDeclarations,
    link_targets: &'a FileLinkTargets,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    type_table: &'a Vec<TypeDeclIr>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    constants: &'a Vec<ConstIr>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    executables: &'a Vec<ExecutableIr>,
    external_refs: &'a ExternalRefTable,
}

impl<'a> FileIrIdentityPayload<'a> {
    fn from_unit(unit: &'a FileIrUnit) -> Self {
        Self {
            schema_version: &unit.schema_version,
            module_path: &unit.module_path,
            ir_format_version: &unit.ir_format_version,
            opcode_table_version: &unit.opcode_table_version,
            required_receiver_builtin_capability_version: unit
                .required_receiver_builtin_capability_version,
            source_map: SourceMapIdentityPayload::from_unit(unit),
            actor_declarations: &unit.actor_declarations,
            declarations: &unit.declarations,
            link_targets: &unit.link_targets,
            type_table: &unit.type_table,
            constants: &unit.constants,
            executables: &unit.executables,
            external_refs: &unit.external_refs,
        }
    }
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceMapIdentityPayload<'a> {
    format: &'a str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    sources: Vec<SourceMapSourceIdentityPayload<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    spans: &'a Vec<SourceMapSpan>,
}

impl<'a> SourceMapIdentityPayload<'a> {
    fn from_unit(unit: &'a FileIrUnit) -> Self {
        Self {
            format: &unit.source_map.format,
            sources: unit
                .source_map
                .sources
                .iter()
                .map(SourceMapSourceIdentityPayload::from_source)
                .collect(),
            spans: &unit.source_map.spans,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceMapSourceIdentityPayload<'a> {
    id: u64,
    path: &'a str,
    module_path: &'a str,
}

impl<'a> SourceMapSourceIdentityPayload<'a> {
    fn from_source(source: &'a SourceMapSource) -> Self {
        Self {
            id: source.id,
            path: &source.path,
            module_path: &source.module_path,
        }
    }
}
