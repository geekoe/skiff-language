use skiff_artifact_model::{CallableEffectSummary, CallableMayEffects};

pub(crate) fn analyzed(effects: CallableMayEffects) -> CallableEffectSummary {
    CallableEffectSummary::Analyzed { effects }
}

pub(crate) fn bottom() -> CallableMayEffects {
    CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_pending: false,
        pending_effect_categories: Vec::new(),
        inout_path_effects: Vec::new(),
    }
}
