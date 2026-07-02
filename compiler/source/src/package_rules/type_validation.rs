use super::*;
use crate::parsed_sources::ParsedCompilerSource;

pub(super) fn package_source_type_names(
    parsed_sources: &[ParsedCompilerSource],
) -> BTreeSet<String> {
    parsed_sources
        .iter()
        .flat_map(|parsed| {
            parsed
                .ast()
                .types
                .iter()
                .map(|ty| ty.name.clone())
                .chain(parsed.ast().aliases.iter().map(|alias| alias.name.clone()))
        })
        .collect()
}

pub(super) fn collect_package_std_type_dependency_violations(
    path: &str,
    ast: &SourceFile,
    dependencies: &[PackageDependency],
    package_type_names: &BTreeSet<String>,
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
    let explicit_dependency_roots = dependencies
        .iter()
        .filter_map(|dependency| dependency.id.strip_prefix("std."))
        .collect::<BTreeSet<_>>();
    let mut dependency_roots = available_std_roots
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    dependency_roots.extend(explicit_dependency_roots);

    for ty in &ast.types {
        for implemented in &ty.implements {
            collect_package_std_type_name_violations(
                path,
                &implemented.name,
                &imported_std_roots,
                &dependency_roots,
                package_type_names,
                violations,
            );
        }
        if let Some(alias) = &ty.alias {
            collect_package_std_type_name_violations(
                path,
                &alias.name,
                &imported_std_roots,
                &dependency_roots,
                package_type_names,
                violations,
            );
        }
        for field in &ty.fields {
            collect_package_std_type_name_violations(
                path,
                &field.ty.name,
                &imported_std_roots,
                &dependency_roots,
                package_type_names,
                violations,
            );
        }
    }
    for alias in &ast.aliases {
        collect_package_std_type_name_violations(
            path,
            &alias.target_type.name,
            &imported_std_roots,
            &dependency_roots,
            package_type_names,
            violations,
        );
    }
    for constant in &ast.consts {
        collect_package_const_std_type_violations(
            path,
            constant,
            &imported_std_roots,
            &dependency_roots,
            package_type_names,
            violations,
        );
    }
    for interface in &ast.interfaces {
        for operation in &interface.operations {
            collect_package_operation_std_type_violations(
                path,
                operation,
                &imported_std_roots,
                &dependency_roots,
                package_type_names,
                violations,
            );
        }
    }
    for operation in &ast.function_signatures {
        collect_package_operation_std_type_violations(
            path,
            operation,
            &imported_std_roots,
            &dependency_roots,
            package_type_names,
            violations,
        );
    }
    for function in &ast.functions {
        collect_package_function_std_type_violations(
            path,
            function,
            &imported_std_roots,
            &dependency_roots,
            package_type_names,
            violations,
        );
    }
    for implementation in &ast.impls {
        collect_package_std_type_name_violations(
            path,
            &implementation.target,
            &imported_std_roots,
            &dependency_roots,
            package_type_names,
            violations,
        );
        for method in &implementation.methods {
            collect_package_operation_std_type_violations(
                path,
                method,
                &imported_std_roots,
                &dependency_roots,
                package_type_names,
                violations,
            );
        }
        for method in &implementation.method_bodies {
            collect_package_function_std_type_violations(
                path,
                method,
                &imported_std_roots,
                &dependency_roots,
                package_type_names,
                violations,
            );
        }
    }
}
