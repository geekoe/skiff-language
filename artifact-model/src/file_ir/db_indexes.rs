use std::collections::{BTreeMap, BTreeSet};

use crate::types::{LiteralIr, TypeDescriptorIr, TypeRefIr};

use super::{DbDeclarationIr, DbFieldStorageIr, DbIndexDirectionIr, FileIrUnit};

pub const DB_INDEXABLE_SCALAR_BUILTINS: &[&str] = &[
    "string", "number", "integer", "bool", "bytes", "Date", "Duration",
];

pub fn is_db_indexable_scalar_builtin(name: &str) -> bool {
    DB_INDEXABLE_SCALAR_BUILTINS.contains(&name)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIrDbIndexValidationError {
    pub location: String,
    pub message: String,
}

impl std::fmt::Display for FileIrDbIndexValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.location, self.message)
    }
}

impl std::error::Error for FileIrDbIndexValidationError {}

/// Validates the closed ordinary/unique index shape carried by File IR.
///
/// This is the artifact-admission owner for logical names, ordered key
/// specifications, field paths and indexable stored types. Partial predicates
/// are absent from the DTO, so old `where` wire data is rejected by strict
/// deserialization before this validator runs.
pub fn validate_file_ir_db_indexes(unit: &FileIrUnit) -> Result<(), FileIrDbIndexValidationError> {
    for (db_name, declaration) in &unit.declarations.db {
        let db_location = format!("declarations.db[{db_name}]");
        validate_primary_key(unit, declaration, &db_location)?;

        let mut names = BTreeSet::new();
        let mut specs = BTreeMap::<Vec<(String, u8)>, String>::new();
        for (index_position, index) in declaration.indexes.iter().enumerate() {
            let location = format!("{db_location}.indexes[{index_position}]");
            if index.name.is_empty() {
                return db_index_error(location, "logical index name must not be empty");
            }
            if index.name == "_id_" {
                return db_index_error(
                    location,
                    "logical index name _id_ is reserved for the primary key",
                );
            }
            if !names.insert(index.name.as_str()) {
                return db_index_error(
                    location,
                    format!(
                        "logical index name {} is declared more than once",
                        index.name
                    ),
                );
            }
            if index.fields.is_empty() {
                return db_index_error(location, "index must declare at least one field");
            }

            let mut fields = BTreeSet::new();
            let mut spec = Vec::with_capacity(index.fields.len());
            for (field_position, field) in index.fields.iter().enumerate() {
                let field_location = format!("{location}.fields[{field_position}]");
                validate_field_path_shape(
                    &field.field.text,
                    &field.field.segments,
                    &field_location,
                )?;
                if !fields.insert(field.field.text.as_str()) {
                    return db_index_error(
                        field_location,
                        format!(
                            "field path {} is declared more than once in the ordered key",
                            field.field.text
                        ),
                    );
                }
                let field_ty = resolve_stored_field_path(
                    unit,
                    declaration,
                    &field.field.segments,
                    &field_location,
                )?;
                if !is_indexable_scalar(unit, field_ty, true, &mut BTreeSet::new()) {
                    return db_index_error(
                        field_location,
                        format!(
                            "field {} must be an indexable scalar or nullable scalar",
                            field.field.text
                        ),
                    );
                }
                spec.push((
                    field.field.text.clone(),
                    match field.direction {
                        DbIndexDirectionIr::Asc => 0,
                        DbIndexDirectionIr::Desc => 1,
                    },
                ));
            }
            if let Some(first_name) = specs.insert(spec, index.name.clone()) {
                return db_index_error(
                    location,
                    format!(
                        "indexes {first_name} and {} declare the same ordered key specification",
                        index.name
                    ),
                );
            }
        }
    }
    Ok(())
}

fn validate_primary_key(
    unit: &FileIrUnit,
    declaration: &DbDeclarationIr,
    location: &str,
) -> Result<(), FileIrDbIndexValidationError> {
    if declaration.key.name == "_id" {
        return db_index_error(
            format!("{location}.key"),
            "business primary key name _id is reserved",
        );
    }
    let mut field_names = BTreeSet::new();
    for field in &declaration.fields {
        if field.name == "_id" {
            return db_index_error(
                format!("{location}.fields"),
                "business field name _id is reserved",
            );
        }
        if !field_names.insert(field.name.as_str()) {
            return db_index_error(
                format!("{location}.fields"),
                format!("stored field {} is declared more than once", field.name),
            );
        }
        if field.name == declaration.key.name && field.storage == DbFieldStorageIr::Encrypted {
            return db_index_error(
                format!("{location}.key"),
                "primary key cannot use encrypted storage",
            );
        }
    }
    if !is_indexable_scalar(unit, &declaration.key.ty, false, &mut BTreeSet::new()) {
        return db_index_error(
            format!("{location}.key"),
            format!(
                "primary key field {} must be a non-null indexable scalar",
                declaration.key.name
            ),
        );
    }
    Ok(())
}

fn validate_field_path_shape(
    text: &str,
    segments: &[String],
    location: &str,
) -> Result<(), FileIrDbIndexValidationError> {
    if segments.is_empty()
        || segments.iter().any(String::is_empty)
        || text != segments.join(".")
        || segments.iter().any(|segment| segment == "_id")
    {
        return db_index_error(
            location,
            "field path text and non-empty business-field segments must be canonical",
        );
    }
    Ok(())
}

fn resolve_stored_field_path<'a>(
    unit: &'a FileIrUnit,
    declaration: &'a DbDeclarationIr,
    segments: &[String],
    location: &str,
) -> Result<&'a TypeRefIr, FileIrDbIndexValidationError> {
    let Some(first) = segments.first() else {
        return db_index_error(location, "field path must not be empty");
    };
    let mut ty = if first == &declaration.key.name {
        &declaration.key.ty
    } else {
        let Some(field) = declaration.fields.iter().find(|field| &field.name == first) else {
            return db_index_error(
                location,
                format!("field path references unknown field {first}"),
            );
        };
        if field.storage == DbFieldStorageIr::Encrypted {
            return db_index_error(
                location,
                format!("encrypted storage field {first} cannot be indexed"),
            );
        }
        &field.ty
    };

    for segment in &segments[1..] {
        let Some(fields) = record_fields(unit, ty, &mut BTreeSet::new()) else {
            return db_index_error(
                location,
                format!("field path cannot traverse non-record before {segment}"),
            );
        };
        let Some(next) = fields.get(segment) else {
            return db_index_error(
                location,
                format!("field path references unknown nested field {segment}"),
            );
        };
        ty = next;
    }
    Ok(ty)
}

fn record_fields<'a>(
    unit: &'a FileIrUnit,
    ty: &'a TypeRefIr,
    seen: &mut BTreeSet<u32>,
) -> Option<&'a BTreeMap<String, TypeRefIr>> {
    match ty {
        TypeRefIr::Nullable { inner } => record_fields(unit, inner, seen),
        TypeRefIr::Record { fields } => Some(fields),
        TypeRefIr::LocalType { type_index } => {
            record_fields_for_type_index(unit, *type_index, seen)
        }
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } if module_path == &unit.module_path => {
            record_fields_for_type_index(unit, *type_index, seen)
        }
        _ => None,
    }
}

fn record_fields_for_type_index<'a>(
    unit: &'a FileIrUnit,
    type_index: u32,
    seen: &mut BTreeSet<u32>,
) -> Option<&'a BTreeMap<String, TypeRefIr>> {
    if !seen.insert(type_index) {
        return None;
    }
    let declaration = unit.type_table.get(type_index as usize)?;
    match &declaration.descriptor {
        TypeDescriptorIr::Record { fields } => Some(fields),
        TypeDescriptorIr::Alias { target } => record_fields(unit, target, seen),
        TypeDescriptorIr::Representation { .. }
        | TypeDescriptorIr::Union { .. }
        | TypeDescriptorIr::Interface => None,
    }
}

fn is_indexable_scalar(
    unit: &FileIrUnit,
    ty: &TypeRefIr,
    nullable: bool,
    seen: &mut BTreeSet<u32>,
) -> bool {
    match ty {
        TypeRefIr::Literal {
            value: LiteralIr::String { .. },
        } => true,
        TypeRefIr::Union { items }
            if !items.is_empty()
                && items.iter().all(|item| {
                    matches!(
                        item,
                        TypeRefIr::Literal {
                            value: LiteralIr::String { .. }
                        }
                    )
                }) =>
        {
            true
        }
        TypeRefIr::Builtin { name, args }
            if args.is_empty() && is_db_indexable_scalar_builtin(name) =>
        {
            true
        }
        TypeRefIr::Nullable { inner } if nullable => is_indexable_scalar(unit, inner, false, seen),
        TypeRefIr::Union { items } if nullable && items.len() == 2 => {
            let mut scalar = None;
            let mut null_count = 0;
            for item in items {
                if matches!(item, TypeRefIr::Builtin { name, args } if name == "null" && args.is_empty())
                {
                    null_count += 1;
                } else if scalar.replace(item).is_some() {
                    return false;
                }
            }
            null_count == 1
                && scalar.is_some_and(|item| is_indexable_scalar(unit, item, false, seen))
        }
        TypeRefIr::LocalType { type_index } => {
            is_indexable_nominal(unit, *type_index, nullable, seen)
        }
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } if module_path == &unit.module_path => {
            is_indexable_nominal(unit, *type_index, nullable, seen)
        }
        TypeRefIr::Builtin { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::AppliedNominal { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Record { .. }
        | TypeRefIr::Union { .. }
        | TypeRefIr::Nullable { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::Function { .. } => false,
    }
}

fn is_indexable_nominal(
    unit: &FileIrUnit,
    type_index: u32,
    nullable: bool,
    seen: &mut BTreeSet<u32>,
) -> bool {
    if !seen.insert(type_index) {
        return false;
    }
    let Some(declaration) = unit.type_table.get(type_index as usize) else {
        return false;
    };
    match &declaration.descriptor {
        TypeDescriptorIr::Representation { representation } => {
            is_indexable_scalar(unit, representation, nullable, seen)
        }
        TypeDescriptorIr::Alias { target } => is_indexable_scalar(unit, target, nullable, seen),
        TypeDescriptorIr::Record { .. }
        | TypeDescriptorIr::Union { .. }
        | TypeDescriptorIr::Interface => false,
    }
}

fn db_index_error<T>(
    location: impl Into<String>,
    message: impl Into<String>,
) -> Result<T, FileIrDbIndexValidationError> {
    Err(FileIrDbIndexValidationError {
        location: location.into(),
        message: message.into(),
    })
}
