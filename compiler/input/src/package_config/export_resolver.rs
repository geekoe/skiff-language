use std::collections::BTreeMap;

use super::{package_manifest_key, PackageDependency, PackageManifest, PackageManifestKey};

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

    pub fn alias_bindings(
        dependencies: &[PackageDependency],
        available: &BTreeMap<PackageManifestKey, PackageManifest>,
    ) -> BTreeMap<String, Vec<String>> {
        dependencies
            .iter()
            .filter_map(|dependency| {
                let alias = dependency.alias.as_ref()?;
                let manifest =
                    available.get(&package_manifest_key(&dependency.id, &dependency.version))?;
                let roots = Self::export_roots_for_alias(manifest);
                (!roots.is_empty()).then(|| (alias.clone(), roots))
            })
            .collect()
    }

    pub fn declared_import_dependency_id<'b>(
        import: &[String],
        dependencies: &'b [PackageDependency],
    ) -> Option<&'b str> {
        let full = import.join(".");
        dependencies.iter().find_map(|dependency| {
            if dependency.effective_alias() == full {
                Some(dependency.id.as_str())
            } else if dependency.alias.is_none()
                && Self::can_import_package_without_alias(&dependency.id)
            {
                Self::package_id_matches_import(&dependency.id, import)
                    .then_some(dependency.id.as_str())
            } else {
                None
            }
        })
    }

    pub fn complex_dependency_requiring_alias_for_import<'b>(
        import: &[String],
        dependencies: &'b [PackageDependency],
    ) -> Option<&'b str> {
        let import_root = import.first()?;
        dependencies.iter().find_map(|dependency| {
            if !Self::is_complex_package_id(&dependency.id) || dependency.alias.is_some() {
                return None;
            }
            let package_root = dependency.id.rsplit('/').next()?;
            (import_root == package_root).then_some(dependency.id.as_str())
        })
    }

    pub fn dependency_ids_for_import(
        import: &[String],
        dependencies: &[PackageDependency],
        available: &BTreeMap<PackageManifestKey, PackageManifest>,
    ) -> PackageImportResolution {
        let full = import.join(".");
        let alias_matches = dependencies
            .iter()
            .filter(|dependency| dependency.effective_alias() == full)
            .map(|dependency| package_manifest_key(&dependency.id, &dependency.version))
            .collect::<Vec<_>>();
        if !alias_matches.is_empty() {
            return PackageImportResolution {
                package_keys: alias_matches,
                blocked_complex_package_ids: Vec::new(),
            };
        }

        let declared_matches = dependencies
            .iter()
            .filter(|dependency| {
                dependency.alias.is_none()
                    && Self::can_import_package_without_alias(&dependency.id)
                    && Self::package_id_matches_import(&dependency.id, import)
            })
            .map(|dependency| package_manifest_key(&dependency.id, &dependency.version))
            .collect::<Vec<_>>();
        if !declared_matches.is_empty() {
            return PackageImportResolution {
                package_keys: declared_matches,
                blocked_complex_package_ids: Vec::new(),
            };
        }

        if let Some((key, _manifest)) = available.iter().find(|((id, _version), manifest)| {
            id == &full && Self::can_import_package_without_alias(manifest.id.as_str())
        }) {
            return PackageImportResolution {
                package_keys: vec![key.clone()],
                blocked_complex_package_ids: Vec::new(),
            };
        }

        let mut package_keys = Vec::new();
        let mut blocked_complex_package_ids = Vec::new();
        for (key, manifest) in available {
            if !Self::import_matches_package(import, manifest) {
                continue;
            }
            if Self::can_import_package_without_alias(manifest.id.as_str()) {
                package_keys.push(key.clone());
            } else {
                blocked_complex_package_ids.push(manifest.id.to_string());
            }
        }
        for dependency in dependencies {
            if Self::complex_dependency_matches_import_root(import, dependency) {
                blocked_complex_package_ids.push(dependency.id.clone());
            }
        }
        blocked_complex_package_ids.sort();
        blocked_complex_package_ids.dedup();

        PackageImportResolution {
            package_keys,
            blocked_complex_package_ids,
        }
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

    pub fn canonical_alias_path(&self, path: &str) -> Option<String> {
        let (root, rest) = path.split_once('.')?;
        self.alias_export_roots
            .contains_key(root)
            .then(|| self.canonical_symbol_path(root, rest))
    }

    pub fn is_package_dependency_root(&self, root: &str) -> bool {
        self.alias_export_roots.contains_key(root)
    }

    pub fn is_default_package_root(root: &str) -> bool {
        matches!(root, "std" | "ext")
    }

    pub fn is_complex_package_id(package_id: &str) -> bool {
        package_id.contains('.') || package_id.contains('/')
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

    fn export_roots_for_alias(manifest: &PackageManifest) -> Vec<String> {
        let mut roots =
            manifest
                .api
                .entries()
                .map(|entry| Self::export_root_for_alias(manifest, &entry.public_path_string()))
                .chain(manifest.api.public_instances().map(|entry| {
                    Self::export_root_for_alias(manifest, &entry.public_path_string())
                }))
                .collect::<Vec<_>>();
        roots.sort();
        roots.dedup();
        roots
    }

    fn export_root_for_alias(manifest: &PackageManifest, path: &str) -> String {
        let scoped_path = Self::package_scoped_export_path(manifest.id.as_str(), path.trim());
        let mut parts = scoped_path.split('.');
        let Some(first) = parts.next() else {
            return String::new();
        };
        match parts.next() {
            Some(second) => format!("{first}.{second}"),
            None => first.to_string(),
        }
    }

    fn package_scoped_export_path(_package_id: &str, export_path: &str) -> String {
        if export_path.is_empty() {
            return String::new();
        }
        export_path.to_string()
    }

    fn can_import_package_without_alias(package_id: &str) -> bool {
        !Self::is_complex_package_id(package_id)
    }

    fn complex_dependency_matches_import_root(
        import: &[String],
        dependency: &PackageDependency,
    ) -> bool {
        if !Self::is_complex_package_id(&dependency.id) || dependency.alias.is_some() {
            return false;
        }
        let Some(import_root) = import.first() else {
            return false;
        };
        let Some(package_root) = dependency.id.rsplit('/').next() else {
            return false;
        };
        import_root == package_root
    }

    fn import_matches_package(import: &[String], manifest: &PackageManifest) -> bool {
        let full = import.join(".");
        if import.len() == 1 {
            let root_prefix = format!("{full}.");
            if manifest
                .api
                .public_modules(manifest.id.as_str())
                .any(|module| module.starts_with(&root_prefix))
            {
                return true;
            }
        }
        if manifest
            .api
            .public_modules(manifest.id.as_str())
            .any(|module| module == full)
        {
            return true;
        }
        if import.len() <= 1 {
            return false;
        }
        let module = import[..import.len() - 1].join(".");
        manifest
            .api
            .public_modules(manifest.id.as_str())
            .any(|exported| exported == module)
    }
}

pub struct PackageImportResolution {
    pub package_keys: Vec<PackageManifestKey>,
    pub blocked_complex_package_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ManifestOwner, ManifestProvenance, PublicationApiPublicInstanceEntry, PublicationApiSpec,
        PublicationManifest,
    };
    use skiff_compiler_core::id::PublicationId;

    #[test]
    fn alias_empty_api_root_exposes_symbols_directly_under_alias() {
        let aliases = BTreeMap::from([("llm".to_string(), vec![String::new()])]);
        let resolver = PackageExportResolver::new(&aliases);

        let symbol = resolver
            .resolve_package_symbol_path("llm.chat")
            .expect("alias symbol should resolve");

        assert_eq!(symbol.dependency_ref, "llm");
        assert_eq!(symbol.symbol_path, "chat");
    }

    #[test]
    fn alias_public_path_requires_explicit_public_path_segment() {
        let aliases = BTreeMap::from([("llm".to_string(), vec!["llm".to_string()])]);
        let resolver = PackageExportResolver::new(&aliases);

        let shorthand = resolver
            .resolve_package_symbol_path("llm.chat")
            .expect("alias root should still be recognized");
        let explicit = resolver
            .resolve_package_symbol_path("llm.llm.chat")
            .expect("explicit public path should resolve");

        assert_eq!(shorthand.symbol_path, "chat");
        assert_eq!(explicit.symbol_path, "llm.chat");
    }

    #[test]
    fn default_std_root_keeps_canonical_std_symbol_prefix() {
        let aliases = BTreeMap::new();
        let resolver = PackageExportResolver::new(&aliases);

        let symbol = resolver
            .resolve_package_symbol_path("std.websocket.TextConnectionMessage")
            .expect("std symbol should resolve through default root");

        assert_eq!(symbol.dependency_ref, "std");
        assert_eq!(symbol.symbol_path, "std.websocket.TextConnectionMessage");
    }

    #[test]
    fn alias_bindings_include_public_instance_roots() {
        let dependency = PackageDependency {
            id: "example.com/llm".to_string(),
            version: "0.1.0".to_string(),
            alias: Some("llm".to_string()),
            config: serde_json::json!({}),
            collection_name_mapping: BTreeMap::new(),
        };
        let manifest = PackageManifest::new(PublicationManifest::new(
            PublicationId::parse("example.com/llm").unwrap(),
            "0.1.0".to_string(),
            PublicationApiSpec::from_public_instances(vec![
                PublicationApiPublicInstanceEntry::for_source(
                    "managedLlm",
                    "root.llm.managedLlm",
                    ["root.llm.ManagedLlm"],
                )
                .unwrap(),
            ]),
            Vec::new(),
            ManifestProvenance::synthetic("package.yml", ManifestOwner::UserOrBuiltinPackage),
        ));
        let available = BTreeMap::from([(
            package_manifest_key(&dependency.id, &dependency.version),
            manifest,
        )]);

        let aliases = PackageExportResolver::alias_bindings(&[dependency], &available);
        let resolver = PackageExportResolver::new(&aliases);
        let symbol = resolver
            .resolve_package_symbol_path("llm.managedLlm.sendChat")
            .expect("public instance alias root should resolve");

        assert_eq!(aliases.get("llm"), Some(&vec!["managedLlm".to_string()]));
        assert_eq!(symbol.dependency_ref, "llm");
        assert_eq!(symbol.symbol_path, "managedLlm.sendChat");
    }
}
