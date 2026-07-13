use std::collections::{BTreeMap, BTreeSet};

use skiff_compiler::test_support::{package_public_path, TestPackageManifest as PackageManifest};
use skiff_compiler_core::id::{SKIFF_STD_PUBLICATION_ID, STD_SOURCE_ALIAS};
use skiff_syntax::ast::SourceFile as AstSourceFile;

use super::{
    PackageTestSource, ParsedSource, ProductionModuleSymbols, ProductionSymbol,
    ProductionSymbolKind,
};

pub(super) fn production_function_return_types(
    sources: &[PackageTestSource],
) -> BTreeMap<String, String> {
    let mut return_types = BTreeMap::new();
    for source in sources {
        collect_function_return_types_for_ast(&mut return_types, &source.module_path, &source.ast);
    }
    return_types
}

pub(super) fn service_function_return_types(sources: &[ParsedSource]) -> BTreeMap<String, String> {
    let mut return_types = BTreeMap::new();
    for source in sources.iter().filter(|source| !source.source.is_test_file) {
        collect_function_return_types_for_ast(
            &mut return_types,
            &source.source.module_path,
            &source.ast,
        );
    }
    return_types
}

pub(super) fn merge_function_return_types(
    base: &mut BTreeMap<String, String>,
    next: BTreeMap<String, String>,
) {
    for (name, return_type) in next {
        base.entry(name).or_insert(return_type);
    }
}

fn collect_function_return_types_for_ast(
    return_types: &mut BTreeMap<String, String>,
    module_path: &str,
    ast: &AstSourceFile,
) {
    let type_names = exported_type_names(ast);
    for function in &ast.function_signatures {
        insert_function_return_type(
            return_types,
            module_path,
            &function.name,
            &qualify_public_return_type(&function.return_type.name, module_path, &type_names),
        );
    }
    for function in &ast.functions {
        if function.exported {
            insert_function_return_type(
                return_types,
                module_path,
                &function.name,
                &qualify_public_return_type(&function.return_type.name, module_path, &type_names),
            );
        }
    }
}

fn insert_function_return_type(
    return_types: &mut BTreeMap<String, String>,
    module_path: &str,
    name: &str,
    return_type: &str,
) {
    return_types.insert(format!("{module_path}.{name}"), return_type.to_string());
}

fn exported_type_names(ast: &AstSourceFile) -> BTreeSet<String> {
    ast.types
        .iter()
        .filter(|ty| ty.exported)
        .map(|ty| ty.name.clone())
        .chain(
            ast.interfaces
                .iter()
                .filter(|interface| interface.exported)
                .map(|interface| interface.name.clone()),
        )
        .chain(
            ast.aliases
                .iter()
                .filter(|alias| alias.exported)
                .map(|alias| alias.name.clone()),
        )
        .chain(ast.dbs.iter().map(|db| db.name.clone()))
        .collect()
}

fn qualify_public_return_type(
    return_type: &str,
    module_path: &str,
    type_names: &BTreeSet<String>,
) -> String {
    qualify_public_type_text(return_type.trim(), module_path, type_names)
}

fn qualify_public_type_text(ty: &str, module_path: &str, type_names: &BTreeSet<String>) -> String {
    let ty = ty.trim();
    if let Some(inner) = skiff_compiler::test_support::generic_inner(ty, "Stream") {
        return format!(
            "Stream<{}>",
            qualify_public_type_text(inner, module_path, type_names)
        );
    }
    if let Some(inner) = ty.strip_suffix('?') {
        return format!(
            "{}?",
            qualify_public_type_text(inner, module_path, type_names)
        );
    }
    if type_names.contains(ty) {
        return format!("{module_path}.{ty}");
    }
    ty.to_string()
}

pub(super) fn production_function_exports(
    manifest: &PackageManifest,
    sources: &[PackageTestSource],
    include_public_paths: bool,
) -> BTreeMap<String, ProductionModuleSymbols> {
    let mut exports = sources
        .iter()
        .map(|source| {
            let mut symbols = ProductionModuleSymbols::default();
            for ty in &source.ast.types {
                insert_production_symbol(
                    &mut symbols.symbols,
                    &ty.name,
                    ProductionSymbolKind::Type,
                    false,
                );
            }
            for alias in &source.ast.aliases {
                insert_production_symbol(
                    &mut symbols.symbols,
                    &alias.name,
                    ProductionSymbolKind::Type,
                    false,
                );
            }
            for db in &source.ast.dbs {
                symbols.db_objects.insert(db.name.clone());
                symbols
                    .symbols
                    .entry(db.name.clone())
                    .or_insert_with(|| ProductionSymbol {
                        kind: ProductionSymbolKind::DbObject,
                        exported: false,
                    });
            }
            for interface in &source.ast.interfaces {
                insert_production_symbol(
                    &mut symbols.symbols,
                    &interface.name,
                    ProductionSymbolKind::Interface,
                    false,
                );
            }
            for function in &source.ast.functions {
                insert_production_symbol(
                    &mut symbols.symbols,
                    &function.name,
                    ProductionSymbolKind::Function,
                    false,
                );
            }
            for constant in &source.ast.consts {
                insert_production_symbol(
                    &mut symbols.symbols,
                    &constant.name,
                    ProductionSymbolKind::Const,
                    false,
                );
            }
            for implementation in &source.ast.impls {
                for method in &implementation.methods {
                    let symbol = format!("{}.{}", implementation.target, method.name);
                    insert_production_symbol(
                        &mut symbols.member_symbols,
                        &symbol,
                        ProductionSymbolKind::Method,
                        false,
                    );
                }
                for body in &implementation.method_bodies {
                    let symbol = format!("{}.{}", implementation.target, body.name);
                    insert_production_symbol(
                        &mut symbols.member_symbols,
                        &symbol,
                        ProductionSymbolKind::Method,
                        false,
                    );
                }
            }
            (source.module_path.clone(), symbols)
        })
        .collect();
    apply_api_exports(manifest, sources, include_public_paths, &mut exports);
    exports
}

fn apply_api_exports(
    manifest: &PackageManifest,
    sources: &[PackageTestSource],
    include_public_paths: bool,
    exports: &mut BTreeMap<String, ProductionModuleSymbols>,
) {
    let symbol_kinds = source_symbol_kind_index(sources);
    for entry in &manifest.api {
        let source_module = api_entry_source_module(manifest, &entry.module);
        let source_symbol = entry.symbol.clone();
        let Some(kind) = symbol_kinds
            .get(&(source_module.clone(), source_symbol.clone()))
            .copied()
        else {
            continue;
        };
        if include_public_paths {
            for (module_path, symbol_path) in
                dependency_export_visibility_paths(manifest, &entry.path)
            {
                mark_exported_symbol(exports, &module_path, &symbol_path, kind);
            }
        } else {
            mark_exported_symbol(exports, &source_module, &source_symbol, kind);
        }
    }
}

fn api_entry_source_module(manifest: &PackageManifest, module: &str) -> String {
    if manifest.id == SKIFF_STD_PUBLICATION_ID {
        package_public_path(STD_SOURCE_ALIAS, module)
    } else {
        module.to_string()
    }
}

fn source_symbol_kind_index(
    sources: &[PackageTestSource],
) -> BTreeMap<(String, String), ProductionSymbolKind> {
    let mut symbols = BTreeMap::new();
    for source in sources {
        for ty in &source.ast.types {
            symbols.insert(
                (source.module_path.clone(), ty.name.clone()),
                ProductionSymbolKind::Type,
            );
        }
        for alias in &source.ast.aliases {
            symbols.insert(
                (source.module_path.clone(), alias.name.clone()),
                ProductionSymbolKind::Type,
            );
        }
        for db in &source.ast.dbs {
            symbols.insert(
                (source.module_path.clone(), db.name.clone()),
                ProductionSymbolKind::DbObject,
            );
        }
        for interface in &source.ast.interfaces {
            symbols.insert(
                (source.module_path.clone(), interface.name.clone()),
                ProductionSymbolKind::Interface,
            );
        }
        for function in &source.ast.functions {
            symbols.insert(
                (source.module_path.clone(), function.name.clone()),
                ProductionSymbolKind::Function,
            );
        }
        for constant in &source.ast.consts {
            symbols.insert(
                (source.module_path.clone(), constant.name.clone()),
                ProductionSymbolKind::Const,
            );
        }
    }
    symbols
}

fn dependency_export_visibility_paths(
    manifest: &PackageManifest,
    public_symbol_path: &str,
) -> Vec<(String, String)> {
    let Some((public_module, public_symbol)) = public_symbol_path.rsplit_once('.') else {
        return vec![(manifest.id.clone(), public_symbol_path.to_string())];
    };
    let mut paths = vec![(public_module.to_string(), public_symbol.to_string())];
    let package_public_root = if manifest.id == SKIFF_STD_PUBLICATION_ID {
        STD_SOURCE_ALIAS
    } else {
        &manifest.id
    };
    let package_public_module = package_public_path(package_public_root, public_module);
    if package_public_module != public_module {
        paths.push((package_public_module, public_symbol.to_string()));
    }
    paths
}

fn mark_exported_symbol(
    exports: &mut BTreeMap<String, ProductionModuleSymbols>,
    module_path: &str,
    symbol_path: &str,
    kind: ProductionSymbolKind,
) {
    let symbols = exports.entry(module_path.to_string()).or_default();
    if kind == ProductionSymbolKind::DbObject {
        symbols.db_objects.insert(symbol_path.to_string());
    }
    symbols
        .symbols
        .entry(symbol_path.to_string())
        .and_modify(|symbol| symbol.exported = true)
        .or_insert(ProductionSymbol {
            kind,
            exported: true,
        });
}
pub(super) fn merge_production_exports(
    base: &mut BTreeMap<String, ProductionModuleSymbols>,
    next: BTreeMap<String, ProductionModuleSymbols>,
) {
    for (module_path, symbols) in next {
        base.entry(module_path).or_insert(symbols);
    }
}
pub(super) fn service_production_exports(
    sources: &[ParsedSource],
) -> BTreeMap<String, ProductionModuleSymbols> {
    sources
        .iter()
        .filter(|source| !source.source.is_test_file)
        .map(|source| {
            (
                source.source.module_path.clone(),
                production_symbols_for_ast(&source.ast, true),
            )
        })
        .collect()
}
pub(super) fn production_symbols_for_ast(
    ast: &AstSourceFile,
    module_exported: bool,
) -> ProductionModuleSymbols {
    let mut symbols = ProductionModuleSymbols::default();
    for ty in &ast.types {
        insert_production_symbol(
            &mut symbols.symbols,
            &ty.name,
            ProductionSymbolKind::Type,
            module_exported && ty.exported,
        );
    }
    for alias in &ast.aliases {
        insert_production_symbol(
            &mut symbols.symbols,
            &alias.name,
            ProductionSymbolKind::Type,
            module_exported && alias.exported,
        );
    }
    for db in &ast.dbs {
        symbols.db_objects.insert(db.name.clone());
        symbols
            .symbols
            .entry(db.name.clone())
            .or_insert_with(|| ProductionSymbol {
                kind: ProductionSymbolKind::DbObject,
                exported: module_exported,
            });
    }
    for interface in &ast.interfaces {
        insert_production_symbol(
            &mut symbols.symbols,
            &interface.name,
            ProductionSymbolKind::Interface,
            module_exported && interface.exported,
        );
    }
    for function in &ast.functions {
        insert_production_symbol(
            &mut symbols.symbols,
            &function.name,
            ProductionSymbolKind::Function,
            module_exported && function.exported,
        );
    }
    for constant in &ast.consts {
        insert_production_symbol(
            &mut symbols.symbols,
            &constant.name,
            ProductionSymbolKind::Const,
            module_exported && constant.exported,
        );
    }
    for implementation in &ast.impls {
        for method in &implementation.methods {
            insert_production_symbol(
                &mut symbols.member_symbols,
                &format!("{}.{}", implementation.target, method.name),
                ProductionSymbolKind::Method,
                module_exported && implementation.exported,
            );
        }
        for body in &implementation.method_bodies {
            insert_production_symbol(
                &mut symbols.member_symbols,
                &format!("{}.{}", implementation.target, body.name),
                ProductionSymbolKind::Method,
                module_exported && implementation.exported,
            );
        }
    }
    symbols
}
pub(super) fn insert_production_symbol(
    symbols: &mut BTreeMap<String, ProductionSymbol>,
    name: &str,
    kind: ProductionSymbolKind,
    exported: bool,
) {
    symbols.insert(name.to_string(), ProductionSymbol { kind, exported });
}
