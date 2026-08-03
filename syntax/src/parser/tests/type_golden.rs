//! Phase 0 baseline: golden corpus for the type-text channel
//! (`TypeRef.name`) described in doc/implementation/parser-rs-refactor.md
//! section 2.5 item 4 and Phase 0 item 2.
//!
//! Each row is "input source → expected `TypeRef.name` bytes". The control
//! character cases pin the divergence between `quote_string_type`
//! (parser.rs:4044) and `serde_json` escaping used by
//! `type_expr::TypeExpr::to_type_string`.

use crate::parser::parse_source;
use crate::type_expr::TypeExpr;

fn alias_name(source: &str) -> String {
    let ast = parse_source(source)
        .unwrap_or_else(|error| panic!("type declaration should parse {source:?}: {error}"));
    assert_eq!(
        ast.types.len(),
        1,
        "expected one type declaration in {source:?}"
    );
    ast.types[0]
        .alias
        .as_ref()
        .unwrap_or_else(|| panic!("expected type alias target in {source:?}"))
        .name
        .clone()
}

fn param_type(source: &str, index: usize) -> String {
    let ast = parse_source(source).expect("function should parse");
    assert_eq!(
        ast.functions.len(),
        1,
        "expected one function in {source:?}"
    );
    ast.functions[0]
        .params
        .get(index)
        .unwrap_or_else(|| panic!("expected param {index} in {source:?}"))
        .ty
        .name
        .clone()
}

fn return_type(source: &str) -> String {
    let ast = parse_source(source).expect("function should parse");
    assert_eq!(
        ast.functions.len(),
        1,
        "expected one function in {source:?}"
    );
    ast.functions[0].return_type.name.clone()
}

#[test]
fn type_ref_name_golden_corpus() {
    let cases: &[(&str, &str)] = &[
        // primitives and generics
        ("type T = string", "string"),
        (
            // existing coverage: parses_generic_type_declaration_params / nominal aliases
            "type T = Array<UserId>",
            "Array<UserId>",
        ),
        (
            "type T = Map<string, Array<UserId>>",
            "Map<string, Array<UserId>>",
        ),
        (
            "type T = Array<Map<string, Array<User?>>>",
            "Array<Map<string, Array<User?>>>",
        ),
        (
            "type T = Array<fn(item: string) -> number>",
            "Array<fn(item: string) -> number>",
        ),
        // fn types
        (
            // existing coverage: parses_function_type_in_native_method_signature
            "type T = fn(item: T) -> R",
            "fn(item: T) -> R",
        ),
        (
            "type T = fn(input: User, ctx: Context) -> Result<string, Error>",
            "fn(input: User, ctx: Context) -> Result<string, Error>",
        ),
        (
            "type T = fn(cb: fn(x: number) -> void) -> void",
            "fn(cb: fn(x: number) -> void) -> void",
        ),
        // record types
        (
            "type T = { id: string, name: string }",
            "{ id: string, name: string }",
        ),
        ("type T = {}", "{}"),
        (
            // existing coverage: parses_type_discriminator_declaration
            r#"type T discriminator "kind" = { kind: "ok", value: string } | { kind: "err", message: string }"#,
            r#"{ kind: "ok", value: string } | { kind: "err", message: string }"#,
        ),
        // unions and nullable
        ("type T = A | B | C", "A | B | C"),
        ("type T = string?", "string?"),
        ("type T = Array<User?>", "Array<User?>"),
        ("type T = A? | B?", "A? | B?"),
        // `any I`
        (
            // existing coverage: parses_any_interface_type_annotations
            "type T = any ToolProvider",
            "any ToolProvider",
        ),
        ("type T = any Array<Tool>", "any Array<Tool>"),
        (
            // fixture shape: test-runner/fixtures/alias-return-catch-once/main.skiff
            "type T = any HostHttpDispatchObserver?",
            "any HostHttpDispatchObserver?",
        ),
        // `/` dependency paths
        (
            // existing coverage: parses_dependency_source_type_annotations_with_slash
            "type T = widget/internal.codec.Private",
            "widget/internal.codec.Private",
        ),
        (
            // `/` is only matched directly after the first type segment; the
            // remaining path is dot-qualified (`root/pkg.NS.Type`).
            "type T = root/pkg.NS.Type",
            "root/pkg.NS.Type",
        ),
        // string literal types
        ("type T = \"ok\"", "\"ok\""),
        (
            r#"type T = { kind: "ok", value: string }"#,
            r#"{ kind: "ok", value: string }"#,
        ),
        (
            // fixture shape: std/http.skiff HttpResponseStreamEvent
            r#"type T discriminator "tag" = { tag: "start", status: integer, headers: Array<HttpHeader> } | { tag: "chunk", value: bytes } | { tag: "end" }"#,
            r#"{ tag: "start", status: integer, headers: Array<HttpHeader> } | { tag: "chunk", value: bytes } | { tag: "end" }"#,
        ),
        (
            // nested record/fn/union shape from type_expr round-trip tests
            r#"type T discriminator "ok" = Result<{ ok: "yes", value: Array<User?> }, fn(err: Error) -> void> | { ok: "other" }"#,
            r#"Result<{ ok: "yes", value: Array<User?> }, fn(err: Error) -> void> | { ok: "other" }"#,
        ),
    ];

    for (source, expected) in cases {
        assert_eq!(&alias_name(source), expected, "for {source:?}");
    }
}

#[test]
fn type_ref_name_golden_function_signatures() {
    let cases: &[(&str, usize, &str)] = &[
        (
            // existing coverage: prelude/collection.skiff map signature
            r#"function run(callback: fn(value: string) -> string) -> void { return }"#,
            0,
            "fn(value: string) -> string",
        ),
        (
            // existing coverage: parses_any_interface_type_annotations
            r#"function useProvider(provider: any ToolProvider, mapper: fn(input: any ToolProvider) -> any ToolProvider) -> void { return }"#,
            0,
            "any ToolProvider",
        ),
        (
            r#"function useProvider(provider: any ToolProvider, mapper: fn(input: any ToolProvider) -> any ToolProvider) -> void { return }"#,
            1,
            "fn(input: any ToolProvider) -> any ToolProvider",
        ),
        (
            // existing coverage: parses_dependency_source_type_annotations_with_slash
            r#"function roundTrip(value: widget/internal.codec.Private) -> void { return }"#,
            0,
            "widget/internal.codec.Private",
        ),
        (
            r#"function retry(value: Array<Result<{ ok: bool }, string?>>) -> void { return }"#,
            0,
            "Array<Result<{ ok: bool }, string?>>",
        ),
    ];

    for (source, index, expected) in cases {
        assert_eq!(&param_type(source, *index), expected, "for {source:?}");
    }
}

#[test]
fn type_ref_name_golden_return_types() {
    let cases: &[(&str, &str)] = &[
        (r#"function run() -> void { return }"#, "void"),
        (
            r#"function run() -> Map<string, Array<User?>> { return null }"#,
            "Map<string, Array<User?>>",
        ),
    ];

    for (source, expected) in cases {
        assert_eq!(&return_type(source), expected, "for {source:?}");
    }
}

fn type_literal_name(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    let source = format!("type T = \"{escaped}\"");
    alias_name(&source)
}

#[test]
fn string_literal_type_quote_escaped_chars_agree_with_serde_json() {
    for value in ["a\"b", "a\\b", "a\nb", "a\rb", "a\tb"] {
        let name = type_literal_name(value);
        let serde_json_name = serde_json::to_string(value).unwrap();
        assert_eq!(
            name, serde_json_name,
            "quote_string_type must agree with serde_json for {value:?}"
        );
        assert_eq!(
            TypeExpr::parse(&name),
            TypeExpr::StringLiteral(value.to_string())
        );
        assert_eq!(TypeExpr::parse(&name).to_type_string(), serde_json_name);
    }
}

#[test]
fn string_literal_type_control_chars_lock_quote_vs_serde_json_divergence() {
    // quote_string_type (parser.rs:4044) only escapes `" \ \n \r \t`; other
    // control characters pass through raw into TypeRef.name, while
    // serde_json would escape them. The raw-control-character text form cannot
    // be decoded back by TypeExpr (serde_json rejects raw control characters
    // in JSON strings), so it stays opaque.
    for (value, raw_bytes) in [
        ("a\u{1}b", b"\"a\x01b\"".as_slice()),
        ("a\u{8}b", b"\"a\x08b\"".as_slice()),
        ("a\u{c}b", b"\"a\x0cb\"".as_slice()),
    ] {
        let name = type_literal_name(value);
        assert_eq!(name.as_bytes(), raw_bytes, "for {value:?}");
        assert_ne!(name, serde_json::to_string(value).unwrap(), "for {value:?}");
        assert!(
            !matches!(TypeExpr::parse(&name), TypeExpr::StringLiteral(_)),
            "raw control character text must not decode as a string literal"
        );
        assert_eq!(TypeExpr::parse(&name).to_type_string(), name);
    }
}

#[test]
fn string_literal_type_del_round_trips_through_type_expr() {
    let name = type_literal_name("a\u{7f}b");
    assert_eq!(name.as_bytes(), b"\"a\x7fb\"");
    // Raw DEL is legal inside a JSON string, so the text form still decodes.
    assert_eq!(
        TypeExpr::parse(&name),
        TypeExpr::StringLiteral("a\u{7f}b".to_string())
    );
    assert_eq!(TypeExpr::parse(&name).to_type_string(), name);
}
