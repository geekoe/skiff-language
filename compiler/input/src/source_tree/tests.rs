use std::{
    env, fs, process,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "skiff-source-tree-{name}-{}-{unique}",
            process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn collects_sorted_skiff_sources_and_skips_generated_dirs() {
    let temp = TestDir::new("collect");
    fs::create_dir_all(temp.path.join("api")).unwrap();
    fs::create_dir_all(temp.path.join("internal")).unwrap();
    fs::create_dir_all(temp.path.join("target")).unwrap();
    fs::create_dir_all(temp.path.join(".cache")).unwrap();
    fs::write(
        temp.path.join("api").join("websocket_fixture.skiff"),
        "type A {}\n",
    )
    .unwrap();
    fs::write(
        temp.path
            .join("internal")
            .join("websocket_fixture_service.skiff"),
        "type B {}\n",
    )
    .unwrap();
    fs::write(temp.path.join("target").join("ignored.skiff"), "").unwrap();
    fs::write(temp.path.join(".cache").join("ignored.skiff"), "").unwrap();

    let source_tree = collect_source_tree(&temp.path).unwrap();
    let modules = source_tree
        .sources
        .iter()
        .map(|source| source.module_path.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        modules,
        vec![
            "api.websocket_fixture",
            "internal.websocket_fixture_service"
        ]
    );
    assert_eq!(
        source_tree.sources[0].file_path,
        PathBuf::from("api").join("websocket_fixture.skiff")
    );
    assert!(!source_tree.sources[0].is_test_file);
    assert_eq!(source_tree.sources[0].byte_len, 10);
}

#[test]
fn collects_test_skiff_sources_with_test_file_marking() {
    let temp = TestDir::new("collect-test");
    fs::create_dir_all(temp.path.join("api")).unwrap();
    fs::write(
        temp.path.join("api").join("handler.test.skiff"),
        "test \"handler\" { assert true }\n",
    )
    .unwrap();
    fs::write(
        temp.path.join("api").join("handler.skiff"),
        "function run() -> number { return 1 }",
    )
    .unwrap();

    let source_tree = collect_source_tree(&temp.path).unwrap();
    let test_source = source_tree
        .sources
        .iter()
        .find(|source| source.file_path.ends_with("handler.test.skiff"))
        .unwrap();
    let source_source = source_tree
        .sources
        .iter()
        .find(|source| source.file_path.ends_with("handler.skiff"))
        .unwrap();

    assert_eq!(test_source.module_path, "api.handler");
    assert_eq!(source_source.module_path, "api.handler");
    assert!(test_source.is_test_file);
    assert!(!source_source.is_test_file);
}

#[test]
fn rejects_user_sources_in_compiler_generated_namespace() {
    let temp = TestDir::new("reserved-namespace");
    fs::create_dir_all(temp.path.join("__skiff")).unwrap();
    fs::write(
        temp.path.join("__skiff").join("http_routes.skiff"),
        "type A {}\n",
    )
    .unwrap();

    let error = collect_source_tree(&temp.path)
        .expect_err("user source must not occupy compiler generated namespace");

    assert!(
        matches!(&error, SourceTreeError::ReservedGeneratedNamespace { .. }),
        "unexpected error: {error}"
    );
    let message = error.to_string();
    assert!(message.contains("__skiff/http_routes.skiff"));
    assert!(message.contains("reserved compiler generated namespace __skiff"));
}

#[cfg(unix)]
#[test]
fn skips_nested_source_symlink_directories() {
    use std::os::unix::fs as unix_fs;

    let temp = TestDir::new("nested-symlink");
    let outside = TestDir::new("nested-outside");
    fs::create_dir_all(temp.path.join("api")).unwrap();
    fs::create_dir_all(outside.path.join("leaked")).unwrap();
    fs::write(temp.path.join("api").join("http.skiff"), "type A {}\n").unwrap();
    fs::write(
        outside.path.join("leaked").join("secret.skiff"),
        "type B {}\n",
    )
    .unwrap();
    unix_fs::symlink(
        outside.path.join("leaked"),
        temp.path.join("api").join("leaked"),
    )
    .unwrap();

    let source_tree = collect_source_tree(&temp.path).unwrap();
    let modules = source_tree
        .sources
        .iter()
        .map(|source| source.module_path.as_str())
        .collect::<Vec<_>>();

    assert_eq!(modules, vec!["api.http"]);
}
