use super::{super::*, support::*};

#[test]
fn mongo_provider_builds_db_capability_source_from_valid_opaque_config() {
    let source = MongoServiceDbProviderFactory::default()
        .build(provider_input(json!({
            "mongoUrl": inert_mongo_url("provider-valid")
        })))
        .expect("valid provider config should build DB capability source");
    let context = source.context_for_request("owner", "request");

    context
        .require_store("std.db.findOne", "serviceDb is required")
        .expect("provider-built source should create a DB store");
}

#[test]
fn mongo_provider_rejects_invalid_opaque_config() {
    for (config, expected) in [
        (
            Value::Null,
            "serviceDb provider config must be a JSON object",
        ),
        (
            json!({}),
            "serviceDb provider config field mongoUrl is required",
        ),
        (
            json!({ "mongoUrl": 42 }),
            "serviceDb provider config field mongoUrl must be a string",
        ),
        (
            json!({ "mongoUrl": "" }),
            "serviceDb provider config field mongoUrl must be a non-empty string",
        ),
        (
            json!({
                "mongoUrl": inert_mongo_url("provider-unknown"),
                "retryWrites": false
            }),
            "serviceDb provider config field retryWrites is not supported",
        ),
    ] {
        let error = match MongoServiceDbProviderFactory::default().build(provider_input(config)) {
            Ok(_) => panic!("invalid provider config should fail"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains(expected),
            "expected {expected:?}, got {error}"
        );
    }
}

#[tokio::test]
async fn mongo_provider_provision_with_no_db_metadata_performs_no_config_or_storage_io() {
    MongoServiceDbProviderFactory::default()
        .provision(vec![provider_input(json!({
            "mongoUrl": "not even a valid Mongo URL"
        }))])
        .await
        .expect("no DB metadata must make provisioning inert");
}

fn provider_input(config: Value) -> DbProviderBuildInput {
    DbProviderBuildInput {
        environment: "test".to_string(),
        service_id: service_id("provider"),
        config: DbProviderConfig::opaque(config),
        runtime_program_db: Vec::new(),
    }
}
