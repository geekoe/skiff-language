use skiff_artifact_model::{ActorMetadataIr, DbMetadataIr, FileIrUnit};
use skiff_compiler_input_model::{actor_declaration_inputs, ActorDeclarationInput};
use skiff_compiler_lowering::{CompiledPackageSource, LoweredPackage};
use skiff_compiler_source::{
    source_identity::PublicationDeclarationAnchors, CompileParsedPackageSourcesInput,
    PackageSourceModel, PublicationApiSeed, SourceCompileError,
};

mod package_callable_signatures;
pub mod projection_input;

pub use package_callable_signatures::ProjectionInputBuildError;

#[cfg(feature = "test-support")]
use skiff_compiler_source::{ConfigRequirementSet, ExportBindingModel};

#[derive(Debug)]
pub struct CompiledPackage {
    model: PackageSourceModel,
    lowered: LoweredPackage,
    actor_declarations: Vec<CompiledActorDeclaration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledActorDeclaration {
    pub module_path: String,
    pub declaration: ActorDeclarationInput,
}

impl CompiledActorDeclaration {
    pub fn new(module_path: impl Into<String>, declaration: ActorDeclarationInput) -> Self {
        Self {
            module_path: module_path.into(),
            declaration,
        }
    }
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
        let actor_declarations = model
            .sources()
            .parsed_sources()
            .iter()
            .flat_map(|source| {
                actor_declaration_inputs(source.ast())
                    .into_iter()
                    .map(|declaration| {
                        CompiledActorDeclaration::new(source.module_path(), declaration)
                    })
            })
            .collect();
        Self {
            model,
            lowered,
            actor_declarations,
        }
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

    #[cfg(feature = "test-support")]
    pub fn dependency_config_requirements(&self) -> &ConfigRequirementSet {
        self.model.dependency_config_requirements()
    }

    #[cfg(feature = "test-support")]
    pub fn effective_config_requirements(&self) -> &ConfigRequirementSet {
        self.model.effective_config_requirements()
    }

    pub fn service_db_metadata(&self) -> &[DbMetadataIr] {
        self.lowered.service_db_metadata()
    }

    pub fn service_actor_metadata(&self) -> &[ActorMetadataIr] {
        self.lowered.service_actor_metadata()
    }

    pub fn actor_declarations(&self) -> &[CompiledActorDeclaration] {
        &self.actor_declarations
    }

    pub fn has_service_storage_metadata(&self) -> bool {
        self.lowered.has_service_storage_metadata()
    }
}

#[cfg(test)]
mod tests {
    use skiff_compiler_input_model::{ActorDeclarationInput, ActorFieldInput};
    use skiff_syntax::ast::TypeRef;

    use super::*;

    #[test]
    fn compiled_actor_fact_keeps_module_id_and_bootstrap_shape() {
        let fact = CompiledActorDeclaration::new(
            "docs",
            ActorDeclarationInput {
                name: "DocHub".to_string(),
                id_type: TypeRef {
                    name: "DocId".to_string(),
                },
                fields: vec![ActorFieldInput {
                    name: "nextSeq".to_string(),
                    ty: TypeRef {
                        name: "number".to_string(),
                    },
                }],
            },
        );
        assert_eq!(fact.module_path, "docs");
        assert_eq!(fact.declaration.id_type.name, "DocId");
        assert_eq!(fact.declaration.fields[0].name, "nextSeq");
    }
}
