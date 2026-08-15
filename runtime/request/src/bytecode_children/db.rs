//! X6 registration for the D6R recoverable/DB child composition.
//!
//! D6R owns the prepared capability and recoverable codec facts. X6 only owns
//! request child registration: the concrete VM DB child/effect target and the
//! transaction token remain with F6/K6 until their J2 central seams land.

use skiff_runtime_capability_context::{DbCapabilityContext, DbRecoverableRuntimeContext};

/// Request-scoped DB child registration.
///
/// The fields are intentionally optional. A missing context fails closed
/// before any provider call; this module never fabricates a DB target or a
/// transaction authority.
#[derive(Clone, Default)]
pub struct BytecodeDbChildComposition {
    pub capability_context: Option<DbCapabilityContext>,
    pub recoverable_context: Option<DbRecoverableRuntimeContext>,
}

impl BytecodeDbChildComposition {
    #[cfg(test)]
    pub(crate) fn is_available(&self) -> bool {
        self.capability_context.is_some() && self.recoverable_context.is_some()
    }
}

#[cfg(test)]
pub(crate) fn db_child_required_fact() -> String {
    "F6 must emit a VM DB child/effect carrying exact DbObjectTargetId and result plan; \
     K6 must add the VM DB child/effect target and own the exact transaction token/pending cleanup; \
     X6 will then consume D6R DbCapabilityStoreApi::prepare_*_runtime, DbRuntimeFinalizer and \
     DbRecoverableRuntimeContext through the same flat child lifecycle"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_child_composition_defaults_to_fail_closed() {
        let composition = BytecodeDbChildComposition::default();
        assert!(
            !composition.is_available(),
            "an unconfigured DB child must never reach a provider"
        );
    }

    #[test]
    fn db_child_required_fact_names_f6_k6_and_d6r_seams() {
        let required = db_child_required_fact();
        assert!(
            required.contains("F6"),
            "missing F6 requirement: {required}"
        );
        assert!(
            required.contains("K6"),
            "missing K6 requirement: {required}"
        );
        assert!(
            required.contains("DbObjectTargetId"),
            "missing exact DB target identity: {required}"
        );
        assert!(
            required.contains("DbRecoverableRuntimeContext"),
            "missing D6R capability context: {required}"
        );
    }
}
