use crate::{
    parsed_sources::{package_semantic_publication, ParsedCompilerSource},
    semantic::SemanticPublication,
    shared::error::CompileError,
    shared::publication_error::PublicationError,
    shared::source_role::PublicationSourceRole,
};
use compiler_input_model::PackageCompilePolicy;

pub struct PackageCompilePlan<'a> {
    package_id: &'a str,
    pub file_role_policy: PackageFileRolePolicy,
    pub diagnostics: PackageCompileDiagnostics<'a>,
}

impl<'a> PackageCompilePlan<'a> {
    pub fn from_policy(policy: PackageCompilePolicy<'a>) -> Self {
        let package_id = policy.package_id();
        Self {
            package_id,
            file_role_policy: PackageFileRolePolicy,
            diagnostics: PackageCompileDiagnostics { package_id },
        }
    }

    pub fn semantic_publication(
        &self,
        parsed_sources: &'a [ParsedCompilerSource],
    ) -> SemanticPublication<'a> {
        package_semantic_publication(self.package_id, parsed_sources)
    }
}

#[derive(Clone, Copy)]
pub struct PackageFileRolePolicy;

impl PackageFileRolePolicy {
    pub fn file_role(self, source: &ParsedCompilerSource) -> PublicationSourceRole {
        let _ = source;
        PublicationSourceRole::Package
    }
}

#[derive(Clone, Copy)]
pub struct PackageCompileDiagnostics<'a> {
    package_id: &'a str,
}

impl PackageCompileDiagnostics<'_> {
    pub fn publication_semantic_context_error(self, error: CompileError) -> PublicationError {
        PublicationError::ContractValidation {
            message: error.to_string(),
        }
    }

    pub fn publication_db_metadata_index_error(self, error: CompileError) -> PublicationError {
        PublicationError::ContractValidation {
            message: format!(
                "failed to build package {} db metadata index: {error}",
                self.package_id
            ),
        }
    }

    pub fn source_semantic_context_error(
        self,
        source_path: &str,
        error: CompileError,
    ) -> PublicationError {
        PublicationError::ContractValidation {
            message: format!(
                "failed to find package {} semantic context for source {source_path}: {error}",
                self.package_id
            ),
        }
    }

    pub fn source_file_ir_unit_error(
        self,
        source_path: &str,
        error: CompileError,
    ) -> PublicationError {
        PublicationError::ContractValidation {
            message: format!(
                "failed to lower package {} source {source_path} to typed File IR unit: {error}",
                self.package_id
            ),
        }
    }
}
