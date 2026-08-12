use skiff_artifact_model::CallableEffectSummary;
use skiff_runtime_linked_bytecode::LinkedFunction;

use crate::bytecode::{BytecodeLinkError, BytecodeLinkObligation};

use super::{unsatisfied, DeploymentLinker};

impl DeploymentLinker<'_> {
    /// Rejects source-owned capability facts after the exact reachable closure
    /// has been linked and before any executable candidate can be published.
    ///
    /// This gate deliberately walks only `functions`: raw private artifact
    /// functions that are unreachable from the canonical publication-root
    /// union never become linked functions and therefore cannot poison the
    /// deployment. Conversely, one unsupported fact in any reachable function
    /// rejects the complete deployment candidate.
    pub(super) fn admit_phase_1_capabilities(
        &self,
        functions: &[LinkedFunction],
    ) -> Result<(), BytecodeLinkError> {
        for function in functions {
            let (package, source) = self.source_function(function.key())?;
            let location = self.function_location(package, source);
            let effects = match function.declarative_effect_summary() {
                CallableEffectSummary::Unknown { .. } => {
                    return Err(unsatisfied(
                        BytecodeLinkObligation::CallableEffectPlan,
                        location,
                        "Phase 1 capability gate rejected an unknown callable effect summary"
                            .to_string(),
                    ));
                }
                CallableEffectSummary::Analyzed { effects } => effects,
            };

            if let Some(category) = effects.pending_effect_categories.first() {
                return Err(unsatisfied(
                    BytecodeLinkObligation::CallableEffectPlan,
                    location,
                    format!(
                        "Phase 1 capability gate rejected reachable pending effect {category:?}"
                    ),
                ));
            }
            if effects.may_pending {
                return Err(unsatisfied(
                    BytecodeLinkObligation::CallableEffectPlan,
                    location,
                    "Phase 1 capability gate rejected reachable pending effect without an exact category"
                        .to_string(),
                ));
            }
            if !effects.inout_path_effects.is_empty() {
                return Err(unsatisfied(
                    BytecodeLinkObligation::CallableEffectPlan,
                    location,
                    "Phase 1 capability gate rejected reachable InOut effects".to_string(),
                ));
            }
            if effects.escapes_caller_value
                || effects.requires_same_heap_identity
                || effects.invokes_unknown_target
            {
                return Err(unsatisfied(
                    BytecodeLinkObligation::CallableEffectPlan,
                    location,
                    "Phase 1 capability gate rejected reachable non-scalar effects".to_string(),
                ));
            }
        }
        Ok(())
    }
}
