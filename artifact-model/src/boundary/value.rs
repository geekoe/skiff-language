use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BoundaryValuePlan {
    Linkable {
        carrier: BoundaryValueCarrier,
        encoding: BoundaryValueEncoding,
        owner: BoundaryValueOwner,
        lifetime: BoundaryValueLifetime,
    },
    Unsupported {
        reason: BoundaryValuePlanUnavailableReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BoundaryValueCarrier {
    DetachedValueGraph,
    CallbackCapability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BoundaryValueEncoding {
    CanonicalValue,
    OpaqueCapability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BoundaryValueOwner {
    Caller,
    Provider,
    CapabilityOwner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BoundaryValueLifetime {
    Call,
    Request,
    Stream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BoundaryValuePlanUnavailableReason {
    LanguageUnsupported,
    NativeAdapterRequired,
    CallbackAdapterRequired,
    UnknownType,
}
