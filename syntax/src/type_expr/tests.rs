use super::*;

fn named_roots(ty: &TypeExpr) -> Vec<String> {
    let mut roots = Vec::new();
    ty.for_each_named(|name| roots.push(name.to_string()));
    roots
}

#[test]
fn type_expr_parses_nullable() {
    assert_eq!(
        TypeExpr::parse("User?"),
        TypeExpr::Nullable(Box::new(TypeExpr::Named {
            name: "User".to_string(),
            args: Vec::new()
        }))
    );
}

#[test]
fn type_expr_parses_union() {
    let ty = TypeExpr::parse("A | B? | \"ok\"");
    assert_eq!(
        ty,
        TypeExpr::Union(vec![
            TypeExpr::Named {
                name: "A".to_string(),
                args: Vec::new(),
            },
            TypeExpr::Nullable(Box::new(TypeExpr::Named {
                name: "B".to_string(),
                args: Vec::new(),
            })),
            TypeExpr::StringLiteral("ok".to_string()),
        ])
    );
    assert_eq!(named_roots(&ty), vec!["A", "B"]);
}

#[test]
fn type_expr_parses_generic() {
    let ty = TypeExpr::parse("Map<string, Array<User?>>");
    assert_eq!(ty.to_type_string(), "Map<string, Array<User?>>");
    assert_eq!(named_roots(&ty), vec!["Map", "string", "Array", "User"]);
}

#[test]
fn type_expr_parses_any_interface() {
    let ty = TypeExpr::parse("Array<any tools.ToolProvider<Context>?>");

    assert_eq!(
        ty.to_type_string(),
        "Array<any tools.ToolProvider<Context>?>"
    );
    assert_eq!(
        named_roots(&ty),
        vec!["Array", "tools.ToolProvider", "Context"]
    );
    assert!(!ty.contains_function_type());
}

#[test]
fn type_expr_any_interface_can_contain_function_types_for_boundary_walkers() {
    let ty = TypeExpr::parse("any fn(input: Payload) -> Result");

    assert!(ty.contains_function_type());
    let mut functions = Vec::new();
    ty.for_each_function_type(|function_type| functions.push(function_type.to_type_string()));
    assert_eq!(functions, vec!["fn(input: Payload) -> Result"]);
}

#[test]
fn type_expr_parses_record() {
    let ty = TypeExpr::parse("{ a: string, b: Array<User> }");
    assert_eq!(ty.to_type_string(), "{ a: string, b: Array<User> }");
    assert_eq!(named_roots(&ty), vec!["string", "Array", "User"]);
}

#[test]
fn type_expr_parses_record_fields_with_nested_type_boundaries() {
    let raw = r#"{ kind: "ok:a,b", meta: { id: string, tags: Array<"x,y"> }, handler: fn(input: { value: Map<string, User> }) -> Result<{ ok: bool }, Error> }"#;

    let ty = TypeExpr::parse(raw);

    assert_eq!(ty.to_type_string(), raw);
    assert_eq!(
        named_roots(&ty),
        vec!["string", "Array", "Map", "string", "User", "Result", "bool", "Error",]
    );
}

#[test]
fn type_expr_parses_function_type() {
    let ty = TypeExpr::parse("fn(input: User, ctx: Context) -> Result<string, Error>");
    assert!(ty.contains_function_type());
    assert_eq!(
        ty.to_type_string(),
        "fn(input: User, ctx: Context) -> Result<string, Error>"
    );
    assert_eq!(
        named_roots(&ty),
        vec!["User", "Context", "Result", "string", "Error"]
    );
}

#[test]
fn type_expr_named_roots_include_function_params_and_return() {
    let ty = TypeExpr::parse(
        "Array<fn(input: std.http.Request) -> Result<std.http.Response, Error>> | User",
    );
    let mut roots = Vec::new();
    ty.for_each_named(|name| roots.push(name.to_string()));
    assert_eq!(
        roots,
        vec![
            "Array",
            "std.http.Request",
            "Result",
            "std.http.Response",
            "Error",
            "User",
        ]
    );
}

#[test]
fn type_expr_named_roots_outside_function_types_skip_function_params_and_return() {
    let ty = TypeExpr::parse(
        "Array<fn(input: std.http.Request) -> Result<std.http.Response, Error>> | User",
    );
    let mut roots = Vec::new();
    ty.for_each_named_outside_function_types(|name| roots.push(name.to_string()));
    assert_eq!(roots, vec!["Array", "User"]);
}

#[test]
fn type_expr_function_type_iteration_reports_outermost_function_only() {
    let ty =
        TypeExpr::parse("Array<fn(input: fn(event: Event) -> void) -> fn(result: Result) -> void>");
    let mut function_types = Vec::new();
    ty.for_each_function_type(|function_type| {
        function_types.push(function_type.to_type_string());
    });
    assert_eq!(
        function_types,
        vec!["fn(input: fn(event: Event) -> void) -> fn(result: Result) -> void"]
    );
}

#[test]
fn type_expr_parses_escaped_string_literal() {
    let ty = TypeExpr::parse(r#""a \"quoted\" value""#);
    assert_eq!(
        ty,
        TypeExpr::StringLiteral("a \"quoted\" value".to_string())
    );
    assert_eq!(ty.to_type_string(), r#""a \"quoted\" value""#);
    assert!(named_roots(&ty).is_empty());
}

#[test]
fn type_expr_round_trips_nested_formatting() {
    let raw = r#"Result<{ ok: "yes", value: Array<User?> }, fn(err: Error) -> void> | {}"#;
    assert_eq!(TypeExpr::parse(raw).to_type_string(), raw);
}

#[test]
fn type_expr_map_named_types_renames_nested_named_roots() {
    let ty = TypeExpr::parse(r#"Map<string, Array<{ kind: "x", inner: User? }>>"#);
    let mapped = ty.map_named_types(|name| format!("{name}_mapped"));
    assert_eq!(
        mapped.to_type_string(),
        r#"Map_mapped<string_mapped, Array_mapped<{ kind: "x", inner: User_mapped? }>>"#
    );
}

#[test]
fn type_expr_map_named_types_preserves_structure() {
    let ty = TypeExpr::parse("Array<Result<User?, std.http.Response>>");
    let mapped = ty.map_named_types(|name| format!("{name}Ref"));
    assert_eq!(
        mapped.to_type_string(),
        "ArrayRef<ResultRef<UserRef?, std.http.ResponseRef>>"
    );
}
