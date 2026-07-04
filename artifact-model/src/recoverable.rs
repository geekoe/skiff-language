//! Persisted recoverable artifact contract.
//!
//! These structs are the durable schema emitted into compiler artifacts and
//! consumed by runtime/linker code. Runtime-only recoverable diagnostic views
//! live in `skiff_runtime_model::recoverable` and are not authoritative
//! artifact metadata.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::types::TypeRefIr;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverableArtifactMetadata {
    #[serde(default, skip_serializing_if = "RecoverableIdentityTables::is_empty")]
    pub identity_tables: RecoverableIdentityTables,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub custom_restore_plans: BTreeMap<String, RecoverableCustomRestorePlan>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub native_adapter_plans: BTreeMap<String, RecoverableNativeAdapterPlan>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub boundary_plans: BTreeMap<String, RecoverableBoundaryPlan>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub storage_lanes: BTreeMap<String, RecoverableStorageLanePlan>,
    #[serde(default, skip_serializing_if = "RecoverableCapabilitySet::is_empty")]
    pub capabilities: RecoverableCapabilitySet,
}

impl RecoverableArtifactMetadata {
    pub fn is_empty(&self) -> bool {
        self.identity_tables.is_empty()
            && self.custom_restore_plans.is_empty()
            && self.native_adapter_plans.is_empty()
            && self.boundary_plans.is_empty()
            && self.storage_lanes.is_empty()
            && self.capabilities.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverableIdentityTables {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub types: BTreeMap<String, RecoverableTypeIdentityFact>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub interface_projections: BTreeMap<String, RecoverableInterfaceProjectionIdentityFact>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub interface_methods: BTreeMap<String, RecoverableInterfaceMethodIdentityFact>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub union_branches: BTreeMap<String, RecoverableUnionBranchIdentityFact>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, RecoverableFieldIdentityFact>,
}

impl RecoverableIdentityTables {
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
            && self.interface_projections.is_empty()
            && self.interface_methods.is_empty()
            && self.union_branches.is_empty()
            && self.fields.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecoverableTypeIdentityRef(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecoverableInterfaceProjectionIdentityRef(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecoverableInterfaceMethodIdentityRef(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecoverableUnionBranchIdentityRef(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecoverableFieldIdentityRef(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecoverableStorageLaneRef(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecoverableCustomRestorePlanRef(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecoverableNativeAdapterPlanRef(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverableTypeIdentityFact {
    #[serde(rename = "type")]
    pub ty: TypeRefIr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abi_type_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverableInterfaceProjectionIdentityFact {
    pub interface_type: TypeRefIr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implemented_by: Option<TypeRefIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface_abi_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverableInterfaceMethodIdentityFact {
    pub interface_projection_ref: RecoverableInterfaceProjectionIdentityRef,
    pub method_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method_abi_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<RecoverableExpectedTypePlan>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverableUnionBranchIdentityFact {
    pub union_type_ref: RecoverableTypeIdentityRef,
    pub branch_index: u32,
    pub branch_type: TypeRefIr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_abi_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverableFieldIdentityFact {
    pub owner_type_ref: RecoverableTypeIdentityRef,
    pub field_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_type: Option<TypeRefIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_abi_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverableExpectedTypePlan {
    pub root: RecoverableExpectedTypeRoot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_type_identity_ref: Option<RecoverableTypeIdentityRef>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub runtime_carrier_check_required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interface_projection_refs: Vec<RecoverableInterfaceProjectionIdentityRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interface_method_refs: Vec<RecoverableInterfaceMethodIdentityRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_refs: Vec<RecoverableFieldIdentityRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub union_branch_refs: Vec<RecoverableUnionBranchIdentityRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum RecoverableExpectedTypeRoot {
    TypeRef {
        #[serde(rename = "type")]
        ty: TypeRefIr,
    },
    TypeIdentityRef {
        type_identity_ref: RecoverableTypeIdentityRef,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecoverableBoundaryKind {
    DbPayload,
    SpawnPayload,
    QueuePayload,
    RuntimeWirePayload,
    ServicePayload,
    PublicApiPayload,
    Materialization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecoverableTrustBoundary {
    OwnerInternal,
    CrossService,
    ExternalUntrusted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecoverableStorageLane {
    SchemaProjectable,
    RecoverableEnvelope,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverableBoundaryContext {
    pub boundary_kind: RecoverableBoundaryKind,
    pub trust_boundary: RecoverableTrustBoundary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_service: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_service: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub explicit_recoverable_slot: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverableBoundaryPlan {
    pub context: RecoverableBoundaryContext,
    pub expected_type: RecoverableExpectedTypePlan,
    #[serde(default, skip_serializing_if = "is_false")]
    pub runtime_carrier_check_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_lane_ref: Option<RecoverableStorageLaneRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_restore_plan_ref: Option<RecoverableCustomRestorePlanRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_adapter_plan_ref: Option<RecoverableNativeAdapterPlanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverableStorageLanePlan {
    pub lane: RecoverableStorageLane,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_type: Option<RecoverableExpectedTypePlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_projection_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envelope_slot_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverableCustomRestorePlan {
    pub concrete_type_identity: String,
    pub durable_state_type_plan: RecoverableExpectedTypePlan,
    pub encode_hook_id: String,
    pub decode_hook_id: String,
    pub restore_capability: RecoverableRestoreCapability,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverableNativeAdapterPlan {
    pub adapter_identity: String,
    pub adapter_schema_version: String,
    pub native_type_identity: String,
    pub durable_state_type_plan: RecoverableExpectedTypePlan,
    pub encode_hook_id: String,
    pub decode_hook_id: String,
    pub owner: RecoverableNativeAdapterOwner,
    pub schema_compatibility: RecoverableAdapterSchemaCompatibility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecoverableRestoreCapability {
    Exact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverableNativeAdapterOwner {
    pub service_identity: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecoverableAdapterSchemaCompatibility {
    Exact,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverableCapabilitySet {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub flags: BTreeMap<String, RecoverableCapabilityFlag>,
}

impl RecoverableCapabilitySet {
    pub fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }
}

pub fn recoverable_expected_type_plans_compatible(
    expected: &RecoverableExpectedTypePlan,
    actual: &RecoverableExpectedTypePlan,
) -> bool {
    recoverable_expected_type_roots_compatible(expected, actual)
        && string_set(
            expected
                .interface_projection_refs
                .iter()
                .map(|item| item.0.as_str()),
        ) == string_set(
            actual
                .interface_projection_refs
                .iter()
                .map(|item| item.0.as_str()),
        )
        && string_set(
            expected
                .interface_method_refs
                .iter()
                .map(|item| item.0.as_str()),
        ) == string_set(
            actual
                .interface_method_refs
                .iter()
                .map(|item| item.0.as_str()),
        )
        && string_set(expected.field_refs.iter().map(|item| item.0.as_str()))
            == string_set(actual.field_refs.iter().map(|item| item.0.as_str()))
        && string_set(
            expected
                .union_branch_refs
                .iter()
                .map(|item| item.0.as_str()),
        ) == string_set(actual.union_branch_refs.iter().map(|item| item.0.as_str()))
}

fn recoverable_expected_type_roots_compatible(
    expected: &RecoverableExpectedTypePlan,
    actual: &RecoverableExpectedTypePlan,
) -> bool {
    if same_non_empty_type_identity_ref(
        expected.root_type_identity_ref.as_ref(),
        actual.root_type_identity_ref.as_ref(),
    ) {
        return true;
    }
    if same_non_empty_root_type_identity_ref(&expected.root, &actual.root) {
        return true;
    }
    expected.root == actual.root && expected.root_type_identity_ref == actual.root_type_identity_ref
}

fn same_non_empty_root_type_identity_ref(
    expected: &RecoverableExpectedTypeRoot,
    actual: &RecoverableExpectedTypeRoot,
) -> bool {
    match (expected, actual) {
        (
            RecoverableExpectedTypeRoot::TypeIdentityRef {
                type_identity_ref: expected_ref,
            },
            RecoverableExpectedTypeRoot::TypeIdentityRef {
                type_identity_ref: actual_ref,
            },
        ) => same_non_empty_type_identity_ref(Some(expected_ref), Some(actual_ref)),
        _ => false,
    }
}

fn same_non_empty_type_identity_ref(
    expected: Option<&RecoverableTypeIdentityRef>,
    actual: Option<&RecoverableTypeIdentityRef>,
) -> bool {
    matches!(
        (expected, actual),
        (Some(expected_ref), Some(actual_ref))
            if !expected_ref.0.is_empty() && expected_ref == actual_ref
    )
}

pub fn validate_recoverable_artifact_metadata(
    metadata: &RecoverableArtifactMetadata,
) -> Result<(), RecoverableArtifactMetadataValidationError> {
    let mut violations = Vec::new();
    for (key, plan) in &metadata.custom_restore_plans {
        require_non_empty(
            key,
            "concrete_type_identity",
            &plan.concrete_type_identity,
            &mut violations,
        );
        require_non_empty(key, "encode_hook_id", &plan.encode_hook_id, &mut violations);
        require_non_empty(key, "decode_hook_id", &plan.decode_hook_id, &mut violations);
        validate_expected_type_plan(
            key,
            "durable_state_type_plan",
            &plan.durable_state_type_plan,
            &mut violations,
        );
    }
    for (key, plan) in &metadata.native_adapter_plans {
        require_non_empty(
            key,
            "adapter_identity",
            &plan.adapter_identity,
            &mut violations,
        );
        require_non_empty(
            key,
            "adapter_schema_version",
            &plan.adapter_schema_version,
            &mut violations,
        );
        require_non_empty(
            key,
            "native_type_identity",
            &plan.native_type_identity,
            &mut violations,
        );
        require_non_empty(key, "encode_hook_id", &plan.encode_hook_id, &mut violations);
        require_non_empty(key, "decode_hook_id", &plan.decode_hook_id, &mut violations);
        require_non_empty(
            key,
            "owner.service_identity",
            &plan.owner.service_identity,
            &mut violations,
        );
        validate_expected_type_plan(
            key,
            "durable_state_type_plan",
            &plan.durable_state_type_plan,
            &mut violations,
        );
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(RecoverableArtifactMetadataValidationError { violations })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverableArtifactMetadataValidationError {
    pub violations: Vec<String>,
}

impl std::fmt::Display for RecoverableArtifactMetadataValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.violations.join("\n"))
    }
}

impl std::error::Error for RecoverableArtifactMetadataValidationError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverableCapabilityFlag {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u32>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn string_set<'a>(items: impl Iterator<Item = &'a str>) -> BTreeSet<&'a str> {
    items.collect()
}

fn require_non_empty(owner: &str, field: &str, value: &str, violations: &mut Vec<String>) {
    if value.is_empty() {
        violations.push(format!("{owner} {field} is required"));
    }
}

fn validate_expected_type_plan(
    owner: &str,
    field: &str,
    plan: &RecoverableExpectedTypePlan,
    violations: &mut Vec<String>,
) {
    match &plan.root {
        RecoverableExpectedTypeRoot::TypeRef { .. } => {}
        RecoverableExpectedTypeRoot::TypeIdentityRef { type_identity_ref }
            if !type_identity_ref.0.is_empty() => {}
        RecoverableExpectedTypeRoot::TypeIdentityRef { .. } => {
            violations.push(format!("{owner} {field}.type_identity_ref is required"));
        }
    }
}
