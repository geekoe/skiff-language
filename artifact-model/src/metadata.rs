use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MetadataValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<MetadataValue>),
    Object(BTreeMap<String, MetadataValue>),
}

impl MetadataValue {
    /// Convert an arbitrary JSON value into its `MetadataValue` mirror. Since
    /// `MetadataValue` is `#[serde(untagged)]` over the JSON shapes, this is a
    /// structural copy.
    pub fn from_json(value: serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => MetadataValue::Null,
            serde_json::Value::Bool(value) => MetadataValue::Bool(value),
            serde_json::Value::Number(value) => MetadataValue::Number(value),
            serde_json::Value::String(value) => MetadataValue::String(value),
            serde_json::Value::Array(items) => {
                MetadataValue::Array(items.into_iter().map(MetadataValue::from_json).collect())
            }
            serde_json::Value::Object(entries) => MetadataValue::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, MetadataValue::from_json(value)))
                    .collect(),
            ),
        }
    }

    /// Project a strongly-typed value into `MetadataValue` via its serialized
    /// JSON form. Used where metadata is built from typed IR rather than from a
    /// hand-rolled JSON blob.
    pub fn from_serializable<T: Serialize>(value: &T) -> Self {
        MetadataValue::from_json(
            serde_json::to_value(value).expect("metadata source value must serialize"),
        )
    }
}
