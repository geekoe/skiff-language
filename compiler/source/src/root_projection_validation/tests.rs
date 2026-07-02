use super::*;

use crate::shared::parser::parse_source_with_bodies_tolerant;

#[test]
fn std_root_projection_allows_builtin_roots_without_import() {
    let ast = parse_source_with_bodies_tolerant(
        r#"
                function run() -> string {
                    return std.string
                }
            "#,
    )
    .expect("source should parse");
    let mut violations = Vec::new();

    collect_std_root_projection_violations("test.skiff", &ast, &mut violations);

    assert!(violations.is_empty());
}

#[test]
fn implicit_std_roots_keep_projection_validation_in_semantic_phase() {
    let ast = parse_source_with_bodies_tolerant(
        r#"
                function run() -> string {
                    return std.string
                }
            "#,
    )
    .expect("source should parse");
    let mut violations = Vec::new();

    collect_std_root_projection_violations_with_implicit_roots(
        "test.skiff",
        &ast,
        &[String::from("string")],
        &mut violations,
    );

    assert!(violations.is_empty());
}
