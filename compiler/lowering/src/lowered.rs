use std::collections::{BTreeMap, BTreeSet};

use super::storage_projection::CompiledPublicationStorageProjection;
use super::{
    callable_return_types::{extend_callable_return_types_for_source, CallableReturnType},
    source_file_lowering::{
        compile_publication_source_file_ir_unit, PublicationSourceLoweringInput,
    },
    type_ref_ir_source_text_with_local_types, CompiledPublicationSource, EntryFunctionSignature,
    EntryParamSpec, EntryTypeSpec, EntrypointAbiIndex, LoweringDependencyOperationIndexes,
};
use crate::file_ir::{
    assign_file_ir_identity, CallTargetIr, ConstLinkTargetIr, ExecutableIr, ExecutableKind,
    ExecutableLinkTargetIr, ExprIr, FileIrUnit, MetadataValue, TypeDescriptorIr, TypeLinkTargetIr,
    TypeRefIr,
};
use skiff_compiler_core::source_role::PublicationSourceRole;
use skiff_compiler_source::api::PublicSymbolKind;
use skiff_compiler_source::parsed_sources::ParsedCompilerSource;
use skiff_compiler_source::SourceCompileError as PublicationError;
use skiff_compiler_source::SourceCompileModel;
use skiff_compiler_source::{PublicationApiSeed, ServiceIngressHandler, ServiceIngressModel};

#[derive(Debug)]
pub struct LoweredPublication {
    file_ir_units: Vec<FileIrUnit>,
    sources: Vec<CompiledPublicationSource>,
    service_storage_projection: CompiledPublicationStorageProjection,
    diagnostics: LoweringDiagnostics,
    metadata: LoweringMetadata,
    entrypoint_abi: EntrypointAbiIndex,
}

#[derive(Debug, Default)]
pub struct LoweringDiagnostics;

#[derive(Debug, Default)]
pub struct LoweringMetadata {
    synthetic_operations: SyntheticOperationIndex,
}

#[derive(Debug, Clone, Default)]
pub struct SyntheticOperationIndex {
    entrypoints: SyntheticEntrypointIndex,
}

#[derive(Debug, Clone, Default)]
pub struct SyntheticEntrypointIndex {
    modules: BTreeMap<String, SyntheticEntrypointModule>,
}

#[derive(Debug, Clone, Default)]
pub struct SyntheticEntrypointModule {
    types: BTreeSet<String>,
    executables: BTreeMap<String, SyntheticEntrypointExecutable>,
}

#[derive(Debug, Clone)]
pub struct SyntheticEntrypointExecutable {
    kind: SyntheticEntrypointExecutableKind,
    signature: EntryFunctionSignature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntheticEntrypointExecutableKind {
    Function,
    ImplMethod,
}

impl LoweredPublication {
    pub(crate) fn lower(
        model: &SourceCompileModel,
        operation_indexes: &LoweringDependencyOperationIndexes,
    ) -> Result<Self, PublicationError> {
        let plan = model.plan();
        let parsed_sources = model.sources().parsed_sources();
        let mut file_ir_units = Vec::with_capacity(parsed_sources.len());
        let mut sources = Vec::with_capacity(parsed_sources.len());
        let mut callable_return_types = BTreeMap::<String, CallableReturnType>::new();
        for parsed in parsed_sources {
            extend_callable_return_types_for_source(
                &mut callable_return_types,
                parsed.module_path(),
                parsed.ast(),
            );
        }
        let package_interface_methods = model.type_resolution().package_interface_method_index();

        model.with_semantic_context(|semantic_context| {
            for parsed in parsed_sources {
                let source_path = parsed.relative_path().display().to_string();
                let module_path = parsed.module_path();
                let role = model.sources().role_for(parsed);
                let source_semantic_context = semantic_context
                    .source_context(module_path)
                    .map_err(|error| {
                        plan.diagnostics
                            .source_semantic_context_error(&source_path, error)
                    })?;
                let unit =
                    compile_publication_source_file_ir_unit(PublicationSourceLoweringInput {
                        source: parsed.source_text(),
                        role: file_ir_role_for_source_role(role),
                        // pipeline 文档禁止 lowering 重算 name resolution:
                        // package aliases 和 service aliases 必须从 name_resolution model 读,
                        // 不得通过 model.dependencies 重新拿原始数据。
                        package_aliases: model.name_resolution().package_aliases_map(),
                        package_interface_methods: &package_interface_methods,
                        package_operations: operation_indexes.package_operations(),
                        service_dependency_operations: operation_indexes
                            .service_dependency_operations(),
                        external_type_symbols: model.indexes().publication_type_symbols(),
                        service_dependency_aliases: model.name_resolution().service_aliases(),
                        publication_db_metadata: model.indexes().publication_db_metadata_index(),
                        semantic_context: &source_semantic_context,
                        source_alias_targets: model
                            .resolutions()
                            .alias_targets_for_module(module_path),
                        type_resolution: model.type_resolution(),
                        expression_types: Some(model.expression_types()),
                        callable_return_types: &callable_return_types,
                    })
                    .map_err(|error| {
                        plan.diagnostics
                            .source_file_ir_unit_error(&source_path, error)
                    })?;
                sources.push(compiled_publication_source(parsed, role, &unit));
                file_ir_units.push(unit);
            }
            Ok::<(), skiff_compiler_source::SourceCompileError>(())
        })?;

        // File IR `link_targets` (the set of names a package/service can link and
        // encode across its boundary) are no longer driven by the per-declaration
        // `exported` modifier. They are re-derived here from the re-export set plus
        // the ABI/schema closure of those re-exported symbols. See doc §5: a type
        // reachable from a re-exported symbol's signature must be LINKABLE even if
        // it is not itself a public writable name.
        derive_file_ir_link_targets(
            &mut file_ir_units,
            model.publication_api().seed(),
            model.service_ingress(),
        );

        let synthetic_operations = SyntheticOperationIndex::from_file_ir_units(&file_ir_units);
        let entrypoint_abi = EntrypointAbiIndex::build(
            parsed_sources,
            model.name_resolution().package_aliases_map(),
            model.indexes(),
            model.resolutions(),
        )
        .map_err(|message| PublicationError::ContractValidation { message })?;

        Ok(Self {
            file_ir_units,
            sources,
            service_storage_projection: CompiledPublicationStorageProjection::default(),
            diagnostics: LoweringDiagnostics,
            metadata: LoweringMetadata {
                synthetic_operations,
            },
            entrypoint_abi,
        })
    }

    pub fn file_ir_units(&self) -> &[FileIrUnit] {
        &self.file_ir_units
    }

    pub fn file_ir_units_mut(&mut self) -> &mut [FileIrUnit] {
        &mut self.file_ir_units
    }

    pub fn sources(&self) -> &[CompiledPublicationSource] {
        &self.sources
    }

    pub fn set_service_storage_projection(
        &mut self,
        service_storage_projection: CompiledPublicationStorageProjection,
    ) {
        self.service_storage_projection = service_storage_projection;
    }

    pub fn service_db_metadata(&self) -> &[skiff_artifact_model::DbMetadataIr] {
        &self.service_storage_projection.db
    }

    pub fn service_actor_metadata(&self) -> &[skiff_artifact_model::ActorMetadataIr] {
        &self.service_storage_projection.actors
    }

    pub fn has_service_storage_metadata(&self) -> bool {
        !self.service_storage_projection.db.is_empty()
            || !self.service_storage_projection.actors.is_empty()
    }

    #[allow(dead_code)]
    pub fn diagnostics(&self) -> &LoweringDiagnostics {
        &self.diagnostics
    }

    #[allow(dead_code)]
    pub fn metadata(&self) -> &LoweringMetadata {
        &self.metadata
    }

    pub fn synthetic_operations(&self) -> &SyntheticOperationIndex {
        self.metadata.synthetic_operations()
    }

    pub fn entrypoint_abi(&self) -> &EntrypointAbiIndex {
        &self.entrypoint_abi
    }
}

impl LoweringMetadata {
    pub fn synthetic_operations(&self) -> &SyntheticOperationIndex {
        &self.synthetic_operations
    }
}

impl SyntheticOperationIndex {
    fn from_file_ir_units(file_ir_units: &[FileIrUnit]) -> Self {
        Self {
            entrypoints: SyntheticEntrypointIndex::from_file_ir_units(file_ir_units),
        }
    }

    pub fn entrypoints(&self) -> &SyntheticEntrypointIndex {
        &self.entrypoints
    }
}

impl SyntheticEntrypointIndex {
    fn from_file_ir_units(file_ir_units: &[FileIrUnit]) -> Self {
        Self {
            modules: file_ir_units
                .iter()
                .map(|unit| {
                    (
                        unit.module_path.clone(),
                        SyntheticEntrypointModule::from_file_ir_unit(unit),
                    )
                })
                .collect(),
        }
    }

    pub fn module(&self, module_path: &str) -> Option<&SyntheticEntrypointModule> {
        self.modules.get(module_path)
    }

    pub fn modules(&self) -> impl Iterator<Item = (&str, &SyntheticEntrypointModule)> {
        self.modules
            .iter()
            .map(|(module_path, module)| (module_path.as_str(), module))
    }
}

impl SyntheticEntrypointModule {
    fn from_file_ir_unit(unit: &FileIrUnit) -> Self {
        let executables = unit
            .declarations
            .executables
            .iter()
            .filter_map(|(declaration_name, declaration)| {
                let executable = unit
                    .executables
                    .get(declaration.executable_index as usize)?;
                Some((
                    declaration_name.clone(),
                    SyntheticEntrypointExecutable {
                        kind: SyntheticEntrypointExecutableKind::from_file_ir(executable.kind),
                        signature: entry_function_signature_from_executable(
                            unit,
                            declaration_name,
                            executable,
                        ),
                    },
                ))
            })
            .collect();

        Self {
            types: unit.declarations.types.keys().cloned().collect(),
            executables,
        }
    }

    pub fn has_type(&self, type_name: &str) -> bool {
        self.types.contains(type_name)
    }

    pub fn executable(&self, declaration_name: &str) -> Option<&SyntheticEntrypointExecutable> {
        self.executables.get(declaration_name)
    }

    pub fn types(&self) -> impl Iterator<Item = &str> {
        self.types.iter().map(String::as_str)
    }

    pub fn executables(&self) -> impl Iterator<Item = (&str, &SyntheticEntrypointExecutable)> {
        self.executables
            .iter()
            .map(|(declaration_name, executable)| (declaration_name.as_str(), executable))
    }
}

impl SyntheticEntrypointExecutable {
    pub fn kind(&self) -> SyntheticEntrypointExecutableKind {
        self.kind
    }

    pub fn signature(&self) -> &EntryFunctionSignature {
        &self.signature
    }

    pub fn signature_with_name(&self, name: &str) -> EntryFunctionSignature {
        let mut signature = self.signature.clone();
        signature.name = name.to_string();
        signature
    }
}

impl SyntheticEntrypointExecutableKind {
    fn from_file_ir(kind: ExecutableKind) -> Self {
        match kind {
            ExecutableKind::Function => Self::Function,
            ExecutableKind::ImplMethod => Self::ImplMethod,
        }
    }
}

fn entry_function_signature_from_executable(
    unit: &FileIrUnit,
    name: &str,
    executable: &ExecutableIr,
) -> EntryFunctionSignature {
    let params = if executable.kind == ExecutableKind::ImplMethod
        && executable
            .params
            .first()
            .is_some_and(|param| param.name == "self")
    {
        executable.params.get(1..).unwrap_or(&[])
    } else {
        executable.params.as_slice()
    };
    let local_type_names = file_ir_local_type_names(unit);
    EntryFunctionSignature {
        name: name.to_string(),
        params: params
            .iter()
            .map(|param| EntryParamSpec {
                name: param.name.clone(),
                ty: EntryTypeSpec {
                    name: type_ref_ir_source_text(unit, &param.ty),
                    ir: param.ty.clone(),
                    local_type_names: local_type_names.clone(),
                },
            })
            .collect(),
        return_type: EntryTypeSpec {
            name: type_ref_ir_source_text(unit, &executable.return_type),
            ir: executable.return_type.clone(),
            local_type_names: local_type_names.clone(),
        },
        local_type_names,
    }
}

fn type_ref_ir_source_text(unit: &FileIrUnit, ty: &TypeRefIr) -> String {
    type_ref_ir_source_text_with_local_types(ty, &|type_index| {
        unit.type_table
            .get(type_index as usize)
            .map(|ty| ty.name.clone())
    })
}

fn file_ir_local_type_names(unit: &FileIrUnit) -> BTreeMap<u32, String> {
    unit.type_table
        .iter()
        .enumerate()
        .map(|(index, ty)| (index as u32, ty.name.clone()))
        .collect()
}

/// Identifies a type declared somewhere in this publication, by the module that
/// declares it and its File IR `type_table` index.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PublicationTypeLocation {
    unit_index: usize,
    type_index: u32,
}

/// Index of every type/const/executable declared across the publication's File
/// IR units, used to resolve named type references (LocalType / ServiceSymbol /
/// PackageSymbol self-references) to the declaring module while computing the
/// link-target closure. References that do not resolve here (std/prelude/other
/// packages) are external and intentionally skipped.
struct PublicationDeclarationIndex {
    /// (module_path, symbol) -> declaring type location.
    types_by_module_symbol: BTreeMap<(String, String), PublicationTypeLocation>,
    /// (module_path, type_index) -> declaring type location (LocalType lookup).
    types_by_module_index: BTreeMap<(String, u32), PublicationTypeLocation>,
    /// "module.symbol" source-symbol path -> declaring type location
    /// (PackageSymbol self-reference fallback).
    types_by_source_path: BTreeMap<String, PublicationTypeLocation>,
}

impl PublicationDeclarationIndex {
    fn build(units: &[FileIrUnit]) -> Self {
        let mut types_by_module_symbol = BTreeMap::new();
        let mut types_by_module_index = BTreeMap::new();
        let mut types_by_source_path = BTreeMap::new();
        for (unit_index, unit) in units.iter().enumerate() {
            for (symbol, declaration) in &unit.declarations.types {
                let location = PublicationTypeLocation {
                    unit_index,
                    type_index: declaration.type_index,
                };
                types_by_module_symbol
                    .entry((unit.module_path.clone(), symbol.clone()))
                    .or_insert(location);
                types_by_module_index
                    .entry((unit.module_path.clone(), declaration.type_index))
                    .or_insert(location);
                types_by_source_path
                    .entry(format!("{}.{}", unit.module_path, symbol))
                    .or_insert(location);
            }
        }
        Self {
            types_by_module_symbol,
            types_by_module_index,
            types_by_source_path,
        }
    }

    fn resolve_module_symbol(
        &self,
        module_path: &str,
        symbol: &str,
    ) -> Option<PublicationTypeLocation> {
        self.types_by_module_symbol
            .get(&(module_path.to_string(), symbol.to_string()))
            .copied()
    }

    fn resolve_local(&self, module_path: &str, type_index: u32) -> Option<PublicationTypeLocation> {
        self.types_by_module_index
            .get(&(module_path.to_string(), type_index))
            .copied()
    }

    fn resolve_source_path(&self, path: &str) -> Option<PublicationTypeLocation> {
        let path = path.strip_prefix("root.").unwrap_or(path);
        self.types_by_source_path.get(path).copied()
    }
}

/// Re-derive every unit's File IR `link_targets` from the re-export set plus the
/// ABI/schema closure of those re-exported symbols.
///
/// link_targets for a unit become:
///   (re-exported symbols in that unit)
///   ∪ (types in that unit reachable from any re-exported symbol's signature
///      anywhere in the publication, transitively).
fn derive_file_ir_link_targets(
    units: &mut [FileIrUnit],
    seed: &PublicationApiSeed,
    service_ingress: Option<&ServiceIngressModel>,
) {
    let index = PublicationDeclarationIndex::build(units);

    // Seed: re-exported callables (functions / impl methods) become executable
    // link targets, and provide the executable seeds for the closure walk.
    // Re-exported types/aliases/interfaces become type link targets and seed the
    // type closure.
    let mut type_worklist: Vec<PublicationTypeLocation> = Vec::new();
    let mut visited_types: BTreeSet<PublicationTypeLocation> = BTreeSet::new();
    let mut executable_seeds: Vec<(usize, u32)> = Vec::new();

    for source_key in seed.publication_schema_symbols.keys() {
        if let Some(location) =
            index.resolve_module_symbol(source_key.module_path(), source_key.symbol())
        {
            if visited_types.insert(location) {
                type_worklist.push(location);
            }
        }
    }

    for source_key in &seed.publication_callable_symbols {
        let module = source_key.module_path();
        let symbol = source_key.symbol();
        let Some(unit_index) = units.iter().position(|unit| unit.module_path == module) else {
            continue;
        };
        if let Some(declaration) = units[unit_index].declarations.executables.get(symbol) {
            executable_seeds.push((unit_index, declaration.executable_index));
        }
    }
    collect_service_ingress_executable_seeds(units, service_ingress, &mut executable_seeds);
    collect_spawn_executable_seeds(units, &mut executable_seeds);

    // Re-exported constants live in the seed's `public_symbols` map keyed by kind
    // (not in `publication_schema_symbols`/`publication_callable_symbols`). Their
    // declared type seeds the type closure; the const itself is recorded as a
    // const link target below.
    let const_seeds: Vec<(String, String)> = seed
        .public_symbols
        .values()
        .filter(|symbol| matches!(symbol.kind, PublicSymbolKind::Const))
        .map(|symbol| (symbol.source_module.clone(), symbol.source_symbol.clone()))
        .collect();
    for (module, symbol) in &const_seeds {
        let Some(unit_index) = units.iter().position(|unit| &unit.module_path == module) else {
            continue;
        };
        if let Some(declaration) = units[unit_index].declarations.constants.get(symbol) {
            let mut refs = Vec::new();
            collect_type_ref_named_locations(&index, module, &declaration.ty, &mut refs);
            for location in refs {
                if visited_types.insert(location) {
                    type_worklist.push(location);
                }
            }
        }
    }

    // Walk each re-exported callable's signature, collecting referenced types.
    for (unit_index, executable_index) in &executable_seeds {
        let unit = &units[*unit_index];
        let module_path = unit.module_path.clone();
        let Some(executable) = unit.executables.get(*executable_index as usize) else {
            continue;
        };
        let mut refs = Vec::new();
        if let Some(self_type) = &executable.self_type {
            collect_type_ref_named_locations(&index, &module_path, self_type, &mut refs);
        }
        for param in &executable.params {
            collect_type_ref_named_locations(&index, &module_path, &param.ty, &mut refs);
        }
        collect_type_ref_named_locations(&index, &module_path, &executable.return_type, &mut refs);
        for location in refs {
            if visited_types.insert(location) {
                type_worklist.push(location);
            }
        }
    }

    // Fixpoint over the reachable type set.
    while let Some(location) = type_worklist.pop() {
        let unit = &units[location.unit_index];
        let module_path = unit.module_path.clone();
        let Some(ty) = unit.type_table.get(location.type_index as usize) else {
            continue;
        };
        let mut refs = Vec::new();
        for implemented in &ty.implements {
            collect_type_ref_named_locations(&index, &module_path, implemented, &mut refs);
        }
        match &ty.descriptor {
            TypeDescriptorIr::Record { fields } => {
                for field in fields.values() {
                    collect_type_ref_named_locations(&index, &module_path, field, &mut refs);
                }
            }
            TypeDescriptorIr::Alias { target } => {
                collect_type_ref_named_locations(&index, &module_path, target, &mut refs);
            }
            TypeDescriptorIr::Union { variants } => {
                for variant in variants {
                    collect_type_ref_named_locations(&index, &module_path, variant, &mut refs);
                }
            }
            TypeDescriptorIr::Native { .. } => {}
        }
        // Interfaces declared in this module contribute their operation signatures.
        if let Some(interface) = unit.declarations.interfaces.get(&ty.name) {
            for operation in &interface.operations {
                if let Some(implicit_self) = &operation.implicit_self {
                    collect_type_ref_named_locations(
                        &index,
                        &module_path,
                        implicit_self,
                        &mut refs,
                    );
                }
                for param in &operation.params {
                    collect_type_ref_named_locations(&index, &module_path, &param.ty, &mut refs);
                }
                collect_type_ref_named_locations(
                    &index,
                    &module_path,
                    &operation.return_type,
                    &mut refs,
                );
            }
        }
        for referenced in refs {
            if visited_types.insert(referenced) {
                type_worklist.push(referenced);
            }
        }
    }

    // Materialize the closure into per-unit link_targets. Clear first so a
    // recompile is idempotent and never carries stale `.exported`-era entries.
    for unit in units.iter_mut() {
        unit.link_targets.types.clear();
        unit.link_targets.constants.clear();
        unit.link_targets.executables.clear();
    }

    for location in &visited_types {
        let unit = &mut units[location.unit_index];
        let Some(name) = unit
            .type_table
            .get(location.type_index as usize)
            .map(|ty| ty.name.clone())
        else {
            continue;
        };
        unit.link_targets.types.insert(
            name,
            TypeLinkTargetIr {
                type_index: location.type_index,
            },
        );
    }

    // Record each re-exported constant as a const link target (its type closure
    // was already walked into `visited_types` during seeding above).
    for (module, symbol) in &const_seeds {
        let Some(unit_index) = units.iter().position(|unit| &unit.module_path == module) else {
            continue;
        };
        if let Some(const_index) = units[unit_index]
            .declarations
            .constants
            .get(symbol)
            .map(|declaration| declaration.const_index)
        {
            units[unit_index]
                .link_targets
                .constants
                .insert(symbol.clone(), ConstLinkTargetIr { const_index });
        }
    }

    for (unit_index, executable_index) in executable_seeds {
        let unit = &mut units[unit_index];
        if let Some((declaration_name, _)) = unit
            .declarations
            .executables
            .iter()
            .find(|(_, declaration)| declaration.executable_index == executable_index)
        {
            let declaration_name = declaration_name.clone();
            unit.link_targets.executables.insert(
                declaration_name,
                ExecutableLinkTargetIr { executable_index },
            );
        }
    }

    // link_targets feed the File IR identity hash, so recompute it now that the
    // closure has settled.
    for unit in units.iter_mut() {
        assign_file_ir_identity(unit);
    }
}

fn collect_service_ingress_executable_seeds(
    units: &[FileIrUnit],
    service_ingress: Option<&ServiceIngressModel>,
    executable_seeds: &mut Vec<(usize, u32)>,
) {
    let Some(service_ingress) = service_ingress else {
        return;
    };
    if let Some(http) = service_ingress.http() {
        if let Some(target) = http.entry_target.as_deref() {
            push_entry_target_method_seed(units, target, "handle", executable_seeds);
        }
        if let Some(handler) = http.guard.as_ref() {
            push_service_ingress_handler_seed(units, handler, executable_seeds);
        }
        if let Some(handler) = http.pre.as_ref() {
            push_service_ingress_handler_seed(units, handler, executable_seeds);
        }
        for route in &http.routes {
            push_service_ingress_handler_seed(units, &route.handler, executable_seeds);
        }
    }
    if let Some(websocket) = service_ingress.websocket() {
        if let Some(target) = websocket.target.as_deref() {
            push_entry_target_method_seed(units, target, "connect", executable_seeds);
            push_entry_target_method_seed(units, target, "receive", executable_seeds);
        } else {
            if let Some(handler) = websocket.connect.as_ref() {
                push_service_ingress_handler_seed(units, handler, executable_seeds);
            }
            if let Some(handler) = websocket.receive.as_ref() {
                push_service_ingress_handler_seed(units, handler, executable_seeds);
            }
        }
    }
}

fn collect_spawn_executable_seeds(units: &[FileIrUnit], executable_seeds: &mut Vec<(usize, u32)>) {
    for (unit_index, unit) in units.iter().enumerate() {
        for executable in &unit.executables {
            for expr in &executable.body.expressions {
                let ExprIr::Call { call } = expr else {
                    continue;
                };
                if !call
                    .metadata
                    .get("spawnSubmit")
                    .is_some_and(spawn_submit_metadata_is_function)
                {
                    continue;
                }
                match &call.target {
                    CallTargetIr::LocalExecutable { executable_index } => {
                        executable_seeds.push((unit_index, *executable_index));
                    }
                    CallTargetIr::ExternalServiceSymbol { symbol } => {
                        let Some(target_unit_index) = units
                            .iter()
                            .position(|unit| unit.module_path == symbol.module_path)
                        else {
                            continue;
                        };
                        if let Some(declaration) = units[target_unit_index]
                            .declarations
                            .executables
                            .get(&symbol.symbol)
                        {
                            executable_seeds
                                .push((target_unit_index, declaration.executable_index));
                        }
                    }
                    CallTargetIr::PackageSymbol { .. } => {}
                    _ => {}
                }
            }
        }
    }
}

fn spawn_submit_metadata_is_function(metadata: &MetadataValue) -> bool {
    matches!(
        metadata,
        MetadataValue::Object(object)
            if matches!(
                object.get("targetKind"),
                Some(MetadataValue::String(target_kind)) if target_kind == "function"
            )
    )
}

fn push_entry_target_method_seed(
    units: &[FileIrUnit],
    target: &str,
    method: &str,
    executable_seeds: &mut Vec<(usize, u32)>,
) {
    let Some((module_path, type_name)) = target.rsplit_once('.') else {
        return;
    };
    push_executable_seed(
        units,
        module_path,
        &format!("{type_name}.{method}"),
        executable_seeds,
    );
}

fn push_service_ingress_handler_seed(
    units: &[FileIrUnit],
    handler: &ServiceIngressHandler,
    executable_seeds: &mut Vec<(usize, u32)>,
) {
    let ServiceIngressHandler::ServiceFunction {
        module_path,
        symbol,
        ..
    } = handler
    else {
        return;
    };
    push_executable_seed(units, module_path, symbol, executable_seeds);
}

fn push_executable_seed(
    units: &[FileIrUnit],
    module_path: &str,
    symbol: &str,
    executable_seeds: &mut Vec<(usize, u32)>,
) {
    let Some(unit_index) = units
        .iter()
        .position(|unit| unit.module_path == module_path)
    else {
        return;
    };
    let Some(declaration) = units[unit_index].declarations.executables.get(symbol) else {
        return;
    };
    executable_seeds.push((unit_index, declaration.executable_index));
}

/// Append the publication-local declaration locations of every named type
/// referenced by `ty` (transitively through structural type constructors). Refs
/// that resolve outside the publication (std / other packages) are skipped.
fn collect_type_ref_named_locations(
    index: &PublicationDeclarationIndex,
    module_path: &str,
    ty: &TypeRefIr,
    out: &mut Vec<PublicationTypeLocation>,
) {
    match ty {
        TypeRefIr::Native { args, .. } => {
            for arg in args {
                collect_type_ref_named_locations(index, module_path, arg, out);
            }
        }
        TypeRefIr::LocalType { type_index } => {
            if let Some(location) = index.resolve_local(module_path, *type_index) {
                out.push(location);
            }
        }
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            if let Some(location) = index.resolve_module_symbol(&symbol.module_path, &symbol.symbol)
            {
                out.push(location);
            }
        }
        TypeRefIr::PackageSymbol { symbol } => {
            if let Some(location) = index.resolve_source_path(&symbol.symbol_path) {
                out.push(location);
            }
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values() {
                collect_type_ref_named_locations(index, module_path, field, out);
            }
        }
        TypeRefIr::Union { items } => {
            for item in items {
                collect_type_ref_named_locations(index, module_path, item, out);
            }
        }
        TypeRefIr::Nullable { inner } => {
            collect_type_ref_named_locations(index, module_path, inner, out);
        }
        TypeRefIr::AnyInterface { interface } => {
            for arg in &interface.canonical_type_args {
                collect_type_ref_named_locations(index, module_path, arg, out);
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for param in params {
                collect_type_ref_named_locations(index, module_path, &param.ty, out);
            }
            collect_type_ref_named_locations(index, module_path, return_type, out);
        }
        TypeRefIr::Literal { .. } | TypeRefIr::TypeParam { .. } => {}
    }
}

fn compiled_publication_source(
    source: &ParsedCompilerSource,
    role: PublicationSourceRole,
    unit: &FileIrUnit,
) -> CompiledPublicationSource {
    let source_map_source = unit.source_map.sources.first();
    CompiledPublicationSource {
        source_path: source.relative_path().display().to_string(),
        module_path: source.module_path().to_string(),
        role,
        source_ast_hash: source_map_source.and_then(|source| source.source_ast_hash.clone()),
    }
}

fn file_ir_role_for_source_role(role: PublicationSourceRole) -> &'static str {
    match role {
        PublicationSourceRole::Contract => "contract",
        PublicationSourceRole::Implementation => "implementation",
        PublicationSourceRole::Package => "package",
    }
}
