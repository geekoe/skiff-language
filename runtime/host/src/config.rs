use std::path::{Path, PathBuf};

pub const DEFAULT_HTTP_RESPONSE_MAX_BYTES: usize = 8 * 1024 * 1024;

pub fn skiff_file_tmp_dir(runtime_home: &Path) -> PathBuf {
    runtime_home.join("tmp").join("skiff-file")
}
