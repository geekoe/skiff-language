use skiff_artifact_model::ValueEscapeLane;

pub(in crate::package_artifact) const fn escape_lane_rank(lane: ValueEscapeLane) -> u8 {
    match lane {
        ValueEscapeLane::Capture => 0,
        ValueEscapeLane::Callback => 1,
        ValueEscapeLane::Stream => 2,
        ValueEscapeLane::Dispatch => 3,
        ValueEscapeLane::Database => 4,
        ValueEscapeLane::Native => 5,
        ValueEscapeLane::External => 6,
    }
}
