use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::metadata::MetadataValue;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeTarget {
    pub namespace: String,
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_key: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, MetadataValue>,
}
