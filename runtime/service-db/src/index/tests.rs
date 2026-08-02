use std::{
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use futures_util::FutureExt;
use mongodb::{
    bson::{doc, Document},
    options::{Collation, IndexOptions},
    Collection, IndexModel,
};
use tokio::sync::Notify;

use super::{
    canonical_managed_index_model, classify_existing_indexes, managed_index_name,
    merge_collection_plan, reconcile_collection, reconcile_database, reconcile_databases_bounded,
    CollectionIndexPlan, DatabaseIndexPlan, ManagedIndexSpec, ServiceDbIndexProvisionPlan,
    MANAGED_INDEX_PREFIX,
};

fn spec(name: &str, unique: bool) -> ManagedIndexSpec {
    ManagedIndexSpec {
        name: name.to_string(),
        keys: vec![
            ("profile.email".to_string(), 1),
            ("createdAt".to_string(), -1),
        ],
        unique,
    }
}

fn collection(indexes: Vec<ManagedIndexSpec>) -> CollectionIndexPlan {
    CollectionIndexPlan {
        package_id: "example.test/package".to_string(),
        logical_collection: "users".to_string(),
        physical_collection: "physical-users".to_string(),
        indexes: indexes
            .into_iter()
            .map(|index| (index.name.clone(), index))
            .collect(),
    }
}

fn live_collection(
    physical_collection: &str,
    logical_collection: &str,
    indexes: Vec<ManagedIndexSpec>,
) -> CollectionIndexPlan {
    CollectionIndexPlan {
        package_id: "example.test/package".to_string(),
        logical_collection: logical_collection.to_string(),
        physical_collection: physical_collection.to_string(),
        indexes: indexes
            .into_iter()
            .map(|index| (index.name.clone(), index))
            .collect(),
    }
}

fn live_database(
    mongo_url: &str,
    database_name: &str,
    collections: Vec<CollectionIndexPlan>,
) -> DatabaseIndexPlan {
    DatabaseIndexPlan {
        mongo_url: mongo_url.to_string(),
        database_name: database_name.to_string(),
        collections: collections
            .into_iter()
            .map(|collection| (collection.physical_collection.clone(), collection))
            .collect(),
    }
}

async fn assert_live_collection_exact(
    collection: &Collection<Document>,
    plan: &CollectionIndexPlan,
) {
    let existing = super::list_indexes_or_empty(collection)
        .await
        .expect("list live managed indexes");
    let missing = classify_existing_indexes(&existing, plan).expect("live managed index spec");
    assert!(missing.is_empty(), "live managed indexes must be complete");
}

struct LiveFutureDropProbe(Arc<AtomicBool>);

impl Drop for LiveFutureDropProbe {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn model(spec: &ManagedIndexSpec) -> IndexModel {
    spec.mongo_model()
}

#[test]
fn managed_name_is_stable_and_namespaced_by_logical_owner() {
    let first = managed_index_name("example.test/package", "users", "byEmail");
    assert!(first.starts_with(MANAGED_INDEX_PREFIX));
    assert_eq!(
        first,
        managed_index_name("example.test/package", "users", "byEmail")
    );
    assert_ne!(
        first,
        managed_index_name("example.test/package", "accounts", "byEmail")
    );
}

#[test]
fn migration_builder_emits_the_same_canonical_name_keys_and_options() {
    let expected = spec(
        &managed_index_name("example.test/package", "users", "byEmail"),
        true,
    );
    let model = canonical_managed_index_model(
        "example.test/package",
        "users",
        "byEmail",
        expected.keys.clone(),
        true,
    )
    .expect("canonical migration model");

    assert_eq!(
        model
            .options
            .as_ref()
            .and_then(|options| options.name.as_deref()),
        Some(expected.name.as_str())
    );
    assert_eq!(
        model.options.as_ref().and_then(|options| options.unique),
        Some(true)
    );
    assert_eq!(
        model
            .options
            .as_ref()
            .and_then(|options| options.collation.as_ref())
            .map(|collation| collation.locale.as_str()),
        Some("simple")
    );
    assert_eq!(model.keys, doc! { "profile.email": 1, "createdAt": -1 });
}

#[test]
fn missing_managed_indexes_are_returned_in_deterministic_order() {
    let first = spec("skiff_midx_v1_a", false);
    let second = spec("skiff_midx_v1_b", true);
    let plan = collection(vec![second.clone(), first.clone()]);
    let missing = classify_existing_indexes(&[], &plan).expect("empty catalog is additive");
    assert_eq!(
        missing.into_iter().collect::<Vec<_>>(),
        vec![first.name, second.name]
    );
}

#[test]
fn exact_managed_and_unmanaged_indexes_pass() {
    let expected = spec("skiff_midx_v1_exact", true);
    let unmanaged = IndexModel::builder()
        .keys(doc! { "legacy": 1 })
        .options(
            IndexOptions::builder()
                .name("operator_owned".to_string())
                .unique(false)
                .build(),
        )
        .build();
    let plan = collection(vec![expected.clone()]);
    let missing = classify_existing_indexes(&[unmanaged, model(&expected)], &plan)
        .expect("unmanaged indexes are preserved and exact managed indexes pass");
    assert!(missing.is_empty());
}

#[test]
fn managed_drift_and_stale_indexes_fail_closed() {
    let expected = spec("skiff_midx_v1_exact", true);
    let plan = collection(vec![expected.clone()]);
    let mut drift = model(&expected);
    drift.options.as_mut().expect("options").unique = Some(false);
    assert!(classify_existing_indexes(&[drift], &plan)
        .expect_err("managed drift must fail")
        .to_string()
        .contains("differs"));

    let stale = spec("skiff_midx_v1_stale", false);
    assert!(classify_existing_indexes(&[model(&stale)], &plan)
        .expect_err("stale managed index must fail")
        .to_string()
        .contains("stale"));
}

#[test]
fn non_simple_collation_is_managed_drift() {
    let expected = spec("skiff_midx_v1_exact", false);
    let plan = collection(vec![expected.clone()]);
    let mut drift = model(&expected);
    drift.options.as_mut().expect("options").collation =
        Some(Collation::builder().locale("en").build());
    assert!(classify_existing_indexes(&[drift], &plan).is_err());
}

#[test]
fn multi_version_collection_plans_union_compatible_indexes() {
    let mut database = DatabaseIndexPlan {
        mongo_url: "mongodb://example.invalid".to_string(),
        database_name: "test".to_string(),
        collections: Default::default(),
    };
    let first = spec("skiff_midx_v1_first", false);
    let second = spec("skiff_midx_v1_second", true);
    merge_collection_plan(&mut database, collection(vec![first.clone()]))
        .expect("first version should establish collection plan");
    merge_collection_plan(&mut database, collection(vec![first, second.clone()]))
        .expect("later version may add a compatible index");
    assert!(database
        .collections
        .get("physical-users")
        .expect("collection")
        .indexes
        .contains_key(&second.name));
}

#[test]
fn multi_version_conflicting_managed_identity_fails_before_io() {
    let mut database = DatabaseIndexPlan {
        mongo_url: "mongodb://example.invalid".to_string(),
        database_name: "test".to_string(),
        collections: Default::default(),
    };
    let first = spec("skiff_midx_v1_same", false);
    let mut conflicting = first.clone();
    conflicting.unique = true;
    merge_collection_plan(&mut database, collection(vec![first]))
        .expect("first version should establish collection plan");
    assert!(
        merge_collection_plan(&mut database, collection(vec![conflicting]))
            .expect_err("same managed identity with a different spec must fail")
            .to_string()
            .contains("conflicting definitions")
    );
}

#[tokio::test]
#[ignore = "requires SKIFF_TEST_MONGO_URL and a live MongoDB server"]
async fn mongo_live_reconciles_missing_exact_unmanaged_and_concurrent_indexes() {
    let mongo_url =
        std::env::var("SKIFF_TEST_MONGO_URL").expect("SKIFF_TEST_MONGO_URL must be set");
    let database_name = format!("skiff_index_live_{}", uuid::Uuid::new_v4().simple());
    let expected = spec("skiff_midx_v1_live", false);
    let collection_plan = collection(vec![expected.clone()]);
    let mut databases = std::collections::BTreeMap::new();
    databases.insert(
        (mongo_url.clone(), database_name.clone()),
        DatabaseIndexPlan {
            mongo_url: mongo_url.clone(),
            database_name: database_name.clone(),
            collections: [(
                collection_plan.physical_collection.clone(),
                collection_plan.clone(),
            )]
            .into_iter()
            .collect(),
        },
    );
    let plan = ServiceDbIndexProvisionPlan { databases };

    let client_options = crate::service_db_client_options(&mongo_url)
        .await
        .expect("Mongo options");
    let client = mongodb::Client::with_options(client_options).expect("Mongo client");
    let collection = client
        .database(&database_name)
        .collection::<mongodb::bson::Document>(&collection_plan.physical_collection);
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! { "operator": 1 })
                .options(
                    IndexOptions::builder()
                        .name("operator_owned".to_string())
                        .build(),
                )
                .build(),
        )
        .await
        .expect("unmanaged index");

    let (left, right) = tokio::join!(plan.reconcile(), plan.reconcile());
    left.expect("first replica reconcile");
    right.expect("concurrent replica reconcile");
    plan.reconcile().await.expect("exact reconcile");
    let names = collection
        .list_index_names()
        .await
        .expect("list index names");
    assert!(names.iter().any(|name| name == "operator_owned"));
    assert!(names.iter().any(|name| name == &expected.name));

    client
        .database(&database_name)
        .drop()
        .await
        .expect("drop temporary live-test database");
}

#[tokio::test]
#[ignore = "requires SKIFF_TEST_MONGO_URL and a live MongoDB server"]
async fn mongo_live_unique_duplicate_is_sanitized_and_nonretryable() {
    let mongo_url =
        std::env::var("SKIFF_TEST_MONGO_URL").expect("SKIFF_TEST_MONGO_URL must be set");
    let database_name = format!("skiff_index_dup_live_{}", uuid::Uuid::new_v4().simple());
    let mut expected = spec("skiff_midx_v1_unique_live", true);
    expected.keys = vec![("email".to_string(), 1)];
    let collection_plan = collection(vec![expected]);
    let mut databases = std::collections::BTreeMap::new();
    databases.insert(
        (mongo_url.clone(), database_name.clone()),
        DatabaseIndexPlan {
            mongo_url: mongo_url.clone(),
            database_name: database_name.clone(),
            collections: [(
                collection_plan.physical_collection.clone(),
                collection_plan.clone(),
            )]
            .into_iter()
            .collect(),
        },
    );
    let plan = ServiceDbIndexProvisionPlan { databases };
    let client_options = crate::service_db_client_options(&mongo_url)
        .await
        .expect("Mongo options");
    let client = mongodb::Client::with_options(client_options).expect("Mongo client");
    let collection = client
        .database(&database_name)
        .collection::<mongodb::bson::Document>(&collection_plan.physical_collection);
    collection
        .insert_many([doc! { "email": "same" }, doc! { "email": "same" }])
        .await
        .expect("duplicate historical rows");

    let error = plan
        .reconcile()
        .await
        .expect_err("unique index creation over duplicate rows must fail");
    assert_eq!(
        error.to_string(),
        "service database unique index cannot be provisioned because existing records violate the declared constraint"
    );

    client
        .database(&database_name)
        .drop()
        .await
        .expect("drop temporary live-test database");
}

#[tokio::test]
#[ignore = "requires SKIFF_TEST_MONGO_URL and a live MongoDB server"]
async fn mongo_live_cross_database_partial_persistence_cancellation_and_exact_rerun() {
    let mongo_url =
        std::env::var("SKIFF_TEST_MONGO_URL").expect("SKIFF_TEST_MONGO_URL must be set");
    let test_id = uuid::Uuid::new_v4().simple().to_string();
    let sibling_database_name = format!("skiff_index_cancel_sibling_{test_id}");
    let failure_database_name = format!("skiff_index_cancel_failure_{test_id}");

    let mut persisted_spec = spec("skiff_midx_v1_cancel_persisted", false);
    persisted_spec.keys = vec![("persisted".to_string(), 1)];
    let persisted_collection = live_collection(
        "a-persisted",
        "cancel-persisted",
        vec![persisted_spec.clone()],
    );
    let mut after_cancel_spec = spec("skiff_midx_v1_cancel_after", false);
    after_cancel_spec.keys = vec![("afterCancel".to_string(), -1)];
    let after_cancel_collection = live_collection(
        "b-after-cancel",
        "cancel-after",
        vec![after_cancel_spec.clone()],
    );
    let mut failure_spec = spec("skiff_midx_v1_cancel_failure", true);
    failure_spec.keys = vec![("email".to_string(), 1)];
    let failure_collection = live_collection(
        "unique-failure",
        "cancel-failure",
        vec![failure_spec.clone()],
    );

    let sibling_database = live_database(
        &mongo_url,
        &sibling_database_name,
        vec![
            persisted_collection.clone(),
            after_cancel_collection.clone(),
        ],
    );
    let failure_database = live_database(
        &mongo_url,
        &failure_database_name,
        vec![failure_collection.clone()],
    );
    let plan = ServiceDbIndexProvisionPlan {
        databases: [
            (
                (mongo_url.clone(), sibling_database_name.clone()),
                sibling_database,
            ),
            (
                (mongo_url.clone(), failure_database_name.clone()),
                failure_database,
            ),
        ]
        .into_iter()
        .collect(),
    };

    let client_options = crate::service_db_client_options(&mongo_url)
        .await
        .expect("Mongo options");
    let client = mongodb::Client::with_options(client_options).expect("Mongo client");
    let outcome = AssertUnwindSafe(async {
        let failure_mongo_collection = client
            .database(&failure_database_name)
            .collection::<Document>(&failure_collection.physical_collection);
        failure_mongo_collection
            .insert_many([
                doc! { "_id": 1, "email": "same" },
                doc! { "_id": 2, "email": "same" },
            ])
            .await
            .expect("duplicate rows for cross-database failure");

        let sibling_partial_persisted = Arc::new(Notify::new());
        let sibling_future_dropped = Arc::new(AtomicBool::new(false));
        let first_result = reconcile_databases_bounded(plan.databases.values(), 2, {
            let client = client.clone();
            let sibling_partial_persisted = Arc::clone(&sibling_partial_persisted);
            let sibling_future_dropped = Arc::clone(&sibling_future_dropped);
            let sibling_database_name = sibling_database_name.clone();
            move |database| {
                let client = client.clone();
                let sibling_partial_persisted = Arc::clone(&sibling_partial_persisted);
                let sibling_future_dropped = Arc::clone(&sibling_future_dropped);
                let sibling_database_name = sibling_database_name.clone();
                async move {
                    if database.database_name == sibling_database_name {
                        let persisted = database
                            .collections
                            .get("a-persisted")
                            .expect("first sibling collection");
                        let _drop_probe = LiveFutureDropProbe(sibling_future_dropped);
                        reconcile_collection(
                            client
                                .database(&database.database_name)
                                .collection::<Document>(&persisted.physical_collection),
                            persisted,
                        )
                        .await?;
                        sibling_partial_persisted.notify_one();
                        std::future::pending().await
                    } else {
                        sibling_partial_persisted.notified().await;
                        reconcile_database(database).await
                    }
                }
            }
        })
        .await
        .expect_err("real sibling failure must cancel its pending peer");
        assert_eq!(first_result.to_string(), super::UNIQUE_PROVISION_FAILURE);
        assert!(
            sibling_future_dropped.load(Ordering::SeqCst),
            "fail-fast database reconciliation must drop the pending sibling future"
        );

        let sibling_mongo_database = client.database(&sibling_database_name);
        assert_live_collection_exact(
            &sibling_mongo_database
                .collection::<Document>(&persisted_collection.physical_collection),
            &persisted_collection,
        )
        .await;
        let after_cancel_names = sibling_mongo_database
            .collection::<Document>(&after_cancel_collection.physical_collection)
            .list_index_names()
            .await
            .unwrap_or_default();
        assert!(
            !after_cancel_names
                .iter()
                .any(|name| name == &after_cancel_spec.name),
            "the sibling future must be cancelled before its second collection persists"
        );

        failure_mongo_collection
            .delete_one(doc! { "_id": 2 })
            .await
            .expect("repair duplicate data");
        plan.reconcile().await.expect("first exact plan rerun");
        plan.reconcile().await.expect("second exact plan rerun");

        assert_live_collection_exact(
            &sibling_mongo_database
                .collection::<Document>(&persisted_collection.physical_collection),
            &persisted_collection,
        )
        .await;
        assert_live_collection_exact(
            &sibling_mongo_database
                .collection::<Document>(&after_cancel_collection.physical_collection),
            &after_cancel_collection,
        )
        .await;
        assert_live_collection_exact(&failure_mongo_collection, &failure_collection).await;
    })
    .catch_unwind()
    .await;

    let sibling_mongo_database = client.database(&sibling_database_name);
    let failure_mongo_database = client.database(&failure_database_name);
    let (sibling_cleanup, failure_cleanup) =
        tokio::join!(sibling_mongo_database.drop(), failure_mongo_database.drop());
    sibling_cleanup.expect("drop temporary sibling live-test database");
    failure_cleanup.expect("drop temporary failure live-test database");
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
#[ignore = "requires SKIFF_TEST_MONGO_URL and a live MongoDB server"]
async fn mongo_live_same_database_partial_unique_failure_repair_and_exact_rerun() {
    let mongo_url =
        std::env::var("SKIFF_TEST_MONGO_URL").expect("SKIFF_TEST_MONGO_URL must be set");
    let database_name = format!(
        "skiff_index_partial_unique_{}",
        uuid::Uuid::new_v4().simple()
    );

    let mut success_spec = spec("skiff_midx_v1_partial_success", false);
    success_spec.keys = vec![("createdAt".to_string(), -1)];
    let success_collection =
        live_collection("a-success", "partial-success", vec![success_spec.clone()]);
    let mut unique_spec = spec("skiff_midx_v1_partial_unique", true);
    unique_spec.keys = vec![("email".to_string(), 1)];
    let unique_collection =
        live_collection("b-unique", "partial-unique", vec![unique_spec.clone()]);
    let database = live_database(
        &mongo_url,
        &database_name,
        vec![success_collection.clone(), unique_collection.clone()],
    );
    let plan = ServiceDbIndexProvisionPlan {
        databases: [((mongo_url.clone(), database_name.clone()), database)]
            .into_iter()
            .collect(),
    };

    let client_options = crate::service_db_client_options(&mongo_url)
        .await
        .expect("Mongo options");
    let client = mongodb::Client::with_options(client_options).expect("Mongo client");
    let outcome = AssertUnwindSafe(async {
        let mongo_database = client.database(&database_name);
        let success_mongo_collection =
            mongo_database.collection::<Document>(&success_collection.physical_collection);
        let unique_mongo_collection =
            mongo_database.collection::<Document>(&unique_collection.physical_collection);
        unique_mongo_collection
            .insert_many([
                doc! { "_id": 1, "email": "same" },
                doc! { "_id": 2, "email": "same" },
            ])
            .await
            .expect("duplicate historical rows");

        let first_error = plan
            .reconcile()
            .await
            .expect_err("ordered unique index creation must fail after the first collection");
        assert_eq!(first_error.to_string(), super::UNIQUE_PROVISION_FAILURE);
        assert_live_collection_exact(&success_mongo_collection, &success_collection).await;
        let unique_names = unique_mongo_collection
            .list_index_names()
            .await
            .expect("list failed unique collection indexes");
        assert!(
            !unique_names.iter().any(|name| name == &unique_spec.name),
            "failed unique index must not be reported as persisted"
        );

        unique_mongo_collection
            .delete_one(doc! { "_id": 2 })
            .await
            .expect("repair duplicate historical row");
        plan.reconcile().await.expect("first repaired exact rerun");
        plan.reconcile().await.expect("second repaired exact rerun");
        assert_live_collection_exact(&success_mongo_collection, &success_collection).await;
        assert_live_collection_exact(&unique_mongo_collection, &unique_collection).await;
    })
    .catch_unwind()
    .await;

    client
        .database(&database_name)
        .drop()
        .await
        .expect("drop temporary partial-unique live-test database");
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
