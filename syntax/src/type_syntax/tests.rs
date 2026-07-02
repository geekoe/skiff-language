use super::*;

#[test]
fn splits_only_at_top_level_delimiters() {
    let input = r#"Map<string, { tag: "a|b", nested: Array<"x,y"> }> | "done" | Result<{ value: string }, Error>"#;

    assert_eq!(
        split_top_level(input, '|'),
        vec![
            r#"Map<string, { tag: "a|b", nested: Array<"x,y"> }>"#,
            r#""done""#,
            r#"Result<{ value: string }, Error>"#,
        ]
    );
}

#[test]
fn splits_escaped_string_literals_without_leaking_delimiters() {
    let input = r#""a,\"|\",b", { value: "x,y" }, Array<"z,w">"#;

    assert_eq!(
        split_top_level(input, ','),
        vec![r#""a,\"|\",b""#, r#"{ value: "x,y" }"#, r#"Array<"z,w">"#]
    );
}

#[test]
fn extracts_generic_parts_and_arguments() {
    let parts =
        generic_parts(r#"Result<Array<string>, { tag: "ok", value: Map<string, number> }>"#)
            .expect("generic parts");

    assert_eq!(parts.root, "Result");
    assert_eq!(
        parts.args,
        vec![
            "Array<string>",
            r#"{ tag: "ok", value: Map<string, number> }"#,
        ]
    );
    assert_eq!(
        parts.inner,
        r#"Array<string>, { tag: "ok", value: Map<string, number> }"#
    );
}

#[test]
fn extracts_named_generic_inner_only_for_exact_root() {
    assert_eq!(
        generic_inner("Stream<{ value: Array<string> }>", "Stream"),
        Some("{ value: Array<string> }")
    );
    assert_eq!(generic_inner("MyStream<string>", "Stream"), None);
    assert_eq!(generic_inner("Stream<string>?", "Stream"), None);
}

#[test]
fn handles_alias_target_type_syntax() {
    let target = "Map<string, Json> | Array<UserId>";

    assert_eq!(
        split_top_level(target, '|'),
        vec!["Map<string, Json>", "Array<UserId>"]
    );

    let parts = generic_parts("Map<string, Json>").expect("generic parts");
    assert_eq!(parts.root, "Map");
    assert_eq!(parts.args, vec!["string", "Json"]);
}

#[test]
fn splits_function_type_arguments_at_type_level_only() {
    let parts = generic_parts("Result<fn(input: pkg.ModelRequest, ctx: Context) -> void, Error>")
        .expect("generic parts");

    assert_eq!(
        parts.args,
        vec!["fn(input: pkg.ModelRequest, ctx: Context) -> void", "Error",]
    );
}

#[test]
fn decodes_json_string_literals() {
    assert_eq!(
        string_literal(r#""database.collection\"quoted\"""#),
        Some("database.collection\"quoted\"".to_string())
    );
    assert_eq!(string_literal(r#""unterminated"#), None);
    assert_eq!(string_literal("string"), None);
}

#[test]
fn parses_record_type_fields_at_top_level_only() {
    let fields = parse_record_type_fields(
        r#"{ kind: "ok:a,b", value: Map<string, { nested: Array<"x:y"> }>, done: bool }"#,
    )
    .expect("record type fields");

    assert_eq!(
        fields,
        vec![
            RecordTypeFieldText {
                name: "kind",
                ty: r#""ok:a,b""#,
            },
            RecordTypeFieldText {
                name: "value",
                ty: r#"Map<string, { nested: Array<"x:y"> }>"#,
            },
            RecordTypeFieldText {
                name: "done",
                ty: "bool",
            },
        ]
    );
}

#[test]
fn parses_empty_record_type_fields() {
    assert_eq!(parse_record_type_fields("{}"), Ok(Vec::new()));
    assert_eq!(record_type_fields(" {  } "), Some(Vec::new()));
}

#[test]
fn reports_record_type_field_parse_boundaries() {
    assert_eq!(
        parse_record_type_fields("User"),
        Err(RecordTypeFieldParseError::NotRecordType)
    );
    assert_eq!(
        parse_record_type_fields("{ valid: string, missing_colon }"),
        Err(RecordTypeFieldParseError::InvalidField("missing_colon"))
    );
}
