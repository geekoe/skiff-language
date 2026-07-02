use std::collections::{BTreeMap, BTreeSet};

use crate::file_ir::{FileIrUnit, SourceMapSource, SourceMapSpan, SourcePosition, SourceSpanRef};
use serde_json::Number;
use skiff_compiler_core::json_utils::sha256_hex;
use skiff_syntax::{
    error::{CompileError, Result, SourceSpan},
    lexer::{lex, TokenKind},
};

pub(super) const SOURCE_ID: u64 = 0;
const SOURCE_AST_HASH_PREFIX: &str = "skiff-source-ast-v1:sha256";

pub(super) fn type_param_scope<'a>(
    inherited: impl IntoIterator<Item = &'a String>,
    local: impl IntoIterator<Item = &'a String>,
) -> BTreeSet<String> {
    inherited
        .into_iter()
        .chain(local)
        .cloned()
        .collect::<BTreeSet<_>>()
}

pub(super) fn type_index(type_indices: &BTreeMap<String, u32>, name: &str) -> Result<u32> {
    type_indices
        .get(name)
        .copied()
        .ok_or_else(|| CompileError::Semantic(format!("missing local type index for `{name}`")))
}

pub(super) fn push_source_span(
    spans: &mut Vec<SourceMapSpan>,
    next_span_id: &mut u64,
    kind: &str,
    name: &str,
    span: SourceSpan,
) {
    spans.push(SourceMapSpan {
        id: *next_span_id,
        source: SOURCE_ID,
        kind: kind.to_string(),
        name: Some(name.to_string()),
        span: source_span_ref(span),
    });
    *next_span_id += 1;
}

pub(super) fn push_source_map_source(
    unit: &mut FileIrUnit,
    path: String,
    module_path: &str,
    source_ast_hash: String,
) {
    unit.source_map.sources.push(SourceMapSource {
        id: SOURCE_ID,
        path,
        module_path: module_path.to_string(),
        source_ast_hash: Some(source_ast_hash),
    });
}

pub(super) fn source_ast_hash(source: &str) -> Result<String> {
    let mut bytes = Vec::new();
    for token in lex(source)? {
        match token.kind {
            TokenKind::Ident(value) => push_hash_token(&mut bytes, "ident", &value),
            TokenKind::Number(value) => {
                let number = Number::from_f64(value).ok_or_else(|| {
                    CompileError::Semantic(format!(
                        "invalid non-finite number literal `{value}` in source hash"
                    ))
                })?;
                push_hash_token(&mut bytes, "number", &number.to_string());
            }
            TokenKind::String(value) => push_hash_token(&mut bytes, "string", &value),
            TokenKind::Symbol(value) => push_hash_token(&mut bytes, "symbol", &value),
            TokenKind::Eof => push_hash_token(&mut bytes, "eof", ""),
        }
    }
    Ok(format!(
        "{SOURCE_AST_HASH_PREFIX}:{}",
        sha256_hex(bytes.as_slice())
    ))
}

fn push_hash_token(bytes: &mut Vec<u8>, kind: &str, value: &str) {
    bytes.extend_from_slice(kind.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(value.len().to_string().as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(0xff);
}

pub(super) fn source_span_ref(span: SourceSpan) -> SourceSpanRef {
    SourceSpanRef {
        source_id: SOURCE_ID,
        start: source_position(span.start.line, span.start.column, span.start.offset),
        end: source_position(span.end.line, span.end.column, span.end.offset),
    }
}

fn source_position(line: usize, column: usize, offset: usize) -> SourcePosition {
    SourcePosition {
        line: line as u32,
        column: column as u32,
        offset: Some(offset as u32),
    }
}

pub(super) fn symbol(module_path: &str, name: &str) -> String {
    format!("{module_path}.{name}")
}
