mod classify;
mod contract;
mod identity;
mod native;
mod normalize;
mod schema;
mod traversal;
mod verify;

pub use classify::{classify_value_lifecycle, normalize_value_lifecycle_type};
pub use contract::{
    PositionalTypeEnvironment, ResolvedPackageValueType, ValueLifecycleFactResolver,
    ValueLifecyclePolicyBudget, ValueLifecyclePolicyError, ValueLifecyclePolicyIdentity,
    ValueLifecycleResolverError,
};
pub use identity::{
    value_lifecycle_policy_identity, VALUE_LIFECYCLE_POLICY_FINGERPRINT,
    VALUE_LIFECYCLE_POLICY_VERSION,
};
pub use verify::verify_value_transfer_plan;

#[cfg(test)]
mod tests;
