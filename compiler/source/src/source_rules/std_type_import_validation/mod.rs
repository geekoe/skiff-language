use std::collections::BTreeSet;

use crate::{
    shared::ast::{ConstDecl, FunctionDecl, InterfaceOperation, SourceFile},
    shared::ast_utils::AstVisitor,
    shared::prelude_registry::prelude_registry,
};

use self::{
    block::collect_block_std_type_import_violations, expr::StdTypeImportCollector,
    refs::collect_std_type_name_import_violations,
};

mod block;
mod expr;
mod refs;

pub fn collect_service_std_type_import_violations(
    path: &str,
    ast: &SourceFile,
    publication_type_names: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    let explicit_imported_std_roots = ast
        .imports
        .iter()
        .filter_map(|import| match import.path.as_slice() {
            [root, module] if root == "std" => Some(module.as_str()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let available_std_roots = prelude_registry().root_projection_roots("std");
    let mut imported_std_roots = available_std_roots
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    imported_std_roots.extend(explicit_imported_std_roots);

    for ty in &ast.types {
        for implemented in &ty.implements {
            collect_std_type_name_import_violations(
                path,
                &implemented.name,
                &imported_std_roots,
                publication_type_names,
                violations,
            );
        }
        if let Some(alias) = &ty.alias {
            collect_std_type_name_import_violations(
                path,
                &alias.name,
                &imported_std_roots,
                publication_type_names,
                violations,
            );
        }
        for field in &ty.fields {
            collect_std_type_name_import_violations(
                path,
                &field.ty.name,
                &imported_std_roots,
                publication_type_names,
                violations,
            );
        }
    }
    for alias in &ast.aliases {
        collect_std_type_name_import_violations(
            path,
            &alias.target_type.name,
            &imported_std_roots,
            publication_type_names,
            violations,
        );
    }
    for interface in &ast.interfaces {
        for operation in &interface.operations {
            collect_operation_std_type_import_violations(
                path,
                operation,
                &imported_std_roots,
                publication_type_names,
                violations,
            );
        }
    }
    for operation in &ast.function_signatures {
        collect_operation_std_type_import_violations(
            path,
            operation,
            &imported_std_roots,
            publication_type_names,
            violations,
        );
    }
    for function in &ast.functions {
        collect_function_std_type_import_violations(
            path,
            function,
            &imported_std_roots,
            publication_type_names,
            violations,
        );
    }
    for constant in &ast.consts {
        collect_const_std_type_import_violations(
            path,
            constant,
            &imported_std_roots,
            publication_type_names,
            violations,
        );
    }
    for implementation in &ast.impls {
        collect_std_type_name_import_violations(
            path,
            &implementation.target,
            &imported_std_roots,
            publication_type_names,
            violations,
        );
        for method in &implementation.methods {
            collect_operation_std_type_import_violations(
                path,
                method,
                &imported_std_roots,
                publication_type_names,
                violations,
            );
        }
        for method in &implementation.method_bodies {
            collect_function_std_type_import_violations(
                path,
                method,
                &imported_std_roots,
                publication_type_names,
                violations,
            );
        }
    }
}

fn collect_operation_std_type_import_violations(
    path: &str,
    operation: &InterfaceOperation,
    imported_std_roots: &BTreeSet<&str>,
    publication_type_names: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    let mut collector =
        StdTypeImportCollector::new(path, imported_std_roots, publication_type_names, violations);
    if let Some(implicit_self) = &operation.implicit_self {
        collector.visit_type_ref(implicit_self);
    }
    for param in &operation.params {
        collector.visit_type_ref(&param.ty);
    }
    collector.visit_type_ref(&operation.return_type);
}

fn collect_function_std_type_import_violations(
    path: &str,
    function: &FunctionDecl,
    imported_std_roots: &BTreeSet<&str>,
    publication_type_names: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    let mut collector =
        StdTypeImportCollector::new(path, imported_std_roots, publication_type_names, violations);
    if let Some(implicit_self) = &function.implicit_self {
        collector.visit_type_ref(implicit_self);
    }
    for param in &function.params {
        collector.visit_type_ref(&param.ty);
    }
    collector.visit_type_ref(&function.return_type);
    collect_block_std_type_import_violations(
        path,
        &function.body,
        imported_std_roots,
        publication_type_names,
        violations,
    );
}

fn collect_const_std_type_import_violations(
    path: &str,
    constant: &ConstDecl,
    imported_std_roots: &BTreeSet<&str>,
    publication_type_names: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    let mut collector =
        StdTypeImportCollector::new(path, imported_std_roots, publication_type_names, violations);
    if let Some(ty) = &constant.ty {
        collector.visit_type_ref(ty);
    }
    collector.visit_expr(&constant.value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::parser::parse_source_with_bodies_tolerant;

    #[test]
    fn match_patterns_validate_nominal_type_args() {
        let ast = parse_source_with_bodies_tolerant(
            r#"
                type Envelope {
                    value: string,
                }

                function demo(value: Envelope) -> string {
                    match value {
                        Envelope<std.http.HttpRequest> { value } => {
                            return value
                        }
                    }
                }
            "#,
        )
        .unwrap();
        let publication_type_names = ast
            .types
            .iter()
            .map(|ty| ty.name.clone())
            .collect::<BTreeSet<_>>();
        let mut violations = Vec::new();

        collect_service_std_type_import_violations(
            "service/api.skiff",
            &ast,
            &publication_type_names,
            &mut violations,
        );

        assert_eq!(violations, Vec::<String>::new());
    }

    #[test]
    fn top_level_consts_validate_std_type_refs_and_generic_args() {
        let ast = parse_source_with_bodies_tolerant(
            r#"
                const request: std.http.HttpRequest = {}
                const headers = Array.empty<std.http.HttpHeader>()
            "#,
        )
        .unwrap();
        let publication_type_names = BTreeSet::new();
        let mut violations = Vec::new();

        collect_service_std_type_import_violations(
            "service/api.skiff",
            &ast,
            &publication_type_names,
            &mut violations,
        );

        assert_eq!(violations, Vec::<String>::new());
    }
}
