use super::*;

fn collect(source: &str) -> Vec<String> {
    let ast = crate::shared::parser::parse_source(source).unwrap();
    let mut violations = Vec::new();
    collect_user_function_type_violations("test.skiff", &ast, &mut violations);
    violations
}

#[test]
fn collects_type_field_and_alias_function_types() {
    let violations = collect(
        r#"
                type HandlerBox {
                    handler: fn(item: string) -> string
                }

                type Callback = fn() -> string
            "#,
    );

    assert_eq!(
            violations,
            vec![
                "test.skiff: callback function type fn(item: string) -> string is only allowed in standard_library/platform native API metadata",
                "test.skiff: callback function type fn() -> string is only allowed in standard_library/platform native API metadata",
            ]
        );
}

#[test]
fn collects_function_param_and_return_function_types() {
    let violations = collect(
        r#"
                function run(callback: fn(item: string) -> string) -> fn(done: bool) -> void {
                    return callback
                }
            "#,
    );

    assert_eq!(
            violations,
            vec![
                "test.skiff: callback function type fn(item: string) -> string is only allowed in standard_library/platform native API metadata",
                "test.skiff: callback function type fn(done: bool) -> void is only allowed in standard_library/platform native API metadata",
            ]
        );
}

#[test]
fn collects_local_annotation_and_generic_type_arg_function_types() {
    let violations = collect(
        r#"
                function run(factory: Factory) -> void {
                    final callback: fn(item: string) -> string = factory
                    factory<fn(value: string) -> string>()
                }
            "#,
    );

    assert_eq!(
            violations,
            vec![
                "test.skiff: callback function type fn(item: string) -> string is only allowed in standard_library/platform native API metadata",
                "test.skiff: callback function type fn(value: string) -> string is only allowed in standard_library/platform native API metadata",
            ]
        );
}
