use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::TypeDeclIr;
use skiff_compiler_core::prelude_registry::{
    compiler_owned_type_symbol, config_prelude_type, is_prelude_canonical_type,
    qualified_prelude_type, PRELUDE_REGISTRY_ID,
};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PreludeProjection {
    identity: String,
    schema_identity: String,
    types: Vec<String>,
    roots: Vec<String>,
    type_declarations: Vec<TypeDeclIr>,
    type_symbols: BTreeMap<String, String>,
    schema_stable_types: BTreeSet<String>,
    package_schema_import_required: BTreeSet<String>,
    bare_raw_http_envelope_types: BTreeSet<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PreludeProjectionParts {
    pub identity: String,
    pub schema_identity: String,
    pub types: Vec<String>,
    pub roots: Vec<String>,
    pub type_declarations: Vec<TypeDeclIr>,
    pub type_symbols: BTreeMap<String, String>,
    pub schema_stable_types: BTreeSet<String>,
    pub package_schema_import_required: BTreeSet<String>,
    pub bare_raw_http_envelope_types: BTreeSet<String>,
}

impl PreludeProjection {
    pub fn new(parts: PreludeProjectionParts) -> Self {
        Self {
            identity: parts.identity,
            schema_identity: parts.schema_identity,
            types: parts.types,
            roots: parts.roots,
            type_declarations: parts.type_declarations,
            type_symbols: parts.type_symbols,
            schema_stable_types: parts.schema_stable_types,
            package_schema_import_required: parts.package_schema_import_required,
            bare_raw_http_envelope_types: parts.bare_raw_http_envelope_types,
        }
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }

    pub fn schema_identity(&self) -> &str {
        &self.schema_identity
    }

    pub fn types(&self) -> &[String] {
        &self.types
    }

    pub fn roots(&self) -> &[String] {
        &self.roots
    }

    pub fn type_declarations(&self) -> &[TypeDeclIr] {
        &self.type_declarations
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
            .unwrap_or_else(|| format!("{PRELUDE_REGISTRY_ID}.unknown"))
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

    pub fn lowered_type_decl(&self, name: &str) -> Option<TypeDeclIr> {
        let symbol = self
            .known_type_symbol(name)
            .unwrap_or_else(|| name.to_string());
        self.type_declarations
            .iter()
            .find(|decl| self.type_symbol(&decl.name) == symbol || decl.name == symbol)
            .cloned()
    }

    pub fn is_prelude_type_name(&self, name: &str) -> bool {
        let name = name.trim();
        self.types.iter().any(|ty| ty == name)
            || qualified_prelude_type(name).is_some_and(|_| self.known_type_symbol(name).is_some())
            || config_prelude_type(name).is_some_and(|_| self.known_type_symbol(name).is_some())
            || self.prelude_type_decl_name(name).is_some()
            || self
                .type_declarations
                .iter()
                .any(|decl| decl.name == name && !self.package_schema_type_requires_import(name))
    }

    pub fn is_schema_stable_type(&self, name: &str) -> bool {
        let name = self.known_type_symbol(name).unwrap_or_else(|| {
            self.prelude_type_decl_name(name)
                .unwrap_or(name.trim())
                .to_string()
        });
        self.schema_stable_types.contains(&name)
    }

    pub fn package_schema_type_requires_import(&self, name: &str) -> bool {
        self.package_schema_import_required.contains(name)
    }

    pub fn is_bare_raw_http_envelope_type(&self, name: &str) -> bool {
        self.bare_raw_http_envelope_types.contains(name)
    }

    fn prelude_type_decl_name<'a>(&'a self, name: &'a str) -> Option<&'a str> {
        let name = name.trim();
        if self.types.iter().any(|ty| ty == name)
            && self.type_declarations.iter().any(|decl| decl.name == name)
        {
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
}
