use std::path::PathBuf;

use super::*;

fn service_source(path: &str, module_path: &str) -> CompilerSourceFile {
    CompilerSourceFile::parse(
        PathBuf::from(path),
        module_path.to_string(),
        false,
        false,
        "type User {}\n".to_string(),
        path,
    )
    .unwrap()
}

#[test]
fn compiler_source_file_clone_shares_parsed_source_owner() {
    let source = service_source("api/user.skiff", "api.user");
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
