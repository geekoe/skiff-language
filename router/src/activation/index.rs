//! Mongo index contract for the activation repository (C-router-activation-state
//! §7): `activation_state.profile` unique, audit query key, and
//! profile+timestamp maintenance key.

use mongodb::{bson::doc, options::IndexOptions, IndexModel};

pub const ACTIVATION_STATE_PROFILE_INDEX: &str = "activation_state_profile_unique";
pub const ACTIVATION_AUDIT_QUERY_INDEX: &str = "activation_audit_query_key";
pub const ACTIVATION_AUDIT_MAINTENANCE_INDEX: &str = "activation_audit_maintenance";

pub fn activation_state_profile_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "state.profile": 1 })
        .options(
            IndexOptions::builder()
                .name(ACTIVATION_STATE_PROFILE_INDEX.to_string())
                .unique(true)
                .build(),
        )
        .build()
}

pub fn activation_audit_query_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! {
            "profile": 1,
            "activationId": 1,
            "operation": 1,
            "expectedGeneration": 1
        })
        .options(
            IndexOptions::builder()
                .name(ACTIVATION_AUDIT_QUERY_INDEX.to_string())
                .unique(true)
                .build(),
        )
        .build()
}

pub fn activation_audit_maintenance_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "profile": 1, "timestamp": 1 })
        .options(
            IndexOptions::builder()
                .name(ACTIVATION_AUDIT_MAINTENANCE_INDEX.to_string())
                .build(),
        )
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_index_is_unique_on_profile() {
        let index = activation_state_profile_index();
        let options = index.options.as_ref().expect("index options");
        assert_eq!(
            options.name.as_deref(),
            Some(ACTIVATION_STATE_PROFILE_INDEX)
        );
        assert_eq!(options.unique, Some(true));
        assert_eq!(
            index.keys.get("state.profile"),
            Some(&mongodb::bson::Bson::Int32(1))
        );
    }

    #[test]
    fn audit_query_index_covers_frozen_dedup_key() {
        let index = activation_audit_query_index();
        assert_eq!(index.keys.len(), 4);
        for key in ["profile", "activationId", "operation", "expectedGeneration"] {
            assert_eq!(
                index.keys.get(key),
                Some(&mongodb::bson::Bson::Int32(1)),
                "missing key {key}"
            );
        }
        assert_eq!(
            index.options.as_ref().and_then(|options| options.unique),
            Some(true)
        );
    }

    #[test]
    fn maintenance_index_is_non_unique_on_profile_timestamp() {
        let index = activation_audit_maintenance_index();
        assert_eq!(
            index.keys.get("profile"),
            Some(&mongodb::bson::Bson::Int32(1))
        );
        assert_eq!(
            index.keys.get("timestamp"),
            Some(&mongodb::bson::Bson::Int32(1))
        );
        assert_ne!(
            index.options.as_ref().and_then(|options| options.unique),
            Some(true)
        );
    }
}
