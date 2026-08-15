use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary,
    CallableProvenanceUnknownReason, PendingEffectCategory, ValueEscapeLane, ValueProjectionPath,
    ValueProjectionStep, ValueProvenance, MAX_VALUE_PROJECTION_PATH_STEPS,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Origin {
    Fresh,
    Constant,
    CallerParameter(u32),
    CallerParameterProjection {
        index: u32,
        path: ValueProjectionPath,
    },
    DependencyReturn(String),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct CallerReference {
    pub parameter: u32,
    pub path: Vec<ValueProjectionStep>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct FreshRoot {
    pub allocation: u32,
    pub path: Vec<ValueProjectionStep>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum EscapeLane {
    Capture,
    Callback,
    Stream,
    Dispatch,
    Database,
    Native,
    External,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(super) struct AbstractValue {
    /// Origins reachable from this value, including its direct root.
    pub origins: BTreeSet<Origin>,
    /// Origins that are possible identities of this value itself.
    pub direct_origins: BTreeSet<Origin>,
    /// Parameters whose reference graph remains reachable from this value.
    pub caller_references: BTreeSet<CallerReference>,
    /// Caller-owned roots that are possible identities of this value itself.
    /// This excludes caller values merely reachable through a fresh
    /// container's payload.
    pub direct_caller_references: BTreeSet<CallerReference>,
    pub unknown: bool,
    pub reference: bool,
    /// A direct `Fresh` summary branch that still needs a caller-site
    /// allocation identity.
    pub needs_fresh_root: bool,
    /// Evaluator-local allocation sites plus structural projections. Never
    /// serialized into an artifact.
    pub fresh_roots: BTreeSet<FreshRoot>,
    /// Fresh roots reachable anywhere in the value graph, including the
    /// direct roots above.
    pub fresh_references: BTreeSet<FreshRoot>,
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
    /// Direct-root subset of `return_origins`, serialized separately so
    /// dependency replay does not confuse a fresh wrapper with its payload.
    pub return_direct_origins: BTreeSet<Origin>,
    pub throw_origins: BTreeSet<Origin>,
    pub escape_lanes: BTreeSet<EscapeLane>,
    /// Formal parameters whose reachable graph is written. An empty set with
    /// the public write effect set is deliberately unscoped/fail-closed.
    pub write_parameters: BTreeSet<u32>,
    /// Formal parameters whose values enter each escape lane. A lane absent
    /// from this map remains deliberately unscoped/fail-closed.
    pub escape_parameters: BTreeMap<EscapeLane, BTreeSet<u32>>,
    /// Formal parameters whose caller-owned graph makes an identity-sensitive
    /// operation observable. An empty set with the public effect set means the
    /// dependency could not be attributed and must remain conservative.
    pub same_heap_identity_parameters: BTreeSet<u32>,
    /// Compiler-internal formal/projection store transfer.
    pub parameter_stores: BTreeMap<CallerReference, AbstractValue>,
    pub unknown: Option<CallableProvenanceUnknownReason>,
}

impl Origin {
    pub fn project(&self, suffix: &ValueProjectionPath) -> Result<Self, ()> {
        match self {
            Self::CallerParameter(index) => Ok(Self::CallerParameterProjection {
                index: *index,
                path: suffix.clone(),
            }),
            Self::CallerParameterProjection { index, path } => path
                .appended(suffix)
                .map(|path| Self::CallerParameterProjection {
                    index: *index,
                    path,
                })
                .map_err(|_| ()),
            other => Ok(other.clone()),
        }
    }
}

impl CallerReference {
    pub fn root(parameter: u32) -> Self {
        Self {
            parameter,
            path: Vec::new(),
        }
    }

    pub fn project(&self, suffix: &ValueProjectionPath) -> Result<Self, ()> {
        Ok(Self {
            parameter: self.parameter,
            path: append_projection_steps(&self.path, suffix)?,
        })
    }

    pub fn is_ancestor_of(&self, other: &Self) -> bool {
        self.parameter == other.parameter && other.path.starts_with(&self.path)
    }

    fn matches_origin(&self, origin: &Origin) -> bool {
        match origin {
            Origin::CallerParameter(index) => self.parameter == *index && self.path.is_empty(),
            Origin::CallerParameterProjection { index, path } => {
                self.parameter == *index && self.path == path.steps()
            }
            _ => false,
        }
    }
}

impl FreshRoot {
    pub fn allocation(allocation: u32) -> Self {
        Self {
            allocation,
            path: Vec::new(),
        }
    }

    pub fn project_step(&self, step: ValueProjectionStep) -> Result<Self, ()> {
        if self.path.len() >= MAX_VALUE_PROJECTION_PATH_STEPS {
            return Err(());
        }
        let mut path = self.path.clone();
        path.push(step);
        Ok(Self {
            allocation: self.allocation,
            path,
        })
    }

    pub fn is_ancestor_of(&self, other: &Self) -> bool {
        self.allocation == other.allocation && other.path.starts_with(&self.path)
    }
}

fn append_projection_steps(
    prefix: &[ValueProjectionStep],
    suffix: &ValueProjectionPath,
) -> Result<Vec<ValueProjectionStep>, ()> {
    if prefix.len().saturating_add(suffix.steps().len()) > MAX_VALUE_PROJECTION_PATH_STEPS {
        return Err(());
    }
    let mut path = Vec::with_capacity(prefix.len() + suffix.steps().len());
    path.extend(prefix.iter().cloned());
    path.extend(suffix.steps().iter().cloned());
    Ok(path)
}

impl AbstractValue {
    pub fn constant(reference: bool) -> Self {
        let origins = BTreeSet::from([Origin::Constant]);
        Self {
            direct_origins: origins.clone(),
            origins,
            caller_references: BTreeSet::new(),
            direct_caller_references: BTreeSet::new(),
            unknown: false,
            reference,
            needs_fresh_root: false,
            fresh_roots: BTreeSet::new(),
            fresh_references: BTreeSet::new(),
            catch_result: None,
        }
    }

    pub fn fresh(reference: bool) -> Self {
        let origins = BTreeSet::from([Origin::Fresh]);
        Self {
            direct_origins: origins.clone(),
            origins,
            caller_references: BTreeSet::new(),
            direct_caller_references: BTreeSet::new(),
            unknown: false,
            reference,
            needs_fresh_root: reference,
            fresh_roots: BTreeSet::new(),
            fresh_references: BTreeSet::new(),
            catch_result: None,
        }
    }

    pub fn parameter(index: u32, reference: bool) -> Self {
        let origins = BTreeSet::from([Origin::CallerParameter(index)]);
        Self {
            direct_origins: origins.clone(),
            origins,
            caller_references: if reference {
                BTreeSet::from([CallerReference::root(index)])
            } else {
                BTreeSet::new()
            },
            direct_caller_references: if reference {
                BTreeSet::from([CallerReference::root(index)])
            } else {
                BTreeSet::new()
            },
            unknown: false,
            reference,
            needs_fresh_root: false,
            fresh_roots: BTreeSet::new(),
            fresh_references: BTreeSet::new(),
            catch_result: None,
        }
    }

    pub fn unknown(reference: bool) -> Self {
        Self {
            origins: BTreeSet::new(),
            direct_origins: BTreeSet::new(),
            caller_references: BTreeSet::new(),
            direct_caller_references: BTreeSet::new(),
            unknown: true,
            reference,
            needs_fresh_root: false,
            fresh_roots: BTreeSet::new(),
            fresh_references: BTreeSet::new(),
            catch_result: None,
        }
    }

    pub fn join(&mut self, other: &Self) {
        self.origins.extend(other.origins.iter().cloned());
        self.direct_origins
            .extend(other.direct_origins.iter().cloned());
        self.caller_references
            .extend(other.caller_references.iter().cloned());
        self.direct_caller_references
            .extend(other.direct_caller_references.iter().cloned());
        self.unknown |= other.unknown;
        self.reference |= other.reference;
        self.needs_fresh_root |= other.needs_fresh_root;
        self.fresh_roots.extend(other.fresh_roots.iter().cloned());
        self.fresh_references
            .extend(other.fresh_references.iter().cloned());
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
        self.direct_origins = BTreeSet::from([Origin::Fresh]);
        self.direct_caller_references.clear();
        self.reference = reference;
        self.needs_fresh_root = reference;
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
            value.direct_caller_references.clear();
            value.fresh_roots.clear();
            value.fresh_references.clear();
            value.needs_fresh_root = false;
        }
        Some(value)
    }

    pub fn contains_caller_reference(&self) -> bool {
        !self.caller_references.is_empty()
    }

    pub fn contains_direct_caller_reference(&self) -> bool {
        !self.direct_caller_references.is_empty()
    }

    pub fn contains_caller_value(&self) -> bool {
        self.contains_caller_reference()
            || self.origins.iter().any(|origin| {
                matches!(
                    origin,
                    Origin::CallerParameter(_) | Origin::CallerParameterProjection { .. }
                )
            })
    }

    pub fn formal_parameters(&self) -> BTreeSet<u32> {
        let mut parameters = self
            .caller_references
            .iter()
            .map(|reference| reference.parameter)
            .collect::<BTreeSet<_>>();
        parameters.extend(self.origins.iter().filter_map(|origin| match origin {
            Origin::CallerParameter(index) | Origin::CallerParameterProjection { index, .. } => {
                Some(*index)
            }
            _ => None,
        }));
        parameters
    }

    pub fn project_direct_caller_parameter_origins(
        &mut self,
        path: &ValueProjectionPath,
        direct_references: &BTreeSet<CallerReference>,
    ) -> Result<(), ()> {
        self.origins =
            project_matching_origins(std::mem::take(&mut self.origins), path, direct_references)?;
        self.direct_origins = project_matching_origins(
            std::mem::take(&mut self.direct_origins),
            path,
            direct_references,
        )?;
        Ok(())
    }
}

fn project_matching_origins(
    origins: BTreeSet<Origin>,
    path: &ValueProjectionPath,
    direct_references: &BTreeSet<CallerReference>,
) -> Result<BTreeSet<Origin>, ()> {
    origins
        .into_iter()
        .map(|origin| {
            if direct_references
                .iter()
                .any(|reference| reference.matches_origin(&origin))
            {
                origin.project(path)
            } else {
                Ok(origin)
            }
        })
        .collect()
}

impl CallableState {
    pub fn bottom() -> Self {
        Self {
            effects: no_effects(),
            return_origins: BTreeSet::new(),
            return_direct_origins: BTreeSet::new(),
            throw_origins: BTreeSet::new(),
            escape_lanes: BTreeSet::new(),
            write_parameters: BTreeSet::new(),
            escape_parameters: BTreeMap::new(),
            same_heap_identity_parameters: BTreeSet::new(),
            parameter_stores: BTreeMap::new(),
            unknown: None,
        }
    }

    pub fn fail_closed(reason: CallableProvenanceUnknownReason) -> Self {
        Self {
            effects: all_effects(),
            return_origins: BTreeSet::new(),
            return_direct_origins: BTreeSet::new(),
            throw_origins: BTreeSet::new(),
            escape_lanes: BTreeSet::from([EscapeLane::External]),
            write_parameters: BTreeSet::new(),
            escape_parameters: BTreeMap::new(),
            same_heap_identity_parameters: BTreeSet::new(),
            parameter_stores: BTreeMap::new(),
            unknown: Some(reason),
        }
    }

    pub fn join(&mut self, other: &Self) -> bool {
        let before = self.clone();
        join_effects(&mut self.effects, &other.effects);
        self.return_origins
            .extend(other.return_origins.iter().cloned());
        self.return_direct_origins
            .extend(other.return_direct_origins.iter().cloned());
        self.throw_origins
            .extend(other.throw_origins.iter().cloned());
        self.escape_lanes.extend(other.escape_lanes.iter().copied());
        self.write_parameters
            .extend(other.write_parameters.iter().copied());
        for (lane, parameters) in &other.escape_parameters {
            self.escape_parameters
                .entry(*lane)
                .or_default()
                .extend(parameters.iter().copied());
        }
        self.same_heap_identity_parameters
            .extend(other.same_heap_identity_parameters.iter().copied());
        for (reference, value) in &other.parameter_stores {
            self.parameter_stores
                .entry(reference.clone())
                .and_modify(|current| current.join(value))
                .or_insert_with(|| value.clone());
        }
        self.unknown = join_unknown(self.unknown, other.unknown);
        *self != before
    }

    pub fn record_return(&mut self, value: &AbstractValue) {
        self.return_origins.extend(value.origins.iter().cloned());
        self.return_direct_origins
            .extend(value.direct_origins.iter().cloned());
        if value.unknown {
            self.mark_unknown_value_if_unowned();
            let fail_closed = Self::fail_closed(CallableProvenanceUnknownReason::UnknownCallTarget);
            join_effects(&mut self.effects, &fail_closed.effects);
            self.escape_lanes
                .extend(fail_closed.escape_lanes.iter().cloned());
        }
    }

    pub fn record_wire_detached_throw(&mut self, value: &AbstractValue) {
        // Runtime throw/rethrow crosses the canonical wire-value boundary
        // before a catch observes it. Operand evaluation has already
        // transferred its calls, writes, escapes, and suspension into this
        // state, but the emitted exception itself is rebuilt in the receiving
        // heap and therefore cannot retain caller alias identity.
        if value.unknown {
            self.join(&Self::fail_closed(
                CallableProvenanceUnknownReason::UnsupportedControlFlow,
            ));
            return;
        }
        self.throw_origins.insert(Origin::Fresh);
    }

    pub fn record_escape(&mut self, value: &AbstractValue, lane: EscapeLane) {
        if value.contains_caller_value() || value.unknown {
            self.effects.escapes_caller_value = true;
            self.escape_lanes.insert(lane);
            self.escape_parameters
                .entry(lane)
                .or_default()
                .extend(value.formal_parameters());
        }
        if value.unknown {
            self.mark_unknown_value_if_unowned();
        }
    }

    /// Detached boundaries materialize scalar caller values as new snapshots.
    /// Only actual caller-owned reference graphs retain an identity escape.
    pub fn record_detached_escape(&mut self, value: &AbstractValue, lane: EscapeLane) {
        if value.contains_caller_reference() || value.unknown {
            self.effects.escapes_caller_value = true;
            self.escape_lanes.insert(lane);
            self.escape_parameters
                .entry(lane)
                .or_default()
                .extend(value.formal_parameters());
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
            self.escape_parameters
                .entry(EscapeLane::Database)
                .or_default()
                .extend(value.formal_parameters());
        }
        if value.unknown {
            self.mark_unknown_value_if_unowned();
        }
    }

    pub fn record_same_heap_identity(&mut self, value: &AbstractValue) {
        if value.contains_direct_caller_reference() {
            self.effects.requires_same_heap_identity = true;
            self.same_heap_identity_parameters.extend(
                value
                    .direct_caller_references
                    .iter()
                    .map(|reference| reference.parameter),
            );
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
                return_origins: public_origins(self.return_origins),
                direct_return_origins: public_origins(self.return_direct_origins),
                throw_origins: public_origins(self.throw_origins),
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
                effects: effects.clone(),
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
                direct_return_origins,
                throw_origins,
                escape_lanes,
            } => {
                state.return_origins = return_origins.iter().cloned().map(Origin::from).collect();
                state.return_direct_origins = direct_return_origins
                    .iter()
                    .cloned()
                    .map(Origin::from)
                    .collect();
                state.throw_origins = throw_origins.iter().cloned().map(Origin::from).collect();
                state.escape_lanes = escape_lanes.iter().copied().map(EscapeLane::from).collect();
            }
        }
        state
    }
}

fn public_origins(origins: BTreeSet<Origin>) -> Vec<ValueProvenance> {
    let mut public = Vec::new();
    for origin in origins {
        let origin = ValueProvenance::from(origin);
        if !public.contains(&origin) {
            public.push(origin);
        }
    }
    public
}

pub(super) fn no_effects() -> CallableMayEffects {
    CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_pending: false,
        pending_effect_categories: Vec::new(),
        inout_path_effects: Vec::new(),
    }
}

pub(super) fn all_effects() -> CallableMayEffects {
    // Unknown analysis must remain boundary-ineligible, but the identity bit is
    // an observed-operation fact rather than a generic fail-closed marker.
    // invokes_unknown_target and the other conservative effects carry the
    // rejection without claiming that an identity comparison occurred.
    CallableMayEffects {
        escapes_caller_value: true,
        requires_same_heap_identity: false,
        invokes_unknown_target: true,
        may_pending: true,
        pending_effect_categories: vec![PendingEffectCategory::Unknown],
        inout_path_effects: Vec::new(),
    }
}

pub(super) fn join_effects(target: &mut CallableMayEffects, source: &CallableMayEffects) {
    target.escapes_caller_value |= source.escapes_caller_value;
    target.requires_same_heap_identity |= source.requires_same_heap_identity;
    target.invokes_unknown_target |= source.invokes_unknown_target;
    target.may_pending |= source.may_pending;
    union_categories(
        &mut target.pending_effect_categories,
        &source.pending_effect_categories,
    );
    target
        .inout_path_effects
        .extend(source.inout_path_effects.iter().cloned());
    target.inout_path_effects.sort();
    target.inout_path_effects.dedup();
}

/// Records that the callable may park/suspend for the given pending category.
/// Sets `may_pending` and appends the category once (deterministic order).
pub(super) fn record_pending_category(
    effects: &mut CallableMayEffects,
    category: PendingEffectCategory,
) {
    union_pending_categories(effects, &[category]);
}

/// Joins pending categories into the target, setting `may_pending` whenever
/// any category is recorded.
pub(super) fn union_pending_categories(
    effects: &mut CallableMayEffects,
    categories: &[PendingEffectCategory],
) {
    if categories.is_empty() {
        return;
    }
    effects.may_pending = true;
    union_categories(&mut effects.pending_effect_categories, categories);
}

fn union_categories(target: &mut Vec<PendingEffectCategory>, source: &[PendingEffectCategory]) {
    for category in source {
        if !target.contains(category) {
            target.push(*category);
        }
    }
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
        CallableProvenanceUnknownReason::UnsupportedHeapStore => 1,
        CallableProvenanceUnknownReason::UnknownCallTarget => 2,
    }
}

impl From<Origin> for ValueProvenance {
    fn from(value: Origin) -> Self {
        match value {
            Origin::Fresh => Self::Fresh,
            Origin::Constant => Self::Constant,
            Origin::CallerParameter(index) => Self::CallerParameter { index },
            Origin::CallerParameterProjection { index, path } => {
                Self::CallerParameterProjection { index, path }
            }
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
            ValueProvenance::CallerParameterProjection { index, path } => {
                Self::CallerParameterProjection { index, path }
            }
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
            EscapeLane::Dispatch => Self::Dispatch,
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
            ValueEscapeLane::Dispatch => Self::Dispatch,
            ValueEscapeLane::Database => Self::Database,
            ValueEscapeLane::Native => Self::Native,
            ValueEscapeLane::External => Self::External,
        }
    }
}
