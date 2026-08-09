mod contract;
mod registry;

pub use contract::{
    NativeResourceDropPlan, NativeValueAdapterRole, NativeValueArgumentPolicy, NativeValueDropPlan,
    NativeValueEmbedding, NativeValueLifecycleAdapter, NativeValueLifecycleConcrete,
    NativeValueLifecycleEntry, NativeValueLifecycleKind, NativeValueLifecycleLookupError,
    NativeValueLifecycleRegistryError, NativeValueLifecycleRegistryIdentity,
    NativeValueLifecycleResolution, NativeValueLifecycleTemplate, NativeValueTypeConstructor,
    NativeValueTypePattern,
};
pub use registry::{
    native_value_lifecycle_registry, native_value_lifecycle_registry_identity,
    NativeValueLifecycleRegistry, MAX_NATIVE_VALUE_LIFECYCLE_ARGUMENTS,
    NATIVE_VALUE_LIFECYCLE_REGISTRY, NATIVE_VALUE_LIFECYCLE_REGISTRY_FINGERPRINT,
    NATIVE_VALUE_LIFECYCLE_REGISTRY_ID, NATIVE_VALUE_LIFECYCLE_REGISTRY_VERSION,
};

#[cfg(test)]
mod tests;
