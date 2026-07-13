use std::path::{Path, PathBuf};

pub fn is_test_file_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".test.skiff"))
}

pub fn module_relative_path_for_test_file(path: &Path) -> PathBuf {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return path.to_path_buf();
    };
    let Some(test_base) = file_name.strip_suffix(".test.skiff") else {
        return path.to_path_buf();
    };
    path.with_file_name(format!("{test_base}.skiff"))
}
