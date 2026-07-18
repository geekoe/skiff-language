use std::collections::BTreeSet;

use crate::{
    parsed_sources::ParsedCompilerSource,
    semantic::validate_db_schema_attachments,
    shared::{ast::DbDecl, publication_error::PublicationError},
};

mod field_paths;
mod storage;
mod type_index;

use field_paths::{collect_db_index_where_field_paths, validate_record_field_path};
use storage::{validate_encrypted_index_path, validate_encrypted_storage};
use type_index::{PackageDbTypeIndex, PackageDbTypeRecord};

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

    let encrypted_fields = db
        .storage
        .iter()
        .map(|storage| storage.field.as_str())
        .collect::<BTreeSet<_>>();
    let mut index_names = BTreeSet::new();
    for index in &db.indexes {
        if !index_names.insert(index.name.as_str()) {
            violations.push(format!(
                "db object {} index name {} is declared more than once",
                db.name, index.name
            ));
        }
        if index.fields.is_empty() {
            violations.push(format!(
                "db object {} index {} must declare at least one field",
                db.name, index.name
            ));
        }
        for field in &index.fields {
            validate_record_field_path(
                "db object index",
                &db.name,
                &field.field_path,
                record,
                type_index,
                violations,
            );
            validate_encrypted_index_path(
                db,
                &index.name,
                &field.field_path,
                &encrypted_fields,
                false,
                violations,
            );
        }
        if let Some(where_expr) = &index.where_expr {
            collect_db_index_where_field_paths(where_expr, &mut |path| {
                validate_record_field_path(
                    &format!("db object {} index {} where", db.name, index.name),
                    &db.name,
                    &path,
                    record,
                    type_index,
                    violations,
                );
                validate_encrypted_index_path(
                    db,
                    &index.name,
                    &path,
                    &encrypted_fields,
                    true,
                    violations,
                );
            });
        }
    }
}

#[cfg(test)]
mod tests;
