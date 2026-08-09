use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
use skiff_runtime_loader::HydratedDeploymentBytecode;

/// Exact hydration-backed resolver port for value-lifecycle policy calls.
///
/// The trait implementation is intentionally left to the P2 resolver owner.
pub(super) struct HydratedValueLifecycleResolver<'a> {
    _hydrated: &'a HydratedDeploymentBytecode,
    _candidate: &'a LinkedBytecodeCandidate,
}

impl<'a> HydratedValueLifecycleResolver<'a> {
    pub(super) const fn new(
        hydrated: &'a HydratedDeploymentBytecode,
        candidate: &'a LinkedBytecodeCandidate,
    ) -> Self {
        Self {
            _hydrated: hydrated,
            _candidate: candidate,
        }
    }
}
