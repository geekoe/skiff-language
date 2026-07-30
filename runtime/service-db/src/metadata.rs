use std::collections::HashMap;
use std::fmt;

use serde_json::Value;
use skiff_artifact_model::{
    DbFieldStorageIr, DbIndexDirectionIr, DbIndexFieldIr, DbLeaseIr, DbMetadataIndexIr,
    DbMetadataIr, DbObjectFieldIr, DbObjectKeyIr, FieldPathIr, PackageRefIr, TypeRefIr,
};
use skiff_runtime_boundary::db as db_boundary;

use skiff_runtime_capability_context::{
    DbOrderDirection, DbOrderEntry, DbProviderTargetMetadata, FieldPath, ServiceDbChange,
};

use crate::{service_storage_collection_name, DbEncryptionCipher, Result, ServiceDbError};

#[derive(Clone)]
pub struct ServiceDbMetadata {
    collections: Vec<DbCollectionMetadata>,
    collections_by_target: HashMap<String, usize>,
}

#[derive(Clone)]
pub struct DbCollectionMetadata {
    pub target_key: String,
    pub type_name: String,
    pub collection_name: String,
    pub key_field: String,
    pub key_ty: Option<db_boundary::DbBoundaryValuePlan>,
    pub storage_environment: String,
    pub storage_service_id: String,
    pub encryption_cipher: Option<DbEncryptionCipher>,
    pub fields: HashMap<String, DbFieldMetadata>,
    pub leases: HashMap<String, DbLeaseMetadata>,
    pub immutable_file_paths: Vec<Vec<String>>,
    // Parsed now so object DB metadata stays schema-complete; index creation/planning
    // will consume this once the runtime owns DB index reconciliation.
    #[allow(dead_code)]
    pub indexes: Vec<DbIndexMetadata>,
}

impl fmt::Debug for DbCollectionMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DbCollectionMetadata")
            .field("type_name", &self.type_name)
            .field("collection_name", &self.collection_name)
            .field("key_field", &self.key_field)
            .field("fields", &self.fields)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct DbFieldMetadata {
    pub name: String,
    pub ty: Option<db_boundary::DbBoundaryValuePlan>,
    pub storage: DbFieldStorageIr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbLeaseMetadata {
    pub name: String,
    pub ttl_ms: u64,
    pub max_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DbIndexMetadata {
    pub name: String,
    pub unique: bool,
    pub fields: Vec<DbOrderEntry>,
    pub where_filter: Option<Value>,
}

impl ServiceDbMetadata {
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_runtime_program_db(entries: &[DbProviderTargetMetadata]) -> Result<Self> {
        Self::from_runtime_program_db_with_encryption(entries, "test", "test.local/service", None)
    }

    pub fn from_runtime_program_db_with_encryption(
        entries: &[DbProviderTargetMetadata],
        storage_environment: &str,
        storage_service_id: &str,
        encryption_cipher: Option<DbEncryptionCipher>,
    ) -> Result<Self> {
        let mut collections = Vec::new();
        let mut collections_by_target = HashMap::new();
        let mut package_builds = HashMap::new();
        let mut physical_collection_owners = HashMap::new();
        for (index, entry) in entries.iter().enumerate() {
            let package = &entry.target.target_id.package_artifact_ref;
            if let Some(first) = package_builds.insert(package.package_id.as_str(), package) {
                if first != package {
                    return Err(ServiceDbError::InvalidDbMetadata(format!(
                        "runtime program DB metadata resolves package ID {} to different exact Package builds",
                        package.package_id
                    )));
                }
            }
            let mut binding = DbCollectionMetadata::from_ir_with_encryption(
                &entry.metadata,
                index,
                &package.package_id,
                storage_environment,
                storage_service_id,
                encryption_cipher.clone(),
            )?;
            binding.target_key = entry.target.lookup_key().to_string();
            let logical_owner = (
                package.package_id.clone(),
                entry.metadata.collection_name.clone(),
                binding.target_key.clone(),
            );
            if let Some(first) =
                physical_collection_owners.insert(binding.collection_name.clone(), logical_owner)
            {
                let current = physical_collection_owners
                    .get(&binding.collection_name)
                    .expect("inserted collection owner");
                let reason = if first.0 == current.0 && first.1 == current.1 {
                    "repeats one Package logical collection identity"
                } else {
                    "has a system physical collection-name encoding collision"
                };
                return Err(ServiceDbError::InvalidDbMetadata(format!(
                    "runtime program DB metadata {reason}: {}",
                    binding.collection_name
                )));
            }
            if collections_by_target
                .insert(entry.target.lookup_key().to_string(), collections.len())
                .is_some()
            {
                return Err(ServiceDbError::InvalidDbMetadata(format!(
                    "runtime program db metadata repeats exact target {}",
                    entry.target.type_name
                )));
            }
            collections.push(binding);
        }
        Ok(Self {
            collections,
            collections_by_target,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn collection_for_target(
        &self,
        target: &skiff_runtime_capability_context::DbCapabilityTarget,
    ) -> Result<&DbCollectionMetadata> {
        self.collection_for_target_key(target.lookup_key())
    }

    pub fn collection_for_target_key(&self, target_key: &str) -> Result<&DbCollectionMetadata> {
        self.collections_by_target
            .get(target_key)
            .and_then(|index| self.collections.get(*index))
            .ok_or_else(|| {
                ServiceDbError::InvalidDbMetadata(
                    "runtime program db metadata does not declare the exact DB target".to_string(),
                )
            })
    }
}

impl DbCollectionMetadata {
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_ir(ir: &DbMetadataIr, index: usize) -> Result<Self> {
        Self::from_ir_with_encryption(
            ir,
            index,
            "test.local/package",
            "test",
            "test.local/service",
            None,
        )
    }

    pub fn from_ir_with_encryption(
        ir: &DbMetadataIr,
        index: usize,
        package_id: &str,
        storage_environment: &str,
        storage_service_id: &str,
        encryption_cipher: Option<DbEncryptionCipher>,
    ) -> Result<Self> {
        let module_path = non_empty_optional_string(&ir.module_path);
        let type_name = required_typed_string(
            &ir.type_name,
            format!("runtime program db[{index}].typeName"),
        )?;
        let collection_name = required_typed_string(
            &ir.collection_name,
            format!("runtime program db[{index}].collectionName"),
        )?;
        let collection_name = service_storage_collection_name(package_id, &collection_name)?;
        let key = parse_object_key(ir.key.as_ref(), index)?;
        let fields = parse_fields(&ir.fields, index)?;
        let leases = parse_leases(&ir.leases, index)?;
        let immutable_file_paths = immutable_file_paths(&fields);
        let indexes = parse_indexes(&ir.indexes, index)?;
        let metadata = Self {
            target_key: module_path
                .as_ref()
                .map(|module_path| format!("{module_path}.{type_name}"))
                .unwrap_or_else(|| type_name.clone()),
            type_name,
            collection_name,
            key_field: key.name,
            key_ty: key.ty,
            storage_environment: storage_environment.to_string(),
            storage_service_id: storage_service_id.to_string(),
            encryption_cipher,
            fields,
            leases,
            immutable_file_paths,
            indexes,
        };
        metadata.validate_indexes().map_err(|error| {
            ServiceDbError::InvalidDbMetadata(format!(
                "runtime program db[{index}].indexes are invalid: {error}"
            ))
        })?;
        metadata.validate_encrypted_storage(index)?;
        Ok(metadata)
    }

    pub fn has_encrypted_fields(&self) -> bool {
        self.fields
            .values()
            .any(|field| field.storage == DbFieldStorageIr::Encrypted)
    }

    fn validate_encrypted_storage(&self, index: usize) -> Result<()> {
        if !self.has_encrypted_fields() {
            return Ok(());
        }
        if self.encryption_cipher.is_none() {
            return Err(ServiceDbError::InvalidDbMetadata(format!(
                "runtime program db[{index}] declares encrypted storage but the runtime host has no service DB encryption keyring"
            )));
        }
        if !self
            .key_ty
            .as_ref()
            .is_some_and(|plan| is_plain_string_type(plan.artifact_type_ref()))
        {
            return Err(ServiceDbError::InvalidDbMetadata(format!(
                "runtime program db[{index}] with encrypted storage must use a plain string key"
            )));
        }
        for field in self
            .fields
            .values()
            .filter(|field| field.storage == DbFieldStorageIr::Encrypted)
        {
            if field.name == self.key_field {
                return Err(ServiceDbError::InvalidDbMetadata(format!(
                    "runtime program db[{index}] encrypted storage cannot target primary key field {}",
                    field.name
                )));
            }
            if !field
                .ty
                .as_ref()
                .is_some_and(|plan| is_plain_string_type(plan.artifact_type_ref()))
            {
                return Err(ServiceDbError::InvalidDbMetadata(format!(
                    "runtime program db[{index}] encrypted field {} must be a plain string",
                    field.name
                )));
            }
        }
        Ok(())
    }

    pub fn has_immutable_file_cascade(&self) -> bool {
        !self.immutable_file_paths.is_empty()
    }

    pub fn lease(&self, slot: &str) -> Result<&DbLeaseMetadata> {
        self.leases.get(slot).ok_or_else(|| {
            ServiceDbError::InvalidDbMetadata(format!(
                "runtime program db metadata for {} does not declare lease {slot}",
                self.type_name
            ))
        })
    }

    pub fn immutable_file_paths_for_change(&self, change: &ServiceDbChange) -> Vec<Vec<String>> {
        if self.immutable_file_paths.is_empty() {
            return Vec::new();
        }
        change
            .touched_fields()
            .flat_map(|field| {
                let touched = field.split('.').collect::<Vec<_>>();
                self.immutable_file_paths
                    .iter()
                    .filter(move |path| path_touches_file_path(&touched, path))
                    .cloned()
            })
            .collect()
    }
}

fn required_typed_string(value: &str, location: String) -> Result<String> {
    (!value.trim().is_empty())
        .then(|| value.to_string())
        .ok_or_else(|| {
            ServiceDbError::InvalidDbMetadata(format!("{location} must be a non-empty string"))
        })
}

fn non_empty_optional_string(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn required_field_path_segment(value: &str, location: String) -> Result<String> {
    (!value.trim().is_empty())
        .then(|| value.to_string())
        .ok_or_else(|| {
            ServiceDbError::InvalidDbMetadata(format!(
                "{location}.segments entries must be non-empty strings"
            ))
        })
}

fn optional_positive_u64(value: Option<u64>, location: String) -> Result<Option<u64>> {
    value
        .map(|value| {
            (value > 0).then_some(value).ok_or_else(|| {
                ServiceDbError::InvalidDbMetadata(format!(
                    "{location} must be positive when present"
                ))
            })
        })
        .transpose()
}

fn required_positive_u64(value: u64, location: String) -> Result<u64> {
    (value > 0).then_some(value).ok_or_else(|| {
        ServiceDbError::InvalidDbMetadata(format!("{location} must be a positive integer"))
    })
}

fn validate_non_empty_name(value: &str, location: String) -> Result<String> {
    (!value.trim().is_empty())
        .then(|| value.to_string())
        .ok_or_else(|| {
            ServiceDbError::InvalidDbMetadata(format!("{location} must be a non-empty string"))
        })
}

struct DbObjectKeyMetadata {
    name: String,
    ty: Option<db_boundary::DbBoundaryValuePlan>,
}

fn parse_object_key(value: Option<&DbObjectKeyIr>, index: usize) -> Result<DbObjectKeyMetadata> {
    let Some(key) = value else {
        return Err(ServiceDbError::InvalidDbMetadata(format!(
            "runtime program db[{index}].key must be an object"
        )));
    };
    let name = validate_non_empty_name(&key.name, format!("runtime program db[{index}].key.name"))?;
    validate_reserved_db_metadata_name(&name, format!("runtime program db[{index}].key.name"))?;
    let ty = Some(db_boundary::DbBoundaryValuePlan::from_artifact_type_ref(
        key.ty.clone(),
    ));
    Ok(DbObjectKeyMetadata { name, ty })
}

fn parse_fields(
    entries: &[DbObjectFieldIr],
    index: usize,
) -> Result<HashMap<String, DbFieldMetadata>> {
    let mut fields = HashMap::new();
    for (field_index, entry) in entries.iter().enumerate() {
        let name = validate_non_empty_name(
            &entry.name,
            format!("runtime program db[{index}].fields[{field_index}].name"),
        )?;
        validate_reserved_db_metadata_name(
            &name,
            format!("runtime program db[{index}].fields[{field_index}].name"),
        )?;
        let ty = Some(db_boundary::DbBoundaryValuePlan::from_artifact_type_ref(
            entry.ty.clone(),
        ));
        if fields
            .insert(
                name.clone(),
                DbFieldMetadata {
                    name,
                    ty,
                    storage: entry.storage,
                },
            )
            .is_some()
        {
            return Err(ServiceDbError::InvalidDbMetadata(format!(
                "runtime program db[{index}] has duplicate field name"
            )));
        }
    }
    Ok(fields)
}

fn is_plain_string_type(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Builtin { name, args } if name == "string" && args.is_empty()
    )
}

fn parse_leases(entries: &[DbLeaseIr], index: usize) -> Result<HashMap<String, DbLeaseMetadata>> {
    let mut leases = HashMap::new();
    for (lease_index, entry) in entries.iter().enumerate() {
        let name = validate_non_empty_name(
            &entry.name,
            format!("runtime program db[{index}].leases[{lease_index}].name"),
        )?;
        validate_reserved_db_metadata_name(
            &name,
            format!("runtime program db[{index}].leases[{lease_index}].name"),
        )?;
        validate_lease_slot_name(
            &name,
            format!("runtime program db[{index}].leases[{lease_index}].name"),
        )?;
        let ttl_ms = required_positive_u64(
            entry.ttl_ms,
            format!("runtime program db[{index}].leases[{lease_index}].ttlMs"),
        )?;
        let max_ms = optional_positive_u64(
            entry.max_ms,
            format!("runtime program db[{index}].leases[{lease_index}].maxMs"),
        )?;
        if leases
            .insert(
                name.clone(),
                DbLeaseMetadata {
                    name,
                    ttl_ms,
                    max_ms,
                },
            )
            .is_some()
        {
            return Err(ServiceDbError::InvalidDbMetadata(format!(
                "runtime program db[{index}] has duplicate lease name"
            )));
        }
    }
    Ok(leases)
}

fn validate_reserved_db_metadata_name(name: &str, location: String) -> Result<()> {
    if db_boundary::validate_db_business_field_name(name).is_err() {
        return Err(ServiceDbError::InvalidDbMetadata(format!(
            "{location} {name:?} is reserved Skiff metadata"
        )));
    }
    Ok(())
}

fn validate_lease_slot_name(name: &str, location: String) -> Result<()> {
    if name.contains('.') || name.contains('$') || name.contains('\0') {
        return Err(ServiceDbError::InvalidDbMetadata(format!(
            "{location} {name:?} cannot contain '.', '$', or NUL"
        )));
    }
    Ok(())
}

fn immutable_file_paths(fields: &HashMap<String, DbFieldMetadata>) -> Vec<Vec<String>> {
    let mut paths = Vec::new();
    for field in fields.values() {
        let Some(ty) = &field.ty else {
            continue;
        };
        collect_immutable_file_paths(ty.artifact_type_ref(), vec![field.name.clone()], &mut paths);
    }
    paths.sort();
    paths.dedup();
    paths
}

fn collect_immutable_file_paths(ty: &TypeRefIr, path: Vec<String>, paths: &mut Vec<Vec<String>>) {
    match ty {
        TypeRefIr::Nullable { inner } => collect_immutable_file_paths(inner, path, paths),
        TypeRefIr::Record { fields } => {
            for (field, field_ty) in fields {
                let mut nested = path.clone();
                nested.push(field.clone());
                collect_immutable_file_paths(field_ty, nested, paths);
            }
        }
        ty if is_immutable_file_type(ty) => paths.push(path),
        _ => {}
    }
}

fn is_immutable_file_type(ty: &TypeRefIr) -> bool {
    match ty {
        TypeRefIr::Builtin { name, .. } => {
            matches!(name.as_str(), "std.file.ImmutableFile" | "ImmutableFile")
        }
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            symbol.symbol_path() == "std.file.ImmutableFile"
        }
        TypeRefIr::PackageSymbol { symbol } => {
            symbol.symbol_path == "std.file.ImmutableFile"
                || matches!(
                    &symbol.package,
                    PackageRefIr::PackageId { package_id } if package_id == "std.file" && symbol.symbol_path == "ImmutableFile"
                )
                || matches!(
                    &symbol.package,
                    PackageRefIr::PackageId { package_id } if package_id == "std" && symbol.symbol_path == "file.ImmutableFile"
                )
        }
        TypeRefIr::Nullable { inner } => is_immutable_file_type(inner),
        _ => false,
    }
}

fn path_touches_file_path(touched: &[&str], file_path: &[String]) -> bool {
    touched.len() <= file_path.len()
        && touched
            .iter()
            .zip(file_path.iter())
            .all(|(left, right)| *left == right)
}

fn parse_indexes(entries: &[DbMetadataIndexIr], index: usize) -> Result<Vec<DbIndexMetadata>> {
    let mut indexes = Vec::new();
    for (index_index, entry) in entries.iter().enumerate() {
        indexes.push(DbIndexMetadata {
            name: validate_non_empty_name(
                &entry.name,
                format!("runtime program db[{index}].indexes[{index_index}].name"),
            )?,
            unique: entry.unique,
            fields: parse_order_entries(&entry.fields, index, "indexes.fields")?,
            where_filter: entry.where_expr.clone(),
        });
    }
    Ok(indexes)
}

fn parse_order_entries(
    entries: &[DbIndexFieldIr],
    index: usize,
    field: &str,
) -> Result<Vec<DbOrderEntry>> {
    entries
        .iter()
        .map(|entry| {
            let direction = match entry.direction {
                DbIndexDirectionIr::Asc => DbOrderDirection::Asc,
                DbIndexDirectionIr::Desc => DbOrderDirection::Desc,
            };
            Ok(DbOrderEntry {
                field: parse_field_path_value(&entry.field, index, field)?,
                direction,
            })
        })
        .collect()
}

fn parse_field_path_value(value: &FieldPathIr, index: usize, field: &str) -> Result<FieldPath> {
    let text = value.text.clone();
    let segments = if value.segments.is_empty() && !text.trim().is_empty() {
        text.split('.').map(str::to_string).collect()
    } else {
        value
            .segments
            .iter()
            .map(|segment| {
                required_field_path_segment(segment, format!("runtime program db[{index}].{field}"))
            })
            .collect::<Result<Vec<_>>>()?
    };
    if text.trim().is_empty() && segments.is_empty() {
        return Err(ServiceDbError::InvalidDbMetadata(format!(
            "runtime program db[{index}].{field} must name a field"
        )));
    }
    Ok(FieldPath { text, segments })
}
