use std::collections::BTreeSet;

use crate::{
    shared::ast::{Block, Expr, SourceFile},
    shared::ast_utils::{block_contains_expr, expr_contains},
    shared::prelude_registry::PRELUDE_REGISTRY_ID,
};
use compiler_input_model::is_standard_package_id;

const INTERNAL_PROVIDER_PRIMITIVES: &[&str] = &[
    "__providerCallFindOne",
    "__providerCallFindMany",
    "__providerCallInsertOne",
    "__providerCallReplaceOne",
    "__providerCallDeleteOne",
];

pub fn collect_removed_connect_provider_violations(
    path: &str,
    ast: &SourceFile,
    violations: &mut Vec<String>,
) {
    if ast
        .imports
        .iter()
        .any(|import| import.path.as_slice() == ["connect"])
    {
        violations.push(format!(
            "{path}: connect package imports have been removed; use skiff.run/mongo with alias mongo instead"
        ));
    }

    if source_uses_connect_mongo_wrapper(ast) {
        violations.push(format!(
            "{path}: connect.mongo provider wrapper has been removed; use skiff.run/mongo with alias mongo instead"
        ));
    }

    for primitive in internal_provider_primitives_used_by_source(ast) {
        violations.push(format!(
            "{path}: internal provider-call primitive {primitive} has been removed from source"
        ));
    }
}

pub fn collect_non_std_package_native_function_violations(
    package_id: &str,
    path: &str,
    ast: &SourceFile,
    violations: &mut Vec<String>,
) {
    if is_standard_package_id(package_id) || package_id == PRELUDE_REGISTRY_ID {
        return;
    }
    collect_native_function_violations(
        path,
        ast,
        &format!("package {package_id}"),
        "native functions are reserved for std",
        violations,
    );
}

pub fn collect_service_native_function_violations(
    path: &str,
    ast: &SourceFile,
    violations: &mut Vec<String>,
) {
    collect_native_function_violations(
        path,
        ast,
        "service source",
        "call std native APIs or provider functions instead",
        violations,
    );
}

pub fn collect_non_std_package_native_type_violations(
    package_id: &str,
    path: &str,
    ast: &SourceFile,
    violations: &mut Vec<String>,
) {
    if is_standard_package_id(package_id) || package_id == PRELUDE_REGISTRY_ID {
        return;
    }
    collect_native_type_violations(path, ast, &format!("package {package_id}"), violations);
}

pub fn collect_service_native_type_violations(
    path: &str,
    ast: &SourceFile,
    violations: &mut Vec<String>,
) {
    collect_native_type_violations(path, ast, "service source", violations);
}

fn collect_native_function_violations(
    path: &str,
    ast: &SourceFile,
    owner: &str,
    guidance: &str,
    violations: &mut Vec<String>,
) {
    for function in ast.functions.iter().filter(|function| function.is_native) {
        violations.push(format!(
            "{path}: {owner} cannot declare native function {}; {guidance}",
            function.name
        ));
    }
    for implementation in &ast.impls {
        for method in implementation
            .methods
            .iter()
            .filter(|method| method.is_native)
        {
            violations.push(format!(
                "{path}: {owner} cannot declare native function {}.{}; {guidance}",
                implementation.target, method.name
            ));
        }
    }
}

fn collect_native_type_violations(
    path: &str,
    ast: &SourceFile,
    owner: &str,
    violations: &mut Vec<String>,
) {
    for ty in ast.types.iter().filter(|ty| ty.is_native) {
        violations.push(format!(
            "{path}: {owner} cannot declare native type {}",
            ty.name
        ));
    }
}

pub(super) fn source_uses_connect_mongo_wrapper(ast: &SourceFile) -> bool {
    ast.consts
        .iter()
        .any(|constant| expr_uses_connect_mongo_wrapper(&constant.value))
        || ast.functions.iter().any(|function| {
            block_contains_expr(&function.body, &mut expr_uses_connect_mongo_wrapper)
        })
        || ast.impls.iter().any(|implementation| {
            implementation.method_bodies.iter().any(|method| {
                block_contains_expr(&method.body, &mut expr_uses_connect_mongo_wrapper)
            })
        })
}

fn expr_uses_connect_mongo_wrapper(expr: &Expr) -> bool {
    expr_contains(expr, |candidate| match candidate {
        Expr::Field { object, field } => {
            field == "mongo"
                && matches!(object.as_ref(), Expr::Identifier(root) if root == "connect")
        }
        _ => false,
    })
}

fn internal_provider_primitives_used_by_source(ast: &SourceFile) -> BTreeSet<String> {
    let mut primitives = BTreeSet::new();
    for constant in &ast.consts {
        collect_internal_provider_primitives_from_expr(&constant.value, &mut primitives);
    }
    for function in &ast.functions {
        collect_internal_provider_primitives_from_block(&function.body, &mut primitives);
    }
    for implementation in &ast.impls {
        for method in &implementation.method_bodies {
            collect_internal_provider_primitives_from_block(&method.body, &mut primitives);
        }
    }
    primitives
}

fn collect_internal_provider_primitives_from_block(
    block: &Block,
    primitives: &mut BTreeSet<String>,
) {
    let _ = block_contains_expr(block, &mut |expr| {
        collect_internal_provider_primitives_from_expr(expr, primitives);
        false
    });
}

fn collect_internal_provider_primitives_from_expr(expr: &Expr, primitives: &mut BTreeSet<String>) {
    expr_contains(expr, |candidate| match candidate {
        Expr::Identifier(name) if INTERNAL_PROVIDER_PRIMITIVES.contains(&name.as_str()) => {
            primitives.insert(name.clone());
            false
        }
        _ => false,
    });
}
