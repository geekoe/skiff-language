//! Router -> runtime service database configuration carried by the
//! `router.bootstrap` frame.
//!
//! The DTO survives the M4 activation-layer removal: it is the bootstrap
//! carrier of the runtime's service database endpoint, independent of any
//! coordination semantics.

use serde::{de, Deserialize, Deserializer, Serialize};

/// Service database configuration projected to the runtime at bootstrap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssemblyActivationServiceDb {
    #[serde(deserialize_with = "deserialize_non_empty_mongo_url")]
    pub mongo_url: String,
}

fn deserialize_non_empty_mongo_url<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.trim().is_empty() {
        return Err(de::Error::custom(
            "serviceDb.mongoUrl must be a non-empty string",
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_db_round_trips_and_rejects_empty_mongo_url() {
        let service_db = AssemblyActivationServiceDb {
            mongo_url: "mongodb://127.0.0.1:27017/skiff".to_string(),
        };
        let value = serde_json::to_value(&service_db).expect("serialize");
        let decoded: AssemblyActivationServiceDb = serde_json::from_value(value).expect("decode");
        assert_eq!(decoded, service_db);
        let invalid = serde_json::json!({ "mongoUrl": "  " });
        assert!(serde_json::from_value::<AssemblyActivationServiceDb>(invalid).is_err());
    }
}
