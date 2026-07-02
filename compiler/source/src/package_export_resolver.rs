use std::collections::BTreeMap;

use compiler_input_model::PackageDependency;

pub use skiff_compiler_core::package_export_resolver::{
    package_public_path, ResolvedPackageSymbol,
};

pub struct PackageExportResolver<'a> {
    alias_export_roots: &'a BTreeMap<String, Vec<String>>,
    inner: skiff_compiler_core::package_export_resolver::PackageExportResolver<'a>,
}

impl<'a> PackageExportResolver<'a> {
    pub fn new(alias_export_roots: &'a BTreeMap<String, Vec<String>>) -> Self {
        Self {
            alias_export_roots,
            inner: skiff_compiler_core::package_export_resolver::PackageExportResolver::new(
                alias_export_roots,
            ),
        }
    }

    pub fn resolve_package_symbol_path(&self, path: &str) -> Option<ResolvedPackageSymbol> {
        self.inner.resolve_package_symbol_path(path)
    }

    pub fn canonical_alias_path(&self, path: &str) -> Option<String> {
        let (root, rest) = path.split_once('.')?;
        self.alias_export_roots
            .contains_key(root)
            .then(|| self.canonical_symbol_path(root, rest))
    }

    pub fn declared_import_dependency_id<'b>(
        import: &[String],
        dependencies: &'b [PackageDependency],
    ) -> Option<&'b str> {
        declared_import_dependency_id(import, dependencies)
    }

    pub fn complex_dependency_requiring_alias_for_import<'b>(
        import: &[String],
        dependencies: &'b [PackageDependency],
    ) -> Option<&'b str> {
        complex_dependency_requiring_alias_for_import(import, dependencies)
    }

    pub fn is_complex_package_id(package_id: &str) -> bool {
        is_complex_package_id(package_id)
    }
}

impl PackageExportResolver<'_> {
    fn canonical_symbol_path(&self, root: &str, rest: &str) -> String {
        if root == "std" && !rest.starts_with("std.") {
            return format!("std.{rest}");
        }
        let Some(target_roots) = self.alias_export_roots.get(root) else {
            return rest.to_string();
        };
        for target_root in target_roots {
            if target_root.is_empty() {
                return rest.to_string();
            }
            if rest == target_root || rest.starts_with(&format!("{}.", target_root)) {
                return rest.to_string();
            }
        }
        rest.to_string()
    }
}

pub fn declared_import_dependency_id<'a>(
    import: &[String],
    dependencies: &'a [PackageDependency],
) -> Option<&'a str> {
    let full = import.join(".");
    dependencies.iter().find_map(|dependency| {
        if dependency.effective_alias() == full {
            Some(dependency.id.as_str())
        } else if dependency.alias.is_none() && can_import_package_without_alias(&dependency.id) {
            package_id_matches_import(&dependency.id, import).then_some(dependency.id.as_str())
        } else {
            None
        }
    })
}

pub fn complex_dependency_requiring_alias_for_import<'a>(
    import: &[String],
    dependencies: &'a [PackageDependency],
) -> Option<&'a str> {
    let import_root = import.first()?;
    dependencies.iter().find_map(|dependency| {
        if !is_complex_package_id(&dependency.id) || dependency.alias.is_some() {
            return None;
        }
        let package_root = dependency.id.rsplit('/').next()?;
        (import_root == package_root).then_some(dependency.id.as_str())
    })
}

pub fn is_complex_package_id(package_id: &str) -> bool {
    package_id.contains('.') || package_id.contains('/')
}

pub fn can_import_package_without_alias(package_id: &str) -> bool {
    !is_complex_package_id(package_id)
}

pub fn package_id_matches_import(package_id: &str, import: &[String]) -> bool {
    let full = import.join(".");
    if package_id == full {
        return true;
    }
    package_id
        .strip_prefix(&format!("{full}."))
        .is_some_and(|suffix| !suffix.is_empty())
}
