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
