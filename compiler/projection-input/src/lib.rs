use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use skiff_artifact_model::{
    AbiAliasId, AbiInterfaceId, AbiTypeId, ActorMetadataIr, DbMetadataIr, FileIrUnit,
    InterfaceMethodSignature, ServiceDependencyConstraint, TypeRefIr,
};
use skiff_compiler_core::source_role::PublicationSourceRole;

#[derive(Debug, Clone)]
pub struct ProjectionInput {
    file_ir_units: Vec<FileIrUnit>,
    source_metadata: Vec<ProjectionSourceMetadata>,
    source: ProjectionSourceFacts,
    lowering: ProjectionLoweringFacts,
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
    ) -> Self {
        Self {
            file_ir_units,
            source_metadata,
            source,
            lowering,
        }
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

    pub fn service_ingress(&self) -> Option<&'a ServiceIngressProjection> {
        self.source().service_ingress()
    }
}

#[derive(Debug, Clone)]
pub struct PackageProjectionInput {
    info: PackagePublicationProjectionInfo,
    compiled: ProjectionInput,
    dependency_config: Value,
}

#[derive(Debug, Clone)]
pub struct PackageProjectionInputParts {
    pub info: PackagePublicationProjectionInfo,
    pub compiled: ProjectionInput,
    pub dependency_config: Value,
}

impl PackageProjectionInput {
    pub fn new(parts: PackageProjectionInputParts) -> Self {
        Self {
            info: parts.info,
            compiled: parts.compiled,
            dependency_config: parts.dependency_config,
        }
    }

    pub fn id(&self) -> &str {
        self.info.id()
    }

    pub fn version(&self) -> &str {
        self.info.version()
    }

    pub fn dependencies(&self) -> &[PackageDependencyProjectionInfo] {
        self.info.dependencies()
    }

    pub fn manifest(&self) -> &PackagePublicationProjectionInfo {
        &self.info
    }

    pub fn source_root(&self) -> &Path {
        self.info.source_root()
    }

    pub fn api_entries(&self) -> &[PackageApiEntryProjectionInfo] {
        self.info.api_entries()
    }

    pub fn api_source(&self) -> Option<&PackageApiSourceProjectionInfo> {
        self.info.api_source()
    }

    pub fn config(&self) -> &Value {
        &self.dependency_config
    }

    pub fn compiled(&self) -> ProjectionView<'_> {
        self.compiled.view()
    }
}

#[derive(Debug, Clone)]
pub struct PackagePublicationProjectionInfo {
    id: String,
    version: String,
    dependencies: Vec<PackageDependencyProjectionInfo>,
    api_entries: Vec<PackageApiEntryProjectionInfo>,
    api_source: Option<PackageApiSourceProjectionInfo>,
    source_root: PathBuf,
    provenance: PackagePublicationProjectionProvenance,
}

impl PackagePublicationProjectionInfo {
    pub fn new(
        id: String,
        version: String,
        dependencies: Vec<PackageDependencyProjectionInfo>,
        api_entries: Vec<PackageApiEntryProjectionInfo>,
        api_source: Option<PackageApiSourceProjectionInfo>,
        source_root: PathBuf,
        provenance: PackagePublicationProjectionProvenance,
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

    pub fn dependencies(&self) -> &[PackageDependencyProjectionInfo] {
        &self.dependencies
    }

    pub fn api_entries(&self) -> &[PackageApiEntryProjectionInfo] {
        &self.api_entries
    }

    pub fn api_source(&self) -> Option<&PackageApiSourceProjectionInfo> {
        self.api_source.as_ref()
    }

    pub fn source_root(&self) -> &Path {
        &self.source_root
    }

    pub fn provenance(&self) -> &PackagePublicationProjectionProvenance {
        &self.provenance
    }
}

#[derive(Debug, Clone)]
pub struct PackagePublicationProjectionProvenance {
    synthetic: bool,
}

impl PackagePublicationProjectionProvenance {
    pub fn new(synthetic: bool) -> Self {
        Self { synthetic }
    }

    pub fn synthetic(&self) -> bool {
        self.synthetic
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageDependencyProjectionInfo {
    id: String,
    version: String,
    alias: Option<String>,
    config: Value,
    collection_name_mapping: BTreeMap<String, String>,
}

impl PackageDependencyProjectionInfo {
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
}

#[derive(Debug, Clone)]
pub struct PackageApiEntryProjectionInfo {
    path: String,
    module: String,
}

impl PackageApiEntryProjectionInfo {
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
pub struct PackageApiSourceProjectionInfo {
    relative_path: PathBuf,
    content_hash: String,
}

impl PackageApiSourceProjectionInfo {
    pub fn new(relative_path: PathBuf, content_hash: String) -> Self {
        Self {
            relative_path,
            content_hash,
        }
    }

    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
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
    service_ingress: Option<ServiceIngressProjection>,
    service_dependencies: ServiceDependencyProjectionFacts,
}

#[derive(Debug, Clone)]
pub struct ProjectionSourceFactsParts {
    pub publication_api_seed: PublicationApiProjectionSeed,
    pub export_bindings: ExportBindingProjection,
    pub config_requirements: ConfigRequirementsSeed,
    pub abi_ids: BTreeMap<ProjectionDeclarationKey, ProjectionAbiDeclarationIds>,
    pub service_ingress: Option<ServiceIngressProjection>,
    pub service_dependencies: ServiceDependencyProjectionFacts,
}

impl ProjectionSourceFacts {
    pub fn new(parts: ProjectionSourceFactsParts) -> Self {
        Self {
            publication_api_seed: parts.publication_api_seed,
            export_bindings: parts.export_bindings,
            config_requirements: parts.config_requirements,
            abi_ids: parts.abi_ids,
            service_ingress: parts.service_ingress,
            service_dependencies: parts.service_dependencies,
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

    pub fn service_ingress(&self) -> Option<&ServiceIngressProjection> {
        self.service_ingress.as_ref()
    }

    pub fn service_dependencies(&self) -> &ServiceDependencyProjectionFacts {
        &self.service_dependencies
    }

    pub fn abi_ids(&self) -> &BTreeMap<ProjectionDeclarationKey, ProjectionAbiDeclarationIds> {
        &self.abi_ids
    }
}

#[derive(Debug, Clone, Default)]
pub struct ServiceDependencyProjectionFacts {
    constraints: Vec<ServiceDependencyConstraint>,
    dependency_lock: Vec<Value>,
}

impl ServiceDependencyProjectionFacts {
    pub fn new(constraints: Vec<ServiceDependencyConstraint>, dependency_lock: Vec<Value>) -> Self {
        Self {
            constraints,
            dependency_lock,
        }
    }

    pub fn constraints(&self) -> &[ServiceDependencyConstraint] {
        &self.constraints
    }

    pub fn dependency_lock(&self) -> &[Value] {
        &self.dependency_lock
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
    pub interfaces: Vec<ExportPublicInstanceInterfaceProjection>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExportPublicInstanceInterfaceProjection {
    pub source_module: String,
    pub source_symbol: String,
    pub implements_interface: bool,
    pub canonical_type_args: Vec<TypeRefIr>,
    pub package_interface_identity: Option<TypeRefIr>,
    pub package_interface_methods: Vec<InterfaceMethodSignature>,
    pub receiver_implements_package_interface: bool,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceIngressProjection {
    pub package_aliases: BTreeMap<String, String>,
    pub http: Option<ServiceHttpIngressProjection>,
    pub websocket: Option<ServiceWebSocketIngressProjection>,
}

impl ServiceIngressProjection {
    pub fn http(&self) -> Option<&ServiceHttpIngressProjection> {
        self.http.as_ref()
    }

    pub fn websocket(&self) -> Option<&ServiceWebSocketIngressProjection> {
        self.websocket.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceHttpIngressProjection {
    pub entry_target: Option<String>,
    pub guard: Option<ServiceIngressHandlerProjection>,
    pub pre: Option<ServiceIngressHandlerProjection>,
    pub routes: Vec<ServiceHttpRouteIngressProjection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceHttpRouteIngressProjection {
    pub method: Option<String>,
    pub path: String,
    pub handler: ServiceIngressHandlerProjection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceWebSocketIngressProjection {
    pub target: Option<String>,
    pub connect: Option<ServiceIngressHandlerProjection>,
    pub receive: Option<ServiceIngressHandlerProjection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceIngressHandlerProjection {
    ServiceFunction {
        source: String,
        module_path: String,
        symbol: String,
    },
    PackageFunction {
        source: String,
        package_id: String,
        alias: String,
        symbol_path: String,
    },
}

impl ServiceIngressHandlerProjection {
    pub fn source(&self) -> &str {
        match self {
            Self::ServiceFunction { source, .. } | Self::PackageFunction { source, .. } => source,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProjectionFileArtifactSource {
    pub unit: FileIrUnit,
    pub source: ProjectionSourceMetadata,
}
