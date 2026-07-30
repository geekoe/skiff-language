use mongodb::{
    bson::doc,
    options::{Collation, IndexOptions},
    IndexModel,
};

use super::{
    canonical_managed_index_model, classify_existing_indexes, managed_index_name,
    merge_collection_plan, CollectionIndexPlan, DatabaseIndexPlan, ManagedIndexSpec,
    ServiceDbIndexProvisionPlan, MANAGED_INDEX_PREFIX,
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
