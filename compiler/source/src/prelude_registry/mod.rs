use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::OnceLock,
};

use skiff_artifact_model::STD_NATIVE_SIGNATURES;
pub use skiff_compiler_core::prelude_registry::PRELUDE_REGISTRY_ID;
use skiff_compiler_core::prelude_registry::{
    compiler_owned_type_symbol, config_prelude_type, is_language_builtin_type_name,
    is_prelude_canonical_type, module_symbol_root, primitive_type_symbols, qualified_prelude_type,
    schema_primitive_type, NativeBindingShape, LANGUAGE_PRIMITIVES, RESERVED_ROOT_NAMES,
};

use crate::{
    shared::ast::{AliasDecl, TypeDecl},
    shared::type_syntax::generic_parts,
};

mod identity;
mod loading;
mod validation;

pub use self::identity::{prelude_identity, prelude_schema_identity};

#[derive(Debug, Clone)]
pub struct PreludeRegistry {
    pub package_id: String,
    pub package_version: String,
    pub schema_version: String,
    pub native_schema_version: String,
    pub native_abi: String,
    manifest_fingerprint: String,
    source_fingerprint: String,
    export_modules: Vec<String>,
    root_projections: BTreeMap<String, BTreeMap<String, String>>,
    prelude_types: Vec<String>,
    prelude_roots: Vec<String>,
    schema_stable_types: Vec<String>,
    native_symbols: Vec<String>,
    native_bindings: BTreeMap<String, NativeBinding>,
    declared_native_bindings: BTreeMap<String, NativeBinding>,
    raw_declared_native_bindings: BTreeMap<String, NativeBinding>,
    type_decls: BTreeMap<String, TypeDecl>,
    type_decls_by_symbol: BTreeMap<String, TypeDecl>,
    type_aliases: BTreeMap<String, AliasDecl>,
    type_aliases_by_symbol: BTreeMap<String, AliasDecl>,
    type_symbols: BTreeMap<String, String>,
    source_modules: Vec<String>,
    native_type_names: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeBinding {
    signature: String,
    shape: NativeBindingShape,
}

impl PreludeRegistry {
    fn empty() -> Self {
        Self {
            package_id: PRELUDE_REGISTRY_ID.to_string(),
            package_version: String::new(),
            schema_version: String::new(),
            native_schema_version: String::new(),
            native_abi: "skiff-native".to_string(),
            manifest_fingerprint: String::new(),
            source_fingerprint: String::new(),
            export_modules: Vec::new(),
            root_projections: BTreeMap::new(),
            prelude_types: Vec::new(),
            prelude_roots: Vec::new(),
            schema_stable_types: Vec::new(),
            native_symbols: Vec::new(),
            native_bindings: BTreeMap::new(),
            declared_native_bindings: BTreeMap::new(),
            raw_declared_native_bindings: BTreeMap::new(),
            type_decls: BTreeMap::new(),
            type_decls_by_symbol: BTreeMap::new(),
            type_aliases: BTreeMap::new(),
            type_aliases_by_symbol: BTreeMap::new(),
            type_symbols: BTreeMap::new(),
            source_modules: Vec::new(),
            native_type_names: BTreeSet::new(),
        }
    }

    pub fn try_from_split_dirs(prelude_dir: &Path, std_dir: &Path) -> Result<Self, String> {
        let mut registry = Self::empty();
        registry.load_std_registry(std_dir)?;
        registry.load_split_sources(prelude_dir, std_dir)?;
        registry.derive_prelude_types();
        registry.canonicalize_prelude_type_symbols();
        registry.native_bindings = registry.declared_native_bindings.clone();
        registry.install_shared_native_aliases();
        registry.native_symbols = registry.native_bindings.keys().cloned().collect();
        registry.native_symbols.sort();
        Ok(registry)
    }

    pub fn type_decl(&self, name: &str) -> Option<&TypeDecl> {
        let name = name.trim();
        if let Some(decl) = self.type_decls_by_symbol.get(name) {
            return Some(decl);
        }
        let name = self.prelude_type_decl_name(name).unwrap_or(name);
        self.type_decls.get(name)
    }

    pub fn type_alias(&self, name: &str) -> Option<&AliasDecl> {
        let name = name.trim();
        if let Some(alias) = self.type_aliases_by_symbol.get(name) {
            return Some(alias);
        }
        let name = self.prelude_type_decl_name(name).unwrap_or(name);
        self.type_aliases.get(name)
    }

    pub fn type_decl_module(&self, name: &str) -> Option<&str> {
        let symbol = self.known_type_symbol(name)?;
        let symbol = self.type_symbols.get(&symbol)?;
        symbol.rsplit_once('.').map(|(module, _)| module)
    }

    pub fn type_symbol(&self, name: &str) -> String {
        if let Some(symbol) = self.known_type_symbol(name) {
            return symbol;
        }
        if let Some(symbol) = compiler_owned_type_symbol(name) {
            return symbol.to_string();
        }
        self.type_symbols
            .get(name)
            .cloned()
            .unwrap_or_else(|| format!("{}.unknown", self.package_id))
    }

    pub fn known_type_symbol(&self, name: &str) -> Option<String> {
        let name = name.trim();
        if self
            .type_symbols
            .get(name)
            .is_some_and(|symbol| symbol == name)
        {
            return Some(name.to_string());
        }
        if let Some(bare) = self.prelude_type_decl_name(name) {
            return self.type_symbols.get(bare).cloned();
        }
        if let Some((_, bare)) = qualified_prelude_type(name) {
            let symbol = self.type_symbols.get(bare)?;
            if is_prelude_canonical_type(bare) {
                return Some(symbol.clone());
            }
            if symbol == name {
                return Some(symbol.clone());
            }
        }
        self.type_symbols.get(name).cloned()
    }

    pub fn is_reserved_name(&self, name: &str) -> bool {
        self.prelude_types.iter().any(|reserved| reserved == name)
            || self.prelude_roots.iter().any(|reserved| reserved == name)
            || self.type_decls.contains_key(name)
            || self.type_aliases.contains_key(name)
            || RESERVED_ROOT_NAMES.contains(&name)
    }

    pub fn is_prelude_type_name(&self, name: &str) -> bool {
        let name = name.trim();
        self.prelude_types.iter().any(|ty| ty == name)
            || qualified_prelude_type(name).is_some_and(|_| self.known_type_symbol(name).is_some())
            || config_prelude_type(name).is_some_and(|_| self.known_type_symbol(name).is_some())
            || self.prelude_type_decl_name(name).is_some()
            || ((self.type_decls.contains_key(name) || self.type_aliases.contains_key(name))
                && !self.package_schema_type_requires_import(name))
    }

    pub fn is_native_type_name(&self, name: &str) -> bool {
        let name = name.trim();
        self.native_type_names.contains(name)
    }

    pub fn native_type_names(&self) -> &BTreeSet<String> {
        &self.native_type_names
    }

    pub fn prelude_types(&self) -> &[String] {
        &self.prelude_types
    }

    pub fn prelude_roots(&self) -> &[String] {
        &self.prelude_roots
    }

    pub fn declared_types(&self) -> impl Iterator<Item = &TypeDecl> {
        self.type_decls.values()
    }

    pub fn type_symbols(&self) -> &BTreeMap<String, String> {
        &self.type_symbols
    }

    pub fn schema_stable_types(&self) -> &[String] {
        &self.schema_stable_types
    }

    fn derive_prelude_types(&mut self) {
        self.prelude_types = LANGUAGE_PRIMITIVES
            .iter()
            .map(|s| s.to_string())
            .chain(self.native_type_names.iter().cloned())
            .chain(["Duration"].into_iter().map(str::to_string))
            .collect();
        self.prelude_types.sort();
        self.prelude_types.dedup();
    }

    pub fn is_schema_stable_type(&self, name: &str) -> bool {
        let name = if let Some(symbol) = self.known_type_symbol(name) {
            symbol
        } else {
            self.prelude_type_decl_name(name)
                .unwrap_or(name.trim())
                .to_string()
        };
        self.schema_stable_types.iter().any(|ty| ty == &name)
    }

    pub fn prelude_type_decl_name<'a>(&'a self, name: &'a str) -> Option<&'a str> {
        let name = name.trim();
        if self.prelude_types.iter().any(|ty| ty == name) && self.type_decls.contains_key(name) {
            return Some(name);
        }
        let (module, bare) = qualified_prelude_type(name).or_else(|| config_prelude_type(name))?;
        if self
            .type_symbols
            .get(name)
            .is_some_and(|symbol| symbol == name)
        {
            return Some(bare);
        }
        let symbol = self.type_symbols.get(bare)?;
        if is_prelude_canonical_type(bare) && module == "std.json" {
            return Some(bare);
        }
        (symbol == &format!("{module}.{bare}")).then_some(bare)
    }

    pub fn package_schema_type_requires_import(&self, name: &str) -> bool {
        let Some(symbol) = self.known_type_symbol(name) else {
            return false;
        };
        self.root_projections.get("std").is_some_and(|roots| {
            roots
                .values()
                .any(|module| symbol.starts_with(&format!("{module}.")))
        })
    }

    pub fn is_bare_raw_http_envelope_type(&self, name: &str) -> bool {
        let name = name.trim();
        if !matches!(name, "HttpRequest" | "HttpResponse") {
            return false;
        }
        self.known_type_symbol(name).is_some_and(|symbol| {
            matches!(
                symbol.as_str(),
                "std.http.HttpRequest" | "std.http.HttpResponse"
            )
        })
    }

    pub fn root_projection_roots(&self, root: &str) -> BTreeSet<String> {
        self.root_projections
            .get(root)
            .map(|projections| projections.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn native_return_type(&self, symbol: &str) -> Option<String> {
        self.native_bindings
            .get(symbol)
            .map(|binding| binding.shape.return_type.clone())
    }

    pub fn native_type_params(&self, symbol: &str) -> Option<&[String]> {
        self.native_bindings
            .get(symbol)
            .map(|binding| binding.shape.type_params.as_slice())
    }

    pub fn native_params(&self, symbol: &str) -> Option<&[String]> {
        self.native_bindings
            .get(symbol)
            .map(|binding| binding.shape.params.as_slice())
    }

    pub fn is_native_symbol(&self, symbol: &str) -> bool {
        self.native_bindings.contains_key(symbol)
    }

    pub fn is_native_symbol_root(&self, root: &str) -> bool {
        self.native_symbols.iter().any(|symbol| {
            symbol
                .split_once('.')
                .is_some_and(|(candidate, _)| candidate == root)
        })
    }

    pub fn native_binding_key(&self, symbol: &str) -> Option<&'static str> {
        self.is_native_symbol(symbol)
            .then(|| shared_native_binding_key(symbol))
            .flatten()
    }

    fn install_shared_native_aliases(&mut self) {
        for signature in STD_NATIVE_SIGNATURES {
            let aliases = signature
                .aliases
                .iter()
                .copied()
                .filter(|alias| shared_native_alias_allowed(alias))
                .collect::<Vec<_>>();
            let source = std::iter::once(signature.target)
                .chain(aliases.iter().copied())
                .find_map(|symbol| self.native_bindings.get(symbol).cloned());
            let Some(binding) = source else {
                continue;
            };
            self.native_bindings
                .entry(signature.target.to_string())
                .or_insert_with(|| binding.clone());
            self.declared_native_bindings
                .entry(signature.target.to_string())
                .or_insert_with(|| binding.clone());
            for alias in aliases {
                self.native_bindings
                    .entry(alias.to_string())
                    .or_insert_with(|| binding.clone());
                self.declared_native_bindings
                    .entry(alias.to_string())
                    .or_insert_with(|| binding.clone());
            }
        }
    }
}

pub fn shared_native_alias_target(symbol: &str) -> Option<&'static str> {
    shared_native_aliases()
        .into_iter()
        .find_map(|(alias, canonical)| (alias == symbol).then_some(canonical))
}

pub fn shared_native_binding_key(symbol: &str) -> Option<&'static str> {
    STD_NATIVE_SIGNATURES.iter().find_map(|signature| {
        (signature.target == symbol
            || signature
                .aliases
                .iter()
                .any(|alias| shared_native_alias_allowed(alias) && *alias == symbol))
        .then_some(signature.binding_key)
    })
}

pub fn prelude_registry() -> &'static PreludeRegistry {
    static REGISTRY: OnceLock<PreludeRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let prelude_dir = default_prelude_dir();
        let std_dir = default_std_dir();
        assert!(
            std_dir.join("registry.yml").is_file(),
            "builtin std package registry is missing registry.yml at {}",
            std_dir.display()
        );
        PreludeRegistry::try_from_split_dirs(&prelude_dir, &std_dir).unwrap_or_else(|message| {
            panic!("failed to load builtin prelude/std registry: {message}")
        })
    })
}

pub fn is_builtin_type_name(name: &str) -> bool {
    let name = name.trim();
    prelude_registry().is_prelude_type_name(name) || is_language_builtin_type_name(name)
}

pub fn default_prelude_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("prelude")
}

pub fn default_std_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("std")
}

fn compiler_owned_schema_stable_type(name: &str) -> bool {
    let _ = name;
    false
}

fn shared_native_aliases() -> impl Iterator<Item = (&'static str, &'static str)> {
    STD_NATIVE_SIGNATURES.iter().flat_map(|signature| {
        signature
            .aliases
            .iter()
            .filter(|alias| shared_native_alias_allowed(alias))
            .map(move |alias| (*alias, signature.target))
    })
}

fn shared_native_alias_allowed(alias: &str) -> bool {
    !is_legacy_http_root_alias(alias)
}

fn is_legacy_http_root_alias(alias: &str) -> bool {
    alias.strip_prefix("std.http").is_some_and(|suffix| {
        suffix
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
    })
}

#[cfg(test)]
fn native_type_expr_def_name(expr: &skiff_artifact_model::NativeTypeExprDef) -> String {
    use skiff_artifact_model::NativeTypeExprDef;

    match expr {
        NativeTypeExprDef::TypeParam(index) => format!("T{index}"),
        NativeTypeExprDef::Builtin(name) => name.to_string(),
        NativeTypeExprDef::Array(item) => format!("Array<{}>", native_type_expr_def_name(item)),
        NativeTypeExprDef::Map(key, value) => format!(
            "Map<{}, {}>",
            native_type_expr_def_name(key),
            native_type_expr_def_name(value)
        ),
        NativeTypeExprDef::Nullable(inner) => format!("{}?", native_type_expr_def_name(inner)),
        NativeTypeExprDef::Stream(item) => format!("Stream<{}>", native_type_expr_def_name(item)),
        NativeTypeExprDef::ActorRef(item) => {
            format!("ActorRef<{}>", native_type_expr_def_name(item))
        }
    }
}

#[cfg(test)]
fn native_type_expr_def_normalized_name(expr: &skiff_artifact_model::NativeTypeExprDef) -> String {
    normalize_native_type_name(&native_type_expr_def_name(expr))
}

#[cfg(test)]
fn normalize_native_type_name(name: &str) -> String {
    let name = name.trim();
    if let Some(inner) = name.strip_suffix('?') {
        return format!("{}?", normalize_native_type_name(inner));
    }
    if let Some(parts) = generic_parts(name) {
        let root = parts.root.trim().to_string();
        let args = parts
            .args
            .into_iter()
            .map(normalize_native_type_name)
            .collect::<Vec<_>>()
            .join(", ");
        return format!("{root}<{args}>");
    }
    name.to_string()
}

fn type_root(name: &str) -> &str {
    let name = name.trim().trim_end_matches('?').trim();
    generic_parts(name).map(|parts| parts.root).unwrap_or(name)
}

#[cfg(test)]
mod tests;
