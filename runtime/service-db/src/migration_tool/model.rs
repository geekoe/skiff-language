use std::collections::{BTreeMap, BTreeSet};

use mongodb::{
    bson::{Bson, Document},
    IndexModel,
};
use sha2::{Digest, Sha256};

use crate::{
    index::canonical_managed_index_model, service_storage_collection_name,
    storage_identity::service_storage_database_name, MigrationTargetContext,
};

use super::{
    input_receipts::{
        parse_validated_receipts, AllowlistEntry, DeclaredIndex, StorageLocation,
        ValidatedSanitization,
    },
    MigrationToolError,
};

const TOOL_PROVIDER_SERVICE: &str = "agine.ai/api";
const TOOL_PROVIDER_PACKAGE: &str = "agine.ai/api";
const TOOL_PROVIDER_COLLECTION: &str = "ToolProvider";

pub(crate) struct ValidatedMigrationPlan {
    pub(crate) mappings: Vec<ValidatedCollectionMapping>,
}

pub(crate) struct ValidatedCollectionMapping {
    pub(crate) mapping_id: String,
    pub(crate) source: StorageEndpoint,
    pub(crate) target: StorageEndpoint,
    pub(crate) source_exists: bool,
    pub(crate) expected_source_count: u64,
    pub(crate) encrypted_fields: Vec<String>,
    pub(crate) target_indexes: Vec<IndexModel>,
    pub(crate) sanitizer: DocumentSanitizer,
}

#[derive(Clone)]
pub(crate) struct StorageEndpoint {
    pub(crate) environment: String,
    pub(crate) service_id: String,
    pub(crate) database: String,
    pub(crate) package_id: String,
    pub(crate) logical_collection: String,
    pub(crate) physical_collection: String,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum DocumentSanitizer {
    None,
    ToolProvider,
}

impl ValidatedMigrationPlan {
    pub(crate) fn parse(
        allowlist_bytes: &[u8],
        sanitization_bytes: &[u8],
        environment: &str,
        allowlist_file_name: &str,
    ) -> Result<Self, MigrationToolError> {
        validate_environment(environment)?;
        let (allowlist, sanitizer) =
            parse_validated_receipts(allowlist_bytes, sanitization_bytes, allowlist_file_name)?;

        let mut mapping_ids = BTreeSet::new();
        let mut sources = BTreeSet::new();
        let mut targets = BTreeSet::new();
        let mut mappings = Vec::with_capacity(allowlist.len());
        for entry in allowlist {
            let mapping = validate_mapping(entry, environment, &sanitizer)?;
            if !mapping_ids.insert(mapping.mapping_id.clone())
                || !sources.insert((
                    mapping.source.database.clone(),
                    mapping.source.physical_collection.clone(),
                ))
                || !targets.insert((
                    mapping.target.database.clone(),
                    mapping.target.physical_collection.clone(),
                ))
            {
                return Err(MigrationToolError::InvalidPlan);
            }
            mappings.push(mapping);
        }
        if mappings.is_empty() {
            return Err(MigrationToolError::InvalidPlan);
        }
        mappings.sort_by(|left, right| left.mapping_id.cmp(&right.mapping_id));
        Ok(Self { mappings })
    }
}

impl ValidatedCollectionMapping {
    pub(crate) fn target_context(&self) -> MigrationTargetContext<'_> {
        MigrationTargetContext {
            environment: &self.target.environment,
            service_id: &self.target.service_id,
            collection_name: &self.target.physical_collection,
        }
    }

    pub(crate) fn sanitize(&self, document: &mut Document) -> Result<(), MigrationToolError> {
        match self.sanitizer {
            DocumentSanitizer::None => Ok(()),
            DocumentSanitizer::ToolProvider => {
                document.insert("presence", "offline");
                for field in ["actorSubjectId", "activeConnectionId", "lastSeenAt"] {
                    document.remove(field);
                }
                let metadata = document
                    .get_mut("metadata")
                    .ok_or_else(|| MigrationToolError::InvalidSource(self.mapping_id.clone()))?;
                let Bson::Document(metadata) = metadata else {
                    return Err(MigrationToolError::InvalidSource(self.mapping_id.clone()));
                };
                metadata.remove("currentDirectory");
                metadata.remove("capabilities");
                Ok(())
            }
        }
    }
}

fn validate_mapping(
    entry: AllowlistEntry,
    environment: &str,
    sanitizer: &ValidatedSanitization,
) -> Result<ValidatedCollectionMapping, MigrationToolError> {
    if entry.disposition != "retain" {
        return Err(MigrationToolError::InvalidPlan);
    }
    let semantic = entry.semantic_target;
    validate_canonical_value(&semantic.service_id)?;
    validate_canonical_value(&semantic.package_id)?;
    validate_canonical_value(&semantic.logical_collection_identity)?;
    validate_canonical_value(&semantic.module_path)?;
    validate_canonical_value(&semantic.symbol)?;
    validate_field_path(&semantic.key_field)?;
    validate_encrypted_fields(&semantic.encrypted_fields)?;
    validate_storage_location(&entry.physical_target.old)?;
    validate_storage_location(&entry.physical_target.new)?;
    if !entry.observed.source_exists && entry.observed.source_record_count != 0 {
        return Err(MigrationToolError::InvalidPlan);
    }
    if entry.static_schema_evidence.is_null()
        || entry.reason.is_null()
        || !matches!(
            entry.migration_class.as_str(),
            "raw-bson-namespace-remap" | "offline-v1-to-v2-reencrypt"
        )
        || entry.observed.duplicate_id_count != 0
        || entry.observed.destination_only_id_count > entry.observed.destination_record_count
        || (!entry.observed.destination_exists && entry.observed.destination_record_count != 0)
        || entry.observed.source_id_sample_hashes.iter().any(|hash| {
            hash.len() != 64
                || !hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        })
    {
        return Err(MigrationToolError::InvalidPlan);
    }

    let target_database = service_storage_database_name(environment, &semantic.service_id)
        .map_err(|_| MigrationToolError::InvalidPlan)?;
    let target_collection = service_storage_collection_name(
        &semantic.package_id,
        &semantic.logical_collection_identity,
    )
    .map_err(|_| MigrationToolError::InvalidPlan)?;
    if entry.physical_target.new.database != target_database
        || entry.physical_target.new.collection != target_collection
    {
        return Err(MigrationToolError::InvalidPlan);
    }

    let target_indexes = validate_indexes(
        &semantic.package_id,
        &semantic.logical_collection_identity,
        &semantic.key_field,
        &semantic.encrypted_fields,
        semantic.declared_indexes,
    )?;
    let is_tool_provider = semantic.service_id == TOOL_PROVIDER_SERVICE
        && semantic.package_id == TOOL_PROVIDER_PACKAGE
        && semantic.logical_collection_identity == TOOL_PROVIDER_COLLECTION;
    let document_sanitizer = if is_tool_provider {
        if sanitizer.tool_provider_source != entry.physical_target.old {
            return Err(MigrationToolError::InvalidPlan);
        }
        DocumentSanitizer::ToolProvider
    } else {
        DocumentSanitizer::None
    };

    let source = StorageEndpoint {
        environment: environment.to_owned(),
        service_id: semantic.service_id.clone(),
        database: entry.physical_target.old.database,
        package_id: semantic.package_id.clone(),
        logical_collection: semantic.logical_collection_identity.clone(),
        physical_collection: entry.physical_target.old.collection,
    };
    let target = StorageEndpoint {
        environment: environment.to_owned(),
        service_id: semantic.service_id,
        database: entry.physical_target.new.database,
        package_id: semantic.package_id,
        logical_collection: semantic.logical_collection_identity,
        physical_collection: entry.physical_target.new.collection,
    };
    let mapping_id = mapping_id(&source, &target);
    Ok(ValidatedCollectionMapping {
        mapping_id,
        source,
        target,
        source_exists: entry.observed.source_exists,
        expected_source_count: entry.observed.source_record_count,
        encrypted_fields: semantic.encrypted_fields,
        target_indexes,
        sanitizer: document_sanitizer,
    })
}

fn validate_indexes(
    package_id: &str,
    logical_collection: &str,
    key_field: &str,
    encrypted_fields: &[String],
    indexes: Vec<DeclaredIndex>,
) -> Result<Vec<IndexModel>, MigrationToolError> {
    let mut names = BTreeSet::new();
    let mut models = BTreeMap::new();
    for index in indexes {
        validate_canonical_value(&index.name)?;
        if !names.insert(index.name.clone()) || index.fields.is_empty() {
            return Err(MigrationToolError::InvalidPlan);
        }
        let mut logical_fields = BTreeSet::new();
        let mut physical_fields = BTreeSet::new();
        let mut keys = Vec::with_capacity(index.fields.len());
        for field in index.fields {
            validate_field_path(&field.path)?;
            if !logical_fields.insert(field.path.clone())
                || encrypted_fields.iter().any(|encrypted| {
                    field.path == *encrypted || field.path.starts_with(&format!("{encrypted}."))
                })
            {
                return Err(MigrationToolError::InvalidPlan);
            }
            let physical = if field.path == key_field {
                "_id".to_owned()
            } else {
                field.path
            };
            if !physical_fields.insert(physical.clone()) {
                return Err(MigrationToolError::InvalidPlan);
            }
            let direction = match field.direction.as_str() {
                "asc" => 1,
                "desc" => -1,
                _ => return Err(MigrationToolError::InvalidPlan),
            };
            keys.push((physical, direction));
        }
        let model = canonical_managed_index_model(
            package_id,
            logical_collection,
            &index.name,
            keys,
            index.unique,
        )
        .map_err(|_| MigrationToolError::InvalidPlan)?;
        let name = model
            .options
            .as_ref()
            .and_then(|options| options.name.clone())
            .ok_or(MigrationToolError::InvalidPlan)?;
        models.insert(name, model);
    }
    Ok(models.into_values().collect())
}

fn mapping_id(source: &StorageEndpoint, target: &StorageEndpoint) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"skiff-service-db-filtered-mapping-v1");
    for value in [
        source.service_id.as_str(),
        source.package_id.as_str(),
        source.logical_collection.as_str(),
        source.database.as_str(),
        source.physical_collection.as_str(),
        target.database.as_str(),
        target.physical_collection.as_str(),
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("m-{}", &hex::encode(hasher.finalize())[..32])
}

fn validate_environment(value: &str) -> Result<(), MigrationToolError> {
    validate_canonical_value(value)?;
    service_storage_database_name(value, "example.test/service")
        .map(|_| ())
        .map_err(|_| MigrationToolError::InvalidPlan)
}

fn validate_canonical_value(value: &str) -> Result<(), MigrationToolError> {
    if value.is_empty() || value != value.trim() || value.contains('\0') {
        return Err(MigrationToolError::InvalidPlan);
    }
    Ok(())
}

fn validate_storage_location(value: &StorageLocation) -> Result<(), MigrationToolError> {
    for name in [&value.database, &value.collection] {
        validate_canonical_value(name)?;
        if name.starts_with("system.")
            || name.contains(['*', '?', '[', ']', '{', '}', '$'])
            || name.contains("..")
        {
            return Err(MigrationToolError::InvalidPlan);
        }
    }
    if matches!(value.database.as_str(), "admin" | "config" | "local") {
        return Err(MigrationToolError::InvalidPlan);
    }
    Ok(())
}

fn validate_field_path(value: &str) -> Result<(), MigrationToolError> {
    validate_canonical_value(value)?;
    if value.starts_with('.')
        || value.ends_with('.')
        || value.split('.').any(|segment| {
            segment.is_empty() || segment.starts_with('$') || segment.contains(['\0', '[', ']'])
        })
    {
        return Err(MigrationToolError::InvalidPlan);
    }
    Ok(())
}

fn validate_encrypted_fields(fields: &[String]) -> Result<(), MigrationToolError> {
    let mut previous = None;
    for field in fields {
        validate_field_path(field)?;
        if field.contains('.') || previous.is_some_and(|previous: &String| previous >= field) {
            return Err(MigrationToolError::InvalidPlan);
        }
        previous = Some(field);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        mapping_id, DocumentSanitizer, StorageEndpoint, ValidatedCollectionMapping,
        ValidatedMigrationPlan,
    };
    use mongodb::bson::doc;

    fn endpoint(database: &str, collection: &str) -> StorageEndpoint {
        StorageEndpoint {
            environment: "dev".to_owned(),
            service_id: "example.test/service".to_owned(),
            database: database.to_owned(),
            package_id: "example.test/package".to_owned(),
            logical_collection: "Item".to_owned(),
            physical_collection: collection.to_owned(),
        }
    }

    #[test]
    fn mapping_identity_is_stable_and_bound_to_both_namespaces() {
        let source = endpoint("old", "items");
        let target = endpoint("new", "_skiff_c1_items");
        let first = mapping_id(&source, &target);
        assert_eq!(first.len(), 34);
        assert_eq!(first, mapping_id(&source, &target));
        assert_ne!(
            first,
            mapping_id(&source, &endpoint("other", "_skiff_c1_items"))
        );
    }

    #[test]
    fn tool_provider_sanitizer_removes_only_live_connection_state() {
        let mapping = ValidatedCollectionMapping {
            mapping_id: "m-00000000000000000000000000000000".to_owned(),
            source: endpoint("old", "ToolProvider"),
            target: endpoint("new", "_skiff_c1_tool_provider"),
            source_exists: true,
            expected_source_count: 1,
            encrypted_fields: Vec::new(),
            target_indexes: Vec::new(),
            sanitizer: DocumentSanitizer::ToolProvider,
        };
        let mut document = doc! {
            "_id": "provider",
            "presence": "online",
            "actorSubjectId": "actor",
            "activeConnectionId": "connection",
            "lastSeenAt": "now",
            "updatedAt": "business-time",
            "metadata": {
                "hostIdHash": "stable",
                "currentDirectory": "/tmp",
                "capabilities": ["tool"],
                "setting": true
            }
        };
        mapping.sanitize(&mut document).expect("sanitize");
        assert_eq!(document.get_str("presence"), Ok("offline"));
        assert!(!document.contains_key("actorSubjectId"));
        assert!(!document.contains_key("activeConnectionId"));
        assert!(!document.contains_key("lastSeenAt"));
        assert_eq!(document.get_str("updatedAt"), Ok("business-time"));
        assert_eq!(
            document.get_document("metadata").expect("metadata").clone(),
            doc! { "hostIdHash": "stable", "setting": true }
        );
    }

    #[test]
    #[ignore = "requires explicit audited filtered receipt paths"]
    fn audited_filtered_receipts_build_an_exact_plan() {
        let allowlist_path =
            std::env::var("SKIFF_TEST_DB_MIGRATION_ALLOWLIST").expect("allowlist path");
        let sanitization_path =
            std::env::var("SKIFF_TEST_DB_MIGRATION_SANITIZATION").expect("sanitization path");
        let allowlist = std::fs::read(&allowlist_path).expect("allowlist");
        let sanitization = std::fs::read(sanitization_path).expect("sanitization");
        let file_name = std::path::Path::new(&allowlist_path)
            .file_name()
            .and_then(|name| name.to_str())
            .expect("allowlist file name");
        let plan = ValidatedMigrationPlan::parse(&allowlist, &sanitization, "dev", file_name)
            .expect("exact audited plan");
        assert_eq!(plan.mappings.len(), 13);
        assert_eq!(
            plan.mappings
                .iter()
                .map(|mapping| mapping.expected_source_count)
                .sum::<u64>(),
            376
        );
        assert_eq!(
            plan.mappings
                .iter()
                .filter(|mapping| !mapping.source_exists)
                .count(),
            5
        );
        assert_eq!(
            plan.mappings
                .iter()
                .map(|mapping| mapping.target_indexes.len())
                .sum::<usize>(),
            19
        );
    }
}
