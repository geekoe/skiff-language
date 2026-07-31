use std::{
    fs,
    path::{Path, PathBuf},
};

use skiff_syntax::{ast::SourceFile, parser::parse_source};

use crate::canonical_fixture::CanonicalFixtureError;

#[derive(Debug, Clone)]
pub struct TestServiceCase {
    /// Stable compiler/runner join key. It deliberately does not contain the
    /// user-facing test name, so renaming a case cannot orphan its effect plan.
    pub case_identity: String,
    pub relative_path: PathBuf,
    pub module_path: String,
    pub name: String,
    pub function_name: String,
    pub test_index: usize,
    pub source_text: String,
    pub source_ast: SourceFile,
}

pub fn discover_test_service_cases(
    input: &Path,
    package_root: &Path,
    input_is_file: bool,
) -> Result<Vec<TestServiceCase>, CanonicalFixtureError> {
    let mut files = Vec::new();
    if input_is_file {
        if is_test_file(input) {
            files.push(input.to_path_buf());
        }
    } else {
        collect_test_files(input, &mut files)?;
    }
    files.sort();
    let mut cases = Vec::new();
    for file in files {
        let source_text =
            fs::read_to_string(&file).map_err(|source| CanonicalFixtureError::Io {
                path: file.display().to_string(),
                source,
            })?;
        let source_ast =
            parse_source(&source_text).map_err(|source| CanonicalFixtureError::Parse {
                path: file.display().to_string(),
                source,
            })?;
        let relative_path = file
            .strip_prefix(package_root)
            .unwrap_or(&file)
            .to_path_buf();
        let module_path = test_module_path(&relative_path)?;
        let default_run = source_ast.test_default_run.unwrap_or(true);
        for (test_index, test) in source_ast.tests.iter().enumerate() {
            if input_is_file || default_run {
                let case_identity = canonical_case_identity(&module_path, test_index);
                cases.push(TestServiceCase {
                    case_identity: case_identity.clone(),
                    relative_path: relative_path.clone(),
                    module_path: module_path.clone(),
                    name: test.name.clone(),
                    function_name: format!("skiffTestCase{test_index}"),
                    test_index,
                    source_text: source_text.clone(),
                    source_ast: source_ast.clone(),
                });
            }
        }
    }
    Ok(cases)
}

fn canonical_case_identity(module_path: &str, test_index: usize) -> String {
    format!("{module_path}::test[{test_index}]")
}

fn collect_test_files(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), CanonicalFixtureError> {
    let entries = fs::read_dir(root).map_err(|source| CanonicalFixtureError::Io {
        path: root.display().to_string(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| CanonicalFixtureError::Io {
            path: root.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|source| CanonicalFixtureError::Io {
                path: path.display().to_string(),
                source,
            })?;
        if kind.is_dir() {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if name != "target" && name != "node_modules" && !name.starts_with('.') {
                collect_test_files(&path, output)?;
            }
        } else if kind.is_file() && is_test_file(&path) {
            output.push(path);
        }
    }
    Ok(())
}

fn is_test_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.ends_with(".test.skiff"))
}

fn test_module_path(path: &Path) -> Result<String, CanonicalFixtureError> {
    let text = path.to_str().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(format!(
            "test source path {} is not valid UTF-8",
            path.display()
        ))
    })?;
    let stem = text.strip_suffix(".test.skiff").ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(format!(
            "test source {} must end with .test.skiff",
            path.display()
        ))
    })?;
    Ok(stem
        .split(std::path::MAIN_SEPARATOR)
        .filter(|part| !part.is_empty())
        .chain(std::iter::once("__test"))
        .collect::<Vec<_>>()
        .join("."))
}

#[cfg(test)]
mod tests;
