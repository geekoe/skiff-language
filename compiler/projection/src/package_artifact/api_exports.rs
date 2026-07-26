use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use skiff_artifact_model::{
    ConstDeclarationIr, DbDeclarationIr, ExecutableIr, ExecutableKind, FileIrUnit,
    FunctionTypeParamIr, InterfaceDeclIr, InterfaceOperationIr, NamedUnionBranchIr,
    NominalTypeRefBaseIr, PackageRefIr, PackageSymbolRef, ServiceSymbolRef, SourceSpanRef,
    TypeDeclIr, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_projection_input::{
    ProjectionSourceSymbolKey, ProjectionView, PublicSymbolKindProjection,
    PublicationApiProjectionSeed,
};

use crate::error::ProjectionError;

/// Typed package API projection: public entries, public-path to source-symbol
/// bindings, and explicit public-instance leaves.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageExports {
    pub entries: Vec<PackageExportEntry>,
    pub symbols: BTreeMap<String, PackageExportSymbol>,
    pub service_call_functions: BTreeSet<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub public_instances: Vec<PackageExportPublicInstance>,
}

pub type PackageExportsProjection = PackageExports;

#[derive(Debug, Clone, Serialize)]
pub struct PackageExportEntry {
    pub path: String,
    pub module: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageExportSymbol {
    pub module: String,
    pub symbol: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageExportPublicInstance {
    pub public_path: String,
    pub module: String,
    pub const_symbol: String,
    pub receiver_module: String,
    pub receiver_symbol: String,
    pub interfaces: Vec<PackageExportPublicInstanceInterface>,
    pub service_call: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageExportPublicInstanceInterface {
    pub module: String,
    pub symbol: String,
    pub methods: Vec<PackageExportPublicInstanceMethod>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageExportPublicInstanceMethod {
    pub name: String,
    pub executable_module: String,
    pub executable_symbol: String,
}

struct PackageArtifactProjectionContext<'a> {
    package_id: &'a str,
}

#[derive(Clone, Copy, Debug)]
enum PackageBoundaryKind {
    PackageLinkEntry,
    PackageSchema,
    PersistentSchema,
}

impl PackageBoundaryKind {
    fn allows_any_interface(self) -> bool {
        matches!(self, Self::PackageLinkEntry | Self::PackageSchema)
    }

    fn description(self) -> &'static str {
        match self {
            Self::PackageLinkEntry => "package link entry boundary",
            Self::PackageSchema => "package public schema boundary",
            Self::PersistentSchema => "persistent payload schema",
        }
    }

    fn allows_applied_nominal(self) -> bool {
        matches!(self, Self::PackageLinkEntry)
    }
}

pub(super) fn project_package_exports(
    input: ProjectionView<'_>,
    package_id: &str,
) -> Result<PackageExportsProjection, ProjectionError> {
    let manifest = PackageArtifactProjectionContext { package_id };
    let sources_by_module = input
        .file_ir_units()
        .iter()
        .map(|unit| (unit.module_path.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    let type_index =
        PackageApiTypeIndex::new(input.file_ir_units(), input.source().publication_api_seed())?;
    let mut symbols = BTreeMap::new();
    let mut abi_violations = BTreeSet::new();

    for symbol in input.source().export_bindings().public_symbols().values() {
        let Some(unit) = sources_by_module.get(symbol.source_module.as_str()) else {
            return Err(ProjectionError::InvalidPackageArtifact {
                message: format!(
                    "package {} api {} has no matching source module {}",
                    package_id, symbol.public_path, symbol.source_module
                ),
            });
        };
        if !matches!(symbol.kind, PublicSymbolKindProjection::Function) {
            collect_package_api_symbol_abi_violations(
                &manifest,
                &type_index,
                unit,
                &symbol.source_symbol,
                &symbol.public_path,
                PackageBoundaryKind::PackageSchema,
                &mut abi_violations,
            );
        }
        insert_package_api_symbol(
            &mut symbols,
            symbol.public_path.clone(),
            symbol.source_module.clone(),
            symbol.source_symbol.clone(),
        );
    }
    for callable in input.source().export_bindings().public_callables().values() {
        let Some(unit) = sources_by_module.get(callable.source_module.as_str()) else {
            return Err(ProjectionError::InvalidPackageArtifact {
                message: format!(
                    "package {} api {} has no matching source module {}",
                    package_id, callable.public_path, callable.source_module
                ),
            });
        };
        collect_package_api_symbol_abi_violations(
            &manifest,
            &type_index,
            unit,
            &callable.source_symbol,
            &callable.public_path,
            PackageBoundaryKind::PackageLinkEntry,
            &mut abi_violations,
        );
        insert_package_api_symbol(
            &mut symbols,
            callable.public_path.clone(),
            callable.source_module.clone(),
            callable.source_symbol.clone(),
        );
    }
    if !abi_violations.is_empty() {
        return Err(ProjectionError::InvalidPackageArtifact {
            message: abi_violations
                .into_iter()
                .map(|violation| format!("- {violation}"))
                .collect::<Vec<_>>()
                .join("\n"),
        });
    }

    let entries = input
        .source()
        .export_bindings()
        .module_exports()
        .iter()
        .map(|entry| PackageExportEntry {
            path: entry.public_path.clone(),
            module: entry.source_module.clone(),
        })
        .collect();

    let public_instances = input
        .source()
        .export_bindings()
        .public_instances()
        .values()
        .map(|public_instance| {
            let interfaces = public_instance
                .interfaces
                .iter()
                .map(|interface| PackageExportPublicInstanceInterface {
                    module: interface.interface.module_path().to_string(),
                    symbol: interface.interface.symbol().to_string(),
                    methods: interface
                        .methods
                        .iter()
                        .map(|method| PackageExportPublicInstanceMethod {
                            name: method.method.clone(),
                            executable_module: method.executable.module_path().to_string(),
                            executable_symbol: method.executable.symbol().to_string(),
                        })
                        .collect(),
                })
                .collect();
            Ok(PackageExportPublicInstance {
                public_path: public_instance.public_path.clone(),
                module: public_instance.source_module.clone(),
                const_symbol: public_instance.source_symbol.clone(),
                receiver_module: public_instance.receiver.module_path().to_string(),
                receiver_symbol: public_instance.receiver.symbol().to_string(),
                interfaces,
                service_call: public_instance.service_call,
            })
        })
        .collect::<Result<Vec<_>, ProjectionError>>()?;

    Ok(PackageExports {
        entries,
        symbols,
        service_call_functions: input
            .source()
            .export_bindings()
            .public_callables()
            .values()
            .filter(|callable| callable.service_call)
            .map(|callable| callable.public_path.clone())
            .collect(),
        public_instances,
    })
}

fn insert_package_api_symbol(
    symbols: &mut BTreeMap<String, PackageExportSymbol>,
    public_id: String,
    module: String,
    symbol: impl Into<String>,
) {
    symbols.insert(
        public_id,
        PackageExportSymbol {
            module,
            symbol: symbol.into(),
        },
    );
}

#[derive(Clone, Copy)]
struct PackageApiTypeBinding<'a> {
    unit: &'a FileIrUnit,
    module_path: &'a str,
    name: &'a str,
    /// True when this type is present in the boundary `link_targets` set, i.e. it
    /// is linkable across the package/service boundary (either re-exported or a
    /// closure member). Distinct from being a public *writable* name.
    exported: bool,
    decl: PackageApiTypeDecl<'a>,
}

#[derive(Clone, Copy)]
enum PackageApiTypeDecl<'a> {
    Type(&'a TypeDeclIr),
    Alias(&'a TypeDeclIr),
    Interface(&'a InterfaceDeclIr),
    Db(&'a DbDeclarationIr),
}

impl PackageApiTypeBinding<'_> {
    fn source_key(&self) -> ProjectionSourceSymbolKey {
        ProjectionSourceSymbolKey::new(self.module_path, self.name)
    }

    fn qualified_name(&self) -> String {
        projection_source_symbol_text(&self.source_key())
    }

    fn is_alias_decl(&self) -> bool {
        matches!(self.decl, PackageApiTypeDecl::Alias(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PackageApiPublicTypePathKey {
    public_module_path: String,
    symbol: String,
}

impl PackageApiPublicTypePathKey {
    fn parse(path: &str) -> Option<Self> {
        let path = path.trim();
        if path.is_empty() {
            return None;
        }
        let (public_module_path, symbol) = path.rsplit_once('.').unwrap_or(("", path));
        if symbol.is_empty() {
            return None;
        }
        Some(Self {
            public_module_path: public_module_path.to_string(),
            symbol: symbol.to_string(),
        })
    }
}

struct PackageApiTypeIndex<'a> {
    by_source_key: BTreeMap<ProjectionSourceSymbolKey, PackageApiTypeBinding<'a>>,
    by_module_local_name: BTreeMap<ProjectionSourceSymbolKey, ProjectionSourceSymbolKey>,
    by_module_type_index: BTreeMap<(String, u32), ProjectionSourceSymbolKey>,
    by_service_symbol: BTreeMap<ProjectionSourceSymbolKey, ProjectionSourceSymbolKey>,
    by_public_type_path: BTreeMap<PackageApiPublicTypePathKey, ProjectionSourceSymbolKey>,
}

impl<'a> PackageApiTypeIndex<'a> {
    fn new(
        file_ir_units: &'a [FileIrUnit],
        publication_api_seed: &PublicationApiProjectionSeed,
    ) -> Result<Self, ProjectionError> {
        let mut index = Self {
            by_source_key: BTreeMap::new(),
            by_module_local_name: BTreeMap::new(),
            by_module_type_index: BTreeMap::new(),
            by_service_symbol: BTreeMap::new(),
            by_public_type_path: BTreeMap::new(),
        };
        for unit in file_ir_units {
            let module_path = unit.module_path.as_str();
            for (name, declaration) in &unit.declarations.types {
                let ty = type_decl_by_index(unit, declaration.type_index)?;
                if ty.name != *name {
                    return Err(ProjectionError::InvalidPackageArtifact {
                        message: format!(
                            "package exports projection found mismatched type declaration {}.{} at File IR type index {} (table name {})",
                            unit.module_path, name, declaration.type_index, ty.name
                        ),
                    });
                }
                let decl = if let Some(interface) = unit.declarations.interfaces.get(name) {
                    PackageApiTypeDecl::Interface(interface)
                } else if type_declaration_is_alias(unit, name, declaration.source_span.as_ref()) {
                    PackageApiTypeDecl::Alias(ty)
                } else {
                    PackageApiTypeDecl::Type(ty)
                };
                index.insert_type(
                    PackageApiTypeBinding {
                        unit,
                        module_path,
                        name,
                        exported: unit.link_targets.types.contains_key(name),
                        decl,
                    },
                    declaration.type_index,
                );
            }
            for (name, db) in &unit.declarations.db {
                index.insert(PackageApiTypeBinding {
                    unit,
                    module_path,
                    name,
                    exported: false,
                    decl: PackageApiTypeDecl::Db(db),
                });
            }
        }
        index.insert_public_type_paths(publication_api_seed);
        Ok(index)
    }

    fn insert_type(&mut self, binding: PackageApiTypeBinding<'a>, type_index: u32) {
        let source_key = binding.source_key();
        self.by_module_type_index
            .entry((binding.module_path.to_string(), type_index))
            .or_insert_with(|| source_key.clone());
        self.insert(binding);
    }

    fn insert(&mut self, binding: PackageApiTypeBinding<'a>) {
        let source_key = binding.source_key();
        self.by_module_local_name
            .entry(source_key.clone())
            .or_insert_with(|| source_key.clone());
        self.by_source_key
            .entry(source_key.clone())
            .or_insert(binding);
        self.by_service_symbol
            .entry(ProjectionSourceSymbolKey::new(
                binding.module_path,
                binding.name,
            ))
            .or_insert(source_key);
    }

    fn insert_public_type_paths(&mut self, publication_api_seed: &PublicationApiProjectionSeed) {
        for schema in publication_api_seed.public_schema_types.values() {
            let Some(public_key) = PackageApiPublicTypePathKey::parse(&schema.public_path) else {
                continue;
            };
            self.by_public_type_path
                .entry(public_key)
                .or_insert_with(|| {
                    ProjectionSourceSymbolKey::new(&schema.source_module, &schema.source_symbol)
                });
        }
    }

    fn resolve(&self, current_module: &str, name: &str) -> Option<PackageApiTypeBinding<'a>> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if let Some(root_path) = name.strip_prefix("root.") {
            return self
                .resolve_public_type_path(root_path)
                .or_else(|| self.resolve_source_symbol_path(root_path));
        }
        if let Some(source_key) = self
            .by_module_local_name
            .get(&ProjectionSourceSymbolKey::new(current_module, name))
        {
            return self.resolve_source_key(source_key);
        }
        self.resolve_source_symbol_path(name)
            .or_else(|| self.resolve_public_type_path(name))
    }

    fn resolve_local_type(
        &self,
        unit: &FileIrUnit,
        type_index: u32,
    ) -> Option<PackageApiTypeBinding<'a>> {
        let source_key = self
            .by_module_type_index
            .get(&(unit.module_path.clone(), type_index))?;
        self.resolve_source_key(source_key)
    }

    fn resolve_publication_type(
        &self,
        module_path: &str,
        type_index: u32,
    ) -> Option<PackageApiTypeBinding<'a>> {
        let source_key = self
            .by_module_type_index
            .get(&(module_path.to_string(), type_index))?;
        self.resolve_source_key(source_key)
    }

    fn resolve_service_symbol(
        &self,
        symbol: &ServiceSymbolRef,
    ) -> Option<PackageApiTypeBinding<'a>> {
        let source_key = self.by_service_symbol.get(&ProjectionSourceSymbolKey::new(
            &symbol.module_path,
            &symbol.symbol,
        ))?;
        self.resolve_source_key(source_key)
    }

    fn resolve_package_symbol(
        &self,
        manifest: &PackageArtifactProjectionContext<'_>,
        symbol: &PackageSymbolRef,
    ) -> Option<PackageApiTypeBinding<'a>> {
        match &symbol.package {
            PackageRefIr::PackageId { package_id } if package_id == manifest.package_id => self
                .resolve_public_type_path(&symbol.symbol_path)
                .or_else(|| self.resolve_source_symbol_path(&symbol.symbol_path)),
            PackageRefIr::Dependency { .. } | PackageRefIr::PackageId { .. } => None,
        }
    }

    fn resolve_source_key(
        &self,
        source_key: &ProjectionSourceSymbolKey,
    ) -> Option<PackageApiTypeBinding<'a>> {
        self.by_source_key.get(source_key).copied()
    }

    fn resolve_source_symbol_path(&self, path: &str) -> Option<PackageApiTypeBinding<'a>> {
        let source_key = source_symbol_key_from_path(path)?;
        self.resolve_source_key(&source_key)
    }

    fn resolve_public_type_path(&self, path: &str) -> Option<PackageApiTypeBinding<'a>> {
        let public_key = PackageApiPublicTypePathKey::parse(path)?;
        let source_key = self.by_public_type_path.get(&public_key)?;
        self.resolve_source_key(source_key)
    }
}

fn source_symbol_key_from_path(path: &str) -> Option<ProjectionSourceSymbolKey> {
    let path = path.trim();
    let (module_path, symbol) = path.rsplit_once('.')?;
    if module_path.is_empty() || symbol.is_empty() {
        return None;
    }
    Some(ProjectionSourceSymbolKey::new(module_path, symbol))
}

fn projection_source_symbol_text(source: &ProjectionSourceSymbolKey) -> String {
    format!("{}.{}", source.module_path(), source.symbol())
}

fn collect_package_api_symbol_abi_violations(
    manifest: &PackageArtifactProjectionContext<'_>,
    type_index: &PackageApiTypeIndex<'_>,
    unit: &FileIrUnit,
    source_symbol: &str,
    public_symbol: &str,
    boundary_kind: PackageBoundaryKind,
    violations: &mut BTreeSet<String>,
) {
    let module_path = unit.module_path.as_str();
    let mut visited = BTreeSet::new();
    let public_symbol = if public_symbol.is_empty() {
        source_symbol
    } else {
        public_symbol
    };

    if let Some(function) = find_executable(unit, source_symbol, ExecutableKind::Function) {
        collect_package_executable_abi_violations(
            manifest,
            type_index,
            unit,
            function,
            public_symbol,
            &format!("function {module_path}.{source_symbol}"),
            boundary_kind,
            &mut visited,
            violations,
        );
        return;
    }

    if let Some(binding) = type_index.resolve(module_path, source_symbol) {
        collect_package_exported_type_binding_abi_violations(
            manifest,
            type_index,
            binding,
            public_symbol,
            boundary_kind,
            &mut visited,
            violations,
        );
        return;
    }

    if let Some(constant) = unit.declarations.constants.get(source_symbol) {
        collect_package_const_abi_violations(
            manifest,
            type_index,
            unit,
            constant,
            public_symbol,
            &format!("const {module_path}.{source_symbol} type"),
            boundary_kind,
            violations,
            &mut visited,
        );
        return;
    }

    if let Some((target, method_name)) = source_symbol.rsplit_once('.') {
        if let Some((declaration_target, method)) =
            find_impl_method_executable(unit, target, method_name)
        {
            if let Some(target_binding) = type_index.resolve(module_path, target) {
                collect_package_type_binding_reference_abi_violations(
                    manifest,
                    type_index,
                    target_binding,
                    public_symbol,
                    &format!("impl {module_path}.{declaration_target} target"),
                    boundary_kind,
                    &mut visited,
                    violations,
                );
            }
            collect_package_executable_abi_violations(
                manifest,
                type_index,
                unit,
                method,
                public_symbol,
                &format!("impl {module_path}.{declaration_target} method {method_name}"),
                boundary_kind,
                &mut visited,
                violations,
            );
        }
    }
}

fn package_impl_target_matches(target: &str, module_path: &str, local_target: &str) -> bool {
    let target = target.strip_prefix("root.").unwrap_or(target);
    target == local_target || target == format!("{module_path}.{local_target}")
}

fn collect_package_executable_abi_violations(
    manifest: &PackageArtifactProjectionContext<'_>,
    type_index: &PackageApiTypeIndex<'_>,
    unit: &FileIrUnit,
    executable: &ExecutableIr,
    public_symbol: &str,
    context: &str,
    boundary_kind: PackageBoundaryKind,
    visited: &mut BTreeSet<ProjectionSourceSymbolKey>,
    violations: &mut BTreeSet<String>,
) {
    if let Some(self_type) = &executable.self_type {
        collect_package_type_ref_abi_violations(
            manifest,
            type_index,
            unit,
            self_type,
            public_symbol,
            &format!("{context} self"),
            boundary_kind,
            visited,
            violations,
        );
    }
    for param in &executable.params {
        collect_package_type_ref_abi_violations(
            manifest,
            type_index,
            unit,
            &param.ty,
            public_symbol,
            &format!("{context} param {}", param.name),
            boundary_kind,
            visited,
            violations,
        );
    }
    collect_package_type_ref_abi_violations(
        manifest,
        type_index,
        unit,
        &executable.return_type,
        public_symbol,
        &format!("{context} return type"),
        boundary_kind,
        visited,
        violations,
    );
}

fn collect_package_operation_abi_violations(
    manifest: &PackageArtifactProjectionContext<'_>,
    type_index: &PackageApiTypeIndex<'_>,
    unit: &FileIrUnit,
    operation: &InterfaceOperationIr,
    public_symbol: &str,
    context: &str,
    boundary_kind: PackageBoundaryKind,
    visited: &mut BTreeSet<ProjectionSourceSymbolKey>,
    violations: &mut BTreeSet<String>,
) {
    if let Some(implicit_self) = &operation.implicit_self {
        collect_package_type_ref_abi_violations(
            manifest,
            type_index,
            unit,
            implicit_self,
            public_symbol,
            &format!("{context} self"),
            boundary_kind,
            visited,
            violations,
        );
    }
    for param in &operation.params {
        collect_package_type_ref_abi_violations(
            manifest,
            type_index,
            unit,
            &param.ty,
            public_symbol,
            &format!("{context} param {}", param.name),
            boundary_kind,
            visited,
            violations,
        );
    }
    collect_package_type_ref_abi_violations(
        manifest,
        type_index,
        unit,
        &operation.return_type,
        public_symbol,
        &format!("{context} return type"),
        boundary_kind,
        visited,
        violations,
    );
}

fn collect_package_const_abi_violations(
    manifest: &PackageArtifactProjectionContext<'_>,
    type_index: &PackageApiTypeIndex<'_>,
    unit: &FileIrUnit,
    constant: &ConstDeclarationIr,
    public_symbol: &str,
    context: &str,
    boundary_kind: PackageBoundaryKind,
    violations: &mut BTreeSet<String>,
    visited: &mut BTreeSet<ProjectionSourceSymbolKey>,
) {
    collect_package_type_ref_abi_violations(
        manifest,
        type_index,
        unit,
        &constant.ty,
        public_symbol,
        context,
        boundary_kind,
        visited,
        violations,
    );
}

fn collect_package_type_ref_abi_violations(
    manifest: &PackageArtifactProjectionContext<'_>,
    type_index: &PackageApiTypeIndex<'_>,
    unit: &FileIrUnit,
    ty: &TypeRefIr,
    public_symbol: &str,
    context: &str,
    boundary_kind: PackageBoundaryKind,
    visited: &mut BTreeSet<ProjectionSourceSymbolKey>,
    violations: &mut BTreeSet<String>,
) {
    match ty {
        TypeRefIr::Builtin { args, .. } => {
            for arg in args {
                collect_package_type_ref_abi_violations(
                    manifest,
                    type_index,
                    unit,
                    arg,
                    public_symbol,
                    context,
                    boundary_kind,
                    visited,
                    violations,
                );
            }
        }
        TypeRefIr::AppliedNominal { base, arguments } => {
            if !boundary_kind.allows_applied_nominal() {
                violations.insert(format!(
                    "package {} api {public_symbol} exposes generic nominal type via {context}; generic nominals cannot be part of {}",
                    manifest.package_id,
                    boundary_kind.description()
                ));
                return;
            }
            collect_package_nominal_base_abi_violations(
                manifest,
                type_index,
                unit,
                base,
                public_symbol,
                context,
                boundary_kind,
                visited,
                violations,
            );
            for (index, argument) in arguments.iter().enumerate() {
                collect_package_type_ref_abi_violations(
                    manifest,
                    type_index,
                    unit,
                    argument,
                    public_symbol,
                    &format!("{context} applied nominal argument {index}"),
                    boundary_kind,
                    visited,
                    violations,
                );
            }
        }
        TypeRefIr::LocalType {
            type_index: local_type_index,
        } => {
            if let Some(binding) = type_index.resolve_local_type(unit, *local_type_index) {
                collect_package_type_binding_reference_abi_violations(
                    manifest,
                    type_index,
                    binding,
                    public_symbol,
                    context,
                    boundary_kind,
                    visited,
                    violations,
                );
            }
        }
        TypeRefIr::PublicationType {
            module_path,
            type_index: publication_type_index,
        } => {
            if let Some(binding) =
                type_index.resolve_publication_type(module_path, *publication_type_index)
            {
                collect_package_type_binding_reference_abi_violations(
                    manifest,
                    type_index,
                    binding,
                    public_symbol,
                    context,
                    boundary_kind,
                    visited,
                    violations,
                );
            }
        }
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            if let Some(binding) = type_index.resolve_service_symbol(symbol) {
                collect_package_type_binding_reference_abi_violations(
                    manifest,
                    type_index,
                    binding,
                    public_symbol,
                    context,
                    boundary_kind,
                    visited,
                    violations,
                );
            }
        }
        TypeRefIr::PackageSymbol { symbol } => {
            if let Some(binding) = type_index.resolve_package_symbol(manifest, symbol) {
                collect_package_type_binding_reference_abi_violations(
                    manifest,
                    type_index,
                    binding,
                    public_symbol,
                    context,
                    boundary_kind,
                    visited,
                    violations,
                );
            }
        }
        TypeRefIr::Record { fields } => {
            for (name, field) in fields {
                collect_package_type_ref_abi_violations(
                    manifest,
                    type_index,
                    unit,
                    field,
                    public_symbol,
                    &format!("{context} record field {name}"),
                    boundary_kind,
                    visited,
                    violations,
                );
            }
        }
        TypeRefIr::Union { items } => {
            for item in items {
                collect_package_type_ref_abi_violations(
                    manifest,
                    type_index,
                    unit,
                    item,
                    public_symbol,
                    context,
                    boundary_kind,
                    visited,
                    violations,
                );
            }
        }
        TypeRefIr::Nullable { inner } => {
            collect_package_type_ref_abi_violations(
                manifest,
                type_index,
                unit,
                inner,
                public_symbol,
                context,
                boundary_kind,
                visited,
                violations,
            );
        }
        TypeRefIr::AnyInterface { interface } => {
            if !boundary_kind.allows_any_interface() {
                violations.insert(format!(
                    "package {} api {public_symbol} exposes any interface type {} via {context}; any interface values cannot be part of {}",
                    manifest.package_id,
                    interface.interface_abi_id,
                    boundary_kind.description()
                ));
            }
            for (index, arg) in interface.canonical_type_args.iter().enumerate() {
                collect_package_type_ref_abi_violations(
                    manifest,
                    type_index,
                    unit,
                    arg,
                    public_symbol,
                    &format!("{context} any interface type argument {index}"),
                    boundary_kind,
                    visited,
                    violations,
                );
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for param in params {
                collect_package_function_type_param_abi_violations(
                    manifest,
                    type_index,
                    unit,
                    param,
                    public_symbol,
                    context,
                    boundary_kind,
                    visited,
                    violations,
                );
            }
            collect_package_type_ref_abi_violations(
                manifest,
                type_index,
                unit,
                return_type,
                public_symbol,
                &format!("{context} function return type"),
                boundary_kind,
                visited,
                violations,
            );
        }
        TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::PackageSchema { .. } => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_package_nominal_base_abi_violations(
    manifest: &PackageArtifactProjectionContext<'_>,
    type_index: &PackageApiTypeIndex<'_>,
    unit: &FileIrUnit,
    base: &NominalTypeRefBaseIr,
    public_symbol: &str,
    context: &str,
    boundary_kind: PackageBoundaryKind,
    visited: &mut BTreeSet<ProjectionSourceSymbolKey>,
    violations: &mut BTreeSet<String>,
) {
    let binding = match base {
        NominalTypeRefBaseIr::LocalType {
            type_index: local_type_index,
        } => type_index.resolve_local_type(unit, *local_type_index),
        NominalTypeRefBaseIr::PublicationType {
            module_path,
            type_index: publication_type_index,
        } => type_index.resolve_publication_type(module_path, *publication_type_index),
        NominalTypeRefBaseIr::ServiceSymbol { symbol } => type_index.resolve_service_symbol(symbol),
        NominalTypeRefBaseIr::PackageSymbol { symbol } => {
            type_index.resolve_package_symbol(manifest, symbol)
        }
        NominalTypeRefBaseIr::PackageSchema { .. } => {
            violations.insert(format!(
                "package {} api {public_symbol} exposes an applied PackageSchema via {context}; current PackageSchema generation does not admit applied nominals",
                manifest.package_id
            ));
            return;
        }
    };
    if let Some(binding) = binding {
        collect_package_type_binding_reference_abi_violations(
            manifest,
            type_index,
            binding,
            public_symbol,
            context,
            boundary_kind,
            visited,
            violations,
        );
    }
}

fn collect_package_function_type_param_abi_violations(
    manifest: &PackageArtifactProjectionContext<'_>,
    type_index: &PackageApiTypeIndex<'_>,
    unit: &FileIrUnit,
    param: &FunctionTypeParamIr,
    public_symbol: &str,
    context: &str,
    boundary_kind: PackageBoundaryKind,
    visited: &mut BTreeSet<ProjectionSourceSymbolKey>,
    violations: &mut BTreeSet<String>,
) {
    collect_package_type_ref_abi_violations(
        manifest,
        type_index,
        unit,
        &param.ty,
        public_symbol,
        &format!("{context} function param {}", param.name),
        boundary_kind,
        visited,
        violations,
    );
}

fn collect_package_type_binding_reference_abi_violations(
    manifest: &PackageArtifactProjectionContext<'_>,
    type_index: &PackageApiTypeIndex<'_>,
    binding: PackageApiTypeBinding<'_>,
    public_symbol: &str,
    context: &str,
    boundary_kind: PackageBoundaryKind,
    visited: &mut BTreeSet<ProjectionSourceSymbolKey>,
    violations: &mut BTreeSet<String>,
) {
    // Per the re-export model (doc §5): a type reachable from a re-exported
    // symbol's signature is part of the ABI/schema closure and is therefore
    // LINKABLE (present in `link_targets`, i.e. `binding.exported`) even when it
    // is not itself a re-exported public writable name (`binding.public`). Such
    // closure-only references are legal and must NOT be rejected here.
    //
    // The only genuinely broken case is a referenced type that is declared in the
    // publication but is NOT linkable across the boundary at all (not in the
    // closure / link_targets). That signals the public symbol leaks a type the
    // boundary cannot encode. Aliases are exempt: they re-expand to their target.
    if !binding.exported && !binding.is_alias_decl() {
        violations.insert(format!(
            "package {} api {public_symbol} exposes non-linkable package type {} via {context}",
            manifest.package_id,
            binding.qualified_name()
        ));
        return;
    }
    collect_package_exported_type_binding_abi_violations(
        manifest,
        type_index,
        binding,
        public_symbol,
        boundary_kind,
        visited,
        violations,
    );
}

fn collect_package_exported_type_binding_abi_violations(
    manifest: &PackageArtifactProjectionContext<'_>,
    type_index: &PackageApiTypeIndex<'_>,
    binding: PackageApiTypeBinding<'_>,
    public_symbol: &str,
    boundary_kind: PackageBoundaryKind,
    visited: &mut BTreeSet<ProjectionSourceSymbolKey>,
    violations: &mut BTreeSet<String>,
) {
    let source_key = binding.source_key();
    if !visited.insert(source_key.clone()) {
        return;
    }
    let qualified_name = projection_source_symbol_text(&source_key);
    match binding.decl {
        PackageApiTypeDecl::Type(ty) => {
            if !ty.type_params.is_empty() && !boundary_kind.allows_applied_nominal() {
                violations.insert(format!(
                    "package {} api {public_symbol} exposes generic declaration {qualified_name}; generic declarations cannot be part of {}",
                    manifest.package_id,
                    boundary_kind.description()
                ));
                return;
            }
            collect_package_type_descriptor_abi_violations(
                manifest,
                type_index,
                binding.unit,
                ty,
                public_symbol,
                &format!("type {qualified_name}"),
                boundary_kind,
                visited,
                violations,
            );
        }
        PackageApiTypeDecl::Alias(alias) => {
            collect_package_type_descriptor_abi_violations(
                manifest,
                type_index,
                binding.unit,
                alias,
                public_symbol,
                &format!("alias {qualified_name}"),
                boundary_kind,
                visited,
                violations,
            );
        }
        PackageApiTypeDecl::Interface(interface) => {
            if !interface.type_params.is_empty() && !boundary_kind.allows_applied_nominal() {
                violations.insert(format!(
                    "package {} api {public_symbol} exposes generic interface {qualified_name}; generic declarations cannot be part of {}",
                    manifest.package_id,
                    boundary_kind.description()
                ));
                return;
            }
            for operation in &interface.operations {
                collect_package_operation_abi_violations(
                    manifest,
                    type_index,
                    binding.unit,
                    operation,
                    public_symbol,
                    &format!("interface {qualified_name} operation {}", operation.name),
                    boundary_kind,
                    visited,
                    violations,
                );
            }
        }
        PackageApiTypeDecl::Db(db) => {
            collect_package_type_ref_abi_violations(
                manifest,
                type_index,
                binding.unit,
                &db.key.ty,
                public_symbol,
                &format!("db {qualified_name} key {}", db.key.name),
                PackageBoundaryKind::PersistentSchema,
                visited,
                violations,
            );
            for field in &db.fields {
                collect_package_type_ref_abi_violations(
                    manifest,
                    type_index,
                    binding.unit,
                    &field.ty,
                    public_symbol,
                    &format!("db {qualified_name} field {}", field.name),
                    PackageBoundaryKind::PersistentSchema,
                    visited,
                    violations,
                );
            }
        }
    }
}

fn collect_package_type_descriptor_abi_violations(
    manifest: &PackageArtifactProjectionContext<'_>,
    type_index: &PackageApiTypeIndex<'_>,
    unit: &FileIrUnit,
    ty: &TypeDeclIr,
    public_symbol: &str,
    context: &str,
    boundary_kind: PackageBoundaryKind,
    visited: &mut BTreeSet<ProjectionSourceSymbolKey>,
    violations: &mut BTreeSet<String>,
) {
    for implemented in &ty.implements {
        collect_package_type_ref_abi_violations(
            manifest,
            type_index,
            unit,
            implemented,
            public_symbol,
            &format!("{context} implements"),
            boundary_kind,
            visited,
            violations,
        );
    }
    match &ty.descriptor {
        TypeDescriptorIr::Record { fields } => {
            for (name, field) in fields {
                collect_package_type_ref_abi_violations(
                    manifest,
                    type_index,
                    unit,
                    field,
                    public_symbol,
                    &format!("{context} field {name}"),
                    boundary_kind,
                    visited,
                    violations,
                );
            }
        }
        TypeDescriptorIr::Alias { target } => {
            collect_package_type_ref_abi_violations(
                manifest,
                type_index,
                unit,
                target,
                public_symbol,
                &format!("{context} target"),
                boundary_kind,
                visited,
                violations,
            );
        }
        TypeDescriptorIr::Representation { representation } => {
            collect_package_type_ref_abi_violations(
                manifest,
                type_index,
                unit,
                representation,
                public_symbol,
                &format!("{context} representation"),
                boundary_kind,
                visited,
                violations,
            );
        }
        TypeDescriptorIr::Union { branches } => {
            for (index, branch) in branches.iter().enumerate() {
                let (ty, branch_kind) = match branch {
                    NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
                        (Some(nominal_type), "concrete nominal")
                    }
                    NamedUnionBranchIr::SyntheticDiscriminator { payload_type, .. } => {
                        (Some(payload_type), "synthetic discriminator")
                    }
                    NamedUnionBranchIr::Literal { .. } => (None, "literal"),
                };
                if let Some(ty) = ty {
                    collect_package_type_ref_abi_violations(
                        manifest,
                        type_index,
                        unit,
                        ty,
                        public_symbol,
                        &format!("{context} {branch_kind} branch {index}"),
                        boundary_kind,
                        visited,
                        violations,
                    );
                }
            }
        }
        TypeDescriptorIr::Interface => {}
    }
}

fn type_decl_by_index(unit: &FileIrUnit, type_index: u32) -> Result<&TypeDeclIr, ProjectionError> {
    unit.type_table.get(type_index as usize).ok_or_else(|| {
        ProjectionError::InvalidPackageArtifact {
            message: format!(
                "package exports projection found missing type index {type_index} in module {}",
                unit.module_path
            ),
        }
    })
}

fn type_declaration_is_alias(
    unit: &FileIrUnit,
    name: &str,
    source_span: Option<&SourceSpanRef>,
) -> bool {
    let Some(source_span) = source_span else {
        return false;
    };
    unit.source_map.spans.iter().any(|span| {
        span.kind == "alias" && span.name.as_deref() == Some(name) && span.span == *source_span
    })
}

fn find_executable<'a>(
    unit: &'a FileIrUnit,
    declaration_name: &str,
    kind: ExecutableKind,
) -> Option<&'a ExecutableIr> {
    let declaration = unit.declarations.executables.get(declaration_name)?;
    unit.executables
        .get(declaration.executable_index as usize)
        .filter(|executable| executable.kind == kind)
}

fn find_impl_method_executable<'a>(
    unit: &'a FileIrUnit,
    local_target: &str,
    method_name: &str,
) -> Option<(String, &'a ExecutableIr)> {
    let expected = format!("{local_target}.{method_name}");
    if let Some(executable) = find_executable(unit, &expected, ExecutableKind::ImplMethod) {
        return Some((local_target.to_string(), executable));
    }

    unit.declarations
        .executables
        .iter()
        .filter_map(|(declaration_name, declaration)| {
            let (target, method) = declaration_name.rsplit_once('.')?;
            if method != method_name
                || !package_impl_target_matches(target, &unit.module_path, local_target)
            {
                return None;
            }
            let executable = unit
                .executables
                .get(declaration.executable_index as usize)?;
            (executable.kind == ExecutableKind::ImplMethod)
                .then_some((target.to_string(), executable))
        })
        .next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::InterfaceInstantiationRef;

    fn manifest_for_boundary_test() -> PackageArtifactProjectionContext<'static> {
        PackageArtifactProjectionContext {
            package_id: "example.com/package",
        }
    }

    fn any_interface_type() -> TypeRefIr {
        TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: "iface:Provider".to_string(),
                canonical_type_args: Vec::new(),
            },
        }
    }

    fn collect_any_interface_errors(
        boundary_kind: PackageBoundaryKind,
        ty: &TypeRefIr,
    ) -> Vec<String> {
        let manifest = manifest_for_boundary_test();
        let unit = FileIrUnit::empty("pkg.main", "hash");
        let type_index = PackageApiTypeIndex::new(
            std::slice::from_ref(&unit),
            &PublicationApiProjectionSeed::default(),
        )
        .expect("empty package type index should build");
        let mut visited = BTreeSet::new();
        let mut violations = BTreeSet::new();

        collect_package_type_ref_abi_violations(
            &manifest,
            &type_index,
            &unit,
            ty,
            "entry",
            "test boundary",
            boundary_kind,
            &mut visited,
            &mut violations,
        );

        violations.into_iter().collect()
    }

    #[test]
    fn package_link_entry_boundary_allows_any_interface_values() {
        let ty = TypeRefIr::Record {
            fields: BTreeMap::from([("provider".to_string(), any_interface_type())]),
        };

        let violations = collect_any_interface_errors(PackageBoundaryKind::PackageLinkEntry, &ty);

        assert!(
            violations.is_empty(),
            "unexpected violations: {violations:?}"
        );
    }

    #[test]
    fn package_schema_boundary_allows_any_interface_values() {
        let violations =
            collect_any_interface_errors(PackageBoundaryKind::PackageSchema, &any_interface_type());

        assert!(
            violations.is_empty(),
            "unexpected violations: {violations:?}"
        );
    }

    #[test]
    fn persistent_schema_boundary_rejects_any_interface_values() {
        let violations = collect_any_interface_errors(
            PackageBoundaryKind::PersistentSchema,
            &any_interface_type(),
        );

        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("persistent payload schema"));
    }

    #[test]
    fn fully_instantiated_applied_nominal_is_valid_for_package_link_entry() {
        let applied = TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: "example.com/models".to_string(),
                    },
                    symbol_path: "Box".to_string(),
                    abi_expectation: Some("abi:models".to_string()),
                },
            },
            arguments: vec![TypeRefIr::builtin("string")],
        };

        let violations =
            collect_any_interface_errors(PackageBoundaryKind::PackageLinkEntry, &applied);

        assert!(
            violations.is_empty(),
            "unexpected package-local violations: {violations:?}"
        );
    }

    #[test]
    fn applied_nominal_fails_closed_for_public_and_persistent_schema() {
        let applied = TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: "example.com/models".to_string(),
                    },
                    symbol_path: "Box".to_string(),
                    abi_expectation: Some("abi:models".to_string()),
                },
            },
            arguments: vec![TypeRefIr::builtin("string")],
        };

        for boundary_kind in [
            PackageBoundaryKind::PackageSchema,
            PackageBoundaryKind::PersistentSchema,
        ] {
            let violations = collect_any_interface_errors(boundary_kind, &applied);
            assert_eq!(violations.len(), 1);
            assert!(violations[0].contains("generic nominal"));
            assert!(violations[0].contains(boundary_kind.description()));
        }
    }
}
