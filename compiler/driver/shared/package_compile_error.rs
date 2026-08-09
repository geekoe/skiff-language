use thiserror::Error;

#[derive(Debug, Error)]
pub enum PackageCompileError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: skiff_syntax::error::CompileError,
    },
    #[error("contract validation failed:\n{message}")]
    ContractValidation { message: String },
    #[error("invalid root reference in {path}:\n{message}")]
    RootPathReference { path: String, message: String },
    #[error("compiled projection input is invalid: {source}")]
    ProjectionInput {
        source: skiff_compiler_compiled::ProjectionInputBuildError,
    },
    #[error("public-instance operation fact handoff failed: {source}")]
    PublicInstanceOperationFacts {
        #[source]
        source: skiff_compiler_contract::ContractDefinitionError,
    },
    #[error(
        "bytecode emission was explicitly enabled after source lowering produced {mir_unit_count} typed MIR unit(s), but this compiler build has no canonical emitter entrypoint or source-owned value-transfer plan bundle"
    )]
    BytecodeEmitterUnavailable { mir_unit_count: usize },
    #[error("bytecode emission failed: {source}")]
    BytecodeEmission {
        #[source]
        source: skiff_compiler_emission::BytecodeEmissionError,
    },
    #[error("emitted bytecode handoff admission failed: {source}")]
    BytecodeHandoff {
        #[source]
        source: skiff_compiler_compiled::BytecodeCompilationHandoffError,
    },
    #[error("bytecode package projection is invalid:\n{message}")]
    BytecodeProjection { message: String },
    #[error("resolved package schema input is invalid:\n{message}")]
    PackageSchemaInput { message: String },
}

impl From<skiff_compiler_source::SourceCompileError> for PackageCompileError {
    fn from(error: skiff_compiler_source::SourceCompileError) -> Self {
        match error {
            skiff_compiler_source::SourceCompileError::Parse { path, source } => {
                Self::Parse { path, source }
            }
            skiff_compiler_source::SourceCompileError::ContractValidation { message } => {
                Self::ContractValidation { message }
            }
            skiff_compiler_source::SourceCompileError::RootPathReference { path, message } => {
                Self::RootPathReference { path, message }
            }
        }
    }
}

impl From<skiff_compiler_contract::ContractDefinitionError> for PackageCompileError {
    fn from(source: skiff_compiler_contract::ContractDefinitionError) -> Self {
        Self::PublicInstanceOperationFacts { source }
    }
}

pub(crate) fn package_projection_error(
    error: skiff_compiler_projection::error::ProjectionError,
) -> PackageCompileError {
    match error {
        skiff_compiler_projection::error::ProjectionError::InvalidPackageArtifact { message } => {
            PackageCompileError::ContractValidation { message }
        }
    }
}

pub(crate) fn projection_input_error(
    source: skiff_compiler_compiled::ProjectionInputBuildError,
) -> PackageCompileError {
    PackageCompileError::ProjectionInput { source }
}

impl From<skiff_compiler_emission::error::EmissionError> for PackageCompileError {
    fn from(error: skiff_compiler_emission::error::EmissionError) -> Self {
        match error {
            skiff_compiler_emission::error::EmissionError::ContractValidation { message } => {
                Self::ContractValidation { message }
            }
            skiff_compiler_emission::error::EmissionError::ArtifactIdentity { source } => {
                Self::ContractValidation {
                    message: source.to_string(),
                }
            }
        }
    }
}

impl From<skiff_compiler_emission::BytecodeEmissionError> for PackageCompileError {
    fn from(source: skiff_compiler_emission::BytecodeEmissionError) -> Self {
        Self::BytecodeEmission { source }
    }
}

impl From<skiff_compiler_compiled::BytecodeCompilationHandoffError> for PackageCompileError {
    fn from(source: skiff_compiler_compiled::BytecodeCompilationHandoffError) -> Self {
        Self::BytecodeHandoff { source }
    }
}
