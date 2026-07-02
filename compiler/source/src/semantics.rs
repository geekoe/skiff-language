use crate::{
    parsed_sources::{
        package_semantic_publication, service_semantic_publication, ParsedCompilerSource,
    },
    semantic::SemanticPublication,
    shared::error::CompileError,
    shared::publication_error::PublicationError,
    shared::source_role::PublicationSourceRole,
};
use compiler_input_model::PublicationCompilePolicy;

pub struct PublicationCompilePlan<'a> {
    semantic_scope: PublicationSemanticScope<'a>,
    pub file_role_policy: PublicationFileRolePolicy,
    pub diagnostics: PublicationCompileDiagnostics<'a>,
}

impl<'a> PublicationCompilePlan<'a> {
    pub fn from_policy(policy: PublicationCompilePolicy<'a>) -> Self {
        match policy {
            PublicationCompilePolicy::Package { package_id } => Self {
                semantic_scope: PublicationSemanticScope::Package { package_id },
                file_role_policy: PublicationFileRolePolicy::FixedPackage,
                diagnostics: PublicationCompileDiagnostics::Package { package_id },
            },
            PublicationCompilePolicy::Service { .. } => Self {
                semantic_scope: PublicationSemanticScope::Service,
                file_role_policy: PublicationFileRolePolicy::ServiceSourceRole,
                diagnostics: PublicationCompileDiagnostics::Service,
            },
        }
    }

    pub fn semantic_publication(
        &self,
        parsed_sources: &'a [ParsedCompilerSource],
    ) -> SemanticPublication<'a> {
        self.semantic_scope.semantic_publication(parsed_sources)
    }
}

#[derive(Clone, Copy)]
enum PublicationSemanticScope<'a> {
    Package { package_id: &'a str },
    Service,
}

impl<'a> PublicationSemanticScope<'a> {
    fn semantic_publication(
        self,
        parsed_sources: &'a [ParsedCompilerSource],
    ) -> SemanticPublication<'a> {
        match self {
            Self::Package { package_id } => {
                package_semantic_publication(package_id, parsed_sources)
            }
            Self::Service => service_semantic_publication(parsed_sources),
        }
    }
}

#[derive(Clone, Copy)]
pub enum PublicationFileRolePolicy {
    FixedPackage,
    ServiceSourceRole,
}

impl PublicationFileRolePolicy {
    pub fn file_role(self, source: &ParsedCompilerSource) -> PublicationSourceRole {
        match self {
            Self::FixedPackage => PublicationSourceRole::Package,
            Self::ServiceSourceRole => source.role(),
        }
    }
}

#[derive(Clone, Copy)]
pub enum PublicationCompileDiagnostics<'a> {
    Package { package_id: &'a str },
    Service,
}

impl PublicationCompileDiagnostics<'_> {
    pub fn publication_semantic_context_error(self, error: CompileError) -> PublicationError {
        PublicationError::ContractValidation {
            message: error.to_string(),
        }
    }

    pub fn publication_db_metadata_index_error(self, error: CompileError) -> PublicationError {
        match self {
            Self::Package { package_id } => PublicationError::ContractValidation {
                message: format!("failed to build package {package_id} db metadata index: {error}"),
            },
            Self::Service => PublicationError::ContractValidation {
                message: format!("failed to build publication db metadata index: {error}"),
            },
        }
    }

    pub fn source_semantic_context_error(
        self,
        source_path: &str,
        error: CompileError,
    ) -> PublicationError {
        match self {
            Self::Package { package_id } => PublicationError::ContractValidation {
                message: format!(
                    "failed to find package {package_id} semantic context for source {source_path}: {error}"
                ),
            },
            Self::Service => PublicationError::ContractValidation {
                message: format!(
                    "failed to find service semantic context for source {source_path}: {error}"
                ),
            },
        }
    }

    pub fn source_file_ir_unit_error(
        self,
        source_path: &str,
        error: CompileError,
    ) -> PublicationError {
        match self {
            Self::Package { package_id } => PublicationError::ContractValidation {
                message: format!(
                    "failed to lower package {package_id} source {source_path} to typed File IR unit: {error}"
                ),
            },
            Self::Service => PublicationError::ContractValidation {
                message: format!("failed to lower {source_path} to typed File IR unit: {error}"),
            },
        }
    }
}
