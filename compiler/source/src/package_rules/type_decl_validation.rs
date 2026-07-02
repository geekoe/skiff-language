use crate::shared::ast::{ConstDecl, FunctionDecl, InterfaceOperation};

use super::*;

pub(super) fn collect_package_const_std_type_violations(
    path: &str,
    constant: &ConstDecl,
    imported_std_roots: &BTreeSet<&str>,
    dependency_roots: &BTreeSet<&str>,
    package_type_names: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    if let Some(ty) = &constant.ty {
        collect_package_std_type_name_violations(
            path,
            &ty.name,
            imported_std_roots,
            dependency_roots,
            package_type_names,
            violations,
        );
    }
    collect_package_expr_std_type_violations(
        path,
        &constant.value,
        imported_std_roots,
        dependency_roots,
        package_type_names,
        violations,
    );
}

pub(super) fn collect_package_operation_std_type_violations(
    path: &str,
    operation: &InterfaceOperation,
    imported_std_roots: &BTreeSet<&str>,
    dependency_roots: &BTreeSet<&str>,
    package_type_names: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    for param in &operation.params {
        collect_package_std_type_name_violations(
            path,
            &param.ty.name,
            imported_std_roots,
            dependency_roots,
            package_type_names,
            violations,
        );
    }
    collect_package_std_type_name_violations(
        path,
        &operation.return_type.name,
        imported_std_roots,
        dependency_roots,
        package_type_names,
        violations,
    );
}

pub(super) fn collect_package_function_std_type_violations(
    path: &str,
    function: &FunctionDecl,
    imported_std_roots: &BTreeSet<&str>,
    dependency_roots: &BTreeSet<&str>,
    package_type_names: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    for param in &function.params {
        collect_package_std_type_name_violations(
            path,
            &param.ty.name,
            imported_std_roots,
            dependency_roots,
            package_type_names,
            violations,
        );
    }
    collect_package_std_type_name_violations(
        path,
        &function.return_type.name,
        imported_std_roots,
        dependency_roots,
        package_type_names,
        violations,
    );
    collect_package_block_std_type_violations(
        path,
        &function.body,
        imported_std_roots,
        dependency_roots,
        package_type_names,
        violations,
    );
}
