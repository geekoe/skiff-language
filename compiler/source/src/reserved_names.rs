use crate::{
    shared::ast::{Block, Param, SourceFile},
    shared::ast_utils::collect_reserved_binding_violations,
    shared::prelude_registry::prelude_registry,
};

pub fn validate_reserved_names(path: &str, ast: &SourceFile, violations: &mut Vec<String>) {
    for import in &ast.imports {
        if matches!(import.path.as_slice(), [root] if root == "connect") {
            continue;
        }
        if import.path.first().map(String::as_str) == Some("std") {
            if !matches!(
                import.path.as_slice(),
                [root] if root == "std"
            ) {
                let import_path = import.path.join(".");
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
        if matches!(import.path.as_slice(), [root] if root == "ext") {
            continue;
        }
        let effective_name = import
            .local_binding
            .as_deref()
            .or_else(|| import.alias.as_deref())
            .or_else(|| import.path.last().map(String::as_str));
        if let Some(name) = effective_name.filter(|name| is_reserved_prelude_name(name)) {
            violations.push(format!(
                "{path}: import alias {name} uses reserved prelude name"
            ));
        }
    }

    for ty in &ast.types {
        if is_reserved_prelude_name(&ty.name) {
            violations.push(format!(
                "{path}: type {} uses reserved prelude name",
                ty.name
            ));
        }
    }

    for alias in &ast.aliases {
        if is_reserved_prelude_name(&alias.name) {
            violations.push(format!(
                "{path}: alias {} uses reserved prelude name",
                alias.name
            ));
        }
    }

    for interface in &ast.interfaces {
        if is_reserved_prelude_name(&interface.name) {
            violations.push(format!(
                "{path}: interface {} uses reserved prelude name",
                interface.name
            ));
        }
    }

    for function in &ast.functions {
        if is_reserved_prelude_name(&function.name) {
            violations.push(format!(
                "{path}: function {} uses reserved prelude name",
                function.name
            ));
        }
        validate_reserved_params(
            path,
            "function",
            &function.name,
            &function.params,
            violations,
        );
        validate_reserved_bindings_in_block(path, &function.body, violations);
    }

    for function in &ast.function_signatures {
        if is_reserved_prelude_name(&function.name) {
            violations.push(format!(
                "{path}: function {} uses reserved prelude name",
                function.name
            ));
        }
        validate_reserved_params(
            path,
            "function",
            &function.name,
            &function.params,
            violations,
        );
    }

    for constant in &ast.consts {
        if is_reserved_prelude_name(&constant.name) {
            violations.push(format!(
                "{path}: const {} uses reserved prelude name",
                constant.name
            ));
        }
    }

    for implementation in &ast.impls {
        if is_reserved_prelude_name(&implementation.target) {
            violations.push(format!(
                "{path}: impl target {} uses reserved prelude name",
                implementation.target
            ));
        }
        for method in &implementation.methods {
            validate_reserved_params(path, "method", &method.name, &method.params, violations);
        }
        for method in &implementation.method_bodies {
            validate_reserved_params(path, "method", &method.name, &method.params, violations);
            validate_reserved_bindings_in_block(path, &method.body, violations);
        }
    }
}

fn is_reserved_prelude_name(name: &str) -> bool {
    prelude_registry().is_reserved_name(name)
}

fn validate_reserved_bindings_in_block(path: &str, block: &Block, violations: &mut Vec<String>) {
    collect_reserved_binding_violations(path, block, violations, is_reserved_prelude_name);
}

fn validate_reserved_params(
    path: &str,
    kind: &str,
    owner: &str,
    params: &[Param],
    violations: &mut Vec<String>,
) {
    for param in params {
        if is_reserved_prelude_name(&param.name) {
            violations.push(format!(
                "{path}: {kind} {owner} parameter {} uses reserved prelude name",
                param.name
            ));
        }
    }
}
