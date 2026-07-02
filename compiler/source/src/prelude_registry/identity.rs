use std::fs;

use sha2::{Digest, Sha256};

use crate::{
    shared::ast::{FunctionDecl, InterfaceOperation, TypeDecl},
    shared::type_syntax::{generic_parts, split_top_level},
};

use super::{
    compiler_owned_schema_stable_type, default_prelude_dir, loading::collect_plain_files,
    PreludeRegistry,
};

impl PreludeRegistry {
    pub fn schema_identity(&self) -> String {
        let schema_declarations = self.schema_declaration_fingerprint();
        let mut schema_stable_types = self.schema_stable_types.clone();
        schema_stable_types.sort();
        schema_stable_types.dedup();
        hashed_identity_with_extra(
            "skiff-prelude-schema-v1:sha256",
            std::iter::once(self.schema_version.as_str())
                .chain(schema_stable_types.iter().map(String::as_str)),
            [schema_declarations.as_str()],
        )
    }

    fn schema_declaration_fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        let mut schema_stable_types = self.schema_stable_types.clone();
        schema_stable_types.sort();
        schema_stable_types.dedup();
        for name in &schema_stable_types {
            hash_schema_part(&mut hasher, "type");
            hash_schema_part(&mut hasher, name);
            if let Some(decl) = self.type_decl(name) {
                hash_schema_part(&mut hasher, &self.type_symbol(name));
                hash_type_decl_schema(&mut hasher, decl);
            } else if compiler_owned_schema_stable_type(name) {
                hash_schema_part(&mut hasher, &self.type_symbol(name));
                hash_schema_part(&mut hasher, "compiler-owned");
            }
        }
        hex::encode(hasher.finalize())
    }

    pub(super) fn native_identity(&self) -> String {
        hashed_identity_with_extra(
            "skiff-prelude-native-v1:sha256",
            std::iter::once(self.native_schema_version.as_str())
                .chain(self.native_symbols.iter().map(String::as_str)),
            [
                self.manifest_fingerprint.as_str(),
                self.source_fingerprint.as_str(),
            ],
        )
    }
}

pub fn prelude_schema_identity() -> String {
    super::prelude_registry().schema_identity()
}

pub fn prelude_identity() -> String {
    let dir = default_prelude_dir();
    let mut parts = Vec::new();
    let mut files = Vec::new();
    collect_plain_files(&dir, &dir, &mut files, &["skiff"]);
    files.sort();
    for relative in files {
        let path = dir.join(&relative);
        if let Ok(text) = fs::read_to_string(&path) {
            parts.push(relative.display().to_string());
            parts.push(text);
        }
    }
    let schema_identity = prelude_schema_identity();
    let native_identity = super::prelude_registry().native_identity();
    hashed_identity(
        "skiff-prelude-v1:sha256",
        parts
            .iter()
            .map(String::as_str)
            .chain([schema_identity.as_str(), native_identity.as_str()]),
    )
}

pub(super) fn format_function_signature(function: &FunctionDecl) -> String {
    let mut operation = InterfaceOperation {
        name: function.name.clone(),
        type_params: function.type_params.clone(),
        params: function.params.clone(),
        return_type: function.return_type.clone(),
        is_native: function.is_native,
        is_provider: function.is_provider,
        is_static: function.is_static,
        implicit_self: function.implicit_self.clone(),
        span: function.span,
    };
    operation.is_native = function.is_native;
    normalize_signature(&format_operation_signature(function.exported, &operation))
}

pub(super) fn format_operation_signature(exported: bool, operation: &InterfaceOperation) -> String {
    let mut parts = Vec::new();
    if exported {
        parts.push("export".to_string());
    }
    if operation.is_native {
        parts.push("native".to_string());
    }
    if operation.is_static {
        parts.push("static".to_string());
    }
    parts.push("function".to_string());
    let type_params = if operation.type_params.is_empty() {
        String::new()
    } else {
        format!("<{}>", operation.type_params.join(", "))
    };
    let params = operation
        .params
        .iter()
        .map(|param| format!("{}: {}", param.name, param.ty.name))
        .collect::<Vec<_>>()
        .join(", ");
    normalize_signature(&format!(
        "{} {}{}({}) -> {}",
        parts.join(" "),
        operation.name,
        type_params,
        params,
        operation.return_type.name
    ))
}

pub(super) fn source_fingerprint<'a>(
    parts: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> String {
    let mut hasher = Sha256::new();
    for (module_path, text) in parts {
        hasher.update(module_path.as_bytes());
        hasher.update(b"\0");
        hasher.update(text.as_bytes());
        hasher.update(b"\0");
    }
    hex::encode(hasher.finalize())
}

fn normalize_signature(signature: &str) -> String {
    signature.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn hashed_identity<'a>(prefix: &str, parts: impl IntoIterator<Item = &'a str>) -> String {
    hashed_identity_with_extra(prefix, parts, std::iter::empty::<&'a str>())
}

fn hashed_identity_with_extra<'a>(
    prefix: &str,
    parts: impl IntoIterator<Item = &'a str>,
    extra: impl IntoIterator<Item = &'a str>,
) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    for part in extra {
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    format!("{prefix}:{}", hex::encode(hasher.finalize()))
}

fn hash_type_decl_schema(hasher: &mut Sha256, decl: &TypeDecl) {
    if !decl.type_params.is_empty() {
        hash_schema_part(hasher, "typeParams");
        for param in &decl.type_params {
            hash_schema_part(hasher, param);
        }
    }
    if let Some(discriminator) = &decl.discriminator {
        hash_schema_part(hasher, "discriminator");
        hash_schema_part(hasher, discriminator);
    }
    if let Some(alias) = &decl.alias {
        hash_schema_part(hasher, "alias");
        hash_schema_part(hasher, &canonical_schema_type_name(&alias.name));
        return;
    }

    hash_schema_part(hasher, "record");
    let mut fields = decl.fields.iter().collect::<Vec<_>>();
    fields.sort_by(|left, right| left.name.cmp(&right.name));
    for field in fields {
        hash_schema_part(hasher, "field");
        hash_schema_part(hasher, &field.name);
        hash_schema_part(hasher, &canonical_schema_type_name(&field.ty.name));
    }
}

fn hash_schema_part(hasher: &mut Sha256, part: &str) {
    hasher.update(part.as_bytes());
    hasher.update(b"\0");
}

fn canonical_schema_type_name(raw_name: &str) -> String {
    let name = raw_name.trim();
    let union = split_top_level(name, '|');
    if union.len() > 1 {
        let mut parts = union
            .into_iter()
            .map(canonical_schema_type_name)
            .collect::<Vec<_>>();
        parts.sort();
        parts.dedup();
        return parts.join("|");
    }

    if let Some(inner) = name.strip_suffix('?') {
        return format!("{}?", canonical_schema_type_name(inner));
    }

    if name.starts_with('{') && name.ends_with('}') {
        let inner = &name[1..name.len() - 1];
        let mut fields = split_top_level(inner, ',')
            .into_iter()
            .filter_map(|field| {
                let (field_name, field_type) = field.split_once(':')?;
                Some(format!(
                    "{}:{}",
                    field_name.trim(),
                    canonical_schema_type_name(field_type)
                ))
            })
            .collect::<Vec<_>>();
        fields.sort();
        fields.dedup();
        return format!("{{{}}}", fields.join(","));
    }

    if let Some(parts) = generic_parts(name) {
        let args = parts
            .args
            .into_iter()
            .map(canonical_schema_type_name)
            .collect::<Vec<_>>()
            .join(",");
        return format!("{}<{}>", parts.root.trim(), args);
    }

    name.to_string()
}
