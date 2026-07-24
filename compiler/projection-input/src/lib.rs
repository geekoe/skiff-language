use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use skiff_artifact_model::{
    package_schema_descriptor_refs, AbiAliasId, AbiInterfaceId, AbiTypeId, ActorMetadataIr,
    CallableSemanticFacts, ContractTypeNameability, DbMetadataIr, FileIrUnit, PackageArtifact,
    PackageBuildId, PackageLocalAbiIdentity, PackageRequirement, PackageSchemaIndex,
    PackageSchemaIndexRef, PackageSchemaTypeId, PackageSchemaTypeRecord, TypeRefIr,
};
use skiff_compiler_core::source_role::PublicationSourceRole;

mod callable_effects;
mod package_callable_signatures;

pub use callable_effects::{ProjectionCallableEffectFacts, ProjectionExecutableKey};
pub use package_callable_signatures::{
    canonical_package_public_path, DuplicateProjectionPackageCallableSignature,
    ProjectionPackageCallableKey, ProjectionPackageCallableSignatureFacts,
};

/// Store-verified schema facts for one exact package dependency binding.
///
/// The compiler driver owns filesystem access and constructs this value only
/// after resolving a canonical PackageArtifact schema. Projection receives
/// immutable DTOs and cannot reopen or infer schema records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPackageSchema {
    alias: String,
    package_id: String,
    exact_version: String,
    package_build_id: PackageBuildId,
    expected_local_abi: PackageLocalAbiIdentity,
    index: PackageSchemaIndex,
    records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResolvedPackageSchemaError {
    #[error("resolved package schema binding {alias} has empty alias")]
    EmptyAlias { alias: String },
    #[error(
        "resolved package schema binding {alias} owner mismatch: binding {package_id}, index {index_package_id}"
    )]
    IndexOwnerMismatch {
        alias: String,
        package_id: String,
        index_package_id: String,
    },
    #[error(
        "resolved package schema binding {alias} entry {stable_schema_key} is not an api.yml public named type"
    )]
    NonPublicNamedType {
        alias: String,
        stable_schema_key: String,
    },
    #[error(
        "resolved package schema binding {alias} entry {stable_schema_key} has no exact type record"
    )]
    MissingTypeRecord {
        alias: String,
        stable_schema_key: String,
    },
    #[error(
        "resolved package schema binding {alias} entry {stable_schema_key} does not match its type record"
    )]
    TypeRecordMismatch {
        alias: String,
        stable_schema_key: String,
    },
    #[error(
        "resolved package schema binding {alias} closure is missing {package_id}:{stable_schema_key}:{package_schema_type_id}"
    )]
    MissingClosureRecord {
        alias: String,
        package_id: String,
        stable_schema_key: String,
        package_schema_type_id: PackageSchemaTypeId,
    },
    #[error(
        "resolved package schema binding {alias} closure record {package_schema_type_id} does not match owner {package_id} and stable key {stable_schema_key}"
    )]
    ClosureRecordMismatch {
        alias: String,
        package_id: String,
        stable_schema_key: String,
        package_schema_type_id: PackageSchemaTypeId,
    },
    #[error(
        "resolved package schema binding {alias} contains a record outside its reachable closure"
    )]
    ExtraClosureRecord { alias: String },
    #[error("resolved package schema binding {alias} does not match exact requirement: {message}")]
    RequirementMismatch { alias: String, message: String },
    #[error(
        "resolved package schema binding {alias} does not match canonical artifact: {message}"
    )]
    ArtifactMismatch { alias: String, message: String },
}

impl ResolvedPackageSchema {
    pub fn new(
        alias: String,
        package_id: String,
        exact_version: String,
        package_build_id: PackageBuildId,
        expected_local_abi: PackageLocalAbiIdentity,
        index: PackageSchemaIndex,
        records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    ) -> Result<Self, ResolvedPackageSchemaError> {
        if alias.is_empty() {
            return Err(ResolvedPackageSchemaError::EmptyAlias { alias });
        }
        if index.package_id != package_id {
            return Err(ResolvedPackageSchemaError::IndexOwnerMismatch {
                alias,
                package_id,
                index_package_id: index.package_id,
            });
        }
        for (stable_schema_key, entry) in &index.types {
            if entry.public_path.as_deref() != Some(stable_schema_key.as_str())
                || entry.nameability != ContractTypeNameability::PublicNameable
            {
                return Err(ResolvedPackageSchemaError::NonPublicNamedType {
                    alias,
                    stable_schema_key: stable_schema_key.clone(),
                });
            }
            let record = records.get(&entry.package_schema_type_id).ok_or_else(|| {
                ResolvedPackageSchemaError::MissingTypeRecord {
                    alias: alias.clone(),
                    stable_schema_key: stable_schema_key.clone(),
                }
            })?;
            if record.package_id != package_id
                || record.stable_schema_key != *stable_schema_key
                || record.package_schema_type_id != entry.package_schema_type_id
            {
                return Err(ResolvedPackageSchemaError::TypeRecordMismatch {
                    alias,
                    stable_schema_key: stable_schema_key.clone(),
                });
            }
        }
        let mut pending = index
            .types
            .values()
            .map(|entry| entry.package_schema_type_id.clone())
            .collect::<Vec<_>>();
        let mut reachable = BTreeSet::new();
        while let Some(type_id) = pending.pop() {
            if !reachable.insert(type_id.clone()) {
                continue;
            }
            let record = records
                .get(&type_id)
                .expect("index records validated above");
            for reference in package_schema_descriptor_refs(&record.canonical_descriptor.descriptor)
            {
                let child = records
                    .get(&reference.package_schema_type_id)
                    .ok_or_else(|| ResolvedPackageSchemaError::MissingClosureRecord {
                        alias: alias.clone(),
                        package_id: reference.package_id.clone(),
                        stable_schema_key: reference.stable_schema_key.clone(),
                        package_schema_type_id: reference.package_schema_type_id.clone(),
                    })?;
                if child.package_id != reference.package_id
                    || child.stable_schema_key != reference.stable_schema_key
                    || child.package_schema_type_id != reference.package_schema_type_id
                {
                    return Err(ResolvedPackageSchemaError::ClosureRecordMismatch {
                        alias,
                        package_id: reference.package_id,
                        stable_schema_key: reference.stable_schema_key,
                        package_schema_type_id: reference.package_schema_type_id,
                    });
                }
                pending.push(child.package_schema_type_id.clone());
            }
        }
        if reachable.len() != records.len() {
            return Err(ResolvedPackageSchemaError::ExtraClosureRecord { alias });
        }
        Ok(Self {
            alias,
            package_id,
            exact_version,
            package_build_id,
            expected_local_abi,
            index,
            records,
        })
    }

    pub fn alias(&self) -> &str {
        &self.alias
    }

    pub fn package_id(&self) -> &str {
        &self.package_id
    }

    pub fn exact_version(&self) -> &str {
        &self.exact_version
    }

    pub fn package_build_id(&self) -> &PackageBuildId {
        &self.package_build_id
    }

    pub fn expected_local_abi(&self) -> &PackageLocalAbiIdentity {
        &self.expected_local_abi
    }

    pub fn index(&self) -> &PackageSchemaIndex {
        &self.index
    }

    pub fn records(&self) -> &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord> {
        &self.records
    }

    pub fn public_type(
        &self,
        stable_schema_key: &str,
    ) -> Option<(&PackageSchemaTypeId, &PackageSchemaTypeRecord)> {
        let entry = self.index.types.get(stable_schema_key)?;
        let record = self.records.get(&entry.package_schema_type_id)?;
        Some((&entry.package_schema_type_id, record))
    }

    pub fn validate_exact_binding(
        &self,
        requirement: &PackageRequirement,
        artifact: &PackageArtifact,
    ) -> Result<(), ResolvedPackageSchemaError> {
        if self.alias != requirement.alias
            || self.package_id != requirement.package_id
            || self.exact_version != requirement.exact_version
            || self.expected_local_abi != requirement.expected_local_abi
        {
            return Err(ResolvedPackageSchemaError::RequirementMismatch {
                alias: self.alias.clone(),
                message: format!(
                    "schema {}@{} ABI {} versus requirement {}={}@{} ABI {}",
                    self.package_id,
                    self.exact_version,
                    self.expected_local_abi,
                    requirement.alias,
                    requirement.package_id,
                    requirement.exact_version,
                    requirement.expected_local_abi
                ),
            });
        }
        if artifact.package_id != self.package_id
            || artifact.package_version != self.exact_version
            || artifact.package_build_id != self.package_build_id
            || artifact.package_local_abi.local_abi_identity != self.expected_local_abi
            || artifact.package_schema_index
                != (PackageSchemaIndexRef {
                    package_id: self.index.package_id.clone(),
                    package_schema_index_identity: self.index.package_schema_index_identity.clone(),
                })
        {
            return Err(ResolvedPackageSchemaError::ArtifactMismatch {
                alias: self.alias.clone(),
                message: format!(
                    "schema {}@{} build {} ABI {} does not match PackageArtifact {}@{} build {} ABI {}",
                    self.package_id,
                    self.exact_version,
                    self.package_build_id,
                    self.expected_local_abi,
                    artifact.package_id,
                    artifact.package_version,
                    artifact.package_build_id,
                    artifact.package_local_abi.local_abi_identity
                ),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ProjectionInput {
    file_ir_units: Vec<FileIrUnit>,
    source_metadata: Vec<ProjectionSourceMetadata>,
    source: ProjectionSourceFacts,
    lowering: ProjectionLoweringFacts,
    callable_signatures: ProjectionPackageCallableSignatureFacts,
    resources: Vec<PublicationResourceProjectionInput>,
}

#[derive(Clone, Copy, Debug)]
pub struct ProjectionView<'a> {
    input: &'a ProjectionInput,
}

impl ProjectionInput {
    pub fn new(
        file_ir_units: Vec<FileIrUnit>,
        source_metadata: Vec<ProjectionSourceMetadata>,
        source: ProjectionSourceFacts,
        lowering: ProjectionLoweringFacts,
        callable_signatures: ProjectionPackageCallableSignatureFacts,
    ) -> Self {
        Self {
            file_ir_units,
            source_metadata,
            source,
            lowering,
            callable_signatures,
            resources: Vec::new(),
        }
    }

    pub fn new_with_resources(
        file_ir_units: Vec<FileIrUnit>,
        source_metadata: Vec<ProjectionSourceMetadata>,
        source: ProjectionSourceFacts,
        lowering: ProjectionLoweringFacts,
        callable_signatures: ProjectionPackageCallableSignatureFacts,
        resources: Vec<PublicationResourceProjectionInput>,
    ) -> Self {
        Self {
            file_ir_units,
            source_metadata,
            source,
            lowering,
            callable_signatures,
            resources,
        }
    }

    pub fn with_resources(mut self, resources: Vec<PublicationResourceProjectionInput>) -> Self {
        self.resources = resources;
        self
    }

    pub fn view(&self) -> ProjectionView<'_> {
        ProjectionView { input: self }
    }
}

impl<'a> ProjectionView<'a> {
    pub fn file_ir_units(&self) -> &'a [FileIrUnit] {
        &self.input.file_ir_units
    }

    pub fn source_metadata(&self) -> &'a [ProjectionSourceMetadata] {
        &self.input.source_metadata
    }

    pub fn source(&self) -> &'a ProjectionSourceFacts {
        &self.input.source
    }

    pub fn lowering(&self) -> &'a ProjectionLoweringFacts {
        &self.input.lowering
    }

    pub fn callable_signatures(&self) -> &'a ProjectionPackageCallableSignatureFacts {
        &self.input.callable_signatures
    }

    pub fn resources(&self) -> &'a [PublicationResourceProjectionInput] {
        &self.input.resources
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationResourceProjectionInput {
    path: String,
    absolute_path: PathBuf,
    byte_len: u64,
    sha256: String,
    content_type: Option<String>,
}

impl PublicationResourceProjectionInput {
    pub fn new(
        path: String,
        absolute_path: PathBuf,
        byte_len: u64,
        sha256: String,
        content_type: Option<String>,
    ) -> Self {
        Self {
            path,
            absolute_path,
            byte_len,
            sha256,
            content_type,
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn absolute_path(&self) -> &Path {
        &self.absolute_path
    }

    pub fn byte_len(&self) -> u64 {
        self.byte_len
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionSourceMetadata {
    pub source_path: String,
    pub module_path: String,
    pub role: PublicationSourceRole,
    pub source_ast_hash: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectionSourceFacts {
    publication_api_seed: PublicationApiProjectionSeed,
    export_bindings: ExportBindingProjection,
    config_requirements: ConfigRequirementsSeed,
    abi_ids: BTreeMap<ProjectionDeclarationKey, ProjectionAbiDeclarationIds>,
    callable_effects: ProjectionCallableEffectFacts,
    callable_semantic_facts: BTreeMap<ProjectionExecutableKey, CallableSemanticFacts>,
}

#[derive(Debug, Clone)]
pub struct ProjectionSourceFactsParts {
    pub publication_api_seed: PublicationApiProjectionSeed,
    pub export_bindings: ExportBindingProjection,
    pub config_requirements: ConfigRequirementsSeed,
    pub abi_ids: BTreeMap<ProjectionDeclarationKey, ProjectionAbiDeclarationIds>,
    pub callable_effects: ProjectionCallableEffectFacts,
    pub callable_semantic_facts: BTreeMap<ProjectionExecutableKey, CallableSemanticFacts>,
}

impl ProjectionSourceFacts {
    pub fn new(parts: ProjectionSourceFactsParts) -> Self {
        Self {
            publication_api_seed: parts.publication_api_seed,
            export_bindings: parts.export_bindings,
            config_requirements: parts.config_requirements,
            abi_ids: parts.abi_ids,
            callable_effects: parts.callable_effects,
            callable_semantic_facts: parts.callable_semantic_facts,
        }
    }

    pub fn publication_api_seed(&self) -> &PublicationApiProjectionSeed {
        &self.publication_api_seed
    }

    pub fn export_bindings(&self) -> &ExportBindingProjection {
        &self.export_bindings
    }

    pub fn config_requirements(&self) -> &ConfigRequirementsSeed {
        &self.config_requirements
    }

    pub fn abi_ids(&self) -> &BTreeMap<ProjectionDeclarationKey, ProjectionAbiDeclarationIds> {
        &self.abi_ids
    }

    pub fn callable_effects(&self) -> &ProjectionCallableEffectFacts {
        &self.callable_effects
    }

    pub fn callable_semantic_facts(
        &self,
    ) -> &BTreeMap<ProjectionExecutableKey, CallableSemanticFacts> {
        &self.callable_semantic_facts
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProjectionLoweringFacts {
    entrypoint_abi: ProjectionEntrypointAbiIndex,
    synthetic_entrypoints: ProjectionSyntheticEntrypointIndex,
    service_db_metadata: Vec<DbMetadataIr>,
    service_actor_metadata: Vec<ActorMetadataIr>,
    package_entrypoints: PackageEntrypointProjectionFacts,
}

impl ProjectionLoweringFacts {
    pub fn new(
        entrypoint_abi: ProjectionEntrypointAbiIndex,
        synthetic_entrypoints: ProjectionSyntheticEntrypointIndex,
        service_db_metadata: Vec<DbMetadataIr>,
        service_actor_metadata: Vec<ActorMetadataIr>,
        package_entrypoints: PackageEntrypointProjectionFacts,
    ) -> Self {
        Self {
            entrypoint_abi,
            synthetic_entrypoints,
            service_db_metadata,
            service_actor_metadata,
            package_entrypoints,
        }
    }

    pub fn entrypoint_abi(&self) -> &ProjectionEntrypointAbiIndex {
        &self.entrypoint_abi
    }

    pub fn synthetic_entrypoints(&self) -> &ProjectionSyntheticEntrypointIndex {
        &self.synthetic_entrypoints
    }

    pub fn service_db_metadata(&self) -> &[DbMetadataIr] {
        &self.service_db_metadata
    }

    pub fn service_actor_metadata(&self) -> &[ActorMetadataIr] {
        &self.service_actor_metadata
    }

    pub fn package_entrypoints(&self) -> &PackageEntrypointProjectionFacts {
        &self.package_entrypoints
    }
}

#[derive(Clone, Debug, Default)]
pub struct ProjectionEntrypointAbiIndex {
    functions_by_module: BTreeMap<String, BTreeMap<String, EntryFunctionSignature>>,
}

impl ProjectionEntrypointAbiIndex {
    pub fn new(
        functions_by_module: BTreeMap<String, BTreeMap<String, EntryFunctionSignature>>,
    ) -> Self {
        Self {
            functions_by_module,
        }
    }

    pub fn function_signature(
        &self,
        module_path: &str,
        symbol: &str,
    ) -> Option<EntryFunctionSignature> {
        self.functions_by_module
            .get(module_path)
            .and_then(|functions| functions.get(symbol))
            .cloned()
    }
}

#[derive(Debug, Clone)]
pub struct EntryFunctionSignature {
    pub name: String,
    pub params: Vec<EntryParamSpec>,
    pub return_type: EntryTypeSpec,
    pub local_type_names: BTreeMap<u32, String>,
    pub may_suspend: bool,
}

#[derive(Debug, Clone)]
pub struct EntryParamSpec {
    pub name: String,
    pub ty: EntryTypeSpec,
}

#[derive(Debug, Clone)]
pub struct EntryTypeSpec {
    pub name: String,
    pub ir: TypeRefIr,
    pub local_type_names: BTreeMap<u32, String>,
}

#[derive(Debug, Clone)]
pub struct PackageAbiType {
    pub name: String,
    pub descriptor: PackageAbiTypeDescriptor,
    pub discriminator: Option<String>,
    pub local_type_names: BTreeMap<u32, String>,
}

#[derive(Debug, Clone)]
pub enum PackageAbiTypeDescriptor {
    Alias { target: TypeRefIr },
    Union { variants: Vec<TypeRefIr> },
    Record { fields: BTreeMap<String, TypeRefIr> },
    External,
}

#[derive(Clone, Debug, Default)]
pub struct ProjectionSyntheticEntrypointIndex {
    modules: BTreeMap<String, ProjectionSyntheticEntrypointModule>,
}

impl ProjectionSyntheticEntrypointIndex {
    pub fn new(modules: BTreeMap<String, ProjectionSyntheticEntrypointModule>) -> Self {
        Self { modules }
    }

    pub fn module(&self, module_path: &str) -> Option<&ProjectionSyntheticEntrypointModule> {
        self.modules.get(module_path)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ProjectionSyntheticEntrypointModule {
    types: BTreeSet<String>,
    executables: BTreeMap<String, ProjectionSyntheticEntrypointExecutable>,
}

impl ProjectionSyntheticEntrypointModule {
    pub fn new(
        types: BTreeSet<String>,
        executables: BTreeMap<String, ProjectionSyntheticEntrypointExecutable>,
    ) -> Self {
        Self { types, executables }
    }

    pub fn has_type(&self, type_name: &str) -> bool {
        self.types.contains(type_name)
    }

    pub fn executable(
        &self,
        declaration_name: &str,
    ) -> Option<&ProjectionSyntheticEntrypointExecutable> {
        self.executables.get(declaration_name)
    }
}

#[derive(Clone, Debug)]
pub struct ProjectionSyntheticEntrypointExecutable {
    kind: ProjectionSyntheticEntrypointExecutableKind,
    signature: EntryFunctionSignature,
}

impl ProjectionSyntheticEntrypointExecutable {
    pub fn new(
        kind: ProjectionSyntheticEntrypointExecutableKind,
        signature: EntryFunctionSignature,
    ) -> Self {
        Self { kind, signature }
    }

    pub fn kind(&self) -> ProjectionSyntheticEntrypointExecutableKind {
        self.kind
    }

    pub fn signature(&self) -> &EntryFunctionSignature {
        &self.signature
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionSyntheticEntrypointExecutableKind {
    Function,
    ImplMethod,
}

#[derive(Clone, Debug, Default)]
pub struct PackageEntrypointProjectionFacts {
    functions_by_symbol_path: BTreeMap<String, PackageEntrypointFunctionProjection>,
    schema_type_names_by_module: BTreeMap<String, Vec<String>>,
    schema_abi_types_by_module: BTreeMap<String, Vec<PackageAbiType>>,
}

impl PackageEntrypointProjectionFacts {
    pub fn new(
        functions_by_symbol_path: BTreeMap<String, PackageEntrypointFunctionProjection>,
        schema_type_names_by_module: BTreeMap<String, Vec<String>>,
        schema_abi_types_by_module: BTreeMap<String, Vec<PackageAbiType>>,
    ) -> Self {
        Self {
            functions_by_symbol_path,
            schema_type_names_by_module,
            schema_abi_types_by_module,
        }
    }

    pub fn function(&self, symbol_path: &str) -> Option<&PackageEntrypointFunctionProjection> {
        self.functions_by_symbol_path.get(symbol_path)
    }

    pub fn schema_type_names_for_module(&self, module_path: &str) -> &[String] {
        self.schema_type_names_by_module
            .get(module_path)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn schema_abi_types_for_module(&self, module_path: &str) -> Option<&[PackageAbiType]> {
        self.schema_abi_types_by_module
            .get(module_path)
            .map(Vec::as_slice)
    }
}

#[derive(Clone, Debug)]
pub struct PackageEntrypointFunctionProjection {
    pub source_module: String,
    pub source_symbol: String,
    pub signature: EntryFunctionSignature,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublicationApiProjectionSeed {
    pub public_modules: BTreeMap<String, String>,
    pub public_symbols: BTreeMap<String, PublicSymbolProjection>,
    pub public_callables: BTreeMap<String, PublicCallableProjection>,
    pub public_schema_types: BTreeMap<String, PublicTypeProjection>,
    pub public_instances: BTreeMap<String, PublicInstanceProjection>,
    pub module_exports: Vec<PublicModuleExportProjection>,
    pub publication_schema_symbols: BTreeMap<ProjectionSourceSymbolKey, String>,
    pub publication_callable_symbols: BTreeSet<ProjectionSourceSymbolKey>,
    pub publication_public_instance_symbols: BTreeSet<ProjectionSourceSymbolKey>,
}

#[derive(Debug, Clone, Default)]
pub struct ExportBindingProjection {
    public_symbols: BTreeMap<String, ExportSymbolProjection>,
    public_callables: BTreeMap<String, ExportCallableProjection>,
    public_schema_types: BTreeMap<String, ExportSchemaProjection>,
    public_instances: BTreeMap<String, ExportPublicInstanceProjection>,
    module_exports: Vec<PublicModuleExportProjection>,
}

impl ExportBindingProjection {
    pub fn new(
        public_symbols: BTreeMap<String, ExportSymbolProjection>,
        public_callables: BTreeMap<String, ExportCallableProjection>,
        public_schema_types: BTreeMap<String, ExportSchemaProjection>,
        public_instances: BTreeMap<String, ExportPublicInstanceProjection>,
        module_exports: Vec<PublicModuleExportProjection>,
    ) -> Self {
        Self {
            public_symbols,
            public_callables,
            public_schema_types,
            public_instances,
            module_exports,
        }
    }

    pub fn public_symbols(&self) -> &BTreeMap<String, ExportSymbolProjection> {
        &self.public_symbols
    }

    pub fn public_callables(&self) -> &BTreeMap<String, ExportCallableProjection> {
        &self.public_callables
    }

    pub fn public_schema_types(&self) -> &BTreeMap<String, ExportSchemaProjection> {
        &self.public_schema_types
    }

    pub fn public_instances(&self) -> &BTreeMap<String, ExportPublicInstanceProjection> {
        &self.public_instances
    }

    pub fn module_exports(&self) -> &[PublicModuleExportProjection] {
        &self.module_exports
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportSymbolProjection {
    pub public_path: String,
    pub source_module: String,
    pub source_symbol: String,
    pub kind: PublicSymbolKindProjection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportCallableProjection {
    pub public_path: String,
    pub source_module: String,
    pub source_symbol: String,
    pub kind: PublicCallableKindProjection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportSchemaProjection {
    pub public_path: String,
    pub source_module: String,
    pub source_symbol: String,
    pub kind: PublicTypeKindProjection,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExportPublicInstanceProjection {
    pub public_path: String,
    pub source_module: String,
    pub source_symbol: String,
    pub receiver: ProjectionSourceSymbolKey,
    pub interfaces: Vec<ExportPublicInstanceInterfaceProjection>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExportPublicInstanceInterfaceProjection {
    pub interface: ProjectionSourceSymbolKey,
    pub methods: Vec<ExportPublicInstanceMethodProjection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportPublicInstanceMethodProjection {
    pub method: String,
    pub executable: ProjectionSourceSymbolKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicModuleExportProjection {
    pub public_path: String,
    pub source_module: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicSymbolProjection {
    pub public_path: String,
    pub source_module: String,
    pub source_symbol: String,
    pub kind: PublicSymbolKindProjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PublicSymbolKindProjection {
    Type,
    Alias,
    Interface,
    Function,
    Const,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicCallableProjection {
    pub public_path: String,
    pub source_module: String,
    pub source_symbol: String,
    pub kind: PublicCallableKindProjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicCallableKindProjection {
    Function,
    Method,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicTypeProjection {
    pub public_path: String,
    pub source_module: String,
    pub source_symbol: String,
    pub kind: PublicTypeKindProjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicTypeKindProjection {
    Type,
    Alias,
    Interface,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicInstanceProjection {
    pub public_path: String,
    pub source_module: String,
    pub source_symbol: String,
    pub interfaces: Vec<PublicInstanceInterfaceProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicInstanceInterfaceProjection {
    pub source_module: String,
    pub source_symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectionSourceSymbolKey {
    module_path: String,
    symbol: String,
}

impl ProjectionSourceSymbolKey {
    pub fn new(module_path: impl Into<String>, symbol: impl Into<String>) -> Self {
        Self {
            module_path: module_path.into(),
            symbol: symbol.into(),
        }
    }

    pub fn module_path(&self) -> &str {
        &self.module_path
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProjectionSourceDeclarationKind {
    Type,
    Alias,
    Interface,
    Function,
    Const,
    DbObject,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProjectionDeclarationKey {
    source: ProjectionSourceSymbolKey,
    kind: ProjectionSourceDeclarationKind,
}

impl ProjectionDeclarationKey {
    pub fn new(source: &ProjectionSourceSymbolKey, kind: ProjectionSourceDeclarationKind) -> Self {
        Self {
            source: source.clone(),
            kind,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProjectionAbiDeclarationIds {
    pub type_id: Option<AbiTypeId>,
    pub alias_id: Option<AbiAliasId>,
    pub interface_id: Option<AbiInterfaceId>,
}

#[derive(Clone, Debug, Default)]
pub struct ConfigRequirementsSeed {
    legacy: ConfigRequirementSetProjection,
    own: ConfigRequirementSetProjection,
    dependency: ConfigRequirementSetProjection,
    effective: ConfigRequirementSetProjection,
}

impl ConfigRequirementsSeed {
    pub fn new(
        legacy: ConfigRequirementSetProjection,
        own: ConfigRequirementSetProjection,
        dependency: ConfigRequirementSetProjection,
        effective: ConfigRequirementSetProjection,
    ) -> Self {
        Self {
            legacy,
            own,
            dependency,
            effective,
        }
    }

    pub fn legacy(&self) -> &ConfigRequirementSetProjection {
        &self.legacy
    }

    pub fn own(&self) -> &ConfigRequirementSetProjection {
        &self.own
    }

    pub fn dependency(&self) -> &ConfigRequirementSetProjection {
        &self.dependency
    }

    pub fn effective(&self) -> &ConfigRequirementSetProjection {
        &self.effective
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigRequirementSetProjection {
    requirements: Vec<ConfigRequirementProjection>,
}

impl ConfigRequirementSetProjection {
    pub fn new(requirements: Vec<ConfigRequirementProjection>) -> Self {
        Self { requirements }
    }

    pub fn requirements(&self) -> &[ConfigRequirementProjection] {
        &self.requirements
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigRequirementProjection {
    pub scope: ConfigRequirementScopeProjection,
    pub path: String,
    pub access: ConfigRequirementAccessProjection,
    pub provenances: Vec<ConfigRequirementProvenanceProjection>,
}

impl ConfigRequirementProjection {
    pub fn scope(&self) -> &ConfigRequirementScopeProjection {
        &self.scope
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn access(&self) -> &ConfigRequirementAccessProjection {
        &self.access
    }

    pub fn provenances(&self) -> &[ConfigRequirementProvenanceProjection] {
        &self.provenances
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfigRequirementScopeProjection {
    Service,
    Package { package_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfigRequirementAccessProjection {
    Require { ty: String },
    Optional { ty: String },
    Has,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigRequirementProvenanceProjection {
    pub source_path: String,
    pub source_span: Option<ConfigSourceSpanProjection>,
    pub declaring_publication: Option<ConfigRequirementPublicationProjection>,
    pub dependency_path: Vec<ConfigRequirementDependencyStepProjection>,
}

impl ConfigRequirementProvenanceProjection {
    pub fn source_path(&self) -> &str {
        &self.source_path
    }

    pub fn source_span(&self) -> Option<ConfigSourceSpanProjection> {
        self.source_span
    }

    pub fn declaring_publication(&self) -> Option<&ConfigRequirementPublicationProjection> {
        self.declaring_publication.as_ref()
    }

    pub fn dependency_path(&self) -> &[ConfigRequirementDependencyStepProjection] {
        &self.dependency_path
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConfigSourceSpanProjection {
    pub start: ConfigSourcePositionProjection,
    pub end: ConfigSourcePositionProjection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConfigSourcePositionProjection {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigRequirementPublicationProjection {
    pub id: String,
    pub version: String,
}

impl ConfigRequirementPublicationProjection {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigRequirementDependencyStepProjection {
    pub id: String,
    pub version: String,
    pub alias: Option<String>,
}

impl ConfigRequirementDependencyStepProjection {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }
}

#[derive(Debug, Clone)]
pub struct ProjectionFileArtifactSource {
    pub unit: FileIrUnit,
    pub source: ProjectionSourceMetadata,
}

#[cfg(test)]
mod resolved_package_schema_tests {
    use super::*;
    use serde_json::json;
    use skiff_artifact_model::{
        ContractTypeDescriptor, PackageSchemaCanonicalDescriptor, PackageSchemaIndexEntry,
    };

    fn schema(
        public_path: Option<&str>,
        nameability: ContractTypeNameability,
    ) -> Result<ResolvedPackageSchema, ResolvedPackageSchemaError> {
        let type_id = PackageSchemaTypeId::new("type:user");
        ResolvedPackageSchema::new(
            "models".to_string(),
            "example.com/models".to_string(),
            "1.2.3".to_string(),
            PackageBuildId::new("build"),
            PackageLocalAbiIdentity::new("abi"),
            PackageSchemaIndex {
                package_id: "example.com/models".to_string(),
                package_schema_index_identity: "index".into(),
                types: BTreeMap::from([(
                    "api.User".to_string(),
                    PackageSchemaIndexEntry {
                        package_schema_type_id: type_id.clone(),
                        public_path: public_path.map(str::to_string),
                        nameability,
                    },
                )]),
            },
            BTreeMap::from([(
                type_id.clone(),
                PackageSchemaTypeRecord {
                    package_id: "example.com/models".to_string(),
                    stable_schema_key: "api.User".to_string(),
                    package_schema_type_id: type_id,
                    canonical_descriptor: PackageSchemaCanonicalDescriptor {
                        type_params: Vec::new(),
                        descriptor: ContractTypeDescriptor::Record {
                            fields: BTreeMap::new(),
                        },
                    },
                },
            )]),
        )
    }

    fn closure_schema(
        records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
        root_id: PackageSchemaTypeId,
    ) -> Result<ResolvedPackageSchema, ResolvedPackageSchemaError> {
        ResolvedPackageSchema::new(
            "providers".to_string(),
            "example.com/providers".to_string(),
            "1.2.3".to_string(),
            PackageBuildId::new("build"),
            PackageLocalAbiIdentity::new("abi"),
            PackageSchemaIndex {
                package_id: "example.com/providers".to_string(),
                package_schema_index_identity: "index".into(),
                types: BTreeMap::from([(
                    "Provider".to_string(),
                    PackageSchemaIndexEntry {
                        package_schema_type_id: root_id,
                        public_path: Some("Provider".to_string()),
                        nameability: ContractTypeNameability::PublicNameable,
                    },
                )]),
            },
            records,
        )
    }

    #[test]
    fn exposes_only_exact_public_schema_records() {
        let schema = schema(Some("api.User"), ContractTypeNameability::PublicNameable).unwrap();
        let (type_id, record) = schema.public_type("api.User").unwrap();
        assert_eq!(type_id, &record.package_schema_type_id);
        assert_eq!(schema.alias(), "models");
        assert_eq!(schema.package_id(), "example.com/models");
        assert_eq!(schema.exact_version(), "1.2.3");
    }

    #[test]
    fn rejects_package_schema_record_without_api_public_path() {
        let error = schema(None, ContractTypeNameability::ClosureOnly).unwrap_err();
        assert!(matches!(
            error,
            ResolvedPackageSchemaError::NonPublicNamedType { .. }
        ));
    }

    #[test]
    fn exact_binding_rejects_wrong_abi_and_build() {
        let schema = schema(Some("api.User"), ContractTypeNameability::PublicNameable).unwrap();
        let artifact = serde_json::from_value::<PackageArtifact>(json!({
            "schemaVersion": "skiff-package-artifact-v2",
            "packageId": "example.com/models",
            "packageVersion": "1.2.3",
            "packageBuildId": "wrong-build",
            "files": [],
            "staticResources": [],
            "packageLocalAbi": { "localAbiIdentity": "abi", "publicSymbols": {} },
            "packageSchemaIndex": {
                "packageId": "example.com/models",
                "packageSchemaIndexIdentity": "index"
            },
            "packageSchemaTypeRecords": {
                "type:user": {
                    "packageId": "example.com/models",
                    "packageSchemaTypeId": "type:user"
                }
            },
            "implementationLinks": {},
            "callableLinks": {},
            "packageRequirements": [],
            "contractRequirements": [],
            "serviceRequirements": [],
            "runtimeRequirements": {
                "config": [],
                "resources": [],
                "runtimeCapabilities": []
            },
            "callableSemanticFacts": {},
            "boundaryProjections": {},
            "serviceCallRefs": []
        }))
        .unwrap();
        let requirement = PackageRequirement {
            alias: "models".to_string(),
            package_id: "example.com/models".to_string(),
            exact_version: "1.2.3".to_string(),
            expected_local_abi: PackageLocalAbiIdentity::new("abi"),
        };
        assert!(matches!(
            schema.validate_exact_binding(&requirement, &artifact),
            Err(ResolvedPackageSchemaError::ArtifactMismatch { .. })
        ));

        let wrong_abi = PackageRequirement {
            expected_local_abi: PackageLocalAbiIdentity::new("wrong-abi"),
            ..requirement
        };
        assert!(matches!(
            schema.validate_exact_binding(&wrong_abi, &artifact),
            Err(ResolvedPackageSchemaError::RequirementMismatch { .. })
        ));
    }

    #[test]
    fn accepts_exact_cross_package_transitive_closure_and_rejects_missing_or_extra_records() {
        let leaf_id = PackageSchemaTypeId::new("type:leaf");
        let leaf = PackageSchemaTypeRecord {
            package_id: "example.com/api".to_string(),
            stable_schema_key: "Format".to_string(),
            package_schema_type_id: leaf_id.clone(),
            canonical_descriptor: PackageSchemaCanonicalDescriptor {
                type_params: Vec::new(),
                descriptor: ContractTypeDescriptor::Record {
                    fields: BTreeMap::new(),
                },
            },
        };
        let root_id = PackageSchemaTypeId::new("type:provider");
        let root = PackageSchemaTypeRecord {
            package_id: "example.com/providers".to_string(),
            stable_schema_key: "Provider".to_string(),
            package_schema_type_id: root_id.clone(),
            canonical_descriptor: PackageSchemaCanonicalDescriptor {
                type_params: Vec::new(),
                descriptor: ContractTypeDescriptor::Record {
                    fields: BTreeMap::from([(
                        "format".to_string(),
                        skiff_artifact_model::ContractTypeRef::package_schema(
                            &leaf.package_id,
                            &leaf.stable_schema_key,
                            leaf_id.clone(),
                        ),
                    )]),
                },
            },
        };
        let exact = BTreeMap::from([
            (root_id.clone(), root.clone()),
            (leaf_id.clone(), leaf.clone()),
        ]);
        assert!(closure_schema(exact.clone(), root_id.clone()).is_ok());

        let missing = BTreeMap::from([(root_id.clone(), root.clone())]);
        assert!(matches!(
            closure_schema(missing, root_id.clone()),
            Err(ResolvedPackageSchemaError::MissingClosureRecord { .. })
        ));

        let extra_id = PackageSchemaTypeId::new("type:extra");
        let mut extra = exact.clone();
        extra.insert(
            extra_id.clone(),
            PackageSchemaTypeRecord {
                package_id: "example.com/unused".to_string(),
                stable_schema_key: "Unused".to_string(),
                package_schema_type_id: extra_id,
                canonical_descriptor: PackageSchemaCanonicalDescriptor {
                    type_params: Vec::new(),
                    descriptor: ContractTypeDescriptor::Record {
                        fields: BTreeMap::new(),
                    },
                },
            },
        );
        assert!(matches!(
            closure_schema(extra, root_id.clone()),
            Err(ResolvedPackageSchemaError::ExtraClosureRecord { .. })
        ));

        let mut wrong_owner = exact;
        wrong_owner.get_mut(&leaf_id).unwrap().package_id = "example.com/wrong".to_string();
        assert!(matches!(
            closure_schema(wrong_owner, root_id),
            Err(ResolvedPackageSchemaError::ClosureRecordMismatch { .. })
        ));
    }
}
