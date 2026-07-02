use std::{borrow::Cow, collections::BTreeMap};

use crate::shared::ast::SourceFile;

pub mod context;
pub mod db_attachment;
pub mod executable;
pub mod executable_semantics;
pub mod interface;

pub use context::{PublicationSemanticContext, SourceSemanticContext};
pub use db_attachment::{validate_db_attachments, DbAttachmentIndex};
pub use executable::{executable_symbol, impl_method_declaration_name, ExecutableIndex};
pub use executable_semantics::{build_executable_semantics, ExecutableSemantics};
pub use interface::InterfaceSemantics;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceOrigin<'a> {
    Service,
    #[allow(dead_code)]
    Package {
        package_id: &'a str,
    },
}

#[derive(Debug, Clone)]
pub struct SemanticSource<'a> {
    pub source_path: Cow<'a, str>,
    pub module_path: &'a str,
    pub origin: SourceOrigin<'a>,
    pub ast: &'a SourceFile,
    pub alias_targets: &'a BTreeMap<String, String>,
}

impl<'a> SemanticSource<'a> {
    pub fn new(
        source_path: impl Into<Cow<'a, str>>,
        module_path: &'a str,
        origin: SourceOrigin<'a>,
        ast: &'a SourceFile,
        alias_targets: &'a BTreeMap<String, String>,
    ) -> Self {
        Self {
            source_path: source_path.into(),
            module_path,
            origin,
            ast,
            alias_targets,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SemanticPublication<'a> {
    pub sources: Vec<SemanticSource<'a>>,
}

impl<'a> SemanticPublication<'a> {
    pub fn new(sources: Vec<SemanticSource<'a>>) -> Self {
        Self { sources }
    }

    pub fn source(&self, module_path: &str) -> Option<&SemanticSource<'a>> {
        self.sources
            .iter()
            .find(|source| source.module_path == module_path)
    }
}
