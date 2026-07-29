mod assembly_seam;
mod capability;
mod context;
mod request_context;

pub use assembly_seam::{RuntimeAssemblyActivationSeamError, RuntimeAssemblyActivationTemplate};
pub use capability::{
    CallbackCapabilityError, CallbackCapabilityPayload, CallbackCapabilityTable,
    CALLBACK_CAPABILITY_TOMBSTONE_LIMIT,
};
pub use context::{
    ActivationContext, ActivationContextError, ActivationId, ActivationIdentity,
    ActivationOwnedBindings, ActivationServiceBinding,
};
pub use request_context::{CallbackLifetime, RequestActivationContext, RequestStreamLease};

#[cfg(test)]
mod tests;
