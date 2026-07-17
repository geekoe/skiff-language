use skiff_artifact_model::{
    CallableProvenanceSummary, CallableSemanticFacts, ValueEscapeLane, ValueProvenance,
};

pub(super) fn normalize_semantic_facts(mut facts: CallableSemanticFacts) -> CallableSemanticFacts {
    if let CallableProvenanceSummary::Analyzed {
        return_origins,
        throw_origins,
        escape_lanes,
    } = &mut facts.provenance
    {
        return_origins.sort_by_key(provenance_sort_key);
        return_origins.dedup();
        throw_origins.sort_by_key(provenance_sort_key);
        throw_origins.dedup();
        escape_lanes.sort_by_key(|lane| escape_lane_rank(*lane));
        escape_lanes.dedup();
    }
    facts
}

fn provenance_sort_key(origin: &ValueProvenance) -> (u8, String) {
    match origin {
        ValueProvenance::Fresh => (0, String::new()),
        ValueProvenance::Constant => (1, String::new()),
        ValueProvenance::CallerParameter { index } => (2, format!("{index:010}")),
        ValueProvenance::DependencyReturn { callable_id } => (3, callable_id.clone()),
    }
}

fn escape_lane_rank(lane: ValueEscapeLane) -> u8 {
    match lane {
        ValueEscapeLane::Capture => 0,
        ValueEscapeLane::Callback => 1,
        ValueEscapeLane::Stream => 2,
        ValueEscapeLane::Spawn => 3,
        ValueEscapeLane::Database => 4,
        ValueEscapeLane::Native => 5,
        ValueEscapeLane::External => 6,
    }
}
