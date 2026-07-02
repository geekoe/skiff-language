use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use skiff_compiler::{
    resolve_root_refs_in_ast as resolve_root_paths_in_ast,
    test_support::{
        package_public_path, TestPackageApiEntry, TestPackageManifest as PackageManifest,
    },
    RootRefIndex as ServiceExportsIndex,
};
use skiff_compiler_core::id::{SKIFF_STD_PUBLICATION_ID, STD_SOURCE_ALIAS};
use skiff_syntax::ast::{
    Block, DbDecl, FunctionDecl, InterfaceDecl, SourceFile as AstSourceFile, TypeDecl, TypeRef,
};
use skiff_syntax::error::SourceSpan;

use super::{
    PackageTestSource, ParsedSource, ProductionModuleSymbols, ProductionSymbolKind, SkiffTestError,
};

pub(super) fn resolve_service_test_root_paths(
    sources: Vec<ParsedSource>,
    production_sources: &[ParsedSource],
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
) -> Result<Vec<ParsedSource>, SkiffTestError> {
    sources
        .into_iter()
        .map(|source| {
            let index = service_exports_index_for_friend_scope(
                production_sources,
                production_exports,
                &source,
            );
            resolve_parsed_source_root_paths(source, &index)
        })
        .collect()
}

pub(super) fn resolve_parsed_root_paths_with_exports(
    sources: Vec<ParsedSource>,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
) -> Result<Vec<ParsedSource>, SkiffTestError> {
    let index = exports_index_from_production_exports(production_exports);
    sources
        .into_iter()
        .map(|source| resolve_parsed_source_root_paths(source, &index))
        .collect()
}
pub(super) fn resolve_parsed_source_root_paths(
    mut source: ParsedSource,
    index: &ServiceExportsIndex,
) -> Result<ParsedSource, SkiffTestError> {
    let path = source.source.file_path.display().to_string();
    let outcome = resolve_root_paths_in_ast(&mut source.ast, index);
    if !outcome.errors.is_empty() {
        return Err(root_path_test_error(path, outcome.errors));
    }
    source.synthetic_imports = synthetic_import_paths(&outcome.synthetic_imports);
    Ok(source)
}
pub(super) fn resolve_package_test_root_paths(
    sources: Vec<PackageTestSource>,
    production_sources: &[PackageTestSource],
) -> Result<Vec<PackageTestSource>, SkiffTestError> {
    let index = current_package_root_index(production_sources);
    sources
        .into_iter()
        .map(|source| resolve_package_test_source_root_paths(source, &index))
        .collect()
}

pub(super) fn resolve_package_production_root_paths(
    sources: Vec<PackageTestSource>,
) -> Result<Vec<PackageTestSource>, SkiffTestError> {
    let index = current_package_root_index(&sources);
    sources
        .into_iter()
        .map(|source| resolve_package_test_source_root_paths(source, &index))
        .collect()
}

fn current_package_root_index(production_sources: &[PackageTestSource]) -> ServiceExportsIndex {
    let mut index = ServiceExportsIndex::new();
    for source in production_sources {
        index.insert_module_all_symbols(&source.module_path, &source.ast);
    }
    index
}
pub(super) fn resolve_official_package_private_root_paths(
    manifest: &PackageManifest,
    sources: Vec<PackageTestSource>,
) -> Result<Vec<PackageTestSource>, SkiffTestError> {
    if !is_official_aggregate_package(&manifest.id) {
        return Ok(sources);
    }
    let source_root = official_package_source_root(&manifest.id);
    let mut index = ServiceExportsIndex::new();
    for source in &sources {
        let module_path = source
            .module_path
            .strip_prefix(&format!("{source_root}."))
            .unwrap_or(&source.module_path);
        index.insert_module_with_root_path(&source.module_path, module_path, &source.ast);
    }
    sources
        .into_iter()
        .map(|source| resolve_package_source_root_paths_unqualified(source, &index))
        .collect()
}
fn resolve_package_source_root_paths_unqualified(
    mut source: PackageTestSource,
    index: &ServiceExportsIndex,
) -> Result<PackageTestSource, SkiffTestError> {
    let path = source.relative_path.display().to_string();
    let outcome = resolve_root_paths_in_ast(&mut source.ast, index);
    if !outcome.errors.is_empty() {
        return Err(root_path_test_error(path, outcome.errors));
    }
    Ok(source)
}
pub(super) fn resolve_package_test_source_root_paths(
    mut source: PackageTestSource,
    index: &ServiceExportsIndex,
) -> Result<PackageTestSource, SkiffTestError> {
    let path = source.relative_path.display().to_string();
    let outcome = resolve_root_paths_in_ast(&mut source.ast, index);
    if !outcome.errors.is_empty() {
        return Err(root_path_test_error(path, outcome.errors));
    }
    source.synthetic_imports = synthetic_import_paths(&outcome.synthetic_imports);
    Ok(source)
}
pub(super) fn synthetic_import_paths(
    synthetic_imports: &BTreeSet<(String, String)>,
) -> BTreeSet<String> {
    synthetic_imports
        .iter()
        .map(|(module_path, symbol)| format!("{module_path}.{symbol}"))
        .collect()
}
pub(super) fn exports_index_from_production_exports(
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
) -> ServiceExportsIndex {
    let mut index = ServiceExportsIndex::new();
    for (module_path, symbols) in production_exports {
        let mut ast = AstSourceFile {
            provider_capability: None,
            functions: Vec::new(),
            function_signatures: Vec::new(),
            imports: Vec::new(),
            types: Vec::new(),
            aliases: Vec::new(),
            interfaces: Vec::new(),
            impls: Vec::new(),
            dbs: Vec::new(),
            consts: Vec::new(),
            tests: Vec::new(),
            test_default_run: None,
            test_default_run_span: None,
            source_spans: Default::default(),
        };
        for name in &symbols.db_objects {
            ast.dbs.push(DbDecl {
                name: name.clone(),
                collection_name: None,
                key: None,
                retention: None,
                indexes: Vec::new(),
                leases: Vec::new(),
                span: SourceSpan::synthetic(),
            });
        }
        for (name, symbol) in &symbols.symbols {
            match symbol.kind {
                ProductionSymbolKind::Type => ast.types.push(TypeDecl {
                    exported: true,
                    is_native: false,
                    name: name.clone(),
                    type_params: Vec::new(),
                    discriminator: None,
                    alias: None,
                    implements: Vec::new(),
                    fields: Vec::new(),
                    span: SourceSpan::synthetic(),
                }),
                ProductionSymbolKind::DbObject => {}
                ProductionSymbolKind::Interface => ast.interfaces.push(InterfaceDecl {
                    exported: true,
                    name: name.clone(),
                    type_params: Vec::new(),
                    operations: Vec::new(),
                    span: SourceSpan::synthetic(),
                }),
                ProductionSymbolKind::Function => ast.functions.push(FunctionDecl {
                    exported: true,
                    name: name.clone(),
                    type_params: Vec::new(),
                    params: Vec::new(),
                    return_type: TypeRef {
                        name: "void".to_string(),
                    },
                    body: Block {
                        statements: Vec::new(),
                    },
                    is_native: true,
                    is_provider: false,
                    is_static: false,
                    implicit_self: None,
                    span: SourceSpan::synthetic(),
                }),
                ProductionSymbolKind::Const | ProductionSymbolKind::Method => {}
            }
        }
        index.insert_module(module_path, &ast);
    }
    index
}
pub(super) fn service_exports_index_for_friend_scope(
    production_sources: &[ParsedSource],
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    source: &ParsedSource,
) -> ServiceExportsIndex {
    let mut index = exports_index_from_production_exports(production_exports);
    if let Some(friend_module_path) = &source.friend_module_path {
        if let Some(production) = production_sources
            .iter()
            .find(|candidate| candidate.source.module_path == *friend_module_path)
        {
            index.insert_module_all_symbols(friend_module_path, &production.ast);
        }
    }
    index
}
pub(super) fn package_exports_index_for_friend_scope(
    production_sources: &[PackageTestSource],
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    source: &PackageTestSource,
) -> ServiceExportsIndex {
    let mut index = exports_index_from_production_exports(production_exports);
    if let Some(friend_module_path) = &source.friend_module_path {
        if let Some(production) = production_sources
            .iter()
            .find(|candidate| candidate.module_path == *friend_module_path)
        {
            index.insert_module_all_symbols(friend_module_path, &production.ast);
        }
    }
    index
}
pub(super) fn root_path_test_error(
    path: String,
    errors: Vec<skiff_compiler::RootRefError>,
) -> SkiffTestError {
    SkiffTestError::RootPathReference {
        path,
        message: errors
            .iter()
            .map(|error| format!("- {error}"))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}
pub(super) fn export_source_paths(
    manifest: &PackageManifest,
    package_root: &Path,
) -> Result<BTreeMap<PathBuf, String>, SkiffTestError> {
    let mut sources = BTreeMap::new();
    for entry in &manifest.api {
        let source_module = api_entry_source_module_hint(entry);
        let relative = match source_path_for_api_module(&manifest.id, package_root, source_module) {
            Some(relative) => relative,
            None if official_package_api_source_is_prelude_owned(
                manifest,
                package_root,
                source_module,
            ) =>
            {
                continue;
            }
            None => {
                return Err(SkiffTestError::RuntimeSetup {
                    message: format!(
                        "package {} api source {} has no source file",
                        manifest.id, source_module
                    ),
                });
            }
        };
        sources.insert(
            relative,
            package_test_module_path_for_api_entry(manifest, entry),
        );
    }
    Ok(sources)
}
fn official_package_api_source_is_prelude_owned(
    manifest: &PackageManifest,
    package_root: &Path,
    module: &str,
) -> bool {
    if !is_official_aggregate_package(&manifest.id) {
        return false;
    }
    let source_root = official_package_source_root(&manifest.id);
    let module = module
        .strip_prefix(&format!("{source_root}."))
        .unwrap_or(module);
    let relative = PathBuf::from(module.replace('.', "/")).with_extension("skiff");
    let Some(std_parent) = package_root.parent() else {
        return false;
    };
    let prelude_source = std_parent.join("prelude").join(relative);
    fs::metadata(prelude_source).is_ok_and(|metadata| metadata.is_file())
}
pub(super) fn source_path_for_api_module(
    package_id: &str,
    package_root: &Path,
    module: &str,
) -> Option<PathBuf> {
    let relative = PathBuf::from(module.replace('.', "/")).with_extension("skiff");
    if package_root.join(&relative).is_file() {
        return Some(relative);
    }
    if is_official_aggregate_package(package_id) {
        let source_root = official_package_source_root(package_id);
        if let Some(stripped) = module.strip_prefix(&format!("{source_root}.")) {
            let relative = PathBuf::from(stripped.replace('.', "/")).with_extension("skiff");
            if package_root.join(&relative).is_file() {
                return Some(relative);
            }
        }
    }
    None
}
pub(super) fn is_official_aggregate_package(package_id: &str) -> bool {
    package_id == SKIFF_STD_PUBLICATION_ID
}
pub(super) fn package_test_module_path_for_api_entry(
    manifest: &PackageManifest,
    entry: &TestPackageApiEntry,
) -> String {
    if is_official_aggregate_package(&manifest.id) {
        package_public_path(
            official_package_source_root(&manifest.id),
            api_entry_source_module_hint(entry),
        )
    } else {
        api_entry_source_module_hint(entry).to_string()
    }
}
pub(super) fn package_export_paths(manifest: &PackageManifest) -> BTreeMap<String, String> {
    let mut paths = BTreeMap::new();
    for entry in &manifest.api {
        let source_module = api_entry_source_module_hint(entry).to_string();
        paths.insert(source_module, entry.path.clone());
        if is_official_aggregate_package(&manifest.id) {
            paths.insert(
                package_public_path(official_package_source_root(&manifest.id), &entry.path),
                entry.path.clone(),
            );
        }
    }
    paths
}
fn api_entry_source_module_hint(entry: &TestPackageApiEntry) -> &str {
    &entry.module
}
pub(super) fn package_module_path(
    manifest: &PackageManifest,
    relative_path: &Path,
    friend_relative_path: Option<&Path>,
    is_test_file: bool,
    export_sources: &BTreeMap<PathBuf, String>,
) -> String {
    let production_relative = if let Some(friend_relative) = friend_relative_path {
        friend_relative.to_path_buf()
    } else if is_test_file {
        test_file_production_relative_path(relative_path)
    } else {
        relative_path.to_path_buf()
    };
    if let Some(module) = export_sources.get(&production_relative) {
        return if is_test_file {
            format!("{module}.__test")
        } else {
            module.clone()
        };
    }
    if is_test_file {
        if let Some(friend_relative) = friend_relative_path {
            return format!(
                "{}.__test",
                module_path_for_package_production_source(
                    manifest,
                    friend_relative,
                    export_sources,
                )
            );
        }
        format!(
            "{}.__test",
            module_path_for_package_source(&production_relative)
        )
    } else {
        fallback_module_path(relative_path, is_test_file)
    }
}
pub(super) fn module_path_for_package_production_source(
    manifest: &PackageManifest,
    relative_path: &Path,
    export_sources: &BTreeMap<PathBuf, String>,
) -> String {
    if let Some(module) = export_sources.get(relative_path) {
        return module.clone();
    }
    if is_official_aggregate_package(&manifest.id) {
        return official_package_internal_module_path(&manifest.id, relative_path);
    }
    module_path_for_package_source(relative_path)
}
pub(super) fn official_package_internal_module_path(
    package_id: &str,
    relative_path: &Path,
) -> String {
    let source_root = official_package_source_root(package_id);
    let mut without_extension = relative_path.to_path_buf();
    without_extension.set_extension("");
    let mut parts = without_extension
        .components()
        .filter_map(|component| component.as_os_str().to_str().map(str::to_string))
        .collect::<Vec<_>>();
    if parts.first().map(String::as_str) != Some(source_root) {
        parts.insert(0, source_root.to_string());
    }
    parts.join(".")
}

fn official_package_source_root(package_id: &str) -> &str {
    if is_official_aggregate_package(package_id) {
        STD_SOURCE_ALIAS
    } else {
        package_id
    }
}

pub(super) fn module_path_for_package_source(relative_path: &Path) -> String {
    let mut without_extension = relative_path.to_path_buf();
    without_extension.set_extension("");
    without_extension
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join(".")
}
pub(super) fn test_file_production_relative_path(relative_path: &Path) -> PathBuf {
    skiff_compiler::test_support::module_relative_path_for_test_file_without_friend(relative_path)
}
pub(super) fn fallback_module_path(relative_path: &Path, is_test_file: bool) -> String {
    let module_relative_path = if is_test_file {
        test_file_production_relative_path(relative_path)
    } else {
        relative_path.to_path_buf()
    };
    let without_extension = module_relative_path.with_extension("");
    let mut parts = without_extension
        .components()
        .filter_map(|component| component.as_os_str().to_str().map(str::to_string))
        .collect::<Vec<_>>();
    if is_test_file {
        parts.push("__test".to_string());
    }
    parts.join(".")
}

#[cfg(test)]
mod tests;
