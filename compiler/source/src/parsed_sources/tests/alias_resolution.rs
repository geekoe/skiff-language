use std::path::{Path, PathBuf};

use super::*;

fn test_source(relative_path: &str, module_path: &str, text: &str) -> CompilerSourceFile {
    CompilerSourceFile::parse(
        PathBuf::from(relative_path),
        module_path.to_string(),
        false,
        false,
        text.to_string(),
        relative_path,
    )
    .expect("test source should parse")
}

#[test]
fn parsed_sources_are_constructed_with_alias_resolution() {
    let sources = vec![test_source(
        "api/user.skiff",
        "api.user",
        r#"
            type User {
              id: string
            }

            alias LocalUser = User
            alias PublicUser = LocalUser
        "#,
    )];

    let parsed_sources = parse_publication_sources(Path::new("/tmp/alias-resolution"), &sources)
        .expect("source aliases should resolve while building parsed sources");
    let parsed = &parsed_sources[0];

    assert!(
        std::ptr::eq(parsed.ast(), &sources[0].ast),
        "ParsedCompilerSource should borrow the AST from CompilerSourceFile"
    );
    assert_eq!(
        parsed.alias_targets().get("LocalUser").map(String::as_str),
        Some("User")
    );
    assert_eq!(
        parsed.alias_targets().get("PublicUser").map(String::as_str),
        Some("api.user.LocalUser")
    );
    assert_eq!(
        parsed
            .alias_targets()
            .get("api.user.PublicUser")
            .map(String::as_str),
        Some("api.user.LocalUser")
    );
}
