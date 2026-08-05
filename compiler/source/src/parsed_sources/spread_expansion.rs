//! Record `spread` expansion pass.
//!
//! Expands `TypeDecl.spreads` entries into copied fields on the target record
//! AST before any consumer of record field sets runs (db attachment checks,
//! type resolution, alias/root-ref AST passes). The product is an expanded
//! AST: `TypeDecl.fields` contains the copied fields directly and `spreads` is
//! cleared. Field type texts are qualified in the source record's declaration
//! context so they keep resolving when the target record lives in another
//! module: same-package bare names become `root.<source module>.<Name>` and
//! cross-package sources carry the dependency alias prefix.
//!
//! Rules (all compile errors, fail closed):
//! - the spread source must be a record shape after transparent alias
//!   expansion; representation / named union / interface sources are errors;
//! - generic sources must be fully instantiated (arguments closed, no
//!   reference to the target's own type parameters);
//! - duplicate field names (between spreads, or spread vs explicit) error;
//! - self-spread and cyclic spread chains error, detected on the source
//!   reference graph before expansion;
//! - expanded results are cached per source type so repeated spreads of the
//!   same source parse and qualify once.

use std::collections::{BTreeMap, BTreeSet};

use compiler_input_model::PackageDependency;
use skiff_artifact_model::{
    NominalTypeRefBaseIr, PackageArtifact, PackageLocalAbiSymbol, PackageRefIr, TypeDescriptorIr,
    TypeRefIr,
};
use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;

use crate::{
    package_dependency_facts::SourceCompilePackageFacts,
    parsed_sources::ParsedCompilerSource,
    shared::{
        ast::{AliasDecl, FieldDecl, SourceFile, TypeDecl, TypeRef},
        publication_error::PublicationError,
        type_expr::TypeExpr,
        type_syntax::{generic_parts, split_top_level},
    },
};

/// Expands every record `spread` entry in the parsed source set.
///
/// Cross-package spread sources are resolved through dependency source facts
/// when provided (test harnesses) or through dependency package artifacts
/// (driver pipeline). Sources without spreads are returned untouched so the
/// AST identity (Arc sharing with the caller's sources) is preserved.
pub fn expand_record_spreads<'a>(
    parsed_sources: Vec<ParsedCompilerSource>,
    package_dependencies: &'a [PackageDependency],
    package_facts: Option<&[SourceCompilePackageFacts<'a>]>,
    package_artifacts: Option<&'a [PackageArtifact]>,
) -> Result<Vec<ParsedCompilerSource>, PublicationError> {
    if !parsed_sources
        .iter()
        .any(|parsed| parsed.ast().types.iter().any(|ty| !ty.spreads.is_empty()))
    {
        return Ok(parsed_sources);
    }
    let index = SourceSetIndex::build(&parsed_sources);
    let mut context = ExpansionContext::new(
        index,
        package_dependencies,
        package_facts,
        package_artifacts,
    );
    context.resolve_and_validate_cycles()?;
    let mut expanded_asts = Vec::with_capacity(parsed_sources.len());
    for parsed in &parsed_sources {
        let module_path = parsed.source().module_path.clone();
        let mut ast = parsed.ast().clone();
        let changed = context.expand_source(&module_path, &mut ast)?;
        expanded_asts.push((changed, ast));
    }
    Ok(parsed_sources
        .into_iter()
        .zip(expanded_asts)
        .map(|(parsed, (changed, ast))| {
            if changed {
                parsed.with_expanded_ast(ast)
            } else {
                parsed
            }
        })
        .collect())
}

/// One resolved `spread` entry on a target record.
#[derive(Debug, Clone)]
struct ResolvedSpread {
    source: SpreadSource,
    args: Vec<String>,
}

#[derive(Debug, Clone)]
enum SpreadSource {
    /// A record (or alias-to-record chain) inside the current source set.
    Local { module: String, name: String },
    /// Fields read from a dependency package; type texts already carry the
    /// dependency alias qualification. `type_params` is the source record's
    /// own parameter list (substituted with the spread arguments).
    External {
        fields: Vec<(String, String)>,
        type_params: Vec<String>,
    },
}

struct SourceSetIndex<'a> {
    modules: BTreeMap<String, ModuleIndex<'a>>,
}

struct ModuleIndex<'a> {
    /// All type-namespace declarations of the module (types, aliases,
    /// actors, interfaces), mirroring `local_type_names` used by the
    /// resolution model for field text qualification.
    local_type_names: BTreeSet<String>,
    /// `name` -> source `TypeDecl` (record or representation).
    types: BTreeMap<String, &'a TypeDecl>,
    /// `name` -> transparent alias declaration.
    aliases: BTreeMap<String, &'a AliasDecl>,
    /// Names declared as actors or interfaces (not spreadable shapes).
    non_record_shapes: BTreeSet<String>,
}

impl<'a> SourceSetIndex<'a> {
    fn build(parsed_sources: &'a [ParsedCompilerSource]) -> Self {
        let mut modules = BTreeMap::new();
        for parsed in parsed_sources {
            let source = parsed.source();
            let ast = &source.ast;
            let mut types = BTreeMap::new();
            let mut aliases = BTreeMap::new();
            let mut non_record_shapes = BTreeSet::new();
            let mut local_type_names = BTreeSet::new();
            for ty in &ast.types {
                types.insert(ty.name.clone(), ty);
                local_type_names.insert(ty.name.clone());
            }
            for alias in &ast.aliases {
                aliases.insert(alias.name.clone(), alias);
                local_type_names.insert(alias.name.clone());
            }
            for actor in &ast.actors {
                non_record_shapes.insert(actor.name.clone());
                local_type_names.insert(actor.name.clone());
            }
            for interface in &ast.interfaces {
                non_record_shapes.insert(interface.name.clone());
                local_type_names.insert(interface.name.clone());
            }
            modules.insert(
                source.module_path.clone(),
                ModuleIndex {
                    local_type_names,
                    types,
                    aliases,
                    non_record_shapes,
                },
            );
        }
        Self { modules }
    }

    fn module(&self, module_path: &str) -> Option<&ModuleIndex<'a>> {
        self.modules.get(module_path)
    }
}

struct ExpansionContext<'a, 'facts> {
    index: SourceSetIndex<'a>,
    dependencies: &'a [PackageDependency],
    package_facts: Option<&'facts [SourceCompilePackageFacts<'a>]>,
    package_artifacts: Option<&'a [PackageArtifact]>,
    /// Resolved spread entries per target record, built once up front.
    resolved_spreads: BTreeMap<(String, String), Vec<ResolvedSpread>>,
    /// Expanded field template per source record (type parameters preserved
    /// as bare names, everything else qualified for the source module).
    expanded_fields: BTreeMap<(String, String), Vec<FieldDecl>>,
    /// Artifact type indexes built lazily per dependency package.
    artifact_indexes: std::cell::RefCell<BTreeMap<String, ArtifactTypeIndex>>,
    /// Fact dependency source index built lazily per dependency package.
    fact_indexes: std::cell::RefCell<BTreeMap<String, Vec<FactSource>>>,
}

#[derive(Debug, Clone)]
struct FactSource {
    module_path: String,
    local_type_names: BTreeSet<String>,
    types: Vec<(String, TypeDecl)>,
    aliases: Vec<(String, AliasDecl)>,
}

impl<'a, 'facts> ExpansionContext<'a, 'facts> {
    fn new(
        index: SourceSetIndex<'a>,
        dependencies: &'a [PackageDependency],
        package_facts: Option<&'facts [SourceCompilePackageFacts<'a>]>,
        package_artifacts: Option<&'a [PackageArtifact]>,
    ) -> Self {
        Self {
            index,
            dependencies,
            package_facts,
            package_artifacts,
            resolved_spreads: BTreeMap::new(),
            expanded_fields: BTreeMap::new(),
            artifact_indexes: std::cell::RefCell::new(BTreeMap::new()),
            fact_indexes: std::cell::RefCell::new(BTreeMap::new()),
        }
    }

    fn failure(&self, message: impl Into<String>) -> PublicationError {
        PublicationError::ContractValidation {
            message: message.into(),
        }
    }

    // ------------------------------------------------------------------
    // Phase A: resolve every spread target and validate the reference graph.
    // ------------------------------------------------------------------

    fn resolve_and_validate_cycles(&mut self) -> Result<(), PublicationError> {
        let mut graph: BTreeMap<(String, String), Vec<(String, String)>> = BTreeMap::new();
        for (module_path, module) in &self.index.modules {
            for ty in module.types.values() {
                if ty.spreads.is_empty() {
                    continue;
                }
                let key = (module_path.clone(), ty.name.clone());
                let mut resolved = Vec::new();
                for spread in &ty.spreads {
                    let (source, args) =
                        self.resolve_spread_text(module_path, &spread.name, &mut Vec::new())?;
                    if let SpreadSource::Local { module, name } = &source {
                        graph
                            .entry(key.clone())
                            .or_default()
                            .push((module.clone(), name.clone()));
                    }
                    resolved.push(ResolvedSpread { source, args });
                }
                self.resolved_spreads.insert(key, resolved);
            }
        }
        self.validate_spread_cycles(&graph)?;
        Ok(())
    }

    fn validate_spread_cycles(
        &self,
        graph: &BTreeMap<(String, String), Vec<(String, String)>>,
    ) -> Result<(), PublicationError> {
        let mut state = BTreeMap::<(String, String), VisitState>::new();
        let mut stack = Vec::new();
        for node in graph.keys() {
            if let Some(cycle) = find_spread_cycle(node, graph, &mut state, &mut stack) {
                let chain = cycle
                    .iter()
                    .map(|(module, name)| format!("{module}.{name}"))
                    .collect::<Vec<_>>()
                    .join(" -> ");
                return Err(self.failure(format!(
                    "spread cycle detected: {chain}; cyclic spread chains are not supported"
                )));
            }
        }
        Ok(())
    }

    /// Resolves a spread source text to a local record or an external record.
    /// `seen` guards alias chains; local aliases may chain to other aliases.
    fn resolve_spread_text(
        &self,
        module_path: &str,
        text: &str,
        seen: &mut Vec<String>,
    ) -> Result<(SpreadSource, Vec<String>), PublicationError> {
        if seen.iter().any(|entry| entry == text) {
            return Err(self.failure(format!(
                "spread source `{text}` resolves through a recursive alias chain"
            )));
        }
        let (root, args) = match generic_parts(text) {
            Some(parts) => (
                parts.root.to_string(),
                parts
                    .args
                    .iter()
                    .map(|arg| arg.trim().to_string())
                    .collect::<Vec<_>>(),
            ),
            None => (text.trim().to_string(), Vec::new()),
        };
        if root.is_empty() {
            return Err(self.failure(format!(
                "spread source `{text}` is not a qualified type name"
            )));
        }
        if split_top_level(&root, '|').len() > 1 {
            return Err(self.failure(format!(
                "spread source `{text}` is a named union, which cannot be spread"
            )));
        }
        let target = self.classify_spread_root(module_path, &root)?;
        let source = match target {
            SpreadTarget::Local { module, name } => {
                let module = module.clone();
                let name = name.clone();
                let shape = self.local_shape(&module, &name).ok_or_else(|| {
                    self.failure(format!(
                        "spread source `{text}` is not visible in module `{module_path}`"
                    ))
                })?;
                match shape {
                    LocalShape::Record => SpreadSource::Local { module, name },
                    LocalShape::Alias(alias) => {
                        seen.push(text.to_string());
                        let (source, args) =
                            self.resolve_spread_text(&module, &alias.target_type.name, seen)?;
                        seen.pop();
                        // Argument counts on the alias level are rejected below
                        // against the final record; an alias cannot carry its
                        // own type parameters.
                        return Ok((source, args));
                    }
                    LocalShape::NonRecord => {
                        return Err(self.failure(format!(
                            "spread source `{text}` is not a record; representation, named union, actor, and interface declarations cannot be spread"
                        )));
                    }
                }
            }
            SpreadTarget::External { alias, symbol_path } => {
                let record = self.external_record(&alias, &symbol_path, &mut BTreeSet::new())?;
                SpreadSource::External {
                    fields: record.fields,
                    type_params: record.type_params,
                }
            }
        };
        Ok((source, args))
    }

    fn classify_spread_root(
        &self,
        module_path: &str,
        root: &str,
    ) -> Result<SpreadTarget, PublicationError> {
        if let Some(rest) = root.strip_prefix("root.") {
            let (module, name) = rest.rsplit_once('.').ok_or_else(|| {
                self.failure(format!(
                    "spread source `root.{rest}` must include a module and a symbol"
                ))
            })?;
            return Ok(SpreadTarget::Local {
                module: module.to_string(),
                name: name.to_string(),
            });
        }
        if root.contains('/') {
            let (alias, symbol_path) = root.split_once('/').ok_or_else(|| {
                self.failure(format!(
                    "spread source `{root}` is not a qualified type name"
                ))
            })?;
            if symbol_path.contains('/') || symbol_path.is_empty() {
                return Err(self.failure(format!(
                    "spread source `{root}` is not a qualified dependency type reference"
                )));
            }
            return Ok(SpreadTarget::External {
                alias: alias.to_string(),
                symbol_path: symbol_path.to_string(),
            });
        }
        if let Some((module, name)) = root.rsplit_once('.') {
            if self.index.module(module).is_some() {
                return Ok(SpreadTarget::Local {
                    module: module.to_string(),
                    name: name.to_string(),
                });
            }
        }
        if let Some((alias, symbol_path)) = root.split_once('.') {
            if self.dependency_alias(alias).is_some() || alias == "std" {
                return Ok(SpreadTarget::External {
                    alias: alias.to_string(),
                    symbol_path: symbol_path.to_string(),
                });
            }
            if root.contains('.') {
                return Err(self.failure(format!(
                    "spread source `{root}` is not a declared dependency alias or a module of the current package"
                )));
            }
        }
        Ok(SpreadTarget::Local {
            module: module_path.to_string(),
            name: root.to_string(),
        })
    }

    fn dependency_alias(&self, alias: &str) -> Option<&'a PackageDependency> {
        self.dependencies.iter().find(|dependency| {
            dependency.effective_alias() == alias
                || dependency.top_level_alias.as_deref() == Some(alias)
        })
    }

    fn local_shape<'index>(
        &'index self,
        module_path: &str,
        name: &str,
    ) -> Option<LocalShape<'index>> {
        let module = self.index.module(module_path)?;
        if let Some(ty) = module.types.get(name) {
            if ty.alias.is_none() {
                return Some(LocalShape::Record);
            }
            return Some(LocalShape::NonRecord);
        }
        if let Some(alias) = module.aliases.get(name) {
            return Some(LocalShape::Alias(alias));
        }
        if module.non_record_shapes.contains(name) {
            return Some(LocalShape::NonRecord);
        }
        None
    }

    // ------------------------------------------------------------------
    // External (cross-package) source resolution.
    // ------------------------------------------------------------------

    fn external_record(
        &self,
        alias: &str,
        symbol_path: &str,
        seen_paths: &mut BTreeSet<String>,
    ) -> Result<ExternalRecord, PublicationError> {
        let dependency = if alias == "std" {
            None
        } else {
            self.dependency_alias(alias)
        };
        if let Some(facts) = self.package_facts {
            let fact = facts.iter().find(|fact| {
                dependency.is_some_and(|dependency| {
                    fact.id() == dependency.id && fact.version() == dependency.version
                }) || (dependency.is_none() && fact.id() == SKIFF_STD_PUBLICATION_ID)
            });
            if let Some(fact) = fact {
                return self.external_record_from_facts(alias, fact, symbol_path, seen_paths);
            }
        }
        if let Some(artifacts) = self.package_artifacts {
            let artifact = artifacts.iter().find(|artifact| {
                dependency.is_some_and(|dependency| {
                    artifact.package_id == dependency.id
                        && artifact.package_version == dependency.version
                }) || (dependency.is_none() && artifact.package_id == SKIFF_STD_PUBLICATION_ID)
            });
            if let Some(artifact) = artifact {
                return self.external_record_from_artifact(
                    alias,
                    artifact,
                    symbol_path,
                    seen_paths,
                );
            }
        }
        Err(self.failure(format!(
            "spread source `{alias}.{symbol_path}` cannot be resolved: dependency `{alias}` is not available for source resolution"
        )))
    }

    fn fact_sources(&self, package_id: &str) -> Result<Vec<FactSource>, PublicationError> {
        if let Some(cached) = self.fact_indexes.borrow().get(package_id) {
            return Ok(cached.clone());
        }
        let fact = self
            .package_facts
            .into_iter()
            .flatten()
            .find(|fact| fact.id() == package_id)
            .ok_or_else(|| {
                self.failure("internal spread expansion error: missing package facts")
            })?;
        let sources = fact
            .compile_model()
            .sources()
            .parsed_sources()
            .iter()
            .map(|parsed| {
                let ast = parsed.ast();
                let mut local_type_names = BTreeSet::new();
                let mut types = Vec::new();
                let mut aliases = Vec::new();
                for ty in &ast.types {
                    types.push((ty.name.clone(), ty.clone()));
                    local_type_names.insert(ty.name.clone());
                }
                for alias in &ast.aliases {
                    aliases.push((alias.name.clone(), alias.clone()));
                    local_type_names.insert(alias.name.clone());
                }
                for actor in &ast.actors {
                    local_type_names.insert(actor.name.clone());
                }
                for interface in &ast.interfaces {
                    local_type_names.insert(interface.name.clone());
                }
                FactSource {
                    module_path: parsed.source().module_path.clone(),
                    local_type_names,
                    types,
                    aliases,
                }
            })
            .collect::<Vec<_>>();
        self.fact_indexes
            .borrow_mut()
            .insert(package_id.to_string(), sources.clone());
        Ok(sources)
    }

    fn external_record_from_facts(
        &self,
        alias: &str,
        fact: &SourceCompilePackageFacts<'_>,
        symbol_path: &str,
        seen_paths: &mut BTreeSet<String>,
    ) -> Result<ExternalRecord, PublicationError> {
        let (module, name) = symbol_path.rsplit_once('.').ok_or_else(|| {
            self.failure(format!(
                "spread source `{alias}.{symbol_path}` must include a module and a symbol"
            ))
        })?;
        let sources = self.fact_sources(fact.id())?;
        let source = sources
            .iter()
            .find(|source| source.module_path == module)
            .ok_or_else(|| {
                self.failure(format!(
                    "spread source `{alias}.{symbol_path}` resolves to module `{module}` which does not exist in dependency `{}`",
                    fact.id()
                ))
            })?;
        let ty = source.types.iter().find(|(type_name, _)| type_name == name);
        if let Some((_, ty)) = ty {
            if ty.alias.is_some() {
                return Err(self.failure(format!(
                    "spread source `{alias}.{symbol_path}` is a representation declaration, which cannot be spread"
                )));
            }
            let fields = ty
                .fields
                .iter()
                .map(|field| {
                    Ok((
                        field.name.clone(),
                        self.qualify_external_fact_field_text(
                            alias,
                            module,
                            source,
                            &ty.type_params,
                            &field.ty.name,
                        ),
                    ))
                })
                .collect::<Result<Vec<_>, PublicationError>>()?;
            return Ok(ExternalRecord {
                fields,
                type_params: ty.type_params.clone(),
            });
        }
        if let Some((_, alias_decl)) = source
            .aliases
            .iter()
            .find(|(alias_name, _)| alias_name == name)
        {
            let path = format!("{alias}.{symbol_path}");
            if !seen_paths.insert(path.clone()) {
                return Err(self.failure(format!(
                    "spread source `{path}` resolves through a recursive alias chain"
                )));
            }
            let target = &alias_decl.target_type.name;
            let resolved = if target.contains('.') {
                self.external_record_from_facts(alias, fact, target, seen_paths)?
            } else {
                self.external_record_from_facts(
                    alias,
                    fact,
                    &format!("{module}.{target}"),
                    seen_paths,
                )?
            };
            return Ok(resolved);
        }
        Err(self.failure(format!(
            "spread source `{alias}.{symbol_path}` does not resolve to a record in dependency `{}`",
            fact.id()
        )))
    }

    /// Qualifies a field type text copied from a dependency source AST: bare
    /// names local to the source module become `<alias>.<module>.<Name>`, and
    /// package-internal `root.` paths are rewritten to `<alias>.<rest>`.
    fn qualify_external_fact_field_text(
        &self,
        alias: &str,
        source_module: &str,
        source: &FactSource,
        source_type_params: &[String],
        text: &str,
    ) -> String {
        let type_params = source_type_params.iter().cloned().collect::<BTreeSet<_>>();
        TypeExpr::parse(text)
            .map_named_types(|name| {
                if let Some(rest) = name.strip_prefix("root.") {
                    format!("{alias}.{rest}")
                } else if source.local_type_names.contains(name) && !type_params.contains(name) {
                    format!("{alias}.{source_module}.{name}")
                } else {
                    name.to_string()
                }
            })
            .to_type_string()
    }

    fn artifact_index(
        &self,
        artifact: &'a PackageArtifact,
    ) -> Result<ArtifactTypeIndex, PublicationError> {
        if let Some(cached) = self.artifact_indexes.borrow().get(&artifact.package_id) {
            return Ok(cached.clone());
        }
        let mut index = ArtifactTypeIndex::default();
        for (selected_path, symbol) in &artifact.package_local_abi.public_symbols {
            let PackageLocalAbiSymbol::Type {
                descriptor,
                is_alias,
                type_params,
                ..
            } = symbol
            else {
                continue;
            };
            let Some(export) = artifact.implementation_links.types.get(selected_path) else {
                continue;
            };
            if export.file.module_path.is_empty() || export.symbol.is_empty() {
                continue;
            }
            let symbol_name = export
                .symbol
                .strip_prefix(&format!("{}.", export.file.module_path))
                .unwrap_or(&export.symbol)
                .to_string();
            let info = ArtifactTypeInfo {
                module_path: export.file.module_path.clone(),
                symbol_name: symbol_name.clone(),
                descriptor: descriptor.clone(),
                is_alias: *is_alias,
                type_params: type_params.clone(),
            };
            index.by_symbol.insert(
                (export.file.module_path.clone(), symbol_name),
                selected_path.clone(),
            );
            index.by_slot.insert(
                (export.file.module_path.clone(), export.type_index),
                selected_path.clone(),
            );
            index.selected.insert(selected_path.clone(), info);
        }
        self.artifact_indexes
            .borrow_mut()
            .insert(artifact.package_id.clone(), index.clone());
        Ok(index)
    }

    fn external_record_from_artifact(
        &self,
        alias: &str,
        artifact: &'a PackageArtifact,
        symbol_path: &str,
        seen_paths: &mut BTreeSet<String>,
    ) -> Result<ExternalRecord, PublicationError> {
        let index = self.artifact_index(artifact)?;
        let selected_path = index
            .selected
            .get(symbol_path)
            .map(|_| symbol_path.to_string())
            .or_else(|| {
                let name = symbol_path.rsplit('.').next().unwrap_or(symbol_path);
                let mut matches = index
                    .selected
                    .iter()
                    .filter(|(_, info)| info.symbol_name == name);
                let first = matches.next().map(|(path, _)| path.clone());
                if matches.next().is_some() {
                    None
                } else {
                    first
                }
            })
            .or_else(|| {
                let (module, name) = symbol_path.rsplit_once('.')?;
                index
                    .by_symbol
                    .get(&(module.to_string(), name.to_string()))
                    .cloned()
            })
            .ok_or_else(|| {
                self.failure(format!(
                    "spread source `{alias}.{symbol_path}` does not resolve to an exported type of dependency `{}`",
                    artifact.package_id
                ))
            })?;
        self.external_record_from_artifact_info(
            alias,
            artifact,
            symbol_path,
            seen_paths,
            selected_path,
        )
    }

    fn external_record_from_artifact_info(
        &self,
        alias: &str,
        artifact: &'a PackageArtifact,
        symbol_path: &str,
        seen_paths: &mut BTreeSet<String>,
        selected_path: String,
    ) -> Result<ExternalRecord, PublicationError> {
        let index = self.artifact_index(artifact)?;
        let info = index
            .selected
            .get(&selected_path)
            .cloned()
            .expect("selected artifact type present");
        match &info.descriptor {
            TypeDescriptorIr::Record { fields } => {
                let fields = fields
                    .iter()
                    .map(|(name, ty)| {
                        let text = self.artifact_field_text(
                            alias,
                            &info.module_path,
                            &index,
                            ty,
                            &info.type_params,
                        )?;
                        Ok((name.clone(), text))
                    })
                    .collect::<Result<Vec<_>, PublicationError>>()?;
                Ok(ExternalRecord {
                    fields,
                    type_params: info.type_params.clone(),
                })
            }
            TypeDescriptorIr::Alias { target } if info.is_alias => {
                let path = format!("{alias}.{symbol_path}");
                if !seen_paths.insert(path.clone()) {
                    return Err(self.failure(format!(
                        "spread source `{path}` resolves through a recursive alias chain"
                    )));
                }
                let resolved = match target {
                    TypeRefIr::ServiceSymbol { symbol } => {
                        let selected = index
                            .by_symbol
                            .get(&(symbol.module_path.clone(), symbol.symbol.clone()))
                            .cloned()
                            .ok_or_else(|| {
                                self.failure(format!(
                                    "spread source `{path}` resolves through alias `{}.{}` which is not an exported type of dependency `{}`",
                                    symbol.module_path, symbol.symbol, artifact.package_id
                                ))
                            })?;
                        self.external_record_from_artifact_info(
                            alias,
                            artifact,
                            symbol_path,
                            seen_paths,
                            selected,
                        )?
                    }
                    TypeRefIr::LocalType { type_index } => {
                        let selected = index
                            .by_slot
                            .get(&(info.module_path.clone(), *type_index))
                            .cloned()
                            .ok_or_else(|| {
                                self.failure(format!(
                                    "spread source `{path}` resolves through alias `#{type_index}` which is not an exported type of dependency `{}`",
                                    artifact.package_id
                                ))
                            })?;
                        self.external_record_from_artifact_info(
                            alias,
                            artifact,
                            symbol_path,
                            seen_paths,
                            selected,
                        )?
                    }
                    TypeRefIr::PublicationType {
                        module_path,
                        type_index,
                    } => {
                        let selected = index
                            .by_slot
                            .get(&(module_path.clone(), *type_index))
                            .cloned()
                            .ok_or_else(|| {
                                self.failure(format!(
                                    "spread source `{path}` resolves through alias `{module_path}#{type_index}` which is not an exported type of dependency `{}`",
                                    artifact.package_id
                                ))
                            })?;
                        self.external_record_from_artifact_info(
                            alias,
                            artifact,
                            symbol_path,
                            seen_paths,
                            selected,
                        )?
                    }
                    _ => {
                        return Err(self.failure(format!(
                            "spread source `{path}` resolves through an alias to a type outside dependency `{}`, which is not supported",
                            artifact.package_id
                        )));
                    }
                };
                Ok(resolved)
            }
            _ => Err(self.failure(format!(
                "spread source `{alias}.{symbol_path}` is not a record; representation, named union, and interface declarations cannot be spread"
            ))),
        }
    }

    /// Renders an artifact field type as source text resolvable in the host:
    /// types owned by the dependency render as `<alias>.<public path>`,
    /// cross-package references keep their own alias qualification, and type
    /// parameters render as bare names (substituted per instantiation).
    fn artifact_field_text(
        &self,
        alias: &str,
        source_module: &str,
        index: &ArtifactTypeIndex,
        ty: &TypeRefIr,
        type_params: &[String],
    ) -> Result<String, PublicationError> {
        let type_params = type_params.iter().cloned().collect::<BTreeSet<_>>();
        self.artifact_type_text(alias, source_module, index, ty, &type_params)
    }

    fn artifact_type_text(
        &self,
        alias: &str,
        source_module: &str,
        index: &ArtifactTypeIndex,
        ty: &TypeRefIr,
        type_params: &BTreeSet<String>,
    ) -> Result<String, PublicationError> {
        Ok(match ty {
            TypeRefIr::Builtin { name, args } if args.is_empty() => name.clone(),
            TypeRefIr::Builtin { name, args } => format!(
                "{name}<{}>",
                args.iter()
                    .map(|arg| self.artifact_type_text(
                        alias,
                        source_module,
                        index,
                        arg,
                        type_params
                    ))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ")
            ),
            TypeRefIr::AppliedNominal { base, arguments } => format!(
                "{}<{}>",
                self.artifact_nominal_base_text(alias, source_module, index, base, type_params)?,
                arguments
                    .iter()
                    .map(|argument| {
                        self.artifact_type_text(alias, source_module, index, argument, type_params)
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ")
            ),
            TypeRefIr::LocalType { type_index } => {
                let selected = index.by_slot.get(&(source_module.to_string(), *type_index)).ok_or_else(|| {
                    self.failure(format!(
                        "spread source field type `#{type_index}` is not an exported type of dependency `{alias}`"
                    ))
                })?;
                format!("{alias}.{selected}")
            }
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => {
                let selected = index.by_slot.get(&(module_path.clone(), *type_index)).ok_or_else(|| {
                    self.failure(format!(
                        "spread source field type `{module_path}#{type_index}` is not an exported type of dependency `{alias}`"
                    ))
                })?;
                format!("{alias}.{selected}")
            }
            TypeRefIr::ServiceSymbol { symbol } => {
                let selected = index
                    .by_symbol
                    .get(&(symbol.module_path.clone(), symbol.symbol.clone()))
                    .ok_or_else(|| {
                        self.failure(format!(
                            "spread source field type `{}.{}` is not an exported type of dependency `{alias}`",
                            symbol.module_path, symbol.symbol
                        ))
                    })?;
                format!("{alias}.{selected}")
            }
            TypeRefIr::PackageSymbol { symbol } => match &symbol.package {
                PackageRefIr::PackageId { package_id }
                    if package_id == SKIFF_STD_PUBLICATION_ID =>
                {
                    format!("std.{}", symbol.symbol_path)
                }
                PackageRefIr::Dependency { dependency_ref } => {
                    format!("{dependency_ref}.{}", symbol.symbol_path)
                }
                PackageRefIr::PackageId { package_id } => {
                    format!("{package_id}.{}", symbol.symbol_path)
                }
            },
            TypeRefIr::Record { fields } => format!(
                "{{ {} }}",
                fields
                    .iter()
                    .map(|(name, field)| {
                        Ok(format!(
                            "{name}: {}",
                            self.artifact_type_text(
                                alias,
                                source_module,
                                index,
                                field,
                                type_params
                            )?
                        ))
                    })
                    .collect::<Result<Vec<_>, PublicationError>>()?
                    .join(", ")
            ),
            TypeRefIr::Union { items } => items
                .iter()
                .map(|item| self.artifact_type_text(alias, source_module, index, item, type_params))
                .collect::<Result<Vec<_>, _>>()?
                .join(" | "),
            TypeRefIr::Nullable { inner } => format!(
                "{}?",
                self.artifact_type_text(alias, source_module, index, inner, type_params)?
            ),
            TypeRefIr::Literal { value } => match value {
                skiff_artifact_model::LiteralIr::String { value } => {
                    serde_json::to_string(value).map_err(|error| self.failure(error.to_string()))?
                }
                skiff_artifact_model::LiteralIr::Bool { value } => value.to_string(),
                skiff_artifact_model::LiteralIr::Number { value } => value.to_string(),
                skiff_artifact_model::LiteralIr::Null => "null".to_string(),
            },
            TypeRefIr::TypeParam { name } => name.clone(),
            TypeRefIr::AnyInterface { interface } => {
                let identity = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).map_err(
                    |error| {
                        self.failure(format!(
                            "spread source field any-interface identity is not a canonical type reference: {error}"
                        ))
                    },
                )?;
                let name =
                    self.artifact_type_text(alias, source_module, index, &identity, type_params)?;
                if interface.canonical_type_args.is_empty() {
                    format!("any {name}")
                } else {
                    format!(
                        "any {name}<{}>",
                        interface
                            .canonical_type_args
                            .iter()
                            .map(|arg| {
                                self.artifact_type_text(
                                    alias,
                                    source_module,
                                    index,
                                    arg,
                                    type_params,
                                )
                            })
                            .collect::<Result<Vec<_>, _>>()?
                            .join(", ")
                    )
                }
            }
            TypeRefIr::DbObjectSymbol { .. } => {
                return Err(self.failure(
                    "spread source field type references a db object symbol, which cannot be copied into a record field",
                ));
            }
            TypeRefIr::PackageSchema { .. } => {
                return Err(self.failure(
                    "spread source field type references a package schema type, which cannot be copied into a record field",
                ));
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => format!(
                "fn({}) -> {}",
                params
                    .iter()
                    .map(|param| {
                        Ok(format!(
                            "{}: {}",
                            param.name,
                            self.artifact_type_text(
                                alias,
                                source_module,
                                index,
                                &param.ty,
                                type_params
                            )?
                        ))
                    })
                    .collect::<Result<Vec<_>, PublicationError>>()?
                    .join(", "),
                self.artifact_type_text(alias, source_module, index, return_type, type_params)?
            ),
        })
    }

    fn artifact_nominal_base_text(
        &self,
        alias: &str,
        source_module: &str,
        index: &ArtifactTypeIndex,
        base: &NominalTypeRefBaseIr,
        type_params: &BTreeSet<String>,
    ) -> Result<String, PublicationError> {
        match base {
            NominalTypeRefBaseIr::LocalType { type_index } => {
                let selected = index.by_slot.get(&(source_module.to_string(), *type_index)).ok_or_else(|| {
                    self.failure(format!(
                        "spread source field base `#{type_index}` is not an exported type of dependency `{alias}`"
                    ))
                })?;
                Ok(format!("{alias}.{selected}"))
            }
            NominalTypeRefBaseIr::PublicationType {
                module_path,
                type_index,
            } => {
                let selected = index.by_slot.get(&(module_path.clone(), *type_index)).ok_or_else(|| {
                    self.failure(format!(
                        "spread source field base `{module_path}#{type_index}` is not an exported type of dependency `{alias}`"
                    ))
                })?;
                Ok(format!("{alias}.{selected}"))
            }
            NominalTypeRefBaseIr::ServiceSymbol { symbol } => {
                let selected = index
                    .by_symbol
                    .get(&(symbol.module_path.clone(), symbol.symbol.clone()))
                    .ok_or_else(|| {
                        self.failure(format!(
                            "spread source field base `{}.{}` is not an exported type of dependency `{alias}`",
                            symbol.module_path, symbol.symbol
                        ))
                    })?;
                Ok(format!("{alias}.{selected}"))
            }
            NominalTypeRefBaseIr::PackageSymbol { symbol } => self
                .artifact_type_text(alias, source_module, index, &TypeRefIr::PackageSymbol {
                    symbol: symbol.clone(),
                }, type_params),
            NominalTypeRefBaseIr::PackageSchema { .. } => Err(self.failure(
                "spread source field base references a package schema type, which cannot be copied into a record field",
            )),
        }
    }

    // ------------------------------------------------------------------
    // Phase B: expand each source AST.
    // ------------------------------------------------------------------

    fn expand_source(
        &mut self,
        module_path: &str,
        ast: &mut SourceFile,
    ) -> Result<bool, PublicationError> {
        let mut changed = false;
        for ty in &mut ast.types {
            if ty.spreads.is_empty() {
                continue;
            }
            let expanded = self.expand_record(module_path, &ty.name)?;
            ty.fields = expanded;
            ty.spreads = Vec::new();
            changed = true;
        }
        Ok(changed)
    }

    /// Returns the fully expanded field template of a local record, with type
    /// parameter references preserved for per-instantiation substitution.
    fn expand_record(
        &mut self,
        module_path: &str,
        name: &str,
    ) -> Result<Vec<FieldDecl>, PublicationError> {
        let key = (module_path.to_string(), name.to_string());
        if let Some(fields) = self.expanded_fields.get(&key) {
            return Ok(fields.clone());
        }
        let fields = self.expand_record_inner(module_path, name)?;
        self.expanded_fields.insert(key, fields.clone());
        Ok(fields)
    }

    fn expand_record_inner(
        &mut self,
        module_path: &str,
        name: &str,
    ) -> Result<Vec<FieldDecl>, PublicationError> {
        let module = self.index.module(module_path).ok_or_else(|| {
            self.failure("internal spread expansion error: missing source module")
        })?;
        let ty = module
            .types
            .get(name)
            .filter(|ty| ty.alias.is_none())
            .ok_or_else(|| {
                self.failure("internal spread expansion error: spread target is not a local record")
            })?;
        let local_type_names = &module.local_type_names;
        let target_type_params = ty.type_params.iter().cloned().collect::<BTreeSet<_>>();
        let source_type_params = ty.type_params.clone();
        let mut fields = Vec::new();
        let mut seen = BTreeSet::new();
        for field in &ty.fields {
            let qualified = self.qualify_local_field_text(
                module_path,
                local_type_names,
                &source_type_params,
                &field.ty.name,
            );
            seen.insert(field.name.clone());
            fields.push(FieldDecl {
                name: field.name.clone(),
                ty: TypeRef { name: qualified },
            });
        }
        let spreads = self
            .resolved_spreads
            .get(&(module_path.to_string(), name.to_string()))
            .cloned()
            .unwrap_or_default();
        for spread in spreads {
            let source_display = display_spread_source(&spread.source);
            let (source_type_params, template) = match spread.source {
                SpreadSource::Local { module, name } => {
                    let source_module = self.index.module(&module).ok_or_else(|| {
                        self.failure(
                            "internal spread expansion error: missing spread source module",
                        )
                    })?;
                    let source = source_module
                        .types
                        .get(&name)
                        .filter(|source| source.alias.is_none())
                        .ok_or_else(|| {
                            self.failure(
                                "internal spread expansion error: missing spread source record",
                            )
                        })?;
                    (
                        source.type_params.clone(),
                        self.expand_record(&module, &name)?,
                    )
                }
                SpreadSource::External {
                    fields,
                    type_params,
                } => (
                    type_params,
                    fields
                        .into_iter()
                        .map(|(field_name, text)| FieldDecl {
                            name: field_name,
                            ty: TypeRef { name: text },
                        })
                        .collect(),
                ),
            };
            if source_type_params.len() != spread.args.len() {
                return Err(self.failure(format!(
                    "spread source `{source_display}` expects {} type arguments, found {}",
                    source_type_params.len(),
                    spread.args.len()
                )));
            }
            for arg in &spread.args {
                let expr = TypeExpr::parse(arg);
                let mut found = None;
                expr.for_each_named_outside_function_types(|name| {
                    if found.is_none() && target_type_params.contains(name) {
                        found = Some(name.to_string());
                    }
                });
                if let Some(param) = found {
                    return Err(self.failure(format!(
                        "spread source `{source_display}` type argument `{arg}` references the target type parameter `{param}`; spread sources must be fully instantiated"
                    )));
                }
            }
            let substitutions = source_type_params
                .iter()
                .cloned()
                .zip(spread.args.iter().cloned())
                .collect::<BTreeMap<_, _>>();
            for field in template {
                if !seen.insert(field.name.clone()) {
                    return Err(self.failure(format!(
                        "spread source `{source_display}` field `{}` conflicts with another field on record `{module_path}.{name}`",
                        field.name
                    )));
                }
                let text = substitute_type_params_in_type_expr(
                    &TypeExpr::parse(&field.ty.name),
                    &substitutions,
                )
                .to_type_string();
                fields.push(FieldDecl {
                    name: field.name,
                    ty: TypeRef { name: text },
                });
            }
        }
        Ok(fields)
    }

    /// Qualifies a field type text of a local record: bare names declared in
    /// the source module become `root.<source module>.<Name>` so the copied
    /// text keeps resolving in the target record's module context.
    fn qualify_local_field_text(
        &self,
        module_path: &str,
        local_type_names: &BTreeSet<String>,
        type_params: &[String],
        text: &str,
    ) -> String {
        let type_params = type_params.iter().cloned().collect::<BTreeSet<_>>();
        TypeExpr::parse(text)
            .map_named_types(|name| {
                if local_type_names.contains(name) && !type_params.contains(name) {
                    format!("root.{module_path}.{name}")
                } else {
                    name.to_string()
                }
            })
            .to_type_string()
    }
}

fn display_spread_source(source: &SpreadSource) -> String {
    match source {
        SpreadSource::Local { module, name } => format!("{module}.{name}"),
        SpreadSource::External { .. } => "<dependency type>".to_string(),
    }
}

fn substitute_type_params_in_type_expr(
    expr: &TypeExpr,
    substitutions: &BTreeMap<String, String>,
) -> TypeExpr {
    match expr {
        TypeExpr::Named { name, args } if args.is_empty() => substitutions
            .get(name)
            .map(|replacement| TypeExpr::parse(replacement))
            .unwrap_or_else(|| expr.clone()),
        TypeExpr::Named { name, args } => TypeExpr::Named {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| substitute_type_params_in_type_expr(arg, substitutions))
                .collect(),
        },
        TypeExpr::Nullable(inner) => TypeExpr::Nullable(Box::new(
            substitute_type_params_in_type_expr(inner, substitutions),
        )),
        TypeExpr::Union(parts) => TypeExpr::Union(
            parts
                .iter()
                .map(|part| substitute_type_params_in_type_expr(part, substitutions))
                .collect(),
        ),
        TypeExpr::AnyInterface { interface } => TypeExpr::AnyInterface {
            interface: Box::new(substitute_type_params_in_type_expr(
                interface,
                substitutions,
            )),
        },
        TypeExpr::Record(fields) => TypeExpr::Record(
            fields
                .iter()
                .map(|field| crate::shared::type_expr::RecordTypeField {
                    name: field.name.clone(),
                    ty: substitute_type_params_in_type_expr(&field.ty, substitutions),
                })
                .collect(),
        ),
        TypeExpr::Function {
            params,
            return_type,
        } => TypeExpr::Function {
            params: params
                .iter()
                .map(|param| crate::shared::type_expr::FunctionTypeParam {
                    name: param.name.clone(),
                    ty: substitute_type_params_in_type_expr(&param.ty, substitutions),
                })
                .collect(),
            return_type: Box::new(substitute_type_params_in_type_expr(
                return_type,
                substitutions,
            )),
        },
        TypeExpr::EmptyRecord | TypeExpr::StringLiteral(_) => expr.clone(),
    }
}

enum LocalShape<'index> {
    Record,
    Alias(&'index AliasDecl),
    NonRecord,
}

enum SpreadTarget {
    Local { module: String, name: String },
    External { alias: String, symbol_path: String },
}

struct ExternalRecord {
    fields: Vec<(String, String)>,
    type_params: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct ArtifactTypeIndex {
    selected: BTreeMap<String, ArtifactTypeInfo>,
    by_symbol: BTreeMap<(String, String), String>,
    by_slot: BTreeMap<(String, u32), String>,
}

#[derive(Debug, Clone)]
struct ArtifactTypeInfo {
    module_path: String,
    symbol_name: String,
    descriptor: TypeDescriptorIr,
    is_alias: bool,
    type_params: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Visiting,
    Done,
}

fn find_spread_cycle(
    node: &(String, String),
    graph: &BTreeMap<(String, String), Vec<(String, String)>>,
    state: &mut BTreeMap<(String, String), VisitState>,
    stack: &mut Vec<(String, String)>,
) -> Option<Vec<(String, String)>> {
    match state.get(node).copied() {
        Some(VisitState::Done) => return None,
        Some(VisitState::Visiting) => {
            if let Some(start) = stack.iter().position(|entry| entry == node) {
                let mut cycle = stack[start..].to_vec();
                cycle.push(node.clone());
                return Some(cycle);
            }
            return None;
        }
        None => {}
    }
    let Some(edges) = graph.get(node) else {
        state.insert(node.clone(), VisitState::Done);
        return None;
    };
    state.insert(node.clone(), VisitState::Visiting);
    stack.push(node.clone());
    for edge in edges {
        if let Some(cycle) = find_spread_cycle(edge, graph, state, stack) {
            return Some(cycle);
        }
    }
    stack.pop();
    state.insert(node.clone(), VisitState::Done);
    None
}
