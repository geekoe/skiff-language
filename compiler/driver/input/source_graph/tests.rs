use std::path::PathBuf;

use skiff_compiler_source::source_graph::{ParsedSourceFile, SourceFileMeta};

#[test]
fn package_source_rejects_removed_provider_syntax() {
    let path = ".skiff-packages/skiff~run~~mongo/1.0.0/mongo.skiff";
    let error = ParsedSourceFile::parse(
        SourceFileMeta::package(
            "skiff.run/mongo",
            PathBuf::from("mongo.skiff"),
            "mongo".to_string(),
        ),
        "provider mongo\n\nexport type MongoTarget {}\n".to_string(),
        path,
    )
    .expect_err("removed provider syntax must fail while parsing package source");
    let message = error.to_string();

    assert!(message.contains(path), "unexpected error: {message}");
    assert!(
        message.contains("legacy provider syntax has been removed"),
        "unexpected error: {message}"
    );
}
