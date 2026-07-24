use serde::{Deserialize, Serialize};

use crate::{ContractOperationId, ContractTypeRef, PackageSchemaTypeRef};

use super::BoundaryValuePlan;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundaryParameter {
    pub name: String,
    pub ty: ContractTypeRef,
    pub value_plan: BoundaryValuePlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundaryReturn {
    pub ty: ContractTypeRef,
    pub value_plan: BoundaryValuePlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BoundaryErrorContract {
    None,
    Typed {
        payload_type: ContractTypeRef,
        value_plan: BoundaryValuePlan,
    },
    Unsupported {
        reason: BoundaryFeatureUnavailableReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BoundaryStreamContract {
    Unary,
    ServerStream {
        item_type: ContractTypeRef,
        item_value_plan: BoundaryValuePlan,
    },
    Unsupported {
        reason: BoundaryFeatureUnavailableReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BoundaryCancellationContract {
    NotCancellable,
    Cooperative,
    Unsupported {
        reason: BoundaryFeatureUnavailableReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BoundaryCallbackContract {
    None,
    RequestScoped {
        interface_types: Vec<PackageSchemaTypeRef>,
        lifetime: BoundaryCallbackLifetime,
        expiration_error: BoundaryCallbackExpirationError,
    },
    Unsupported {
        reason: BoundaryFeatureUnavailableReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BoundaryCallbackLifetime {
    TopLevelRequest,
    Stream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BoundaryCallbackExpirationError {
    CapabilityExpired,
    CapabilityUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BoundaryFeatureUnavailableReason {
    LanguageUnsupported,
    UnknownSemantics,
}

/// Public effect promise visible to contract consumers. Complete provider
/// may-effects and resource requirements live in BoundaryImplementationRequirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundaryEffectGuarantee {
    pub detached_parameters: bool,
    pub detached_return: bool,
    pub detached_error: bool,
    pub no_caller_reachable_mutation: bool,
    pub no_caller_value_escape: bool,
    pub no_same_heap_identity: bool,
}

/// Contract-agnostic boundary operation body shared by package callable
/// projections and typed service-contract definitions. It deliberately has no
/// operation identity or stable key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundaryOperationContract {
    pub parameters: Vec<BoundaryParameter>,
    pub return_value: BoundaryReturn,
    pub errors: BoundaryErrorContract,
    pub stream: BoundaryStreamContract,
    pub cancellation: BoundaryCancellationContract,
    pub callbacks: BoundaryCallbackContract,
    pub may_suspend: bool,
    pub effect_guarantee: BoundaryEffectGuarantee,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundaryOperationDescriptor {
    /// Real service-contract operation identity derived from its service id
    /// and stable public API key. Human-readable version labels are excluded.
    pub operation_id: ContractOperationId,
    pub stable_key: String,
    pub contract: BoundaryOperationContract,
}
