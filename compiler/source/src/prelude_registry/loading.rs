use std::{collections::BTreeMap, fs, path::Path};

#[cfg(test)]
use std::path::PathBuf;

use serde::Deserialize;
use skiff_compiler_core::prelude_registry::validate_package_api_public_path;
use skiff_compiler_input::{
    platform_sources::CompilerPlatformSourceSnapshot, CompilerPlatformSources,
};

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
    pub(super) fn load_std_registry(
        &mut self,
        platform_sources: &CompilerPlatformSources,
    ) -> Result<(), String> {
        platform_sources
            .revalidate()
            .map_err(|error| error.to_string())?;
        let std_registry_path = platform_sources.registry_path();
        let std_registry_text = fs::read_to_string(&std_registry_path)
            .map_err(|error| format!("failed to read {}: {error}", std_registry_path.display()))?;

        self.package_id = PRELUDE_REGISTRY_ID.to_string();
        self.package_version = "1.0.0".to_string();
        self.schema_version = "skiff-prelude-schema-v1".to_string();
        self.native_schema_version = "skiff-prelude-native-v1".to_string();
        self.native_abi = "skiff-native".to_string();
        self.prelude_roots = vec!["std".to_string(), "config".to_string()];
        self.manifest_fingerprint =
            crate::shared::json_utils::sha256_hex(std_registry_text.as_bytes());
        let std_package_exports = platform_sources
            .official_package_roots()
            .map_err(|error| error.to_string())?
            .map(|(package_id, package_dir)| {
                package_export_mappings(package_id, package_dir)
                    .map(|modules| (package_id.to_string(), modules))
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
        for (package_id, _) in platform_sources
            .official_package_roots()
            .map_err(|error| error.to_string())?
        {
            if let Some(modules) = std_package_exports.get(package_id) {
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
        source_snapshot: &CompilerPlatformSourceSnapshot,
    ) -> Result<(), String> {
        let mut sources = source_snapshot
            .sources()
            .iter()
            .map(|(logical_path, text)| {
                snapshot_module_path(logical_path)
                    .map(|module_path| (module_path, logical_path.as_path(), text.as_str()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if sources.is_empty() {
            return Err("compiler platform source snapshot contains no .skiff files".to_string());
        }
        sources.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(right.1)));

        self.type_decls.clear();
        self.type_decls_by_symbol.clear();
        self.type_aliases.clear();
        self.type_aliases_by_symbol.clear();
        self.declared_native_bindings.clear();
        self.raw_declared_native_bindings.clear();
        self.source_modules = sources
            .iter()
            .map(|(module_path, _, _)| module_symbol_root(&self.package_id, module_path))
            .collect();
        self.source_modules.sort();
        self.source_modules.dedup();
        self.prelude_identity_parts = sources
            .iter()
            .filter_map(|(_, logical_path, text)| {
                logical_path
                    .strip_prefix("prelude")
                    .ok()
                    .map(|relative| (relative, *text))
            })
            .flat_map(|(relative, text)| {
                [relative.to_string_lossy().into_owned(), text.to_string()]
            })
            .collect();
        for (module_path, logical_path, text) in &sources {
            self.add_source(module_path, text)
                .map_err(|error| format!("failed to parse {}: {error}", logical_path.display()))?;
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
            sources
                .iter()
                .map(|(module_path, _, text)| (module_path.as_str(), *text)),
        );
        for (module_path, _, text) in sources {
            self.validate_source_type_refs(&module_path, text)
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

    fn add_source(&mut self, module_path: &str, text: &str) -> Result<(), String> {
        let source = parse_source(text).map_err(|error| error.to_string())?;
        let symbol_root = module_symbol_root(&self.package_id, module_path);
        for ty in source.types {
            if skiff_compiler_core::prelude_registry::compiler_builtin_type(&ty.name).is_some() {
                return Err(format!(
                    "standard_library source must not declare compiler builtin type {}",
                    ty.name
                ));
            }
            let symbol = format!("{}.{}", symbol_root, ty.name);
            self.type_symbols.insert(ty.name.clone(), symbol.clone());
            self.type_symbols.insert(symbol.clone(), symbol.clone());
            self.type_decls_by_symbol.insert(symbol, ty.clone());
            self.type_decls.insert(ty.name.clone(), ty);
        }
        for alias in source.aliases {
            if skiff_compiler_core::prelude_registry::compiler_builtin_type(&alias.name).is_some() {
                return Err(format!(
                    "standard_library source must not declare compiler builtin type {}",
                    alias.name
                ));
            }
            let symbol = format!("{}.{}", symbol_root, alias.name);
            self.type_symbols.insert(alias.name.clone(), symbol.clone());
            self.type_symbols.insert(symbol.clone(), symbol.clone());
            self.type_aliases_by_symbol.insert(symbol, alias.clone());
            self.type_aliases.insert(alias.name.clone(), alias);
        }
        for interface in source.interfaces {
            if skiff_compiler_core::prelude_registry::compiler_builtin_type(&interface.name)
                .is_some()
            {
                return Err(format!(
                    "standard_library source must not declare compiler builtin type {}",
                    interface.name
                ));
            }
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

fn snapshot_module_path(logical_path: &Path) -> Result<String, String> {
    let parent = logical_path.parent().ok_or_else(|| {
        format!(
            "platform source snapshot path {} has no logical root",
            logical_path.display()
        )
    })?;
    let stem = logical_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| {
            format!(
                "platform source snapshot path {} has no UTF-8 module name",
                logical_path.display()
            )
        })?;
    match parent.to_str() {
        Some("prelude") => Ok(stem.to_string()),
        Some("std") => Ok(format!("std.{stem}")),
        _ => Err(format!(
            "platform source snapshot path {} has unknown logical root",
            logical_path.display()
        )),
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

#[cfg(test)]
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
