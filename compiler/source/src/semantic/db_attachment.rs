use std::collections::{BTreeMap, BTreeSet};

use crate::{
    shared::ast::{DbDecl, DbStorageCodec, FieldDecl, SourceFile, TypeDecl},
    shared::error::{CompileError, Result},
};

#[derive(Debug, Clone)]
pub struct DbAttachment<'a> {
    pub module_path: &'a str,
    pub db: &'a DbDecl,
    pub ty: &'a TypeDecl,
    pub key: &'a FieldDecl,
    fields_by_name: BTreeMap<String, &'a FieldDecl>,
    storage_by_field: BTreeMap<String, DbStorageCodec>,
}

impl<'a> DbAttachment<'a> {
    pub fn fields(&self) -> impl Iterator<Item = &'a FieldDecl> + '_ {
        self.ty.fields.iter()
    }

    pub fn field_map(&self) -> &BTreeMap<String, &'a FieldDecl> {
        &self.fields_by_name
    }

    pub fn storage_map(&self) -> &BTreeMap<String, DbStorageCodec> {
        &self.storage_by_field
    }
}

#[derive(Debug, Clone)]
pub struct DbAttachmentIndex<'a> {
    by_name: BTreeMap<&'a str, DbAttachment<'a>>,
}

impl<'a> DbAttachmentIndex<'a> {
    pub fn build(module_path: &'a str, ast: &'a SourceFile) -> Result<Self> {
        let violations = validate_db_attachments(module_path, ast);
        if !violations.is_empty() {
            return Err(CompileError::Semantic(violations.join("\n")));
        }

        let mut index = Self {
            by_name: BTreeMap::new(),
        };
        for db in &ast.dbs {
            let ty = ast
                .types
                .iter()
                .find(|ty| ty.name == db.name)
                .expect("validated db attachment type exists");
            let key_name = &db
                .key
                .as_ref()
                .expect("validated db attachment key exists")
                .name;
            let fields = ty
                .fields
                .iter()
                .map(|field| (field.name.clone(), field))
                .collect::<BTreeMap<_, _>>();
            let key = *fields
                .get(key_name)
                .expect("validated db attachment key field exists");
            let storage_by_field = db
                .storage
                .iter()
                .map(|storage| (storage.field.clone(), storage.codec))
                .collect();
            index.by_name.insert(
                db.name.as_str(),
                DbAttachment {
                    module_path,
                    db,
                    ty,
                    key,
                    fields_by_name: fields,
                    storage_by_field,
                },
            );
        }
        Ok(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &DbAttachment<'a>> + '_ {
        self.by_name.values()
    }
}

pub fn validate_db_attachments(_module_path: &str, ast: &SourceFile) -> Vec<String> {
    let mut violations = Vec::new();
    for db in &ast.dbs {
        if ast.aliases.iter().any(|alias| alias.name == db.name) {
            violations.push(format!(
                "db object {} must attach to a type declaration, not an alias",
                db.name
            ));
        }

        let Some(ty) = ast.types.iter().find(|ty| ty.name == db.name) else {
            violations.push(format!(
                "db object {} must attach to a same-module type declaration",
                db.name
            ));
            continue;
        };

        if !ty.type_params.is_empty() {
            violations.push(format!(
                "db object {} cannot attach to generic type {}",
                db.name, ty.name
            ));
        }
        if ty.alias.is_some() {
            violations.push(format!(
                "db object {} must attach to a concrete record type, not an alias or representation",
                db.name
            ));
        }
        if ty.fields.is_empty() {
            violations.push(format!(
                "db object {} must attach to a record type with at least one field",
                db.name
            ));
        }

        let mut fields = BTreeMap::new();
        for field in &ty.fields {
            if fields.insert(field.name.as_str(), field).is_some() {
                violations.push(format!(
                    "db object {} attached type field {} is declared more than once",
                    db.name, field.name
                ));
            }
        }

        if let Some(key) = &db.key {
            if !fields.contains_key(key.name.as_str()) {
                violations.push(format!(
                    "db object {} primary key field {} must be a field on the attached type",
                    db.name, key.name
                ));
            }
        } else {
            violations.push(format!("db object {} must declare key", db.name));
        }
        validate_db_storage_declarations(db, &fields.keys().copied().collect(), &mut violations);
    }
    violations
}

pub(crate) fn validate_db_storage_declarations(
    db: &DbDecl,
    fields: &BTreeSet<&str>,
    violations: &mut Vec<String>,
) {
    let mut storage_fields = BTreeSet::new();
    for storage in &db.storage {
        if !storage_fields.insert(storage.field.as_str()) {
            violations.push(format!(
                "db object {} storage field `{}` is declared more than once",
                db.name, storage.field
            ));
        }
        if !fields.contains(storage.field.as_str()) {
            violations.push(format!(
                "db object {} encrypted storage field `{}` must be a field on the attached type",
                db.name, storage.field
            ));
        }
        if db.key.as_ref().is_some_and(|key| storage.field == key.name) {
            violations.push(format!(
                "db object {} encrypted storage field `{}` cannot be the primary key",
                db.name, storage.field
            ));
        }
    }
}
