use std::collections::BTreeSet;

use serde::Deserialize;

use super::MigrationToolError;

const ALLOWLIST_SCHEMA: &str = "skiff-stable-service-db-filtered-migration-v1";
const ALLOWLIST_KIND: &str = "retain-allowlist";
const SANITIZATION_KIND: &str = "skiff-filtered-db-migration-sanitization-preflight";

fn parse_exact_json<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = T::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

pub(super) fn parse_validated_receipts(
    allowlist_bytes: &[u8],
    sanitization_bytes: &[u8],
    allowlist_file_name: &str,
) -> Result<(Vec<AllowlistEntry>, ValidatedSanitization), MigrationToolError> {
    let allowlist: FilteredAllowlistReceipt =
        parse_exact_json(allowlist_bytes).map_err(|_| MigrationToolError::InvalidPlan)?;
    let sanitization: SanitizationReceipt =
        parse_exact_json(sanitization_bytes).map_err(|_| MigrationToolError::InvalidPlan)?;
    allowlist.validate()?;
    let validated_sanitization = sanitization.validate(&allowlist, allowlist_file_name)?;
    Ok((allowlist.allowlist, validated_sanitization))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FilteredAllowlistReceipt {
    pub(super) schema_version: String,
    pub(super) generated_at: String,
    pub(super) source_receipt: serde_json::Value,
    pub(super) decision: String,
    pub(super) constraints: AllowlistConstraints,
    pub(super) receipt_kind: String,
    pub(super) summary: serde_json::Value,
    pub(super) enforcement: AllowlistEnforcement,
    pub(super) allowlist: Vec<AllowlistEntry>,
}

impl FilteredAllowlistReceipt {
    fn validate(&self) -> Result<(), MigrationToolError> {
        if self.schema_version != ALLOWLIST_SCHEMA
            || self.receipt_kind != ALLOWLIST_KIND
            || self.generated_at.trim().is_empty()
            || self.source_receipt.is_null()
            || self.decision.trim().is_empty()
            || self.summary.is_null()
            || !self.constraints.old_databases_remain_untouched
            || !self
                .constraints
                .complete_source_and_destination_backup_required
            || self
                .constraints
                .current_destination_database_records_retained
            || self.constraints.mongo_mutated_by_this_receipt
            || self.constraints.business_values_included
            || self.constraints.ids_included
            || !self.constraints.id_hashes_only
            || self.enforcement.default_disposition != "discard"
            || self.enforcement.r#match.trim().is_empty()
            || self.enforcement.unknown_semantic_target != "fail"
            || self.enforcement.unknown_physical_source_collection != "do-not-project"
            || self.enforcement.encrypted_records.trim().is_empty()
            || self.allowlist.is_empty()
        {
            return Err(MigrationToolError::InvalidPlan);
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AllowlistConstraints {
    pub(super) old_databases_remain_untouched: bool,
    pub(super) complete_source_and_destination_backup_required: bool,
    pub(super) current_destination_database_records_retained: bool,
    pub(super) mongo_mutated_by_this_receipt: bool,
    pub(super) business_values_included: bool,
    pub(super) ids_included: bool,
    pub(super) id_hashes_only: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AllowlistEnforcement {
    pub(super) default_disposition: String,
    r#match: String,
    pub(super) unknown_semantic_target: String,
    pub(super) unknown_physical_source_collection: String,
    pub(super) encrypted_records: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct AllowlistEntry {
    pub(super) disposition: String,
    pub(super) semantic_target: SemanticTarget,
    pub(super) static_schema_evidence: serde_json::Value,
    pub(super) physical_target: PhysicalTarget,
    pub(super) observed: ObservedSource,
    pub(super) reason: serde_json::Value,
    pub(super) migration_class: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SemanticTarget {
    pub(super) service_id: String,
    pub(super) package_id: String,
    pub(super) logical_collection_identity: String,
    pub(super) module_path: String,
    pub(super) symbol: String,
    pub(super) key_field: String,
    pub(super) encrypted_fields: Vec<String>,
    pub(super) declared_indexes: Vec<DeclaredIndex>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DeclaredIndex {
    pub(super) name: String,
    pub(super) unique: bool,
    pub(super) fields: Vec<DeclaredIndexField>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DeclaredIndexField {
    pub(super) path: String,
    pub(super) direction: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PhysicalTarget {
    pub(super) old: StorageLocation,
    pub(super) new: StorageLocation,
}

#[derive(Clone, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct StorageLocation {
    pub(super) database: String,
    pub(super) collection: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ObservedSource {
    pub(super) source_exists: bool,
    pub(super) source_record_count: u64,
    pub(super) source_id_sample_hashes: Vec<String>,
    pub(super) destination_exists: bool,
    pub(super) destination_record_count: u64,
    pub(super) duplicate_id_count: u64,
    pub(super) destination_only_id_count: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SanitizationReceipt {
    pub(super) schema_version: u32,
    pub(super) receipt_kind: String,
    pub(super) generated_at: String,
    pub(super) mode: String,
    pub(super) source_receipt: String,
    pub(super) scope: SanitizationScope,
    pub(super) unique_audit_policy: serde_json::Value,
    pub(super) unique_constraints: serde_json::Value,
    pub(super) unique_audit_summary: UniqueAuditSummary,
    pub(super) tool_provider_sanitization: ToolProviderSanitization,
    pub(super) overall: SanitizationOverall,
}

pub(super) struct ValidatedSanitization {
    pub(super) tool_provider_source: StorageLocation,
}

impl SanitizationReceipt {
    fn validate(
        self,
        allowlist: &FilteredAllowlistReceipt,
        allowlist_file_name: &str,
    ) -> Result<ValidatedSanitization, MigrationToolError> {
        let unique_count = allowlist
            .allowlist
            .iter()
            .flat_map(|entry| &entry.semantic_target.declared_indexes)
            .filter(|index| index.unique)
            .count();
        if self.schema_version != 1
            || self.receipt_kind != SANITIZATION_KIND
            || self.generated_at.trim().is_empty()
            || self.mode != "read-only"
            || self.source_receipt != allowlist_file_name
            || self.scope.retain_mapping_count != allowlist.allowlist.len()
            || self.scope.discard_mappings_scanned != 0
            || self.scope.unique_index_declarations_found != unique_count
            || self.unique_audit_policy.is_null()
            || self.unique_constraints.is_null()
            || self.unique_audit_summary.constraints_audited != unique_count
            || self.unique_audit_summary.duplicate_group_count != 0
            || self.unique_audit_summary.affected_documents != 0
            || self.unique_audit_summary.malformed_required_field_documents != 0
            || self.unique_audit_summary.unique_blocker
            || self.tool_provider_sanitization.blocker
            || self.overall.unique_blocker
            || self.overall.sanitization_blocker
            || !self.overall.migration_may_proceed
            || self.overall.remaining_preconditions.len() != 2
            || self
                .overall
                .remaining_preconditions
                .iter()
                .any(|precondition| precondition.trim().is_empty())
        {
            return Err(MigrationToolError::InvalidPlan);
        }
        self.tool_provider_sanitization.validate_plan()?;
        Ok(ValidatedSanitization {
            tool_provider_source: self.scope.tool_provider_source,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SanitizationOverall {
    pub(super) unique_blocker: bool,
    pub(super) sanitization_blocker: bool,
    pub(super) migration_may_proceed: bool,
    pub(super) remaining_preconditions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SanitizationScope {
    pub(super) retain_mapping_count: usize,
    pub(super) discard_mappings_scanned: usize,
    pub(super) unique_index_declarations_found: usize,
    pub(super) tool_provider_source: StorageLocation,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UniqueAuditSummary {
    pub(super) constraints_audited: usize,
    pub(super) duplicate_group_count: usize,
    pub(super) affected_documents: usize,
    pub(super) malformed_required_field_documents: usize,
    pub(super) unique_blocker: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolProviderSanitization {
    pub(super) schema_evidence: serde_json::Value,
    pub(super) observed: serde_json::Value,
    pub(super) migration_mutation_plan: Vec<SanitizationMutation>,
    pub(super) preserve_exactly: Vec<String>,
    pub(super) updated_at_policy: String,
    pub(super) blocker: bool,
}

impl ToolProviderSanitization {
    fn validate_plan(&self) -> Result<(), MigrationToolError> {
        let mut saw_set = false;
        let mut saw_top_level_unset = false;
        let mut saw_metadata_unset = false;
        for mutation in &self.migration_mutation_plan {
            match mutation.operation.as_str() {
                "set"
                    if mutation.path.as_deref() == Some("presence")
                        && mutation.value.as_deref() == Some("offline")
                        && mutation.paths.is_none()
                        && mutation.read_semantics_after_unset.is_none() =>
                {
                    saw_set = true;
                }
                "unset"
                    if mutation.path.is_none()
                        && mutation.value.is_none()
                        && mutation.paths.as_ref().is_some_and(|paths| {
                            as_set(paths)
                                == BTreeSet::from([
                                    "actorSubjectId",
                                    "activeConnectionId",
                                    "lastSeenAt",
                                ])
                        })
                        && mutation.read_semantics_after_unset.as_deref() == Some("null") =>
                {
                    saw_top_level_unset = true;
                }
                "unset"
                    if mutation.path.is_none()
                        && mutation.value.is_none()
                        && mutation.paths.as_ref().is_some_and(|paths| {
                            as_set(paths)
                                == BTreeSet::from([
                                    "metadata.currentDirectory",
                                    "metadata.capabilities",
                                ])
                        })
                        && mutation.read_semantics_after_unset.is_none() =>
                {
                    saw_metadata_unset = true;
                }
                _ => return Err(MigrationToolError::InvalidPlan),
            }
        }
        let preserve = as_set(&self.preserve_exactly);
        let required_preserve = BTreeSet::from([
            "id",
            "providerId",
            "userId",
            "name",
            "status",
            "credentialRef",
            "metadata.hostIdHash",
            "all metadata keys other than currentDirectory and capabilities",
            "createdAt",
            "updatedAt",
            "removedAt",
        ]);
        if self.migration_mutation_plan.len() != 3
            || !saw_set
            || !saw_top_level_unset
            || !saw_metadata_unset
            || preserve != required_preserve
            || self.updated_at_policy
                != "Preserve the old business timestamp. Sanitization is a migration transform, not a user-visible provider settings edit."
            || self.schema_evidence.is_null()
            || self.observed.is_null()
            || self
                .migration_mutation_plan
                .iter()
                .any(|mutation| mutation.scope.trim().is_empty() || mutation.reason.trim().is_empty())
        {
            return Err(MigrationToolError::InvalidPlan);
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SanitizationMutation {
    pub(super) operation: String,
    #[serde(default)]
    pub(super) path: Option<String>,
    #[serde(default)]
    pub(super) value: Option<String>,
    #[serde(default)]
    pub(super) paths: Option<Vec<String>>,
    pub(super) scope: String,
    #[serde(default)]
    pub(super) read_semantics_after_unset: Option<String>,
    pub(super) reason: String,
}

fn as_set<'a>(values: &'a [String]) -> BTreeSet<&'a str> {
    values.iter().map(String::as_str).collect()
}
