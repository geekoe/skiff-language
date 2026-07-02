use std::collections::BTreeMap;

pub fn package_public_path(package_id: &str, export_path: &str) -> String {
    if export_path.is_empty() {
        package_id.to_string()
    } else if package_id.is_empty() {
        export_path.to_string()
    } else {
        format!("{package_id}.{export_path}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPackageSymbol {
    pub dependency_ref: String,
    pub symbol_path: String,
}

pub struct PackageExportResolver<'a> {
    alias_export_roots: &'a BTreeMap<String, Vec<String>>,
}

impl<'a> PackageExportResolver<'a> {
    pub fn new(alias_export_roots: &'a BTreeMap<String, Vec<String>>) -> Self {
        Self { alias_export_roots }
    }

    pub fn resolve_package_symbol_path(&self, path: &str) -> Option<ResolvedPackageSymbol> {
        let (root, rest) = path.split_once('.')?;
        if !(self.is_package_dependency_root(root) || Self::is_default_package_root(root)) {
            return None;
        }
        Some(ResolvedPackageSymbol {
            dependency_ref: root.to_string(),
            symbol_path: self.canonical_symbol_path(root, rest),
        })
    }

    pub fn is_package_dependency_root(&self, root: &str) -> bool {
        self.alias_export_roots.contains_key(root)
    }

    pub fn is_default_package_root(root: &str) -> bool {
        matches!(root, "std" | "ext")
    }

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
