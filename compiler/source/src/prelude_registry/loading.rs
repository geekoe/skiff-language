use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use skiff_compiler_core::{prelude_registry::validate_package_api_public_path, registry_helpers};

use crate::{
    api_yml::read_publication_api_yml,
    package_export_resolver::package_public_path,
    shared::id::{SKIFF_STD_PUBLICATION_ID, STD_SOURCE_ALIAS},
    shared::parser::parse_source,
    shared::type_syntax::generic_parts,
};

use super::{
    identity::{format_function_signature, format_operation_signature, source_fingerprint},
    module_symbol_root, type_root,
    validation::validate_root_projection_metadata,
    NativeBinding, NativeBindingShape, PreludeRegistry, PRELUDE_REGISTRY_ID,
};

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SplitStdRegistryManifest {
    schema_version: Option<String>,
    #[serde(default)]
    packages: Vec<SplitStdRegistryPackage>,
}

#[derive(Debug, Deserialize)]
struct SplitStdRegistryPackage {
    id: String,
    path: String,
}

#[derive(Debug, Deserialize, Default)]
struct SplitPackageManifest {
    id: Option<String>,
    api: Option<serde_yaml::Value>,
    exports: Option<serde_yaml::Value>,
}

#[derive(Debug, Clone)]
struct PreludeExportMapping {
    source_module: String,
    public_module: String,
}

impl PreludeRegistry {
    pub(super) fn load_std_registry(&mut self, std_dir: &Path) -> Result<(), String> {
        let std_registry_path = std_dir.join("registry.yml");
        let std_registry_text = fs::read_to_string(&std_registry_path)
            .map_err(|error| format!("failed to read {}: {error}", std_registry_path.display()))?;
        let std_registry = serde_yaml::from_str::<SplitStdRegistryManifest>(&std_registry_text)
            .map_err(|error| format!("failed to parse {}: {error}", std_registry_path.display()))?;
        if std_registry.schema_version.as_deref() != Some("skiff-std-registry-v1") {
            return Err(format!(
                "{}: schemaVersion must be skiff-std-registry-v1",
                std_registry_path.display()
            ));
        }

        self.package_id = PRELUDE_REGISTRY_ID.to_string();
        self.package_version = "1.0.0".to_string();
        self.schema_version = "skiff-prelude-schema-v1".to_string();
        self.native_schema_version = "skiff-prelude-native-v1".to_string();
        self.native_abi = "skiff-native".to_string();
        self.prelude_roots = vec!["std".to_string(), "config".to_string()];
        self.manifest_fingerprint =
            crate::shared::json_utils::sha256_hex(std_registry_text.as_bytes());
        let std_package_exports = std_registry
            .packages
            .iter()
            .map(|package| {
                validate_std_registry_package_id(&std_registry_path, &package.id)?;
                let package_dir = official_registry_package_dir(
                    std_dir,
                    &std_registry_path,
                    &package.id,
                    &package.path,
                )?;
                package_export_mappings(&package.id, &package_dir)
                    .map(|modules| (package.id.clone(), modules))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        self.root_projections = BTreeMap::from([(
            "std".to_string(),
            root_projection_mappings("std", std_package_exports.values().flatten()),
        )]);
        self.export_modules = vec![
            "std.collection".to_string(),
            "std.string".to_string(),
            "std.number".to_string(),
            "std.bytes".to_string(),
            "std.error".to_string(),
            "config".to_string(),
        ];
        for package in std_registry.packages {
            validate_std_registry_package_id(&std_registry_path, &package.id)?;
            official_registry_package_dir(std_dir, &std_registry_path, &package.id, &package.path)?;
            if let Some(modules) = std_package_exports.get(&package.id) {
                self.export_modules
                    .extend(modules.iter().map(|export| export.public_module.clone()));
            }
        }
        self.export_modules.sort();
        self.export_modules.dedup();
        Ok(())
    }

    pub(super) fn canonicalize_prelude_type_symbols(&mut self) {
        for name in ["Date", "Json", "JsonObject"] {
            if self.type_symbols.contains_key(name) {
                self.type_symbols.insert(name.to_string(), name.to_string());
            }
        }
    }

    pub(super) fn load_split_sources(
        &mut self,
        prelude_dir: &Path,
        std_dir: &Path,
    ) -> Result<(), String> {
        let mut files = Vec::new();
        collect_split_skiff_files(prelude_dir, std_dir, &mut files)?;
        if files.is_empty() {
            return Err(format!(
                "no .skiff files found under {} and {}",
                prelude_dir.display(),
                std_dir.display()
            ));
        }

        self.type_decls.clear();
        self.type_decls_by_symbol.clear();
        self.type_aliases.clear();
        self.type_aliases_by_symbol.clear();
        self.declared_native_bindings.clear();
        self.raw_declared_native_bindings.clear();
        let mut source_parts = Vec::new();
        self.source_modules = files
            .iter()
            .map(|(module_path, _)| module_symbol_root(&self.package_id, module_path))
            .collect();
        self.source_modules.sort();
        self.source_modules.dedup();
        for (module_path, path) in files {
            let text = fs::read_to_string(&path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
            source_parts.push((module_path.clone(), text.clone()));
            self.add_source(&module_path, &text)
                .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
        }
        self.schema_stable_types = self
            .type_decls
            .values()
            .map(|decl| decl.name.clone())
            .chain(self.type_aliases.values().map(|alias| alias.name.clone()))
            .chain(self.type_decls_by_symbol.keys().cloned())
            .chain(self.type_aliases_by_symbol.keys().cloned())
            .collect();
        self.schema_stable_types.sort();
        self.schema_stable_types.dedup();
        self.source_fingerprint = source_fingerprint(
            source_parts
                .iter()
                .map(|(module_path, text)| (module_path.as_str(), text.as_str())),
        );
        for (module_path, text) in source_parts {
            self.validate_source_type_refs(&module_path, &text)
                .map_err(|error| format!("failed to validate {module_path}: {error}"))?;
        }
        self.validate_export_modules()?;
        validate_root_projection_metadata(
            &self.prelude_roots,
            &self.root_projections,
            &self.source_modules,
        )?;
        self.validate_schema_stable_types()?;
        Ok(())
    }

    fn add_source(
        &mut self,
        module_path: &str,
        text: &str,
    ) -> Result<(), crate::shared::error::CompileError> {
        let source = parse_source(text)?;
        let symbol_root = module_symbol_root(&self.package_id, module_path);
        for ty in source.types {
            let symbol = format!("{}.{}", symbol_root, ty.name);
            self.type_symbols.insert(ty.name.clone(), symbol.clone());
            self.type_symbols.insert(symbol.clone(), symbol.clone());
            if ty.is_native {
                self.native_type_names.insert(ty.name.clone());
            }
            self.type_decls_by_symbol.insert(symbol, ty.clone());
            self.type_decls.insert(ty.name.clone(), ty);
        }
        for alias in source.aliases {
            let symbol = format!("{}.{}", symbol_root, alias.name);
            self.type_symbols.insert(alias.name.clone(), symbol.clone());
            self.type_symbols.insert(symbol.clone(), symbol.clone());
            self.type_aliases_by_symbol.insert(symbol, alias.clone());
            self.type_aliases.insert(alias.name.clone(), alias);
        }
        for interface in source.interfaces {
            let symbol = format!("{}.{}", symbol_root, interface.name);
            self.type_symbols
                .insert(interface.name.clone(), symbol.clone());
            self.type_symbols.insert(symbol.clone(), symbol);
        }
        for operation in source
            .function_signatures
            .iter()
            .filter(|operation| operation.is_native)
        {
            let symbol = format!("{}.{}", symbol_root, operation.name);
            let binding = NativeBinding {
                signature: format_operation_signature(true, operation),
                shape: NativeBindingShape {
                    type_params: operation.type_params.clone(),
                    params: operation
                        .params
                        .iter()
                        .map(|param| self.canonical_native_shape_type(&module_path, &param.ty.name))
                        .collect(),
                    return_type: self
                        .canonical_native_shape_type(&module_path, &operation.return_type.name),
                },
            };
            self.insert_declared_native_binding(symbol, binding);
        }
        for function in source
            .functions
            .iter()
            .filter(|function| function.is_native)
        {
            let symbol = format!("{}.{}", symbol_root, function.name);
            let binding = NativeBinding {
                signature: format_function_signature(function),
                shape: NativeBindingShape {
                    type_params: function.type_params.clone(),
                    params: function
                        .params
                        .iter()
                        .map(|param| self.canonical_native_shape_type(&module_path, &param.ty.name))
                        .collect(),
                    return_type: self
                        .canonical_native_shape_type(&module_path, &function.return_type.name),
                },
            };
            self.insert_declared_native_binding(symbol, binding);
        }
        for implementation in &source.impls {
            let owner = type_root(&implementation.target);
            for method in implementation
                .methods
                .iter()
                .filter(|method| method.is_native)
            {
                let symbol = format!("{owner}.{}", method.name);
                let params = native_method_shape_params(owner, method);
                let binding = NativeBinding {
                    signature: format_operation_signature(false, method),
                    shape: NativeBindingShape {
                        type_params: method.type_params.clone(),
                        params: params
                            .into_iter()
                            .map(|param| self.canonical_native_shape_type(&module_path, &param))
                            .collect(),
                        return_type: self
                            .canonical_native_shape_type(&module_path, &method.return_type.name),
                    },
                };
                self.insert_declared_native_binding(symbol, binding);
            }
        }
        Ok(())
    }

    fn canonical_native_shape_type(&self, module_path: &str, raw: &str) -> String {
        let name = raw.trim();
        if let Some(inner) = name.strip_suffix('?') {
            return format!(
                "{}?",
                self.canonical_native_shape_type(module_path, inner.trim())
            );
        }
        if let Some(parts) = generic_parts(name) {
            let root = self.canonical_native_shape_type(module_path, parts.root);
            let args = parts
                .args
                .into_iter()
                .map(|arg| self.canonical_native_shape_type(module_path, arg))
                .collect::<Vec<_>>()
                .join(", ");
            return format!("{root}<{args}>");
        }
        if name.contains('.') || self.native_type_names.contains(name) || name == "Duration" {
            return name.to_string();
        }
        if let Some(symbol) = self.type_symbols.get(name) {
            let module_symbol = module_symbol_root(&self.package_id, module_path);
            if symbol == &format!("{module_symbol}.{name}") {
                return symbol.clone();
            }
        }
        name.to_string()
    }

    fn insert_declared_native_binding(&mut self, symbol: String, binding: NativeBinding) {
        self.raw_declared_native_bindings
            .insert(symbol.clone(), binding.clone());
        self.declared_native_bindings.insert(symbol, binding);
    }
}

fn native_method_shape_params(
    owner: &str,
    method: &crate::shared::ast::InterfaceOperation,
) -> Vec<String> {
    let receiver = (!method.is_static).then(|| owner.to_string());
    receiver
        .into_iter()
        .chain(method.params.iter().map(|param| param.ty.name.clone()))
        .collect()
}

pub(super) fn collect_plain_files(
    root: &Path,
    dir: &Path,
    files: &mut Vec<PathBuf>,
    extensions: &[&str],
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_plain_files(root, &path, files, extensions);
            continue;
        }
        let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
            continue;
        };
        if !extensions.iter().any(|allowed| *allowed == extension) {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        files.push(relative.to_path_buf());
    }
}

fn collect_split_skiff_files(
    prelude_dir: &Path,
    std_dir: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    for entry in fs::read_dir(prelude_dir)
        .map_err(|error| format!("failed to read {}: {error}", prelude_dir.display()))?
    {
        let path = entry
            .map_err(|error| format!("failed to read {}: {error}", prelude_dir.display()))?
            .path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("skiff") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        files.push((stem.to_string(), path));
    }

    let registry_path = std_dir.join("registry.yml");
    let registry_text = fs::read_to_string(&registry_path)
        .map_err(|error| format!("failed to read {}: {error}", registry_path.display()))?;
    let registry = serde_yaml::from_str::<SplitStdRegistryManifest>(&registry_text)
        .map_err(|error| format!("failed to parse {}: {error}", registry_path.display()))?;
    for package in registry.packages {
        validate_std_registry_package_id(&registry_path, &package.id)?;
        let package_dir =
            official_registry_package_dir(std_dir, &registry_path, &package.id, &package.path)?;
        collect_std_package_skiff_files(&package_dir, files)?;
    }
    files.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    Ok(())
}

fn collect_std_package_skiff_files(
    package_dir: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    for entry in fs::read_dir(package_dir)
        .map_err(|error| format!("failed to read {}: {error}", package_dir.display()))?
    {
        let path = entry
            .map_err(|error| format!("failed to read {}: {error}", package_dir.display()))?
            .path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("skiff") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if stem.ends_with(".test") || stem.ends_with("_test") {
            continue;
        }
        files.push((format!("std.{stem}"), path));
    }
    Ok(())
}

fn validate_std_registry_package_id(registry_path: &Path, package_id: &str) -> Result<(), String> {
    registry_helpers::validate_std_registry_package_id(package_id)
        .map_err(|error| format!("{}: {error}", registry_path.display()))
}

fn official_registry_package_dir(
    std_dir: &Path,
    registry_path: &Path,
    package_id: &str,
    package_path: &str,
) -> Result<PathBuf, String> {
    registry_helpers::validate_official_registry_package_path(package_id, package_path)
        .map_err(|error| format!("{}: {error}", registry_path.display()))?;
    Ok(std_dir.join(package_path))
}

fn package_export_mappings(
    package_id: &str,
    package_dir: &Path,
) -> Result<Vec<PreludeExportMapping>, String> {
    let public_root = std_registry_public_root(package_id);
    let manifest_path = package_dir.join("package.yml");
    let text = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("failed to read {}: {error}", manifest_path.display()))?;
    let manifest = serde_yaml::from_str::<SplitPackageManifest>(&text)
        .map_err(|error| format!("failed to parse {}: {error}", manifest_path.display()))?;
    if manifest.id.as_deref() != Some(package_id) {
        return Err(format!(
            "{}: package.yml must declare id {package_id}",
            manifest_path.display()
        ));
    }
    let mut violations = Vec::new();
    if manifest.api.is_some() {
        violations.push("api has been removed; declare public API in api.yml".to_string());
    }
    if manifest.exports.is_some() {
        violations.push("exports has been removed; use top-level api".to_string());
    }
    let api = read_publication_api_yml(package_dir).map_err(|error| error.to_string())?;
    let entries = validate_package_api_export_entries(&api, public_root, &mut violations);
    if !violations.is_empty() {
        return Err(format!(
            "{}: {}",
            manifest_path.display(),
            violations.join("; ")
        ));
    }
    let mut modules = entries;
    modules.sort_by(|left, right| {
        left.public_module
            .cmp(&right.public_module)
            .then_with(|| left.source_module.cmp(&right.source_module))
    });
    modules.dedup_by(|left, right| {
        left.public_module == right.public_module && left.source_module == right.source_module
    });
    Ok(modules)
}

fn std_registry_public_root(package_id: &str) -> &str {
    if package_id == SKIFF_STD_PUBLICATION_ID {
        STD_SOURCE_ALIAS
    } else {
        package_id
    }
}

fn validate_package_api_export_entries(
    api: &compiler_input_model::PublicationApiSpec,
    package_id: &str,
    violations: &mut Vec<String>,
) -> Vec<PreludeExportMapping> {
    let mut entries = Vec::new();
    for entry in api.entries() {
        let public_path = entry.public_module_path_segment();
        validate_package_api_public_path(&public_path, package_id, violations);
        entries.push(PreludeExportMapping {
            source_module: entry.source_module_hint().to_string(),
            public_module: package_public_path(package_id, &public_path),
        });
    }
    entries
}

#[cfg(test)]
#[path = "loading/tests.rs"]
mod tests;

fn root_projection_mappings<'a>(
    root: &'a str,
    exports: impl Iterator<Item = &'a PreludeExportMapping>,
) -> BTreeMap<String, String> {
    exports
        .filter_map(|export| {
            export
                .public_module
                .strip_prefix(&format!("{root}."))
                .and_then(|name| name.split('.').next())
                .map(|name| (name.to_string(), export.public_module.clone()))
        })
        .collect()
}
