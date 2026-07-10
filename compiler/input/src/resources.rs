use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use skiff_compiler_input_model::{PublicationResourceInput, PublicationResourceSpec};

use crate::error::InputAssemblyError;

pub const MAX_PUBLICATION_RESOURCES: usize = 256;
pub const MAX_PUBLICATION_RESOURCE_BYTE_LEN: u64 = 1024 * 1024;
pub const MAX_PUBLICATION_RESOURCE_TOTAL_BYTE_LEN: u64 = 16 * 1024 * 1024;

pub fn collect_publication_resource_spec_violations(
    specs: &[PublicationResourceSpec],
    violations: &mut Vec<String>,
) {
    if specs.len() > MAX_PUBLICATION_RESOURCES {
        violations.push(format!(
            "resources must contain at most {MAX_PUBLICATION_RESOURCES} entries"
        ));
    }

    let mut seen = BTreeSet::new();
    for (index, spec) in specs.iter().enumerate() {
        match validate_publication_resource_logical_path(&spec.path) {
            Ok(()) => {
                if !seen.insert(spec.path.clone()) {
                    violations.push(format!(
                        "resources[{index}] {} is declared more than once",
                        spec.path
                    ));
                }
            }
            Err(message) => violations.push(format!(
                "resources[{index}] {} is invalid: {message}",
                resource_path_label(&spec.path)
            )),
        }
    }
}

pub fn read_publication_resources(
    root: &Path,
    specs: &[PublicationResourceSpec],
) -> Result<Vec<PublicationResourceInput>, InputAssemblyError> {
    let mut violations = Vec::new();
    collect_publication_resource_spec_violations(specs, &mut violations);
    if !violations.is_empty() {
        return Err(resource_validation_error(root, violations));
    }

    let mut resources = Vec::with_capacity(specs.len());
    let mut total_byte_len = 0_u64;
    for spec in specs {
        let resource = read_publication_resource(root, &spec.path)?;
        if resource.byte_len > MAX_PUBLICATION_RESOURCE_BYTE_LEN {
            return Err(resource_validation_error(
                root,
                vec![format!(
                    "resource {} is {} bytes; maximum is {} bytes",
                    resource.path, resource.byte_len, MAX_PUBLICATION_RESOURCE_BYTE_LEN
                )],
            ));
        }
        total_byte_len = total_byte_len
            .checked_add(resource.byte_len)
            .ok_or_else(|| {
                resource_validation_error(
                    root,
                    vec!["resources total byte length overflowed u64".to_string()],
                )
            })?;
        if total_byte_len > MAX_PUBLICATION_RESOURCE_TOTAL_BYTE_LEN {
            return Err(resource_validation_error(
                root,
                vec![format!(
                    "resources are {total_byte_len} bytes total; maximum is {} bytes",
                    MAX_PUBLICATION_RESOURCE_TOTAL_BYTE_LEN
                )],
            ));
        }
        resources.push(resource);
    }

    Ok(resources)
}

pub fn validate_publication_resource_logical_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("path must not be empty".to_string());
    }
    if path.starts_with('/') || is_windows_absolute_path(path) {
        return Err("path must be relative to the publication root".to_string());
    }
    if path.contains('\\') {
        return Err("path must use / separators, not backslashes".to_string());
    }
    if path.ends_with('/') {
        return Err("path must not have a trailing /".to_string());
    }
    if path.contains("//") {
        return Err("path must not contain empty segments".to_string());
    }
    if contains_glob_metacharacter(path) {
        return Err("glob patterns are not supported".to_string());
    }

    let segments = path.split('/').collect::<Vec<_>>();
    for segment in &segments {
        if segment.is_empty() {
            return Err("path must not contain empty segments".to_string());
        }
        if *segment == "." {
            return Err("path must not contain . segments".to_string());
        }
        if *segment == ".." {
            return Err("path must not contain .. segments".to_string());
        }
        if segment.starts_with('.') {
            return Err("hidden files and directories are not allowed".to_string());
        }
    }

    let file_name = segments
        .last()
        .expect("non-empty path split should yield a file name");
    if file_name.ends_with(".skiff") {
        return Err("resources must not be .skiff source files".to_string());
    }
    if is_skiff_control_file(file_name) {
        return Err("resources must not be Skiff control files".to_string());
    }

    Ok(())
}

fn read_publication_resource(
    root: &Path,
    logical_path: &str,
) -> Result<PublicationResourceInput, InputAssemblyError> {
    let (absolute_path, byte_len) =
        resolve_publication_resource_file(root, logical_path).map_err(|message| {
            resource_validation_error(
                root,
                vec![format!("resource {} is invalid: {message}", logical_path)],
            )
        })?;
    let bytes = fs::read(&absolute_path).map_err(|source| InputAssemblyError::Read {
        path: absolute_path.display().to_string(),
        source,
    })?;
    let actual_len = bytes.len() as u64;
    if actual_len != byte_len {
        return Err(resource_validation_error(
            root,
            vec![format!(
                "resource {logical_path} changed while reading: metadata length {byte_len}, read length {actual_len}"
            )],
        ));
    }
    let sha256 = hex::encode(Sha256::digest(&bytes));
    Ok(PublicationResourceInput {
        path: logical_path.to_string(),
        absolute_path,
        byte_len,
        sha256,
        content_type: None,
    })
}

fn resolve_publication_resource_file(
    root: &Path,
    logical_path: &str,
) -> Result<(PathBuf, u64), String> {
    let mut current = absolute_root(root);
    let segments = logical_path.split('/').collect::<Vec<_>>();
    for (index, segment) in segments.iter().enumerate() {
        let path = exact_child_path(&current, segment)?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|source| format!("failed to inspect {}: {source}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("{} is a symlink", path.display()));
        }
        let is_final = index + 1 == segments.len();
        if is_final {
            if !metadata.file_type().is_file() {
                return Err(format!("{} is not a regular file", path.display()));
            }
            return Ok((path, metadata.len()));
        }
        if !metadata.file_type().is_dir() {
            return Err(format!("{} is not a directory", path.display()));
        }
        current = path;
    }
    Err("path must not be empty".to_string())
}

fn exact_child_path(parent: &Path, segment: &str) -> Result<PathBuf, String> {
    let entries = fs::read_dir(parent)
        .map_err(|source| format!("failed to read directory {}: {source}", parent.display()))?;
    for entry in entries {
        let entry = entry
            .map_err(|source| format!("failed to read directory {}: {source}", parent.display()))?;
        if entry.file_name() == OsStr::new(segment) {
            return Ok(entry.path());
        }
    }
    Err(format!(
        "{} does not exist with exact case in {}",
        segment,
        parent.display()
    ))
}

fn absolute_root(root: &Path) -> PathBuf {
    if root.is_absolute() {
        return root.to_path_buf();
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(root)
}

fn is_windows_absolute_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == b'/' && bytes[0].is_ascii_alphabetic()
}

fn contains_glob_metacharacter(path: &str) -> bool {
    path.contains(['*', '?', '[', ']', '{', '}'])
}

fn is_skiff_control_file(file_name: &str) -> bool {
    file_name == "package.yml"
        || file_name == "service.yml"
        || file_name == "api.yml"
        || file_name == "config.yml"
        || (file_name.starts_with("service.") && file_name.ends_with(".yml"))
        || (file_name.starts_with("config.") && file_name.ends_with(".yml"))
        || file_name.ends_with(".secret.yml")
}

fn resource_path_label(path: &str) -> String {
    if path.is_empty() {
        "<empty>".to_string()
    } else {
        path.to_string()
    }
}

fn resource_validation_error(root: &Path, violations: Vec<String>) -> InputAssemblyError {
    InputAssemblyError::Validation {
        message: violations
            .into_iter()
            .map(|violation| format!("- {}: {violation}", root.display()))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

#[cfg(test)]
mod tests {
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
            ("service.prod.yml", "control"),
            ("config.prod.yml", "control"),
            ("prod.secret.yml", "control"),
        ] {
            let error = validate_publication_resource_logical_path(path)
                .expect_err("path should be rejected");
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
}
