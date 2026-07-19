use std::collections::{BTreeMap, BTreeSet};

use crate::{
    parsed_sources::ParsedCompilerSource, semantic::validate_db_storage_declarations,
    shared::ast::DbDecl,
};

use super::type_index::PackageDbTypeRecord;

pub(super) fn validate_encrypted_storage(
    parsed: &ParsedCompilerSource,
    db: &DbDecl,
    record: &PackageDbTypeRecord<'_>,
    violations: &mut Vec<String>,
) {
    if db.storage.is_empty() {
        return;
    }
    let fields = record
        .fields
        .iter()
        .map(|field| (field.name.as_str(), field.ty))
        .collect::<BTreeMap<_, _>>();
    validate_db_storage_declarations(db, &fields.keys().copied().collect(), violations);

    let mut seen = BTreeSet::new();
    for storage in &db.storage {
        if !seen.insert(storage.field.as_str()) {
            continue;
        }
        let Some(field_ty) = fields.get(storage.field.as_str()) else {
            continue;
        };
        if db.key.as_ref().is_some_and(|key| storage.field == key.name) {
            continue;
        }
        if !is_exact_non_nullable_string(&field_ty.name, parsed.alias_targets()) {
            violations.push(format!(
                "db object {} encrypted storage field `{}` must be a non-null string",
                db.name, storage.field
            ));
        }
    }

    if let Some(key) = &db.key {
        if let Some(key_ty) = fields.get(key.name.as_str()) {
            if !is_exact_non_nullable_string(&key_ty.name, parsed.alias_targets()) {
                violations.push(format!(
                    "db object {} with encrypted storage field must use a non-null string primary key `{}`",
                    db.name, key.name
                ));
            }
        }
    }
}

pub(super) fn validate_encrypted_index_path(
    db: &DbDecl,
    index_name: &str,
    path: &[String],
    encrypted_fields: &BTreeSet<&str>,
    partial: bool,
    violations: &mut Vec<String>,
) {
    let Some(field) = path.first() else {
        return;
    };
    if encrypted_fields.contains(field.as_str()) {
        violations.push(format!(
            "db object {} encrypted storage field `{field}` cannot be used by {}index `{index_name}`{}",
            db.name,
            if partial { "partial " } else { "" },
            if partial { " where" } else { "" }
        ));
    }
}

fn is_exact_non_nullable_string(ty: &str, alias_targets: &BTreeMap<String, String>) -> bool {
    let mut current = ty.trim();
    let mut seen = BTreeSet::new();
    loop {
        if current == "string" {
            return true;
        }
        if current.ends_with('?')
            || current.contains('|')
            || current.contains('<')
            || current.contains('{')
        {
            return false;
        }
        if !seen.insert(current.to_string()) {
            return false;
        }
        let Some(target) = alias_targets.get(current) else {
            return false;
        };
        current = target.trim();
    }
}
