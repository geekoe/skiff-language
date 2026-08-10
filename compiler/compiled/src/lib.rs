use skiff_artifact_model::{ActorMetadataIr, DbMetadataIr, FileIrUnit};
use skiff_compiler_lowering::{CompiledPackageSource, LoweredPackage};
use skiff_compiler_source::{
    source_identity::PublicationDeclarationAnchors, CompileParsedPackageSourcesInput,
    PackageSourceModel, PublicationApiSeed, SourceCompileError,
};

pub mod bytecode_handoff;
mod package_callable_signatures;
pub mod projection_input;
pub mod service_contract;

pub use bytecode_handoff::{
    BytecodeCompilationAuthorityPins, BytecodeCompilationHandoff, BytecodeCompilationHandoffError,
    BytecodeCompilationOutcome, BytecodeCompilationReceipt, BytecodeStatementManifestReceipt,
};
pub use package_callable_signatures::ProjectionInputBuildError;

#[cfg(feature = "test-support")]
use skiff_compiler_source::{ConfigRequirementSet, ExportBindingModel};

#[derive(Debug)]
pub struct CompiledPackage {
    model: PackageSourceModel,
    lowered: LoweredPackage,
}

pub fn compile_parsed_publication_sources(
    input: CompileParsedPackageSourcesInput<'_, '_>,
) -> Result<CompiledPackage, SourceCompileError> {
    let model = skiff_compiler_source::build_package_from_parsed_sources(input)?;
    compile_source_model(model)
}

pub fn compile_source_model(
    model: PackageSourceModel,
) -> Result<CompiledPackage, SourceCompileError> {
    let lowered = skiff_compiler_lowering::lower(&model)?;
    Ok(CompiledPackage::new(model, lowered))
}

impl CompiledPackage {
    pub fn new(model: PackageSourceModel, lowered: LoweredPackage) -> Self {
        Self { model, lowered }
    }

    pub fn compile_model(&self) -> &PackageSourceModel {
        &self.model
    }

    pub fn lowered(&self) -> &LoweredPackage {
        &self.lowered
    }

    pub fn file_ir_units(&self) -> &[FileIrUnit] {
        self.lowered.file_ir_units()
    }

    #[cfg(feature = "test-support")]
    pub fn file_ir_units_mut(&mut self) -> &mut [FileIrUnit] {
        self.lowered.file_ir_units_mut()
    }

    pub fn source_metadata(&self) -> &[CompiledPackageSource] {
        self.lowered.sources()
    }

    pub fn publication_api_seed(&self) -> &PublicationApiSeed {
        self.model.publication_api().seed()
    }

    #[cfg(feature = "test-support")]
    pub fn publication_api_seed_mut(&mut self) -> &mut PublicationApiSeed {
        self.model.publication_api_mut().seed_mut()
    }

    #[cfg(feature = "test-support")]
    pub fn export_bindings_mut(&mut self) -> &mut ExportBindingModel {
        self.model.export_bindings_mut()
    }

    #[allow(dead_code)]
    pub fn source_identity(&self) -> &str {
        self.model.source_identity()
    }

    #[allow(dead_code)]
    pub fn declaration_anchors(&self) -> &PublicationDeclarationAnchors {
        self.model.declaration_anchors()
    }

    #[cfg(feature = "test-support")]
    pub fn own_config_requirements(&self) -> &ConfigRequirementSet {
        self.model.own_config_requirements()
    }

    pub fn service_db_metadata(&self) -> &[DbMetadataIr] {
        self.lowered.service_db_metadata()
    }

    pub fn service_actor_metadata(&self) -> &[ActorMetadataIr] {
        self.lowered.service_actor_metadata()
    }

    pub fn has_service_storage_metadata(&self) -> bool {
        self.lowered.has_service_storage_metadata()
    }
}
