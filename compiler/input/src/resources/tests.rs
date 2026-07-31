use super::*;

#[test]
fn publication_resource_path_validation_rejects_unsafe_forms() {
    for (path, expected) in [
        ("", "empty"),
        ("./a", ". segments"),
        ("a/./b", ". segments"),
        ("a//b", "empty segments"),
        ("a\\b", "backslashes"),
        ("a/../b", ".. segments"),
        ("a/", "trailing"),
        ("/a", "relative"),
        ("C:/a", "relative"),
        ("*.txt", "glob"),
        (".env", "hidden"),
        ("dir/.env", "hidden"),
        ("main.skiff", ".skiff"),
        ("package.yml", "control"),
        ("http.yml", "control"),
        ("websocket.yml", "control"),
        ("service.prod.yml", "control"),
        ("config.prod.yml", "control"),
        ("prod.secret.yml", "control"),
    ] {
        let error =
            validate_publication_resource_logical_path(path).expect_err("path should be rejected");
        assert!(
            error.contains(expected),
            "expected {path:?} error to contain {expected:?}, got {error:?}"
        );
    }
}

#[test]
fn publication_resource_reader_reads_hash_and_metadata() {
    let root = TestDir::new("resource-reader");
    root.write("prompts/system.md", b"hello");

    let resources = read_publication_resources(
        root.path(),
        &[PublicationResourceSpec::new("prompts/system.md")],
    )
    .expect("resource should read");

    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].path, "prompts/system.md");
    assert_eq!(resources[0].byte_len, 5);
    assert_eq!(
        resources[0].sha256,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
    assert_eq!(resources[0].content_type, None);
    assert!(resources[0].absolute_path.is_absolute());
}

#[test]
fn publication_resource_reader_rejects_duplicate_and_count_limit_before_filesystem() {
    let root = TestDir::new("resource-duplicates");
    let duplicate = read_publication_resources(
        root.path(),
        &[
            PublicationResourceSpec::new("prompts/system.md"),
            PublicationResourceSpec::new("prompts/system.md"),
        ],
    )
    .unwrap_err()
    .to_string();
    assert!(
        duplicate.contains("declared more than once"),
        "unexpected error: {duplicate}"
    );

    let too_many = (0..=MAX_PUBLICATION_RESOURCES)
        .map(|index| PublicationResourceSpec::new(format!("r{index}.txt")))
        .collect::<Vec<_>>();
    let error = read_publication_resources(root.path(), &too_many)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("at most 256 entries"),
        "unexpected error: {error}"
    );
}

#[test]
fn publication_resource_reader_rejects_missing_non_regular_and_case_mismatch() {
    let root = TestDir::new("resource-file-shape");
    root.write("Prompts/System.md", b"hello");
    std::fs::create_dir_all(root.path().join("catalog")).unwrap();

    let missing = read_publication_resources(
        root.path(),
        &[PublicationResourceSpec::new("prompts/System.md")],
    )
    .unwrap_err()
    .to_string();
    assert!(
        missing.contains("exact case"),
        "unexpected error: {missing}"
    );

    let directory =
        read_publication_resources(root.path(), &[PublicationResourceSpec::new("catalog")])
            .unwrap_err()
            .to_string();
    assert!(
        directory.contains("not a regular file"),
        "unexpected error: {directory}"
    );
}

#[cfg(unix)]
#[test]
fn publication_resource_reader_rejects_symlink() {
    let root = TestDir::new("resource-symlink");
    root.write("target.txt", b"target");
    std::os::unix::fs::symlink(root.path().join("target.txt"), root.path().join("link.txt"))
        .unwrap();

    let error =
        read_publication_resources(root.path(), &[PublicationResourceSpec::new("link.txt")])
            .unwrap_err()
            .to_string();

    assert!(error.contains("symlink"), "unexpected error: {error}");
}

#[test]
fn publication_resource_reader_rejects_size_limits() {
    let root = TestDir::new("resource-size");
    root.write(
        "too-large.txt",
        &vec![b'x'; MAX_PUBLICATION_RESOURCE_BYTE_LEN as usize + 1],
    );
    let error = read_publication_resources(
        root.path(),
        &[PublicationResourceSpec::new("too-large.txt")],
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("maximum is 1048576 bytes"),
        "unexpected error: {error}"
    );

    let root = TestDir::new("resource-total-size");
    let specs = (0..17)
        .map(|index| {
            let path = format!("r{index}.bin");
            root.write(&path, &vec![b'x'; 1024 * 1024]);
            PublicationResourceSpec::new(path)
        })
        .collect::<Vec<_>>();
    let error = read_publication_resources(root.path(), &specs)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("maximum is 16777216 bytes"),
        "unexpected error: {error}"
    );
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "skiff-publication-resource-{name}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("temp dir should be created");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.path.join(relative);
        std::fs::create_dir_all(path.parent().expect("test file parent")).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
