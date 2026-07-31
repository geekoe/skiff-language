use std::path::{Path, PathBuf};

use crate::{
    parsed_sources::parse_publication_sources, shared::id::SKIFF_STD_PUBLICATION_ID,
    source_graph::CompilerSourceFile,
};

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
fn official_std_private_modules_do_not_create_implicit_std_projection_roots() {
    let sources = vec![
        test_source(
            "log.skiff",
            "std.log",
            r#"
                    function leak() -> string {
                      return std.__private.helper
                    }
                "#,
        ),
        test_source(
            "helper.skiff",
            "std.__private.helper",
            r#"
                    function helper() -> string {
                      return "internal"
                    }
                "#,
        ),
    ];
    let parsed_sources =
        parse_publication_sources(Path::new("/tmp/std-private-projection"), &sources)
            .expect("sources should parse");

    let error = validate_package_sources(
        SKIFF_STD_PUBLICATION_ID,
        &[],
        Path::new("/tmp/std-private-projection"),
        &parsed_sources,
    )
    .expect_err("std.__private must not be an implicit std projection root")
    .to_string();

    assert!(
        error.contains("std.__private is not permitted as a std module root"),
        "unexpected error: {error}"
    );
}

#[test]
fn package_dependency_alias_roots_are_resolved_by_name_resolution_model() {
    let sources = vec![test_source(
        "main.skiff",
        "main",
        r#"
                function run() -> string {
                  return dep/http/get()
                }
            "#,
    )];
    let parsed_sources =
        parse_publication_sources(Path::new("/tmp/pkg-alias-resolution"), &sources)
            .expect("sources should parse");
    let mut dependency = PackageDependency::id("example.com/dep");
    dependency.alias = Some("dep".to_string());

    validate_package_sources(
        "example.com/pkg",
        &[dependency],
        Path::new("/tmp/pkg-alias-resolution"),
        &parsed_sources,
    )
    .expect("package dependency alias should be available through NameResolutionModel");
}

#[test]
fn official_std_public_modules_keep_internal_projection_roots() {
    let sources = vec![
        test_source(
            "log.skiff",
            "std.log",
            r#"
                    function record() -> string {
                      return std.telemetry
                    }
                "#,
        ),
        test_source(
            "telemetry.skiff",
            "std.telemetry",
            r#"
                    function emit() -> string {
                      return "ok"
                    }
                "#,
        ),
    ];
    let parsed_sources =
        parse_publication_sources(Path::new("/tmp/std-public-projection"), &sources)
            .expect("sources should parse");

    validate_package_sources(
        SKIFF_STD_PUBLICATION_ID,
        &[],
        Path::new("/tmp/std-public-projection"),
        &parsed_sources,
    )
    .expect("public std modules should keep implicit inter-module projections");
}
