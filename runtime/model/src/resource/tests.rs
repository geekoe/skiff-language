use super::*;

#[test]
fn publication_resource_path_accepts_canonical_logical_paths() {
    let path = PublicationResourcePath::parse("prompts/system.md").unwrap();

    assert_eq!(path.as_str(), "prompts/system.md");
}

#[test]
fn publication_resource_path_rejects_non_canonical_paths() {
    for path in [
        "", "/a", "./a", "a/./b", "a//b", "a\\b", "../a", "a/..", "a/",
    ] {
        assert!(
            PublicationResourcePath::parse(path).is_err(),
            "{path:?} should be invalid"
        );
    }
}
