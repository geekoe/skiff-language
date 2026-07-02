use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use serde_json::Value;
use skiff_artifact_model::{ActorMetadataIr, DbMetadataIr, FileIrUnit};
use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;
use skiff_compiler_lowering::{CompiledPublicationSource, LoweredPublication};
use skiff_compiler_source::{
    source_identity::PublicationDeclarationAnchors, CompileParsedPublicationSourcesInput,
    PublicationApiSeed, SourceCompileError, SourceCompileModel, SourceCompilePackageDependencyFact,
    SourceCompilePackageFacts,
};

pub mod projection_input;

#[cfg(feature = "test-support")]
use skiff_compiler_source::{ConfigRequirementSet, ExportBindingModel};

#[derive(Debug)]
pub struct CompiledPublication {
    model: SourceCompileModel,
    lowered: LoweredPublication,
}

#[derive(Debug, Clone)]
pub struct PackagePublication {
    info: PackagePublicationInfo,
    compiled: Arc<CompiledPublication>,
    dependency_config: Value,
}

impl PackagePublication {
    pub fn new(
        info: PackagePublicationInfo,
        compiled: CompiledPublication,
        dependency_config: Value,
    ) -> Self {
        Self {
            info,
            compiled: Arc::new(compiled),
            dependency_config,
        }
    }

    pub fn id(&self) -> &str {
        self.info.id()
    }

    pub fn version(&self) -> &str {
        self.info.version()
    }

    pub fn dependencies(&self) -> &[PackageDependencyInfo] {
        self.info.dependencies()
    }

    pub fn manifest(&self) -> &PackagePublicationInfo {
        &self.info
    }

    pub fn source_root(&self) -> &std::path::Path {
        self.info.source_root()
    }

    pub fn api_entries(&self) -> &[PackageApiEntryInfo] {
        self.info.api_entries()
    }

    pub fn api_source(&self) -> Option<&PackageApiSourceInfo> {
        self.info.api_source()
    }

    pub fn config(&self) -> &Value {
        &self.dependency_config
    }

    pub fn compiled(&self) -> &CompiledPublication {
        self.compiled.as_ref()
    }

    pub fn compiled_arc(&self) -> &Arc<CompiledPublication> {
        &self.compiled
    }

    #[cfg(feature = "test-support")]
    pub fn compiled_arc_mut(&mut self) -> &mut Arc<CompiledPublication> {
        &mut self.compiled
    }
}

#[derive(Debug, Clone)]
pub struct PackagePublicationInfo {
    id: String,
    version: String,
    dependencies: Vec<PackageDependencyInfo>,
    api_entries: Vec<PackageApiEntryInfo>,
    api_source: Option<PackageApiSourceInfo>,
    source_root: PathBuf,
    provenance: PackagePublicationProvenance,
}

impl PackagePublicationInfo {
    pub fn new(
        id: String,
        version: String,
        dependencies: Vec<PackageDependencyInfo>,
        api_entries: Vec<PackageApiEntryInfo>,
        api_source: Option<PackageApiSourceInfo>,
        source_root: PathBuf,
        provenance: PackagePublicationProvenance,
    ) -> Self {
        Self {
            id,
            version,
            dependencies,
            api_entries,
            api_source,
            source_root,
            provenance,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn dependencies(&self) -> &[PackageDependencyInfo] {
        &self.dependencies
    }

    pub fn api_entries(&self) -> &[PackageApiEntryInfo] {
        &self.api_entries
    }

    pub fn api_source(&self) -> Option<&PackageApiSourceInfo> {
        self.api_source.as_ref()
    }

    pub fn source_root(&self) -> &std::path::Path {
        &self.source_root
    }

    pub fn provenance(&self) -> &PackagePublicationProvenance {
        &self.provenance
    }
}

#[derive(Debug, Clone)]
pub struct PackagePublicationProvenance {
    synthetic: bool,
}

impl PackagePublicationProvenance {
    pub fn new(synthetic: bool) -> Self {
        Self { synthetic }
    }

    pub fn synthetic(&self) -> bool {
        self.synthetic
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageDependencyInfo {
    id: String,
    version: String,
    alias: Option<String>,
    config: Value,
    collection_name_mapping: BTreeMap<String, String>,
}

impl PackageDependencyInfo {
    pub fn new(
        id: String,
        version: String,
        alias: Option<String>,
        config: Value,
        collection_name_mapping: BTreeMap<String, String>,
    ) -> Self {
        Self {
            id,
            version,
            alias,
            config,
            collection_name_mapping,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn config(&self) -> &Value {
        &self.config
    }

    pub fn collection_name_mapping(&self) -> &BTreeMap<String, String> {
        &self.collection_name_mapping
    }

    pub fn effective_alias(&self) -> &str {
        self.alias.as_deref().unwrap_or_else(|| {
            if self.id == SKIFF_STD_PUBLICATION_ID {
                "std"
            } else {
                &self.id
            }
        })
    }
}

#[derive(Debug, Clone)]
pub struct PackageApiEntryInfo {
    path: String,
    module: String,
}

impl PackageApiEntryInfo {
    pub fn new(path: String, module: String) -> Self {
        Self { path, module }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn module(&self) -> &str {
        &self.module
    }
}

#[derive(Debug, Clone)]
pub struct PackageApiSourceInfo {
    relative_path: PathBuf,
    content_hash: String,
}

impl PackageApiSourceInfo {
    pub fn new(relative_path: PathBuf, content_hash: String) -> Self {
        Self {
            relative_path,
            content_hash,
        }
    }

    pub fn relative_path(&self) -> &std::path::Path {
        &self.relative_path
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }
}

pub fn compile_parsed_publication_sources(
    input: CompileParsedPublicationSourcesInput<'_, '_>,
) -> Result<CompiledPublication, SourceCompileError> {
    let model = skiff_compiler_source::build_from_parsed_sources(input)?;
    compile_source_model(model)
}

pub fn compile_source_model(
    model: SourceCompileModel,
) -> Result<CompiledPublication, SourceCompileError> {
    let lowered = skiff_compiler_lowering::lower(&model)?;
    Ok(CompiledPublication::new(model, lowered))
}

pub fn source_compile_package_facts_from_publications<'a>(
    package_publications: &'a [PackagePublication],
) -> Vec<SourceCompilePackageFacts<'a>> {
    package_publications
        .iter()
        .map(source_compile_package_fact_from_publication)
        .collect()
}

fn source_compile_package_fact_from_publication<'a>(
    package: &'a PackagePublication,
) -> SourceCompilePackageFacts<'a> {
    SourceCompilePackageFacts::new(
        package.id(),
        package.version(),
        package
            .dependencies()
            .iter()
            .map(|dependency| SourceCompilePackageDependencyFact {
                id: dependency.id().to_string(),
                version: dependency.version().to_string(),
                alias: dependency.alias().map(str::to_string),
            })
            .collect(),
        package.compiled.compile_model(),
        package.compiled.file_ir_units(),
    )
}

impl CompiledPublication {
    pub fn new(model: SourceCompileModel, lowered: LoweredPublication) -> Self {
        Self { model, lowered }
    }

    pub fn compile_model(&self) -> &SourceCompileModel {
        &self.model
    }

    pub fn lowered(&self) -> &LoweredPublication {
        &self.lowered
    }

    pub fn file_ir_units(&self) -> &[FileIrUnit] {
        self.lowered.file_ir_units()
    }

    #[cfg(feature = "test-support")]
    pub fn file_ir_units_mut(&mut self) -> &mut [FileIrUnit] {
        self.lowered.file_ir_units_mut()
    }

    pub fn source_metadata(&self) -> &[CompiledPublicationSource] {
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

    pub fn has_service_storage_metadata(&self) -> bool {
        self.lowered.has_service_storage_metadata()
    }
}
