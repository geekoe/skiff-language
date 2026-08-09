mod contract;
mod registry;

pub use contract::{
    IntrinsicPublicReturnType, IntrinsicReceiverSemantics, IntrinsicRegistryEntry,
    IntrinsicRegistryIdentity, IntrinsicRegistryMatch, IntrinsicRegistryMatchError,
};
pub use registry::{
    intrinsic_registry, intrinsic_registry_identity, IntrinsicRegistry, INTRINSIC_REGISTRY,
    INTRINSIC_REGISTRY_FINGERPRINT, INTRINSIC_REGISTRY_ID, INTRINSIC_REGISTRY_VERSION,
    UNSUPPORTED_INTRINSIC_RECEIVER_KEYS,
};

#[cfg(test)]
mod tests;
