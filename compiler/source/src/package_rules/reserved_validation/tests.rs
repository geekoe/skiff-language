use super::*;
use crate::shared::parser::parse_source_with_bodies_tolerant;

#[test]
fn collects_local_and_pattern_reserved_root_bindings() {
    let ast = parse_source_with_bodies_tolerant(
        r#"
                function demo(value: Payload, items: Payload) -> string {
                    let std = value
                    for connect in items {
                        match value {
                            Payload { root, nested: Payload { config } } => {
                                return "ok"
                            }
                        }
                    }
                    return "done"
                }
            "#,
    )
    .unwrap();
    let mut violations = Vec::new();

    validate_package_reserved_roots_in_block(
        "package/api.skiff",
        &ast.functions[0].body,
        &mut violations,
    );

    assert_eq!(
        violations,
        vec![
            "package/api.skiff: local binding std uses reserved prelude name",
            "package/api.skiff: local binding connect uses reserved prelude name",
            "package/api.skiff: pattern binding root uses reserved prelude name",
            "package/api.skiff: pattern binding config uses reserved prelude name",
        ]
    );
}
