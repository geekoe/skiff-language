mod contract;
mod registry;

pub use contract::{
    HostEffectMetadataMatcher, HostEffectMetadataShape, HostEffectReceiverSemantics,
    HostEffectRegistryBuildError, HostEffectRegistryEntry, HostEffectRegistryIdentity,
    HostEffectRegistryMatch, HostEffectRegistryMatchError, HostEffectRequiredContext,
};
pub use registry::{
    host_effect_registry, host_effect_registry_identity, HostEffectRegistry, HOST_EFFECT_REGISTRY,
    HOST_EFFECT_REGISTRY_FINGERPRINT, HOST_EFFECT_REGISTRY_ID, HOST_EFFECT_REGISTRY_VERSION,
};

#[cfg(test)]
mod tests;
