use super::{super::*, support::*};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicationIdFixture {
    schema_version: u32,
    encoding: String,
    max_bytes: usize,
    valid: Vec<PublicationIdCase>,
    invalid: Vec<InvalidPublicationIdCase>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicationIdCase {
    canonical_id: String,
    applies_to: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct InvalidPublicationIdCase {
    applies_to: Vec<String>,
}

fn runtime_publication_id_fixture() -> PublicationIdFixture {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("runtime crate should live under the skiff repository root")
        .join("cross-system-fixtures/publication-id-cases.json");
    let text = std::fs::read_to_string(&path).expect("publication id fixture should be readable");
    let fixture: PublicationIdFixture =
        serde_json::from_str(&text).expect("publication id fixture should parse");
    assert_eq!(fixture.schema_version, 1);
    assert_eq!(fixture.encoding, "url-like-with-storage-safe-projection");
    assert_eq!(fixture.max_bytes, 63);
    assert_publication_id_fixture_applies_to(&fixture);
    fixture
}

fn assert_publication_id_fixture_applies_to(fixture: &PublicationIdFixture) {
    for applies_to in fixture
        .valid
        .iter()
        .map(|case| &case.applies_to)
        .chain(fixture.invalid.iter().map(|case| &case.applies_to))
    {
        assert!(applies_to.len() >= 2);
        for (index, system) in applies_to.iter().enumerate() {
            assert!(
                matches!(system.as_str(), "compiler" | "runtime" | "router"),
                "invalid appliesTo system {system:?}"
            );
            assert!(
                !applies_to[..index].contains(system),
                "repeated appliesTo system {system:?}"
            );
        }
    }
}

#[test]
fn service_db_runtime_derives_storage_identity_from_profile_and_service_id() {
    let fixture = runtime_publication_id_fixture();
    for case in fixture
        .valid
        .iter()
        .filter(|case| case.applies_to.iter().any(|system| system == "runtime"))
    {
        let runtime = ServiceDbRuntime::new(
            test_profile(),
            case.canonical_id.clone(),
            "mongodb://127.0.0.1:27017".to_string(),
            &[],
        )
        .expect("service DB runtime should derive a storage-safe database name");

        assert_eq!(
            runtime.database_name,
            case.canonical_id.replace('.', "~").replace('/', "~~")
        );
    }

    let exact = ServiceDbRuntime::new(
        "dev".to_string(),
        "example.com/service".to_string(),
        inert_mongo_url("exact-storage-identity"),
        &[],
    )
    .expect("exact service DB identity should build");
    assert_eq!(exact.database_name, "example~com~~service");
}

#[test]
fn package_collection_storage_name_is_stable_bounded_and_mongo_safe() {
    let first =
        service_storage_collection_name("example.com/provider", "internal.events.TrackEvent")
            .expect("physical collection name");
    let repeated =
        service_storage_collection_name("example.com/provider", "internal.events.TrackEvent")
            .expect("physical collection name");

    assert_eq!(first, repeated);
    assert!(
        first.starts_with("internal.events.TrackEvent_"),
        "physical collection name should keep the db object type's readable identity: {first}"
    );
    assert!(
        first.len() <= 45,
        "physical collection name should stay bounded for the historical Mongo namespace budget: {first}"
    );
    assert!(!first.contains('\0') && !first.contains('$'));
    assert!(
        format!("{}.{}", "d".repeat(63), first).len() < 120,
        "system encoding should fit the strict historical Mongo namespace boundary"
    );
}

#[test]
fn package_collection_storage_name_isolates_packages_and_logical_collections() {
    let first_package = service_storage_collection_name("example.com/first", "Session")
        .expect("physical collection name");
    let second_package = service_storage_collection_name("example.com/second", "Session")
        .expect("physical collection name");
    let second_collection = service_storage_collection_name("example.com/first", "Audit")
        .expect("physical collection name");

    assert_ne!(first_package, second_package);
    assert_ne!(first_package, second_collection);
}

#[test]
fn package_collection_storage_name_digest_prevents_sanitized_identity_collisions() {
    // Two logical identities that sanitize to the same readable segment must
    // still map to distinct physical collections because the digest covers
    // the exact declared identity and the Package ID.
    let slash = service_storage_collection_name("example.com/provider", "events/track")
        .expect("physical collection name");
    let question = service_storage_collection_name("example.com/provider", "events?track")
        .expect("physical collection name");
    let other_package = service_storage_collection_name("example.com/other", "events/track")
        .expect("physical collection name");

    assert_eq!(
        slash.rsplit_once('_').map(|(head, _)| head),
        question.rsplit_once('_').map(|(head, _)| head),
        "readable segments sanitize identically"
    );
    assert_ne!(slash, question);
    assert_ne!(slash, other_package);
}

#[test]
fn package_collection_storage_name_is_diamond_path_independent() {
    let direct = service_storage_collection_name("example.com/provider", "Session")
        .expect("direct dependency collection");
    let transitive = service_storage_collection_name("example.com/provider", "Session")
        .expect("transitive dependency collection");

    assert_eq!(direct, transitive);
}

#[test]
fn service_db_runtime_profile_does_not_change_database_name() {
    let config = ServiceDbConfig {
        mongo_url: inert_mongo_url("profile"),
        encryption_cipher: None,
    };
    let service_id = service_id("stateful");
    let first = ServiceDbRuntime::new_with_config(
        "dev".to_string(),
        service_id.clone(),
        config.clone(),
        &[],
    )
    .expect("dev service database");
    let second = ServiceDbRuntime::new_with_config("prod".to_string(), service_id, config, &[])
        .expect("prod service database");
    assert_eq!(
        first.database_name, second.database_name,
        "profile must not participate in the Mongo database name"
    );
}

#[test]
fn service_db_runtime_storage_identity_is_stable_within_each_mongo_endpoint_domain() {
    let service_id = service_id("stable");
    let first = ServiceDbRuntime::new(
        "dev".to_string(),
        service_id.clone(),
        inert_mongo_url("endpoint-a"),
        &[],
    )
    .expect("first endpoint service database");
    let second = ServiceDbRuntime::new(
        "dev".to_string(),
        service_id,
        inert_mongo_url("endpoint-b"),
        &[],
    )
    .expect("second endpoint service database");

    assert_eq!(first.database_name, second.database_name);
    assert!(!Arc::ptr_eq(&first.client, &second.client));
}

#[test]
fn service_db_runtime_rejects_unvalidated_profile() {
    let error = ServiceDbRuntime::new(
        "../prod".to_string(),
        service_id("invalid-profile"),
        inert_mongo_url("invalid-profile"),
        &[],
    )
    .err()
    .expect("invalid profile must fail before provider construction");

    assert!(error.to_string().contains("profile"));
}

#[test]
fn service_db_runtime_rejects_mongo_unsafe_database_names() {
    for service_id in [
        "",
        "std",
        "skiff.run/std$",
        "skiff.run/std value",
        "skiff~run~~std",
        "admin",
        "local",
        "config",
    ] {
        let error = ServiceDbRuntime::new(
            test_profile(),
            service_id.to_string(),
            "mongodb://127.0.0.1:27017".to_string(),
            &[],
        )
        .err()
        .expect("unsafe service database name should be rejected");

        assert!(
            error.to_string().contains("service id"),
            "{service_id}: {error}"
        );
    }
}

#[tokio::test]
async fn service_db_runtime_reuses_client_cell_for_exact_mongo_url() {
    let mongo_url = inert_mongo_url("shared_cell");
    let first = ServiceDbRuntime::new(
        test_profile(),
        service_id("shared_a"),
        mongo_url.clone(),
        &[],
    )
    .expect("first service DB runtime should build");
    let second = ServiceDbRuntime::new(test_profile(), service_id("shared_b"), mongo_url, &[])
        .expect("second service DB runtime should build");

    assert!(
        Arc::ptr_eq(&first.client, &second.client),
        "same exact mongoUrl should share the Mongo client cell"
    );
    assert!(
        first.client.get().is_none(),
        "shared cell should still initialize lazily"
    );

    let _first_client = first
        .client()
        .await
        .expect("inert Mongo URL should still build a client handle");
    assert!(
        second.client.get().is_some(),
        "initializing one runtime should initialize the shared cell for the other"
    );
    let _second_client = second
        .client()
        .await
        .expect("second runtime should clone the shared client handle");
}

#[test]
fn service_db_runtime_does_not_share_client_cell_for_different_mongo_urls() {
    let first = ServiceDbRuntime::new(
        test_profile(),
        service_id("distinct_a"),
        inert_mongo_url("distinct_a"),
        &[],
    )
    .expect("first service DB runtime should build");
    let second = ServiceDbRuntime::new(
        test_profile(),
        service_id("distinct_b"),
        inert_mongo_url("distinct_b"),
        &[],
    )
    .expect("second service DB runtime should build");

    assert!(
        !Arc::ptr_eq(&first.client, &second.client),
        "different exact mongoUrl values should not share the Mongo client cell"
    );
}

#[test]
fn service_db_client_cache_drops_dead_cells_and_urls() {
    let stale_url = inert_mongo_url("drop_stale");
    let stale_cell = {
        let first = ServiceDbRuntime::new(
            test_profile(),
            service_id("drop_a"),
            stale_url.clone(),
            &[],
        )
        .expect("first service DB runtime should build");
        let second = ServiceDbRuntime::new(
            test_profile(),
            service_id("drop_b"),
            stale_url.clone(),
            &[],
        )
        .expect("second service DB runtime should build");

        assert!(
            Arc::ptr_eq(&first.client, &second.client),
            "same mongoUrl should share the cell while runtimes are live"
        );
        Arc::downgrade(&first.client)
    };
    assert!(
        stale_cell.upgrade().is_none(),
        "global cache must not keep dropped runtime cells alive"
    );

    let live_url = inert_mongo_url("drop_live");
    let live_runtime = ServiceDbRuntime::new(
        test_profile(),
        service_id("drop_live"),
        live_url.clone(),
        &[],
    )
    .expect("live service DB runtime should build");

    let cells = SERVICE_DB_CLIENT_CELLS
        .get()
        .expect("service DB client cache should be initialized");
    let cells = cells
        .lock()
        .expect("service DB client cache lock should not be poisoned");
    assert!(
        !cells.contains_key(&stale_url),
        "accessing the cache should remove dead URL entries"
    );
    let live_cell = cells
        .get(&live_url)
        .and_then(std::sync::Weak::upgrade)
        .expect("live runtime cell should remain upgradeable in the cache");
    assert!(
        Arc::ptr_eq(&live_runtime.client, &live_cell),
        "global cache should point at the live runtime cell"
    );
}

#[test]
fn service_db_runtime_keeps_database_name_and_metadata_isolated_when_client_cell_is_shared() {
    let mongo_url = inert_mongo_url("isolated_runtime");
    let account = ServiceDbRuntime::new(
        test_profile(),
        service_id("account"),
        mongo_url.clone(),
        &provider_metadata_from_ir(object_metadata_for_type("AccountOnly")),
    )
    .expect("account service DB runtime should build");
    let registry = ServiceDbRuntime::new(
        test_profile(),
        service_id("registry"),
        mongo_url,
        &provider_metadata_from_ir(object_metadata_for_type("RegistryOnly")),
    )
    .expect("registry service DB runtime should build");

    assert!(
        Arc::ptr_eq(&account.client, &registry.client),
        "same mongoUrl should share only the Mongo client cell"
    );
    assert_ne!(
        account.database_name, registry.database_name,
        "different service ids must keep separate Mongo database names"
    );
    let account_target = test_db_target(0, "", "AccountOnly");
    let registry_target = test_db_target(0, "", "RegistryOnly");
    account
        .metadata
        .collection_for_target(&account_target)
        .expect("account metadata should remain on account runtime");
    assert!(
        account
            .metadata
            .collection_for_target(&registry_target)
            .is_err(),
        "registry metadata must not leak into account runtime"
    );
    registry
        .metadata
        .collection_for_target(&registry_target)
        .expect("registry metadata should remain on registry runtime");
    assert!(
        registry
            .metadata
            .collection_for_target(&account_target)
            .is_err(),
        "account metadata must not leak into registry runtime"
    );
}

#[tokio::test]
async fn service_db_client_cache_does_not_store_failed_initialization() {
    let invalid_url = "http://127.0.0.1:1".to_string();
    let first = ServiceDbRuntime::new(
        test_profile(),
        service_id("invalid_a"),
        invalid_url.clone(),
        &[],
    )
    .expect("first service DB runtime should build before connecting");
    let second = ServiceDbRuntime::new(
        test_profile(),
        service_id("invalid_b"),
        invalid_url,
        &[],
    )
    .expect("second service DB runtime should build before connecting");

    assert!(
        Arc::ptr_eq(&first.client, &second.client),
        "same invalid mongoUrl should still address the same retryable cell"
    );
    first
        .client()
        .await
        .expect_err("invalid Mongo URL should fail client initialization");
    assert!(
        first.client.get().is_none(),
        "failed initialization must not fill the shared cell"
    );
    second
        .client()
        .await
        .expect_err("same invalid Mongo URL should retry and fail again");
    assert!(
        second.client.get().is_none(),
        "failed retry must not permanently poison the shared cell"
    );
}

#[tokio::test]
async fn service_db_client_options_disable_retryable_writes_by_default() {
    let options =
        service_db_client_options("mongodb://127.0.0.1:8500/?directConnection=true&appName=skiff")
            .await
            .expect("service DB Mongo options should parse");

    assert_eq!(options.retry_writes, Some(false));
    assert_eq!(options.direct_connection, Some(true));
    assert_eq!(options.app_name.as_deref(), Some("skiff"));
}

#[tokio::test]
async fn service_db_client_options_ignore_replica_set_for_direct_connection() {
    let options = service_db_client_options(
        "mongodb://127.0.0.1:8500/?directConnection=true&replicaSet=rs0&retryWrites=false",
    )
    .await
    .expect("service DB Mongo options should parse");

    assert_eq!(options.direct_connection, Some(true));
    assert_eq!(options.repl_set_name, None);
    assert_eq!(options.retry_writes, Some(false));
}

#[tokio::test]
async fn service_db_client_options_override_retryable_writes() {
    let options =
        service_db_client_options("mongodb://127.0.0.1:8500/?retryWrites=true&w=majority")
            .await
            .expect("service DB Mongo options should parse");

    assert_eq!(options.retry_writes, Some(false));
    assert!(options.write_concern.is_some());
}
