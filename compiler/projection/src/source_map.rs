use serde::Serialize;
use skiff_artifact_model::{FileIrUnit, SourcePosition};

use crate::error::ProjectionError;
use skiff_compiler_core::json_utils::sha256_hex;

/// Publication source map: the `{format, sources, spans}` object embedded in
/// service/package assemblies, mapping File IR span ids back to source
/// positions. Field order matches the former `json!`.
#[derive(Debug, Clone, Serialize)]
pub struct PublicationSourceMap {
    pub format: &'static str,
    pub sources: Vec<SourceMapSourceEntry>,
    pub spans: Vec<SourceMapSpanEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMapSourceEntry {
    id: u64,
    path: String,
    module_path: String,
    file_ir_identity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_ast_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMapSpanEntry {
    id: u64,
    source: u64,
    kind: String,
    start: SourcePosition,
    end: SourcePosition,
    module_path: String,
    file_ir_identity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

pub fn publication_source_map_from_file_ir_units(
    file_ir_units: &[FileIrUnit],
) -> Result<PublicationSourceMap, ProjectionError> {
    let mut sources = Vec::new();
    let mut spans = Vec::new();
    for unit in file_ir_units {
        let source_map = source_file_map_from_file_ir_unit(unit)?;
        sources.push(source_map.source);
        spans.extend(source_map.spans);
    }
    Ok(PublicationSourceMap {
        format: "skiff-source-map-v1",
        sources,
        spans,
    })
}

struct FileIrUnitSourceMap {
    source: SourceMapSourceEntry,
    spans: Vec<SourceMapSpanEntry>,
}

fn source_file_map_from_file_ir_unit(
    unit: &FileIrUnit,
) -> Result<FileIrUnitSourceMap, ProjectionError> {
    let source = file_ir_unit_source(unit)?;
    let source_id = publication_file_source_id(source.path.as_str(), source.module_path.as_str());
    let source_value = SourceMapSourceEntry {
        id: source_id,
        path: source.path.clone(),
        module_path: source.module_path.clone(),
        file_ir_identity: unit.file_ir_identity.clone(),
        source_ast_hash: source.source_ast_hash.clone(),
    };
    let spans = unit
        .source_map
        .spans
        .iter()
        .map(|span| SourceMapSpanEntry {
            id: span.id,
            source: source_id,
            kind: span.kind.clone(),
            start: span.span.start.clone(),
            end: span.span.end.clone(),
            module_path: unit.module_path.clone(),
            file_ir_identity: unit.file_ir_identity.clone(),
            name: span.name.clone(),
        })
        .collect();
    Ok(FileIrUnitSourceMap {
        source: source_value,
        spans,
    })
}

struct FileIrSourceMetadata {
    path: String,
    module_path: String,
    source_ast_hash: Option<String>,
}

fn file_ir_unit_source(unit: &FileIrUnit) -> Result<FileIrSourceMetadata, ProjectionError> {
    unit.source_map.sources.first().map_or_else(
        || {
            Err(ProjectionError::ContractValidation {
                message: format!(
                    "compiled File IR unit {} has no source map source",
                    unit.file_ir_identity
                ),
            })
        },
        |source| {
            Ok(FileIrSourceMetadata {
                path: source.path.clone(),
                module_path: source.module_path.clone(),
                source_ast_hash: source.source_ast_hash.clone(),
            })
        },
    )
}

fn publication_file_source_id(source_path: &str, module_path: &str) -> u64 {
    let hash = sha256_hex(format!("{source_path}\0{module_path}").as_bytes());
    u64::from_str_radix(&hash[..13], 16).expect("sha256 hex prefix should parse")
}
