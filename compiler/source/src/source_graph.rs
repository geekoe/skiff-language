use std::{
    ops::Deref,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    shared::ast::SourceFile, shared::parser::parse_source,
    shared::publication_error::PublicationError,
};
use compiler_input_model::{
    CompilerRawSourceFile, RawPublicationSourceGraph, RawSourceFileMeta, RawSourceOrigin,
};

pub use skiff_compiler_core::source_role::PublicationSourceRole as CompilerSourceRole;

#[cfg(test)]
#[path = "source_graph/tests/compiler_source_file.rs"]
mod compiler_source_file_tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceOrigin {
    Service,
    Package { package_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileMeta {
    relative_path: PathBuf,
    module_path: String,
    is_test_file: bool,
    pub origin: SourceOrigin,
}

impl SourceFileMeta {
    pub fn service(relative_path: PathBuf, module_path: String, is_test_file: bool) -> Self {
        Self {
            relative_path,
            module_path,
            is_test_file,
            origin: SourceOrigin::Service,
        }
    }

    pub fn package(
        package_id: impl Into<String>,
        relative_path: PathBuf,
        module_path: String,
    ) -> Self {
        Self {
            relative_path,
            module_path,
            is_test_file: false,
            origin: SourceOrigin::Package {
                package_id: package_id.into(),
            },
        }
    }

    fn from_raw(raw: &RawSourceFileMeta) -> Self {
        let origin = match &raw.origin {
            RawSourceOrigin::Service => SourceOrigin::Service,
            RawSourceOrigin::Package { package_id } => SourceOrigin::Package {
                package_id: package_id.clone(),
            },
        };
        Self {
            relative_path: raw.relative_path.clone(),
            module_path: raw.module_path.clone(),
            is_test_file: raw.is_test_file,
            origin,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParsedSourceFile {
    pub relative_path: PathBuf,
    pub module_path: String,
    pub is_test_file: bool,
    origin: SourceOrigin,
    pub text: String,
    pub ast: SourceFile,
}

impl ParsedSourceFile {
    pub fn parse(
        meta: SourceFileMeta,
        text: String,
        diagnostic_path: impl Into<String>,
    ) -> Result<Self, PublicationError> {
        let path = diagnostic_path.into();
        let ast = parse_source(&text).map_err(|source| PublicationError::Parse { path, source })?;
        Ok(Self::from_parsed_ast(meta, text, ast))
    }

    pub fn from_parsed_ast(meta: SourceFileMeta, text: String, ast: SourceFile) -> Self {
        Self {
            relative_path: meta.relative_path,
            module_path: meta.module_path,
            is_test_file: meta.is_test_file,
            origin: meta.origin,
            text,
            ast,
        }
    }

    pub fn diagnostic_path(&self, root: &Path) -> String {
        root.join(&self.relative_path).display().to_string()
    }

    pub fn origin(&self) -> &SourceOrigin {
        &self.origin
    }
}

#[derive(Debug, Clone)]
pub struct CompilerSourceFile {
    parsed: Arc<ParsedSourceFile>,
    role: CompilerSourceRole,
}

impl Deref for CompilerSourceFile {
    type Target = ParsedSourceFile;

    fn deref(&self) -> &Self::Target {
        self.parsed.as_ref()
    }
}

impl CompilerSourceFile {
    pub fn from_parsed_file(parsed: ParsedSourceFile, role: CompilerSourceRole) -> Self {
        Self {
            parsed: Arc::new(parsed),
            role,
        }
    }

    fn parse_raw(raw: &CompilerRawSourceFile, root: &Path) -> Result<Self, PublicationError> {
        let diagnostic_path = root.join(&raw.meta.relative_path).display().to_string();
        let parsed = ParsedSourceFile::parse(
            SourceFileMeta::from_raw(&raw.meta),
            raw.text.clone(),
            diagnostic_path,
        )?;
        Ok(Self::from_parsed_file(parsed, raw.role))
    }

    pub fn role(&self) -> CompilerSourceRole {
        self.role
    }

    pub fn parse(
        relative_path: PathBuf,
        module_path: String,
        is_api: bool,
        is_test_file: bool,
        text: String,
        diagnostic_path: impl Into<String>,
    ) -> Result<Self, PublicationError> {
        let parsed = ParsedSourceFile::parse(
            SourceFileMeta::service(relative_path, module_path, is_test_file),
            text,
            diagnostic_path,
        )?;
        Ok(Self::from_parsed_file(
            parsed,
            CompilerSourceRole::from_api_flag(is_api),
        ))
    }

    pub fn from_parsed_ast(
        relative_path: PathBuf,
        module_path: String,
        is_api: bool,
        is_test_file: bool,
        text: String,
        ast: SourceFile,
    ) -> Self {
        let parsed = ParsedSourceFile::from_parsed_ast(
            SourceFileMeta {
                relative_path,
                module_path,
                is_test_file,
                origin: SourceOrigin::Service,
            },
            text,
            ast,
        );
        Self::from_parsed_file(parsed, CompilerSourceRole::from_api_flag(is_api))
    }

    pub fn diagnostic_path_from_root(&self, root: &Path) -> String {
        self.parsed.diagnostic_path(root)
    }
}

#[derive(Debug, Clone)]
pub struct PublicationSourceGraph {
    files: Vec<CompilerSourceFile>,
}

impl PublicationSourceGraph {
    pub fn from_compiler_sources(files: Vec<CompilerSourceFile>) -> Self {
        Self { files }
    }

    pub fn parse_raw_publication_sources(
        raw: &RawPublicationSourceGraph,
    ) -> Result<Self, PublicationError> {
        let files = raw
            .files
            .iter()
            .map(|source| CompilerSourceFile::parse_raw(source, &raw.root))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { files })
    }

    pub fn files(&self) -> &[CompilerSourceFile] {
        &self.files
    }

    pub fn production(&self) -> impl Iterator<Item = &CompilerSourceFile> {
        self.files.iter().filter(|source| !source.is_test_file)
    }

    pub fn production_files(&self) -> Vec<CompilerSourceFile> {
        self.production().cloned().collect()
    }
}
