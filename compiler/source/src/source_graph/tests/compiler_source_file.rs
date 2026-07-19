use super::*;

#[test]
fn compiler_source_file_clone_shares_parsed_source_owner() {
    let source = CompilerSourceFile::parse(
        "api/user.skiff".into(),
        "api.user".to_string(),
        false,
        false,
        "type User {}\n".to_string(),
        "api/user.skiff",
    )
    .unwrap();
    let cloned = source.clone();

    assert!(
        std::sync::Arc::ptr_eq(&source.parsed, &cloned.parsed),
        "CompilerSourceFile clone should share the parsed source allocation"
    );
    assert!(
        std::ptr::eq(&source.ast, &cloned.ast),
        "shared parsed source should expose the same AST allocation"
    );
}
