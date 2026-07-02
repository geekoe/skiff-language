use serde::Serialize;
use serde_json::Value;
pub use skiff_artifact_identity::FILE_IR_IDENTITY_PREFIX;
use skiff_artifact_model::{
    ConstIr, ExecutableIr, ExternalRefTable, FileDeclarations, FileIrUnit, FileLinkTargets,
    SourceMapSource, SourceMapSpan, TypeDeclIr,
};
use skiff_compiler_core::json_utils::{canonical_json_value, sha256_hex};

pub fn file_ir_identity(unit: &FileIrUnit) -> String {
    let bytes = canonical_file_ir_identity_bytes(unit);
    format!("{FILE_IR_IDENTITY_PREFIX}:{}", sha256_hex(&bytes))
}

pub fn assign_file_ir_identity(unit: &mut FileIrUnit) -> String {
    let computed = file_ir_identity(unit);
    unit.file_ir_identity = computed.clone();
    computed
}

#[cfg(test)]
pub fn canonical_file_ir_identity_value(unit: &FileIrUnit) -> Value {
    file_ir_identity_value(unit)
}

fn canonical_file_ir_identity_bytes(unit: &FileIrUnit) -> Vec<u8> {
    serde_json::to_vec(&file_ir_identity_value(unit)).expect("File IR identity JSON must serialize")
}

fn file_ir_identity_value(unit: &FileIrUnit) -> Value {
    let value = serde_json::to_value(FileIrIdentityPayload::from_unit(unit))
        .expect("File IR identity payload must serialize to JSON value");
    canonical_json_value(&value)
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

#[cfg(test)]
mod tests {
    use super::canonical_file_ir_identity_value;
    use crate::file_ir::{FileIrUnit, SourceMapSource};

    #[test]
    fn identity_payload_omits_excluded_fields_by_type() {
        let mut unit = FileIrUnit::empty("internal.example", "source-ast-hash-a");
        unit.file_ir_identity = "stale-file-ir-identity".to_string();
        unit.source_map.sources.push(SourceMapSource {
            id: 0,
            path: "internal/example.skiff".to_string(),
            module_path: "internal.example".to_string(),
            source_ast_hash: Some("source-map-ast-hash-a".to_string()),
        });

        let value = canonical_file_ir_identity_value(&unit);

        assert!(value.get("fileIrIdentity").is_none());
        assert!(value.get("sourceAstHash").is_none());
        assert!(value
            .pointer("/sourceMap/sources/0/sourceAstHash")
            .is_none());
        assert_eq!(value["modulePath"], "internal.example");
        assert_eq!(
            value.pointer("/sourceMap/sources/0/path"),
            Some(&serde_json::json!("internal/example.skiff"))
        );
    }
}
