use serde_json::{json, Value};
use skiff_runtime_linked_program::{ExecutableAddr, LinkedFileUnit, SourceMapDto};

pub fn program_source_context_frame(
    addr: &ExecutableAddr,
    file: &LinkedFileUnit,
    source_id: u64,
) -> Value {
    let mut frame = source_frame_for(&file.source_map, source_id);
    if let Some(object) = frame.as_object_mut() {
        object.insert("unit".to_string(), Value::String(addr.unit.to_string()));
        object.insert(
            "fileIrIdentity".to_string(),
            Value::String(file.file_ir_identity.clone()),
        );
    }
    frame
}

fn source_frame_for(source_map: &SourceMapDto, source_id: u64) -> Value {
    let Some(span) = source_map
        .spans
        .iter()
        .find(|span| span_id(span) == Some(source_id))
    else {
        return json!({ "sourceId": source_id });
    };
    let mut frame = json!({
        "sourceId": source_id,
        "span": span,
    });
    if let Some(source) = source_for_span(source_map, span) {
        frame["source"] = source.clone();
    }
    frame
}

fn span_id(span: &Value) -> Option<u64> {
    span.get("id").and_then(Value::as_u64)
}

fn source_for_span<'a>(source_map: &'a SourceMapDto, span: &Value) -> Option<&'a Value> {
    let source_id = span.get("source").and_then(Value::as_u64)?;
    source_map
        .sources
        .iter()
        .find(|source| source.get("id").and_then(Value::as_u64) == Some(source_id))
        .or_else(|| {
            usize::try_from(source_id)
                .ok()
                .and_then(|index| source_map.sources.get(index))
        })
}

#[cfg(all(test, any()))]
mod tests {
    use super::*;

    #[test]
    fn source_frame_prefers_source_id_lookup() {
        let span = json!({
            "id": 42,
            "source": 7,
            "kind": "CallExpression",
        });
        let source_map = SourceMapDto {
            format: None,
            sources: vec![
                json!({ "id": 99, "path": "wrong-index.skiff" }),
                json!({ "id": 7, "path": "by-id.skiff" }),
            ],
            spans: vec![span.clone()],
        };

        let frame = source_frame_for(&source_map, 42);

        assert_eq!(frame["sourceId"], 42);
        assert_eq!(frame["span"], span);
        assert_eq!(frame["source"]["path"], "by-id.skiff");
    }

    #[test]
    fn source_frame_falls_back_to_source_index() {
        let source_map = SourceMapDto {
            format: None,
            sources: vec![
                json!({ "path": "index-0.skiff" }),
                json!({ "path": "index-1.skiff" }),
            ],
            spans: vec![json!({
                "id": 12,
                "source": 1,
                "kind": "MemberExpression",
            })],
        };

        let frame = source_frame_for(&source_map, 12);

        assert_eq!(frame["source"]["path"], "index-1.skiff");
    }
}
