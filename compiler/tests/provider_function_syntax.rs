use skiff_syntax::parser::{parse_source, parse_source_with_bodies_tolerant};

#[test]
fn rejects_module_provider_capability_declaration() {
    let error = parse_source(
        r#"
            provider mongo

            function main() -> number {
                return 1
            }
        "#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("legacy provider syntax has been removed"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_provider_impl_function_without_body() {
    let error = parse_source_with_bodies_tolerant(
        r#"
            provider mongo

            impl MongoCollection<T> {
              provider function findMany(query: Query<T>) -> Array<T>
            }
        "#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("legacy provider syntax has been removed"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_provider_function_with_body() {
    let error = parse_source(
        r#"
            provider mongo

            provider function findMany(query: Query<User>) -> Array<User> {
              return []
            }
        "#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("legacy provider syntax has been removed"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_provider_function_without_module_provider_capability() {
    let error = parse_source(
        r#"
            provider function findMany(query: Query<User>) -> Array<User>
        "#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("legacy provider syntax has been removed"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_provider_function_in_interface_operation() {
    let error = parse_source(
        r#"
            provider mongo

            interface MongoOps {
              provider function findMany(query: Query<User>) -> Array<User>
            }
        "#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("legacy provider syntax has been removed"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_native_provider_function_modifier_combination() {
    let error = parse_source(
        r#"
            provider mongo
            native provider function findMany(query: Query<User>) -> Array<User>
        "#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("legacy provider syntax has been removed"),
        "unexpected error: {error}"
    );
}
