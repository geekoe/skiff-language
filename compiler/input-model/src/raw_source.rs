use std::path::PathBuf;

use crate::CompilerSourceRole;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawSourceOrigin {
    Service,
    Package { package_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSourceFileMeta {
    pub relative_path: PathBuf,
    pub module_path: String,
    pub is_test_file: bool,
    pub is_generated: bool,
    pub origin: RawSourceOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerRawSourceFile {
    pub meta: RawSourceFileMeta,
    pub text: String,
    pub role: CompilerSourceRole,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSourceTreeFile {
    pub module_path: String,
    pub file_path: PathBuf,
    pub is_test_file: bool,
    pub is_generated: bool,
    pub byte_len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSourceTree {
    pub root: PathBuf,
    pub sources: Vec<RawSourceTreeFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPublicationSourceGraph {
    pub root: PathBuf,
    pub files: Vec<CompilerRawSourceFile>,
}
