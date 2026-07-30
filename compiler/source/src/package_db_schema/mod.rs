use std::collections::{BTreeMap, BTreeSet};

use crate::{
    parsed_sources::ParsedCompilerSource,
    semantic::validate_db_schema_attachments,
    shared::{
        ast::{DbDecl, DbIndexDirection},
        publication_error::PublicationError,
    },
};

mod field_paths;
mod storage;
mod type_index;

use field_paths::validate_record_field_path;
use storage::{validate_encrypted_index_path, validate_encrypted_storage};
use type_index::{PackageDbTypeIndex, PackageDbTypeRecord};

/// Rejects partial-index authoring before general source-root and expression
/// analysis can reinterpret its retired predicate as an executable expression.
pub(super) fn reject_partial_db_indexes(
    parsed_sources: &[ParsedCompilerSource],
) -> Result<(), PublicationError> {
    let violations = parsed_sources
        .iter()
        .flat_map(|parsed| {
            parsed.ast().dbs.iter().flat_map(|db| {
                db.indexes.iter().filter_map(|index| {
                    index.where_expr.as_ref().map(|_| {
                        format!(
                            "db object {} index {} uses unsupported partial index authoring; remove the where clause",
                            db.name, index.name
                        )
                    })
                })
            })
        })
        .collect::<Vec<_>>();
    if violations.is_empty() {
        Ok(())
    } else {
        Err(PublicationError::ContractValidation {
            message: violations
                .into_iter()
                .map(|violation| format!("- {violation}"))
                .collect::<Vec<_>>()
                .join("\n"),
        })
    }
}

/// Validates package-owned logical DB declarations before source facts are built.
///
/// This owner consumes only parsed logical declarations. Lowering consumes only
/// declarations that passed these source-level shape checks.
pub(super) fn validate_package_db_schema(
    parsed_sources: &[ParsedCompilerSource],
) -> Result<(), PublicationError> {
    let type_index = PackageDbTypeIndex::build(parsed_sources);
    let mut violations = Vec::new();

    for parsed in parsed_sources {
        violations.extend(validate_db_schema_attachments(parsed.ast()));
        for db in &parsed.ast().dbs {
            let Some(record) = type_index.resolve_local_record(parsed, &db.name) else {
                continue;
            };
            validate_db_declaration(parsed, db, record, &type_index, &mut violations);
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(PublicationError::ContractValidation {
            message: violations
                .into_iter()
                .map(|violation| format!("- {violation}"))
                .collect::<Vec<_>>()
                .join("\n"),
        })
    }
}

fn validate_db_declaration(
    parsed: &ParsedCompilerSource,
    db: &DbDecl,
    record: &PackageDbTypeRecord<'_>,
    type_index: &PackageDbTypeIndex<'_>,
    violations: &mut Vec<String>,
) {
    if db.key.as_ref().is_some_and(|key| key.name == "_id") {
        violations.push(format!(
            "db object {} key cannot use reserved _id field",
            db.name
        ));
    }
    for field in &record.fields {
        if field.name == "_id" {
            violations.push(format!(
                "db object {} field cannot use reserved _id field",
                db.name
            ));
        }
    }

    validate_encrypted_storage(parsed, db, record, violations);

    if let Some(key) = &db.key {
        if let Some(field) = validate_record_field_path(
            "db object primary key",
            &db.name,
            std::slice::from_ref(&key.name),
            record,
            type_index,
            violations,
        ) {
            if !type_index.is_indexable_scalar(field.owner, field.ty, false) {
                violations.push(format!(
                    "db object {} primary key field {} must be a non-null indexable scalar",
                    db.name, key.name
                ));
            }
        }
    }

    let encrypted_fields = db
        .storage
        .iter()
        .map(|storage| storage.field.as_str())
        .collect::<BTreeSet<_>>();
    let mut index_names = BTreeSet::new();
    let mut index_specs = BTreeMap::<Vec<(String, &'static str)>, String>::new();
    for index in &db.indexes {
        if !index_names.insert(index.name.as_str()) {
            violations.push(format!(
                "db object {} index name {} is declared more than once",
                db.name, index.name
            ));
        }
        if index.name == "_id_" {
            violations.push(format!(
                "db object {} index name _id_ is reserved for the primary key",
                db.name
            ));
        }
        if index.fields.is_empty() {
            violations.push(format!(
                "db object {} index {} must declare at least one field",
                db.name, index.name
            ));
        }
        let ordered_spec = index
            .fields
            .iter()
            .map(|field| {
                (
                    field.field_path.join("."),
                    match field.direction {
                        DbIndexDirection::Asc => "asc",
                        DbIndexDirection::Desc => "desc",
                    },
                )
            })
            .collect::<Vec<_>>();
        if let Some(first_name) = index_specs.insert(ordered_spec, index.name.clone()) {
            violations.push(format!(
                "db object {} indexes {} and {} declare the same ordered key specification",
                db.name, first_name, index.name
            ));
        }
        let mut index_fields = BTreeSet::new();
        for field in &index.fields {
            let path_text = field.field_path.join(".");
            if !index_fields.insert(path_text.clone()) {
                violations.push(format!(
                    "db object {} index {} declares field path {} more than once",
                    db.name, index.name, path_text
                ));
            }
            let validated = validate_record_field_path(
                "db object index",
                &db.name,
                &field.field_path,
                record,
                type_index,
                violations,
            );
            if let Some(validated) = validated {
                if !type_index.is_indexable_scalar(validated.owner, validated.ty, true) {
                    violations.push(format!(
                        "db object {} index {} field {} must be an indexable scalar or nullable scalar",
                        db.name, index.name, path_text
                    ));
                }
            }
            validate_encrypted_index_path(
                db,
                &index.name,
                &field.field_path,
                &encrypted_fields,
                violations,
            );
        }
    }
}

#[cfg(test)]
mod tests;
