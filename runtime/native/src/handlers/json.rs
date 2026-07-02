use serde_json::Value;

use crate::error::{Result, RuntimeError};

pub(crate) fn json_codec_requires_runtime_dispatch(_args: &[Value]) -> Result<Value> {
    Err(RuntimeError::Unsupported(
        "std.json encode/decode requires typed runtime dispatch".to_string(),
    ))
}

/// Shallow object overlay merge for `std.json.merge(base, overlay)`.
///
/// When both arguments are JSON objects, overlay's top-level keys override base's
/// same-named keys, base-only keys are kept, and overlay-only keys are appended.
/// Overlay null values are kept as null (they do not delete the key). When overlay
/// is not an object it replaces base entirely, except null overlay which returns
/// base unchanged. Field order follows insertion order: existing base keys keep
/// their position while overlay-only keys are appended.
pub(crate) fn json_merge(args: &[Value]) -> Result<Value> {
    let base = args.first().ok_or_else(|| {
        RuntimeError::Decode("std.json.merge requires a base argument".to_string())
    })?;
    let overlay = args.get(1).ok_or_else(|| {
        RuntimeError::Decode("std.json.merge requires an overlay argument".to_string())
    })?;

    match (base, overlay) {
        (Value::Object(base_object), Value::Object(overlay_object)) => {
            let mut merged = base_object.clone();
            for (key, value) in overlay_object {
                merged.insert(key.clone(), value.clone());
            }
            Ok(Value::Object(merged))
        }
        (_, Value::Null) => Ok(base.clone()),
        (_, _) => Ok(overlay.clone()),
    }
}
