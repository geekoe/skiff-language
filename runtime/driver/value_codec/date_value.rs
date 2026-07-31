// B1a temporary adapter for legacy value_codec imports.
//
// Owner: B1.
// Deletion/narrowing point: after date boundary users import
// `skiff_runtime_boundary::date_value` directly.
#[allow(unused_imports)]
pub(crate) use skiff_runtime_boundary::date_value::{
    format_epoch_millis, is_valid_epoch_millis, parse_rfc3339_millis, try_parse_rfc3339_millis,
    validate_epoch_millis, MAX_EPOCH_MILLIS, MIN_EPOCH_MILLIS,
};

#[cfg(test)]
mod tests;
