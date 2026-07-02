use std::collections::{BTreeMap, BTreeSet};

use skiff_compiler::test_support::{
    expr_path, package_public_path, TestPackageManifest as PackageManifest,
};
use skiff_compiler_core::id::{SKIFF_STD_PUBLICATION_ID, STD_SOURCE_ALIAS};
use skiff_syntax::ast::{
    Block, DbBody, DbChangeOp, DbOperation, DbQuery, DbQueryBlock, DbSelector, DbWhereClause, Expr,
    FunctionDecl, InterfaceOperation, PatchOperation, Pattern, SourceFile as AstSourceFile, Stmt,
    TypeRef,
};

use super::{
    PackageTestCase, PackageTestSource, ParsedSource, PrivateVisibilityScope,
    ProductionModuleSymbols, ProductionSymbol, ProductionSymbolKind, SymbolUseKind,
    TestLocalSymbols,
};

pub(super) const ALL_PRIVATE_MODULES: &str = "__skiff_test_all_private_modules";

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
pub(super) fn private_visibility_error(
    test: &PackageTestCase,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    package_ids: &BTreeSet<String>,
    package_aliases: &BTreeMap<String, Vec<String>>,
    disallowed_import_modules: &BTreeSet<String>,
) -> Option<String> {
    if test.source.is_test_file {
        private_visibility_error_in_ast(
            &test.source.ast,
            &test.source.synthetic_imports,
            test.test_index,
            production_exports,
            &private_visibility_scope_for_package_source(&test.source),
            package_ids,
            package_aliases,
            disallowed_import_modules,
        )
    } else {
        private_visibility_error_in_test_body(
            &test.source.ast,
            &test.source.synthetic_imports,
            test.test_index,
            production_exports,
            &PrivateVisibilityScope::Module(test.source.module_path.clone()),
            package_ids,
            package_aliases,
            &BTreeSet::new(),
        )
    }
}

fn private_visibility_scope_for_package_source(
    source: &PackageTestSource,
) -> PrivateVisibilityScope {
    source
        .friend_module_path
        .clone()
        .map(PrivateVisibilityScope::Module)
        .unwrap_or_default()
}

pub(super) fn private_visibility_error_in_ast(
    ast: &AstSourceFile,
    synthetic_imports: &BTreeSet<String>,
    test_index: usize,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    private_visibility_scope: &PrivateVisibilityScope,
    package_ids: &BTreeSet<String>,
    package_aliases: &BTreeMap<String, Vec<String>>,
    disallowed_import_modules: &BTreeSet<String>,
) -> Option<String> {
    let mut production_exports = production_exports.clone();
    allow_private_production_exports(&mut production_exports, private_visibility_scope);
    let mut test_locals = test_local_symbols(
        ast,
        synthetic_imports,
        &production_exports,
        package_ids,
        package_aliases,
        disallowed_import_modules,
    );
    if let PrivateVisibilityScope::Module(module_path) = private_visibility_scope {
        test_locals.imports.insert(module_path.to_string());
    }
    if let Some(message) = check_block_private_visibility(
        &ast.tests
            .iter()
            .nth(test_index)
            .expect("test case came from source AST")
            .body,
        &production_exports,
        &test_locals,
    ) {
        return Some(message);
    }
    for ty in &ast.types {
        if let Some(message) =
            check_type_ref_private_visibility(ty.alias.as_ref(), &production_exports, &test_locals)
        {
            return Some(message);
        }
        for implemented in &ty.implements {
            if let Some(message) = check_type_name_private_visibility(
                &implemented.name,
                &production_exports,
                &test_locals,
            ) {
                return Some(message);
            }
        }
        for field in &ty.fields {
            if let Some(message) = check_type_name_private_visibility(
                &field.ty.name,
                &production_exports,
                &test_locals,
            ) {
                return Some(message);
            }
        }
    }
    for interface in &ast.interfaces {
        for operation in &interface.operations {
            if let Some(message) =
                check_operation_private_visibility(operation, &production_exports, &test_locals)
            {
                return Some(message);
            }
        }
    }
    for signature in &ast.function_signatures {
        if let Some(message) =
            check_operation_private_visibility(signature, &production_exports, &test_locals)
        {
            return Some(message);
        }
    }
    for function in &ast.functions {
        if let Some(message) =
            check_function_private_visibility(function, &production_exports, &test_locals)
        {
            return Some(message);
        }
    }
    for constant in &ast.consts {
        if let Some(ty) = &constant.ty {
            if let Some(message) =
                check_type_name_private_visibility(&ty.name, &production_exports, &test_locals)
            {
                return Some(message);
            }
        }
        if let Some(message) =
            check_expr_private_visibility(&constant.value, &production_exports, &test_locals)
        {
            return Some(message);
        }
    }
    for implementation in &ast.impls {
        if let Some(message) = check_type_name_private_visibility(
            &implementation.target,
            &production_exports,
            &test_locals,
        ) {
            return Some(message);
        }
        for method in &implementation.methods {
            if let Some(message) =
                check_operation_private_visibility(method, &production_exports, &test_locals)
            {
                return Some(message);
            }
        }
        for body in &implementation.method_bodies {
            if let Some(message) =
                check_function_private_visibility(body, &production_exports, &test_locals)
            {
                return Some(message);
            }
        }
    }
    None
}
pub(super) fn private_visibility_error_in_test_body(
    ast: &AstSourceFile,
    synthetic_imports: &BTreeSet<String>,
    test_index: usize,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    private_visibility_scope: &PrivateVisibilityScope,
    package_ids: &BTreeSet<String>,
    package_aliases: &BTreeMap<String, Vec<String>>,
    disallowed_import_modules: &BTreeSet<String>,
) -> Option<String> {
    let mut production_exports = production_exports.clone();
    allow_private_production_exports(&mut production_exports, private_visibility_scope);
    let mut test_locals = test_local_symbols(
        ast,
        synthetic_imports,
        &production_exports,
        package_ids,
        package_aliases,
        disallowed_import_modules,
    );
    if let PrivateVisibilityScope::Module(module_path) = private_visibility_scope {
        test_locals.imports.insert(module_path.to_string());
    }
    check_block_private_visibility(
        &ast.tests
            .iter()
            .nth(test_index)
            .expect("test case came from source AST")
            .body,
        &production_exports,
        &test_locals,
    )
}

fn allow_private_production_exports(
    production_exports: &mut BTreeMap<String, ProductionModuleSymbols>,
    private_visibility_scope: &PrivateVisibilityScope,
) {
    match private_visibility_scope {
        PrivateVisibilityScope::AllModules => {
            for symbols in production_exports.values_mut() {
                export_all_symbols(symbols);
            }
        }
        PrivateVisibilityScope::Module(module_path) => {
            if let Some(symbols) = production_exports.get_mut(module_path) {
                export_all_symbols(symbols);
            }
        }
        PrivateVisibilityScope::Modules(module_paths) => {
            for module_path in module_paths {
                if let Some(symbols) = production_exports.get_mut(module_path) {
                    export_all_symbols(symbols);
                }
            }
        }
        PrivateVisibilityScope::PublicOnly => {}
    }
}

fn export_all_symbols(symbols: &mut ProductionModuleSymbols) {
    for symbol in symbols.symbols.values_mut() {
        symbol.exported = true;
    }
    for symbol in symbols.member_symbols.values_mut() {
        symbol.exported = true;
    }
}

pub(super) fn test_local_symbols(
    ast: &AstSourceFile,
    synthetic_imports: &BTreeSet<String>,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    package_ids: &BTreeSet<String>,
    package_aliases: &BTreeMap<String, Vec<String>>,
    disallowed_import_modules: &BTreeSet<String>,
) -> TestLocalSymbols {
    let mut symbols = TestLocalSymbols::default();
    symbols.package_ids.extend(package_ids.iter().cloned());
    symbols.package_aliases.extend(package_aliases.clone());
    symbols
        .disallowed_import_modules
        .extend(disallowed_import_modules.iter().cloned());
    symbols
        .synthetic_imports
        .extend(synthetic_imports.iter().cloned());
    for import_path in synthetic_imports {
        add_imported_production_symbol(&mut symbols, import_path, production_exports);
    }
    symbols
        .imports
        .extend(ast.imports.iter().map(|import| import.path.join(".")));
    for import in &ast.imports {
        add_imported_production_symbol(&mut symbols, &import.path.join("."), production_exports);
    }
    for ty in &ast.types {
        symbols.types.insert(ty.name.clone());
    }
    for interface in &ast.interfaces {
        symbols.types.insert(interface.name.clone());
    }
    for function in &ast.functions {
        symbols.values.insert(function.name.clone());
        symbols.types.extend(function.type_params.iter().cloned());
    }
    for signature in &ast.function_signatures {
        symbols.types.extend(signature.type_params.iter().cloned());
    }
    for implementation in &ast.impls {
        for method in &implementation.methods {
            symbols.types.extend(method.type_params.iter().cloned());
        }
        for body in &implementation.method_bodies {
            symbols.values.insert(body.name.clone());
            symbols.types.extend(body.type_params.iter().cloned());
        }
    }
    symbols
}
pub(super) fn add_imported_production_symbol(
    symbols: &mut TestLocalSymbols,
    import_path: &str,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
) {
    let mut module_paths = production_exports
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    module_paths.sort_by_key(|module_path| std::cmp::Reverse(module_path.len()));
    for module_path in module_paths {
        let Some(rest) = import_path.strip_prefix(&format!("{module_path}.")) else {
            continue;
        };
        let module_symbols = production_exports
            .get(module_path)
            .expect("module path came from map keys");
        if let Some(symbol) = module_symbols
            .symbols
            .get(rest)
            .filter(|symbol| symbol.exported)
        {
            match symbol.kind {
                ProductionSymbolKind::Type
                | ProductionSymbolKind::DbObject
                | ProductionSymbolKind::Interface => {
                    symbols.types.insert(rest.to_string());
                }
                ProductionSymbolKind::Function
                | ProductionSymbolKind::Const
                | ProductionSymbolKind::Method => {
                    symbols.values.insert(rest.to_string());
                }
            }
        }
        return;
    }
}
pub(super) fn check_function_private_visibility(
    function: &FunctionDecl,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
) -> Option<String> {
    for param in &function.params {
        if let Some(message) =
            check_type_name_private_visibility(&param.ty.name, production_exports, test_locals)
        {
            return Some(message);
        }
    }
    check_type_name_private_visibility(&function.return_type.name, production_exports, test_locals)
        .or_else(|| {
            function.implicit_self.as_ref().and_then(|implicit_self| {
                check_type_name_private_visibility(
                    &implicit_self.name,
                    production_exports,
                    test_locals,
                )
            })
        })
        .or_else(|| check_block_private_visibility(&function.body, production_exports, test_locals))
}
pub(super) fn check_operation_private_visibility(
    operation: &InterfaceOperation,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
) -> Option<String> {
    for param in &operation.params {
        if let Some(message) =
            check_type_name_private_visibility(&param.ty.name, production_exports, test_locals)
        {
            return Some(message);
        }
    }
    check_type_name_private_visibility(&operation.return_type.name, production_exports, test_locals)
        .or_else(|| {
            operation.implicit_self.as_ref().and_then(|implicit_self| {
                check_type_name_private_visibility(
                    &implicit_self.name,
                    production_exports,
                    test_locals,
                )
            })
        })
}
pub(super) fn check_block_private_visibility(
    block: &Block,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
) -> Option<String> {
    block.statements.iter().find_map(|statement| {
        check_stmt_private_visibility(statement, production_exports, test_locals)
    })
}
pub(super) fn check_stmt_private_visibility(
    statement: &Stmt,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
) -> Option<String> {
    match statement {
        Stmt::Assert { condition, .. }
        | Stmt::Emit(condition)
        | Stmt::Spawn { call: condition }
        | Stmt::Expr(condition) => {
            check_expr_private_visibility(condition, production_exports, test_locals)
        }
        Stmt::Return(condition) => condition.as_ref().and_then(|condition| {
            check_expr_private_visibility(condition, production_exports, test_locals)
        }),
        Stmt::Throw { value } => {
            check_expr_private_visibility(value, production_exports, test_locals)
        }
        Stmt::Rethrow { exception } => {
            check_expr_private_visibility(exception, production_exports, test_locals)
        }
        Stmt::Let { ty, value, .. } => {
            check_type_ref_private_visibility(ty.as_ref(), production_exports, test_locals)
                .or_else(|| check_expr_private_visibility(value, production_exports, test_locals))
        }
        Stmt::Assign { target, value } => {
            check_expr_private_visibility(target, production_exports, test_locals)
                .or_else(|| check_expr_private_visibility(value, production_exports, test_locals))
        }
        Stmt::If {
            condition,
            then_block,
            else_block,
        } => check_expr_private_visibility(condition, production_exports, test_locals)
            .or_else(|| check_block_private_visibility(then_block, production_exports, test_locals))
            .or_else(|| {
                else_block.as_ref().and_then(|block| {
                    check_block_private_visibility(block, production_exports, test_locals)
                })
            }),
        Stmt::For { iterable, body, .. } => {
            check_expr_private_visibility(iterable, production_exports, test_locals)
                .or_else(|| check_block_private_visibility(body, production_exports, test_locals))
        }
        Stmt::DbTransaction { body } => {
            check_block_private_visibility(body, production_exports, test_locals)
        }
        Stmt::Match { value, arms } => {
            if let Some(message) =
                check_expr_private_visibility(value, production_exports, test_locals)
            {
                return Some(message);
            }
            for arm in arms {
                if let Some(message) =
                    check_pattern_private_visibility(&arm.pattern, production_exports, test_locals)
                {
                    return Some(message);
                }
                if let Some(message) =
                    check_block_private_visibility(&arm.body, production_exports, test_locals)
                {
                    return Some(message);
                }
            }
            None
        }
        Stmt::Break | Stmt::Continue => None,
    }
}
pub(super) fn check_expr_private_visibility(
    expr: &Expr,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
) -> Option<String> {
    match expr {
        Expr::Call { callee, args } => {
            if let Some(path) = expr_path(callee) {
                if let Some(message) = private_visibility_error_for_symbol_path(
                    &path,
                    production_exports,
                    test_locals,
                    SymbolUseKind::Value,
                ) {
                    return Some(message);
                }
            }
            check_expr_private_visibility(callee, production_exports, test_locals).or_else(|| {
                args.iter().find_map(|arg| {
                    check_expr_private_visibility(arg, production_exports, test_locals)
                })
            })
        }
        Expr::Generic { callee, type_args } => type_args
            .iter()
            .find_map(|ty| {
                check_type_name_private_visibility(&ty.name, production_exports, test_locals)
            })
            .or_else(|| check_expr_private_visibility(callee, production_exports, test_locals)),
        Expr::InterfaceBox { value, interface } => {
            check_type_ref_private_visibility(Some(interface), production_exports, test_locals)
                .or_else(|| check_expr_private_visibility(value, production_exports, test_locals))
        }
        Expr::Unary { expr, .. } => {
            check_expr_private_visibility(expr, production_exports, test_locals)
        }
        Expr::Binary { left, right, .. } => {
            check_expr_private_visibility(left, production_exports, test_locals)
                .or_else(|| check_expr_private_visibility(right, production_exports, test_locals))
        }
        Expr::Field { object, .. } => {
            if let Some(path) = expr_path(expr) {
                if path.contains('.') {
                    if let Some(message) = private_visibility_error_for_symbol_path(
                        &path,
                        production_exports,
                        test_locals,
                        SymbolUseKind::Value,
                    ) {
                        return Some(message);
                    }
                }
            }
            check_expr_private_visibility(object, production_exports, test_locals)
        }
        Expr::Record {
            type_name,
            type_args,
            fields,
        } => check_type_name_private_visibility(type_name, production_exports, test_locals)
            .or_else(|| {
                type_args.iter().find_map(|ty| {
                    check_type_name_private_visibility(&ty.name, production_exports, test_locals)
                })
            })
            .or_else(|| {
                fields.iter().find_map(|(_, value)| {
                    check_expr_private_visibility(value, production_exports, test_locals)
                })
            }),
        Expr::ObjectLiteral { entries } => entries.iter().find_map(|entry| {
            check_expr_private_visibility(&entry.value, production_exports, test_locals)
        }),
        Expr::Patch { target, operations } => {
            check_type_ref_private_visibility(Some(target), production_exports, test_locals)
                .or_else(|| {
                    operations.iter().find_map(|operation| match operation {
                        PatchOperation::Set { value, .. } | PatchOperation::Inc { value, .. } => {
                            check_expr_private_visibility(value, production_exports, test_locals)
                        }
                    })
                })
        }
        Expr::Throw { value } => {
            check_expr_private_visibility(value, production_exports, test_locals)
        }
        Expr::Rethrow { exception } => {
            check_expr_private_visibility(exception, production_exports, test_locals)
        }
        Expr::Catch {
            catch_type,
            try_expr,
        } => check_type_name_private_visibility(&catch_type.name, production_exports, test_locals)
            .or_else(|| check_expr_private_visibility(try_expr, production_exports, test_locals)),
        Expr::DbOperation(operation) => {
            check_db_operation_private_visibility(operation, production_exports, test_locals)
        }
        Expr::DbQuery(query) => {
            check_db_query_value_private_visibility(query, production_exports, test_locals)
        }
        Expr::DbLeaseClaim(claim) => {
            check_db_target_private_visibility(&claim.target, production_exports, test_locals)
                .or_else(|| {
                    check_expr_private_visibility(&claim.key, production_exports, test_locals)
                })
                .or_else(|| {
                    check_block_private_visibility(&claim.body, production_exports, test_locals)
                })
        }
        Expr::DbLeaseRead(read) => {
            check_db_target_private_visibility(&read.target, production_exports, test_locals)
                .or_else(|| {
                    check_expr_private_visibility(&read.key, production_exports, test_locals)
                })
        }
        Expr::DbTransaction(transaction) => {
            check_block_private_visibility(&transaction.body, production_exports, test_locals)
        }
        Expr::Literal(_) | Expr::Identifier(_) | Expr::RemotePublicInstanceSource(_) => None,
    }
}

fn check_db_query_value_private_visibility(
    query: &DbQuery,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
) -> Option<String> {
    check_db_target_private_visibility(&query.target, production_exports, test_locals).or_else(
        || check_db_query_private_visibility(&query.query, production_exports, test_locals),
    )
}

fn check_db_operation_private_visibility(
    operation: &DbOperation,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
) -> Option<String> {
    check_db_target_private_visibility(&operation.target, production_exports, test_locals)
        .or_else(|| {
            operation.selector.as_ref().and_then(|selector| {
                check_db_selector_private_visibility(selector, production_exports, test_locals)
            })
        })
        .or_else(|| {
            operation.query.as_ref().and_then(|query| {
                check_db_query_private_visibility(query, production_exports, test_locals)
            })
        })
        .or_else(|| {
            [&operation.body, &operation.insert_body]
                .into_iter()
                .flatten()
                .find_map(|body| {
                    check_db_body_private_visibility(body, production_exports, test_locals)
                })
        })
        .or_else(|| {
            operation.change.as_ref().and_then(|change| {
                change.ops.iter().find_map(|op| match op {
                    DbChangeOp::Set { value, .. }
                    | DbChangeOp::Inc { value, .. }
                    | DbChangeOp::AddToSet { value, .. }
                    | DbChangeOp::Remove { value, .. } => {
                        check_expr_private_visibility(value, production_exports, test_locals)
                    }
                    DbChangeOp::Unset { .. } => None,
                })
            })
        })
}

fn check_db_target_private_visibility(
    target: &TypeRef,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
) -> Option<String> {
    type_name_candidates(&target.name)
        .into_iter()
        .find_map(|path| {
            if db_target_path_accessible_without_import(&path, production_exports, test_locals) {
                None
            } else {
                private_visibility_error_for_symbol_path(
                    &path,
                    production_exports,
                    test_locals,
                    SymbolUseKind::Type,
                )
            }
        })
}

fn db_target_path_accessible_without_import(
    path: &str,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
) -> bool {
    let path = canonical_test_symbol_path(path, test_locals);
    let mut module_paths = production_exports
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    module_paths.sort_by_key(|module_path| std::cmp::Reverse(module_path.len()));
    for module_path in module_paths {
        let Some(rest) = path.strip_prefix(&format!("{module_path}.")) else {
            continue;
        };
        let first_segment = rest.split('.').next().unwrap_or(rest);
        let symbols = production_exports
            .get(module_path)
            .expect("module path came from map keys");
        let is_db_object = symbols.db_objects.contains(first_segment)
            || symbols
                .symbols
                .get(first_segment)
                .is_some_and(|symbol| matches!(symbol.kind, ProductionSymbolKind::DbObject));
        let is_exported = symbols
            .symbols
            .get(first_segment)
            .is_some_and(|symbol| symbol.exported);
        return is_db_object && is_exported;
    }
    false
}

fn check_db_selector_private_visibility(
    selector: &DbSelector,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
) -> Option<String> {
    match selector {
        DbSelector::Key { value } => {
            check_expr_private_visibility(value, production_exports, test_locals)
        }
        DbSelector::Query { query } => {
            check_db_query_private_visibility(query, production_exports, test_locals)
        }
    }
}

fn check_db_query_private_visibility(
    query: &DbQueryBlock,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
) -> Option<String> {
    query
        .where_clauses
        .iter()
        .find_map(|clause| {
            check_db_where_clause_private_visibility(clause, production_exports, test_locals)
        })
        .or_else(|| {
            query.limit.as_ref().and_then(|limit| {
                check_expr_private_visibility(limit, production_exports, test_locals)
            })
        })
        .or_else(|| {
            query.offset.as_ref().and_then(|offset| {
                check_expr_private_visibility(offset, production_exports, test_locals)
            })
        })
        .or_else(|| {
            query.after.as_ref().and_then(|after| {
                check_expr_private_visibility(after, production_exports, test_locals)
            })
        })
}

fn check_db_where_clause_private_visibility(
    clause: &DbWhereClause,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
) -> Option<String> {
    match clause {
        DbWhereClause::Predicate { predicate } => {
            check_expr_private_visibility(predicate, production_exports, test_locals)
        }
        DbWhereClause::Conditional {
            condition,
            predicate,
        } => check_expr_private_visibility(condition, production_exports, test_locals)
            .or_else(|| check_expr_private_visibility(predicate, production_exports, test_locals)),
    }
}

fn check_db_body_private_visibility(
    body: &DbBody,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
) -> Option<String> {
    match body {
        DbBody::ObjectFields { fields } => fields.iter().find_map(|field| {
            check_expr_private_visibility(&field.value, production_exports, test_locals)
        }),
        DbBody::Values { value } => {
            check_expr_private_visibility(value, production_exports, test_locals)
        }
    }
}
pub(super) fn check_pattern_private_visibility(
    pattern: &Pattern,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
) -> Option<String> {
    match pattern {
        Pattern::Nominal {
            name,
            type_args,
            fields,
        } => check_type_name_private_visibility(name, production_exports, test_locals)
            .or_else(|| {
                type_args.iter().find_map(|ty| {
                    check_type_name_private_visibility(&ty.name, production_exports, test_locals)
                })
            })
            .or_else(|| {
                fields.iter().find_map(|field| {
                    field.pattern.as_ref().and_then(|pattern| {
                        check_pattern_private_visibility(pattern, production_exports, test_locals)
                    })
                })
            }),
        Pattern::Record { fields } => fields.iter().find_map(|field| {
            field.pattern.as_ref().and_then(|pattern| {
                check_pattern_private_visibility(pattern, production_exports, test_locals)
            })
        }),
        Pattern::Or(patterns) => patterns.iter().find_map(|pattern| {
            check_pattern_private_visibility(pattern, production_exports, test_locals)
        }),
        Pattern::Wildcard | Pattern::Binding(_) | Pattern::Literal(_) => None,
    }
}
pub(super) fn check_type_ref_private_visibility(
    ty: Option<&TypeRef>,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
) -> Option<String> {
    ty.and_then(|ty| check_type_name_private_visibility(&ty.name, production_exports, test_locals))
}
pub(super) fn check_type_name_private_visibility(
    type_name: &str,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
) -> Option<String> {
    type_name_candidates(type_name)
        .into_iter()
        .find_map(|path| {
            private_visibility_error_for_symbol_path(
                &path,
                production_exports,
                test_locals,
                SymbolUseKind::Type,
            )
        })
}
pub(super) fn type_name_candidates(type_name: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    for name in skiff_compiler::test_support::type_name_candidates(type_name) {
        if !name.is_empty() && !is_builtin_type_name(&name) {
            candidates.push(name);
        }
    }
    candidates
}
pub(super) fn is_builtin_type_name(name: &str) -> bool {
    skiff_compiler::is_builtin_type_name(name) || is_test_runner_wrapper_type_name(name)
}
fn is_test_runner_wrapper_type_name(name: &str) -> bool {
    // Test wrapper-only generic result surface; not part of compiler prelude.
    matches!(name, "Result")
}
pub(super) fn private_visibility_error_for_symbol_path(
    path: &str,
    production_exports: &BTreeMap<String, ProductionModuleSymbols>,
    test_locals: &TestLocalSymbols,
    use_kind: SymbolUseKind,
) -> Option<String> {
    let mut module_paths = production_exports
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    module_paths.sort_by_key(|module_path| std::cmp::Reverse(module_path.len()));
    let original_path = path;
    let is_imported_package_alias_path =
        imported_package_alias_symbol_path(original_path, test_locals);
    let path = canonical_test_symbol_path(path, test_locals);
    for module_path in module_paths {
        let Some(rest) = path.strip_prefix(&format!("{module_path}.")) else {
            continue;
        };
        let symbols = production_exports
            .get(module_path)
            .expect("module path came from map keys");
        if let Some(symbol) = symbols
            .member_symbols
            .get(rest)
            .filter(|symbol| symbol.kind.matches_use(use_kind))
        {
            if !symbol.exported {
                return Some(private_visibility_message(symbol.kind, module_path, rest));
            }
            if !is_imported_package_alias_path
                && !test_locals.imports_module(module_path)
                && !test_locals.imports_path(&path)
            {
                return Some(missing_import_message(symbol.kind, module_path, rest));
            }
        }
        let first_segment = rest.split('.').next().unwrap_or(rest);
        if let Some(symbol) = symbols
            .symbols
            .get(first_segment)
            .filter(|symbol| symbol.kind.matches_use(use_kind))
        {
            if !symbol.exported {
                return Some(private_visibility_message(
                    symbol.kind,
                    module_path,
                    first_segment,
                ));
            }
            if !is_imported_package_alias_path
                && !test_locals.imports_module(module_path)
                && !test_locals.imports_path(&path)
            {
                return Some(missing_import_message(
                    symbol.kind,
                    module_path,
                    first_segment,
                ));
            }
        }
        return None;
    }
    if test_locals.contains(&path, use_kind) {
        return None;
    }
    let mut imported_package_alias_fallback = None;
    for (module_path, symbols) in production_exports {
        if let Some(symbol) = symbols
            .symbols
            .get(&path)
            .filter(|symbol| symbol.kind.matches_use(use_kind))
        {
            if imported_package_alias_accesses_symbol(
                original_path,
                &path,
                module_path,
                &path,
                test_locals,
            ) {
                if !symbol.exported {
                    return Some(private_visibility_message(symbol.kind, module_path, &path));
                }
                return None;
            }
            if !symbol.exported {
                let message = private_visibility_message(symbol.kind, module_path, &path);
                if is_imported_package_alias_path {
                    imported_package_alias_fallback.get_or_insert(message);
                    continue;
                }
                return Some(message);
            }
            if is_imported_package_alias_path {
                imported_package_alias_fallback.get_or_insert_with(|| {
                    unqualified_production_symbol_message(symbol.kind, module_path, &path)
                });
                continue;
            }
            return Some(unqualified_production_symbol_message(
                symbol.kind,
                module_path,
                &path,
            ));
        }
        if let Some(symbol) = symbols
            .member_symbols
            .get(&path)
            .filter(|symbol| symbol.kind.matches_use(use_kind))
        {
            if imported_package_alias_accesses_symbol(
                original_path,
                &path,
                module_path,
                &path,
                test_locals,
            ) {
                if !symbol.exported {
                    return Some(private_visibility_message(symbol.kind, module_path, &path));
                }
                return None;
            }
            if !symbol.exported {
                let message = private_visibility_message(symbol.kind, module_path, &path);
                if is_imported_package_alias_path {
                    imported_package_alias_fallback.get_or_insert(message);
                    continue;
                }
                return Some(message);
            }
            if is_imported_package_alias_path {
                imported_package_alias_fallback.get_or_insert_with(|| {
                    unqualified_production_symbol_message(symbol.kind, module_path, &path)
                });
                continue;
            }
            return Some(unqualified_production_symbol_message(
                symbol.kind,
                module_path,
                &path,
            ));
        }
    }
    imported_package_alias_fallback
}
fn imported_package_alias_accesses_symbol(
    original_path: &str,
    canonical_path: &str,
    module_path: &str,
    symbol_path: &str,
    test_locals: &TestLocalSymbols,
) -> bool {
    canonical_path == symbol_path
        && test_locals.package_ids.contains(module_path)
        && !test_locals.disallowed_import_modules.contains(module_path)
        && imported_package_alias_symbol_path(original_path, test_locals)
}

fn imported_package_alias_symbol_path(path: &str, test_locals: &TestLocalSymbols) -> bool {
    let Some((root, rest)) = path.split_once('.') else {
        return false;
    };
    if !test_locals.imports.contains(root) {
        return false;
    }
    let Some(target_roots) = test_locals.package_aliases.get(root) else {
        return false;
    };
    target_roots.iter().any(|target_root| {
        target_root.is_empty()
            || rest == target_root
            || rest.starts_with(&format!("{target_root}."))
    })
}
pub(super) fn canonical_test_symbol_path(path: &str, test_locals: &TestLocalSymbols) -> String {
    let Some((root, rest)) = path.split_once('.') else {
        return path.to_string();
    };
    let Some(target_roots) = test_locals.package_aliases.get(root) else {
        return path.to_string();
    };
    for target_root in target_roots {
        if target_root.is_empty() {
            return rest.to_string();
        }
        if rest == target_root || rest.starts_with(&format!("{target_root}.")) {
            return rest.to_string();
        }
    }
    path.to_string()
}
pub(super) fn private_visibility_message(
    kind: ProductionSymbolKind,
    module_path: &str,
    symbol_path: &str,
) -> String {
    format!(
        "private {} {module_path}.{symbol_path} is not visible",
        kind.label()
    )
}
pub(super) fn missing_import_message(
    kind: ProductionSymbolKind,
    module_path: &str,
    symbol_path: &str,
) -> String {
    format!(
        "production {} {module_path}.{symbol_path} must be imported before use",
        kind.label()
    )
}
pub(super) fn unqualified_production_symbol_message(
    kind: ProductionSymbolKind,
    module_path: &str,
    symbol_path: &str,
) -> String {
    format!(
        "production {} {module_path}.{symbol_path} must be accessed through an imported module",
        kind.label()
    )
}

#[cfg(test)]
mod tests;
