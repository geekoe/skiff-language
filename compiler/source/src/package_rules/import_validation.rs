use super::*;
use crate::package_export_resolver::PackageExportResolver;

pub(super) fn validate_package_import_dependencies(
    path: &str,
    ast: &SourceFile,
    dependencies: &[PackageDependency],
    allowed_internal_imports: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    for import in &ast.imports {
        let import_path = import.path.join(".");
        if matches!(import.path.as_slice(), [root] if root == "std") {
            continue;
        }
        if allowed_internal_imports.contains(&import_path) {
            continue;
        }
        let Some(package_id) =
            PackageExportResolver::declared_import_dependency_id(&import.path, dependencies)
        else {
            if let Some(package_id) =
                PackageExportResolver::complex_dependency_requiring_alias_for_import(
                    &import.path,
                    dependencies,
                )
            {
                violations.push(format!(
                    "{path}: import {import_path} requires top-level packages alias for {package_id}"
                ));
                continue;
            }
            violations.push(format!(
                "{path}: import {import_path} requires top-level packages to include {import_path}"
            ));
            continue;
        };
        if PackageExportResolver::is_complex_package_id(package_id)
            && !dependencies.iter().any(|dependency| {
                dependency.id == package_id
                    && dependency.alias.as_deref() == Some(import_path.as_str())
            })
        {
            violations.push(format!(
                "{path}: import {import_path} requires top-level packages alias for {package_id}"
            ));
        }
    }
}

pub(super) fn implicit_std_package_root(package_id: &str, module_path: &str) -> Option<String> {
    if package_id == PRELUDE_REGISTRY_ID {
        return implicit_std_module_roots(module_path).into_iter().next();
    }
    if is_standard_package_id(package_id) {
        return implicit_std_module_roots(module_path).into_iter().next();
    }
    package_id
        .strip_prefix("std.")
        .filter(|root| module_path == format!("std.{root}"))
        .map(str::to_string)
}

pub fn implicit_std_module_roots(module_path: &str) -> Vec<String> {
    match module_path.split('.').collect::<Vec<_>>().as_slice() {
        ["std", root] => vec![(*root).to_string()],
        _ => Vec::new(),
    }
}
