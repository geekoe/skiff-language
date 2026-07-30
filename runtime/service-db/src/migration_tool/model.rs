use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    service_storage_collection_name, storage_identity::service_storage_database_name,
    MigrationTargetContext,
};

use super::MigrationToolError;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MigrationPlan {
    schema_version: String,
    pub(crate) offline: bool,
    pub(crate) mappings: Vec<CollectionMapping>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CollectionMapping {
    pub(crate) mapping_id: String,
    pub(crate) source: StorageEndpoint,
    pub(crate) target: StorageEndpoint,
    pub(crate) encrypted_fields: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StorageEndpoint {
    pub(crate) environment: String,
    pub(crate) service_id: String,
    pub(crate) database: String,
    pub(crate) package_id: String,
    pub(crate) logical_collection: String,
    pub(crate) physical_collection: String,
}

pub(crate) struct ValidatedMigrationPlan {
    pub(crate) source: MigrationPlan,
    pub(crate) mappings: Vec<ValidatedCollectionMapping>,
}

pub(crate) struct ValidatedCollectionMapping {
    pub(crate) mapping_id: String,
    pub(crate) source: StorageEndpoint,
    pub(crate) target: StorageEndpoint,
    pub(crate) encrypted_fields: Vec<String>,
}

impl MigrationPlan {
    pub(crate) fn parse(bytes: &[u8], schema: &str) -> Result<Self, MigrationToolError> {
        let mut deserializer = serde_json::Deserializer::from_slice(bytes);
        let plan =
            Self::deserialize(&mut deserializer).map_err(|_| MigrationToolError::InvalidPlan)?;
        deserializer
            .end()
            .map_err(|_| MigrationToolError::InvalidPlan)?;
        if plan.schema_version != schema {
            return Err(MigrationToolError::InvalidPlan);
        }
        Ok(plan)
    }

    pub(crate) fn validate(self) -> Result<ValidatedMigrationPlan, MigrationToolError> {
        if self.mappings.is_empty() {
            return Err(MigrationToolError::InvalidPlan);
        }
        let mut mapping_ids = BTreeSet::new();
        let mut sources = BTreeSet::new();
        let mut targets = BTreeSet::new();
        let mut validated = Vec::with_capacity(self.mappings.len());
        for mapping in &self.mappings {
            validate_mapping_id(&mapping.mapping_id)?;
            if !mapping_ids.insert(mapping.mapping_id.as_str()) {
                return Err(MigrationToolError::InvalidPlan);
            }
            validate_endpoint(&mapping.source)?;
            validate_endpoint(&mapping.target)?;
            if mapping.source.database == mapping.target.database
                && mapping.source.physical_collection == mapping.target.physical_collection
            {
                return Err(MigrationToolError::InvalidPlan);
            }
            let derived_target_database = service_storage_database_name(
                &mapping.target.environment,
                &mapping.target.service_id,
            )
            .map_err(|_| MigrationToolError::InvalidPlan)?;
            let derived_target_collection = service_storage_collection_name(
                &mapping.target.package_id,
                &mapping.target.logical_collection,
            )
            .map_err(|_| MigrationToolError::InvalidPlan)?;
            if mapping.target.database != derived_target_database
                || mapping.target.physical_collection != derived_target_collection
            {
                return Err(MigrationToolError::InvalidPlan);
            }
            // Source semantic identity is explicit and validated, but its old
            // database/collection encoding is intentionally not re-derived.
            service_storage_database_name(&mapping.source.environment, &mapping.source.service_id)
                .map_err(|_| MigrationToolError::InvalidPlan)?;
            service_storage_database_name(&mapping.source.environment, &mapping.source.package_id)
                .map_err(|_| MigrationToolError::InvalidPlan)?;
            service_storage_database_name(&mapping.target.environment, &mapping.target.package_id)
                .map_err(|_| MigrationToolError::InvalidPlan)?;
            service_storage_collection_name(
                &mapping.source.package_id,
                &mapping.source.logical_collection,
            )
            .map_err(|_| MigrationToolError::InvalidPlan)?;
            if !sources.insert((
                mapping.source.database.as_str(),
                mapping.source.physical_collection.as_str(),
            )) || !targets.insert((
                mapping.target.database.as_str(),
                mapping.target.physical_collection.as_str(),
            )) {
                return Err(MigrationToolError::InvalidPlan);
            }
            validate_encrypted_fields(&mapping.encrypted_fields)?;
            validated.push(ValidatedCollectionMapping {
                mapping_id: mapping.mapping_id.clone(),
                source: mapping.source.clone(),
                target: mapping.target.clone(),
                encrypted_fields: mapping.encrypted_fields.clone(),
            });
        }
        Ok(ValidatedMigrationPlan {
            source: self,
            mappings: validated,
        })
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
}

fn validate_mapping_id(value: &str) -> Result<(), MigrationToolError> {
    if value.len() != 34
        || !value.starts_with("m-")
        || !value[2..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(MigrationToolError::InvalidPlan);
    }
    Ok(())
}

fn validate_endpoint(endpoint: &StorageEndpoint) -> Result<(), MigrationToolError> {
    if !is_exact_name(&endpoint.database)
        || !is_exact_name(&endpoint.physical_collection)
        || endpoint.database.starts_with("system.")
        || endpoint.physical_collection.starts_with("system.")
        || matches!(endpoint.database.as_str(), "admin" | "config" | "local")
        || endpoint.package_id.trim().is_empty()
        || endpoint.logical_collection.trim().is_empty()
    {
        return Err(MigrationToolError::InvalidPlan);
    }
    Ok(())
}

fn is_exact_name(value: &str) -> bool {
    !value.is_empty()
        && value == value.trim()
        && !value.contains(['*', '?', '[', ']', '{', '}', '$', '\0'])
        && !value.contains("..")
}

fn validate_encrypted_fields(fields: &[String]) -> Result<(), MigrationToolError> {
    let mut previous = None;
    for field in fields {
        if field.is_empty()
            || field != field.trim()
            || field == "_id"
            || field.contains('.')
            || previous.is_some_and(|previous: &String| previous >= field)
        {
            return Err(MigrationToolError::InvalidPlan);
        }
        previous = Some(field);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MigrationPlan, MigrationToolError};
    use crate::{service_storage_collection_name, storage_identity::service_storage_database_name};

    #[test]
    fn target_must_equal_system_derived_database_and_collection() {
        let database =
            service_storage_database_name("dev", "skiff.run/agine").expect("derived database");
        let collection = service_storage_collection_name("skiff.run/agent", "Provider")
            .expect("derived collection");
        let plan = format!(
            r#"{{
              "schemaVersion":"schema",
              "offline":true,
              "mappings":[{{
                "mappingId":"m-00000000000000000000000000000000",
                "source":{{
                  "environment":"dev",
                  "serviceId":"skiff.run/agine",
                  "database":"skiff~run~~agine",
                  "packageId":"skiff.run/agent",
                  "logicalCollection":"Provider",
                  "physicalCollection":"providers"
                }},
                "target":{{
                  "environment":"dev",
                  "serviceId":"skiff.run/agine",
                  "database":"{database}",
                  "packageId":"skiff.run/agent",
                  "logicalCollection":"Provider",
                  "physicalCollection":"{collection}"
                }},
                "encryptedFields":["apiKey"]
              }}]
            }}"#
        );
        MigrationPlan::parse(plan.as_bytes(), "schema")
            .expect("shape")
            .validate()
            .expect("exact derived target");

        let wrong = plan.replace(&database, "operator-chosen-database");
        assert!(matches!(
            MigrationPlan::parse(wrong.as_bytes(), "schema")
                .expect("shape")
                .validate(),
            Err(MigrationToolError::InvalidPlan)
        ));
    }
}
