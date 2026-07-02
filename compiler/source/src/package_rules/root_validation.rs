use super::*;

pub(super) fn validate_package_reserved_roots(
    path: &str,
    ast: &SourceFile,
    _dependencies: &[PackageDependency],
    violations: &mut Vec<String>,
) {
    for import in &ast.imports {
        if import.path.first().map(String::as_str) == Some("std") {
            let import_path = import.path.join(".");
            if !matches!(import.path.as_slice(), [root] if root == "std") {
                let legacy_values_import = ["std", "values"].join(".");
                if import_path == legacy_values_import {
                    violations.push(format!(
                        "{path}: import {legacy_values_import} is invalid: use config.require<T>(path) or config.optional<T>(path)"
                    ));
                } else {
                    violations.push(format!(
                        "{path}: import {import_path} is invalid: use import std"
                    ));
                }
            }
            continue;
        }
        if import.path.first().is_some_and(|root| root == "connect") {
            violations.push(format!(
                "{path}: import {} uses reserved prelude name",
                import.path.join(".")
            ));
        }
    }
    for ty in &ast.types {
        if is_reserved_root(&ty.name) {
            violations.push(format!(
                "{path}: type {} uses reserved prelude name",
                ty.name
            ));
        }
    }
    for alias in &ast.aliases {
        if is_reserved_root(&alias.name) {
            violations.push(format!(
                "{path}: alias {} uses reserved prelude name",
                alias.name
            ));
        }
    }
    for interface in &ast.interfaces {
        if is_reserved_root(&interface.name) {
            violations.push(format!(
                "{path}: interface {} uses reserved prelude name",
                interface.name
            ));
        }
    }
    for function in &ast.functions {
        if is_reserved_root(&function.name) {
            violations.push(format!(
                "{path}: function {} uses reserved prelude name",
                function.name
            ));
        }
        validate_package_reserved_roots_in_block(path, &function.body, violations);
    }
    for constant in &ast.consts {
        if is_reserved_root(&constant.name) {
            violations.push(format!(
                "{path}: const {} uses reserved prelude name",
                constant.name
            ));
        }
    }
    for implementation in &ast.impls {
        if is_reserved_root(&implementation.target) {
            violations.push(format!(
                "{path}: impl target {} uses reserved prelude name",
                implementation.target
            ));
        }
        for method in &implementation.method_bodies {
            validate_package_reserved_roots_in_block(path, &method.body, violations);
        }
    }
}
