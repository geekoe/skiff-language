use std::collections::BTreeSet;

use skiff_artifact_model::{
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary,
    CallableProvenanceUnknownReason, ValueEscapeLane, ValueProvenance,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Origin {
    Fresh,
    Constant,
    CallerParameter(u32),
    DependencyReturn(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum EscapeLane {
    Capture,
    Callback,
    Stream,
    Spawn,
    Database,
    Native,
    External,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(super) struct AbstractValue {
    pub origins: BTreeSet<Origin>,
    /// Parameters whose reference graph remains reachable from this value.
    pub caller_references: BTreeSet<u32>,
    pub unknown: bool,
    pub reference: bool,
    /// Field-sensitive payloads owned by a typed `catch`.  A catch result is
    /// an owner-local container, but reading its success value after tag
    /// narrowing must recover the try expression's exact provenance instead
    /// of treating the whole tagged container as the field value.
    pub catch_result: Option<Box<CatchResultValue>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(super) struct CatchResultValue {
    pub success: AbstractValue,
    pub error: AbstractValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CallableState {
    pub effects: CallableMayEffects,
    pub return_origins: BTreeSet<Origin>,
    pub throw_origins: BTreeSet<Origin>,
    pub escape_lanes: BTreeSet<EscapeLane>,
    /// Formal parameters whose caller-owned graph makes an identity-sensitive
    /// operation observable. An empty set with the public effect set means the
    /// dependency could not be attributed and must remain conservative.
    pub same_heap_identity_parameters: BTreeSet<u32>,
    pub unknown: Option<CallableProvenanceUnknownReason>,
}

impl AbstractValue {
    pub fn constant(reference: bool) -> Self {
        Self {
            origins: BTreeSet::from([Origin::Constant]),
            caller_references: BTreeSet::new(),
            unknown: false,
            reference,
            catch_result: None,
        }
    }

    pub fn fresh(reference: bool) -> Self {
        Self {
            origins: BTreeSet::from([Origin::Fresh]),
            caller_references: BTreeSet::new(),
            unknown: false,
            reference,
            catch_result: None,
        }
    }

    pub fn parameter(index: u32, reference: bool) -> Self {
        Self {
            origins: BTreeSet::from([Origin::CallerParameter(index)]),
            caller_references: reference
                .then(|| BTreeSet::from([index]))
                .unwrap_or_default(),
            unknown: false,
            reference,
            catch_result: None,
        }
    }

    pub fn unknown(reference: bool) -> Self {
        Self {
            origins: BTreeSet::new(),
            caller_references: BTreeSet::new(),
            unknown: true,
            reference,
            catch_result: None,
        }
    }

    pub fn join(&mut self, other: &Self) {
        self.origins.extend(other.origins.iter().cloned());
        self.caller_references
            .extend(other.caller_references.iter().copied());
        self.unknown |= other.unknown;
        self.reference |= other.reference;
        match (&mut self.catch_result, &other.catch_result) {
            (Some(current), Some(other)) => {
                current.success.join(&other.success);
                current.error.join(&other.error);
            }
            (None, None) => {}
            _ => self.catch_result = None,
        }
    }

    pub fn with_fresh_container(mut self, reference: bool) -> Self {
        self.origins.insert(Origin::Fresh);
        self.reference = reference;
        self
    }

    pub fn catch_result(success: Self, reference: bool) -> Self {
        // The catch envelope and its typed exception branch are materialized
        // in the current heap.  Do not reuse success provenance for the error
        // branch; the callable's throw effects remain tracked separately.
        let error = Self::fresh(true);
        let mut container = success.clone().with_fresh_container(reference);
        container.catch_result = Some(Box::new(CatchResultValue { success, error }));
        container
    }

    pub fn catch_field(&self, field: &str, reference: bool) -> Option<Self> {
        let result = self.catch_result.as_ref()?;
        let mut value = match field {
            "tag" => Self::constant(false),
            "value" => result.success.clone(),
            "exception" => result.error.clone(),
            _ => return None,
        };
        value.reference = reference;
        if !reference {
            value.caller_references.clear();
        }
        Some(value)
    }

    pub fn contains_caller_reference(&self) -> bool {
        !self.caller_references.is_empty()
    }

    pub fn contains_caller_value(&self) -> bool {
        self.contains_caller_reference()
            || self
                .origins
                .iter()
                .any(|origin| matches!(origin, Origin::CallerParameter(_)))
    }
}

impl CallableState {
    pub fn bottom() -> Self {
        Self {
            effects: no_effects(),
            return_origins: BTreeSet::new(),
            throw_origins: BTreeSet::new(),
            escape_lanes: BTreeSet::new(),
            same_heap_identity_parameters: BTreeSet::new(),
            unknown: None,
        }
    }

    pub fn fail_closed(reason: CallableProvenanceUnknownReason) -> Self {
        Self {
            effects: all_effects(),
            return_origins: BTreeSet::new(),
            throw_origins: BTreeSet::new(),
            escape_lanes: BTreeSet::from([EscapeLane::External]),
            same_heap_identity_parameters: BTreeSet::new(),
            unknown: Some(reason),
        }
    }

    pub fn join(&mut self, other: &Self) -> bool {
        let before = self.clone();
        join_effects(&mut self.effects, &other.effects);
        self.return_origins
            .extend(other.return_origins.iter().cloned());
        self.throw_origins
            .extend(other.throw_origins.iter().cloned());
        self.escape_lanes.extend(other.escape_lanes.iter().copied());
        self.same_heap_identity_parameters
            .extend(other.same_heap_identity_parameters.iter().copied());
        self.unknown = join_unknown(self.unknown, other.unknown);
        *self != before
    }

    pub fn record_return(&mut self, value: &AbstractValue) {
        self.return_origins.extend(value.origins.iter().cloned());
        self.effects.returns_caller_alias |= value.contains_caller_reference();
        if value.unknown {
            self.mark_unknown_value_if_unowned();
            self.effects.returns_caller_alias = true;
        }
    }

    pub fn record_throw(&mut self, value: &AbstractValue) {
        self.throw_origins.extend(value.origins.iter().cloned());
        self.effects.throws_caller_alias |= value.contains_caller_reference();
        if value.unknown {
            self.mark_unknown_value_if_unowned();
            self.effects.throws_caller_alias = true;
        }
    }

    pub fn record_escape(&mut self, value: &AbstractValue, lane: EscapeLane) {
        if value.contains_caller_value() || value.unknown {
            self.effects.escapes_caller_value = true;
            self.escape_lanes.insert(lane);
        }
        if value.unknown {
            self.mark_unknown_value_if_unowned();
        }
    }

    pub fn record_persistent_escape(&mut self, value: &AbstractValue) {
        // Database writes materialize values. Scalar caller inputs and fresh
        // containers assembled from them are detached at this boundary; an
        // actual caller-owned reference graph must still be rejected.
        if value.contains_caller_reference() || value.unknown {
            self.effects.escapes_caller_value = true;
            self.escape_lanes.insert(EscapeLane::Database);
        }
        if value.unknown {
            self.mark_unknown_value_if_unowned();
        }
    }

    pub fn record_same_heap_identity(&mut self, value: &AbstractValue) {
        if value.contains_caller_reference() || value.unknown {
            self.effects.requires_same_heap_identity = true;
            self.same_heap_identity_parameters
                .extend(value.caller_references.iter().copied());
        }
    }

    pub fn mark_unknown(&mut self, reason: CallableProvenanceUnknownReason) {
        self.unknown = join_unknown(self.unknown, Some(reason));
    }

    fn mark_unknown_value_if_unowned(&mut self) {
        if self.unknown.is_none() {
            self.unknown = Some(CallableProvenanceUnknownReason::UnknownCallTarget);
        }
    }

    pub fn into_summaries(self) -> (CallableEffectSummary, CallableProvenanceSummary) {
        let provenance = match self.unknown {
            Some(reason) => CallableProvenanceSummary::Unknown { reason },
            None => CallableProvenanceSummary::Analyzed {
                return_origins: self
                    .return_origins
                    .into_iter()
                    .map(ValueProvenance::from)
                    .collect(),
                throw_origins: self
                    .throw_origins
                    .into_iter()
                    .map(ValueProvenance::from)
                    .collect(),
                escape_lanes: self
                    .escape_lanes
                    .into_iter()
                    .map(ValueEscapeLane::from)
                    .collect(),
            },
        };
        (
            CallableEffectSummary::Analyzed {
                effects: self.effects,
            },
            provenance,
        )
    }

    pub fn from_semantic_facts(
        effects: &CallableEffectSummary,
        provenance: &CallableProvenanceSummary,
    ) -> Self {
        let mut state = match effects {
            CallableEffectSummary::Analyzed { effects } => Self {
                effects: *effects,
                ..Self::bottom()
            },
            CallableEffectSummary::Unknown { .. } => {
                Self::fail_closed(CallableProvenanceUnknownReason::AnalysisPending)
            }
        };
        match provenance {
            CallableProvenanceSummary::Unknown { reason } => state.mark_unknown(*reason),
            CallableProvenanceSummary::Analyzed {
                return_origins,
                throw_origins,
                escape_lanes,
            } => {
                state.return_origins = return_origins.iter().cloned().map(Origin::from).collect();
                state.throw_origins = throw_origins.iter().cloned().map(Origin::from).collect();
                state.escape_lanes = escape_lanes.iter().copied().map(EscapeLane::from).collect();
            }
        }
        state
    }
}

pub(super) fn no_effects() -> CallableMayEffects {
    CallableMayEffects {
        writes_caller_reachable: false,
        returns_caller_alias: false,
        throws_caller_alias: false,
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_suspend: false,
    }
}

pub(super) fn all_effects() -> CallableMayEffects {
    CallableMayEffects {
        writes_caller_reachable: true,
        returns_caller_alias: true,
        throws_caller_alias: true,
        escapes_caller_value: true,
        requires_same_heap_identity: true,
        invokes_unknown_target: true,
        may_suspend: true,
    }
}

pub(super) fn join_effects(target: &mut CallableMayEffects, source: &CallableMayEffects) {
    target.writes_caller_reachable |= source.writes_caller_reachable;
    target.returns_caller_alias |= source.returns_caller_alias;
    target.throws_caller_alias |= source.throws_caller_alias;
    target.escapes_caller_value |= source.escapes_caller_value;
    target.requires_same_heap_identity |= source.requires_same_heap_identity;
    target.invokes_unknown_target |= source.invokes_unknown_target;
    target.may_suspend |= source.may_suspend;
}

fn join_unknown(
    left: Option<CallableProvenanceUnknownReason>,
    right: Option<CallableProvenanceUnknownReason>,
) -> Option<CallableProvenanceUnknownReason> {
    match (left, right) {
        (None, other) | (other, None) => other,
        (Some(left), Some(right)) => Some(if unknown_rank(left) >= unknown_rank(right) {
            left
        } else {
            right
        }),
    }
}

fn unknown_rank(reason: CallableProvenanceUnknownReason) -> u8 {
    match reason {
        CallableProvenanceUnknownReason::AnalysisPending => 0,
        CallableProvenanceUnknownReason::UnsupportedControlFlow => 1,
        CallableProvenanceUnknownReason::UnknownCallTarget => 2,
    }
}

impl From<Origin> for ValueProvenance {
    fn from(value: Origin) -> Self {
        match value {
            Origin::Fresh => Self::Fresh,
            Origin::Constant => Self::Constant,
            Origin::CallerParameter(index) => Self::CallerParameter { index },
            Origin::DependencyReturn(callable_id) => Self::DependencyReturn { callable_id },
        }
    }
}

impl From<ValueProvenance> for Origin {
    fn from(value: ValueProvenance) -> Self {
        match value {
            ValueProvenance::Fresh => Self::Fresh,
            ValueProvenance::Constant => Self::Constant,
            ValueProvenance::CallerParameter { index } => Self::CallerParameter(index),
            ValueProvenance::DependencyReturn { callable_id } => {
                Self::DependencyReturn(callable_id)
            }
        }
    }
}

impl From<EscapeLane> for ValueEscapeLane {
    fn from(value: EscapeLane) -> Self {
        match value {
            EscapeLane::Capture => Self::Capture,
            EscapeLane::Callback => Self::Callback,
            EscapeLane::Stream => Self::Stream,
            EscapeLane::Spawn => Self::Spawn,
            EscapeLane::Database => Self::Database,
            EscapeLane::Native => Self::Native,
            EscapeLane::External => Self::External,
        }
    }
}

impl From<ValueEscapeLane> for EscapeLane {
    fn from(value: ValueEscapeLane) -> Self {
        match value {
            ValueEscapeLane::Capture => Self::Capture,
            ValueEscapeLane::Callback => Self::Callback,
            ValueEscapeLane::Stream => Self::Stream,
            ValueEscapeLane::Spawn => Self::Spawn,
            ValueEscapeLane::Database => Self::Database,
            ValueEscapeLane::Native => Self::Native,
            ValueEscapeLane::External => Self::External,
        }
    }
}
