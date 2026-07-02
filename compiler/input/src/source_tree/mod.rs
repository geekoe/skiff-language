use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use thiserror::Error;

use crate::test_rules::{
    is_test_file_path, module_relative_path_for_test_file_without_friend,
    production_relative_path_for_test_file,
};

const COMPILER_GENERATED_NAMESPACE: &str = "__skiff";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTree {
    pub root: PathBuf,
    pub sources: Vec<SourceTreeFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTreeFile {
    pub module_path: String,
    pub file_path: PathBuf,
    pub is_test_file: bool,
    pub byte_len: u64,
}

#[derive(Debug, Error)]
pub enum SourceTreeError {
    #[error("failed to read directory {path}: {source}")]
    ReadDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to inspect {path}: {source}")]
    Metadata {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("source path {path} is not valid UTF-8")]
    NonUtf8Path { path: String },
    #[error("source path {path} must be a relative path inside the service root")]
    InvalidSourcePath { path: String },
    #[error("source path {path} must be a directory")]
    SourceNotDirectory { path: String },
    #[error(
        "source path {path} uses reserved compiler generated namespace __skiff; rename the source so its root module segment is not __skiff"
    )]
    ReservedGeneratedNamespace { path: String },
}

pub fn collect_source_tree(root: &Path) -> Result<SourceTree, SourceTreeError> {
    let mut sources = Vec::new();
    validate_source_root(root, Path::new("."))?;
    collect_from_dir(root, root, &mut sources)?;
    sources.sort_by(|left, right| {
        left.module_path
            .cmp(&right.module_path)
            .then_with(|| left.file_path.cmp(&right.file_path))
    });
    Ok(SourceTree {
        root: root.to_path_buf(),
        sources,
    })
}

fn validate_source_root(root: &Path, source_path: &Path) -> Result<(), SourceTreeError> {
    let mut current = root.to_path_buf();
    for component in source_path.components() {
        match component {
            Component::CurDir => continue,
            Component::Normal(part) => current.push(part),
            _ => {
                return Err(SourceTreeError::InvalidSourcePath {
                    path: source_path.display().to_string(),
                })
            }
        }
        let metadata =
            fs::symlink_metadata(&current).map_err(|source| SourceTreeError::Metadata {
                path: current.display().to_string(),
                source,
            })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(SourceTreeError::SourceNotDirectory {
                path: current.display().to_string(),
            });
        }
    }
    Ok(())
}

fn collect_from_dir(
    root: &Path,
    dir: &Path,
    sources: &mut Vec<SourceTreeFile>,
) -> Result<(), SourceTreeError> {
    let entries = fs::read_dir(dir).map_err(|source| SourceTreeError::ReadDir {
        path: dir.display().to_string(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| SourceTreeError::ReadDir {
            path: dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| SourceTreeError::Metadata {
                path: path.display().to_string(),
                source,
            })?;

        if file_type.is_symlink() {
            continue;
        }

        if file_type.is_dir() {
            if should_skip_dir(&path) {
                continue;
            }
            collect_from_dir(root, &path, sources)?;
            continue;
        }

        if !file_type.is_file()
            || path.extension().and_then(|extension| extension.to_str()) != Some("skiff")
        {
            continue;
        }

        let metadata = fs::metadata(&path).map_err(|source| SourceTreeError::Metadata {
            path: path.display().to_string(),
            source,
        })?;

        let relative_path = path.strip_prefix(root).unwrap_or(&path);
        let is_test_file = is_test_file(relative_path);
        let module_relative_path = if is_test_file {
            let production_path = production_relative_path_for_test_file(&path)
                .and_then(|path| path.strip_prefix(root).ok().map(Path::to_path_buf));
            production_path
                .unwrap_or_else(|| module_relative_path_for_test_file_without_friend(relative_path))
        } else {
            relative_path.to_path_buf()
        };
        let module_path = module_path(&module_relative_path, relative_path)?;
        validate_user_source_namespace(relative_path, &module_path)?;
        sources.push(SourceTreeFile {
            module_path,
            file_path: relative_path.to_path_buf(),
            is_test_file,
            byte_len: metadata.len(),
        });
    }

    Ok(())
}

fn validate_user_source_namespace(
    relative_path: &Path,
    module_path: &str,
) -> Result<(), SourceTreeError> {
    let first_module_segment = module_path.split('.').next().unwrap_or_default();
    if first_module_segment == COMPILER_GENERATED_NAMESPACE {
        return Err(SourceTreeError::ReservedGeneratedNamespace {
            path: relative_path.display().to_string(),
        });
    }
    Ok(())
}

fn should_skip_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == "target" || name == "node_modules" || name.starts_with('.')
}

fn module_path(module_relative_path: &Path, error_path: &Path) -> Result<String, SourceTreeError> {
    let without_extension = module_relative_path.with_extension("");
    let components = without_extension.components().collect::<Vec<_>>();
    let mut parts = Vec::new();
    for component in components {
        let text = component
            .as_os_str()
            .to_str()
            .ok_or_else(|| SourceTreeError::NonUtf8Path {
                path: error_path.display().to_string(),
            })?;
        parts.push(text.to_string());
    }
    Ok(parts.join("."))
}

fn is_test_file(path: &Path) -> bool {
    is_test_file_path(path)
}

#[cfg(test)]
mod tests;
