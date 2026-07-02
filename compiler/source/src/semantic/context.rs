use crate::shared::error::{CompileError, Result};

use super::{
    build_executable_semantics, ExecutableIndex, ExecutableSemantics, InterfaceSemantics,
    SemanticPublication, SemanticSource,
};

#[derive(Debug, Clone)]
pub struct PublicationSemanticContext<'a> {
    publication: &'a SemanticPublication<'a>,
    interface_semantics: InterfaceSemantics,
    executable_semantics: ExecutableSemantics<'a>,
}

impl<'a> PublicationSemanticContext<'a> {
    pub fn build(publication: &'a SemanticPublication<'a>) -> Result<Self> {
        Ok(Self {
            publication,
            interface_semantics: InterfaceSemantics::build(publication)?,
            executable_semantics: build_executable_semantics(publication)?,
        })
    }

    pub fn source_context<'context>(
        &'context self,
        module_path: &str,
    ) -> Result<SourceSemanticContext<'context, 'a>> {
        let source = self.publication.source(module_path).ok_or_else(|| {
            CompileError::Semantic(format!("missing semantic source for module {module_path}"))
        })?;
        let executable_index = self
            .executable_semantics
            .executable_index(module_path)
            .ok_or_else(|| {
                CompileError::Semantic(format!(
                    "missing semantic executable index for module {module_path}"
                ))
            })?;
        Ok(SourceSemanticContext {
            source,
            executable_index,
            interface_semantics: &self.interface_semantics,
        })
    }

    pub fn interface_semantics(&self) -> &InterfaceSemantics {
        &self.interface_semantics
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SourceSemanticContext<'context, 'publication> {
    pub source: &'context SemanticSource<'publication>,
    pub executable_index: &'context ExecutableIndex,
    pub interface_semantics: &'context InterfaceSemantics,
}
