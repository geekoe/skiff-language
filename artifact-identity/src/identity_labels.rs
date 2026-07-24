use serde::Serialize;
use serde_json::Value;

use crate::{ArtifactIdentityError, Result};

const HUMAN_VERSION_LABEL_KEYS: &[&str] = &["packageVersion", "contractVersion", "exactVersion"];

pub(crate) fn without_human_version_labels<T: Serialize>(
    value: &T,
    error: fn(serde_json::Error) -> ArtifactIdentityError,
) -> Result<Value> {
    let mut value = serde_json::to_value(value).map_err(error)?;
    remove_human_version_labels(&mut value);
    Ok(value)
}

fn remove_human_version_labels(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                remove_human_version_labels(value);
            }
        }
        Value::Object(fields) => {
            for key in HUMAN_VERSION_LABEL_KEYS {
                fields.remove(*key);
            }
            for value in fields.values_mut() {
                remove_human_version_labels(value);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}
