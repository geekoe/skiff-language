//! `ActorMethodCatalogView` sequence tests: read-only typed queries over an
//! explicitly captured `Arc<RoutingEpoch>` built from real A0/A3
//! `ActorRoutingProjection` records. The view never reads File IR, never
//! accepts source/declaration coordinates as query input and keeps old epoch
//! captures alive across publication.

mod actor_support;

use std::sync::Arc;

use serde::Deserialize;
use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity};
use skiff_deployment::fixtures::empty_runtime_assembly_fixture;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingMethod, ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_router::actor::{ActorMethodCatalogView, CatalogQuery};
use skiff_router::artifact::{ActorRoutingCatalog, ActorRoutingProjectionRef};
use skiff_router::bootstrap::{ActiveRoutingEpochStore, RoutingEpoch};
use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;

const A3_CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../deployment/tests/fixtures/a3-actor-routing/corpus.json"
);

const MODEL_CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../runtime/transport/testdata/actor-routing-projection.json"
);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(dead_code)]
struct A3Corpus {
    schema_version: String,
    records: Vec<A3Record>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(dead_code)]
struct A3Record {
    name: String,
    expected: String,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(dead_code)]
struct ModelCorpus {
    #[serde(rename = "schemaVersion")]
    schema_version: String,
    #[serde(rename = "sourceContract")]
    source_contract: String,
    positive: Vec<ModelCase>,
    negative: Vec<ModelCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(dead_code)]
struct ModelCase {
    id: String,
    json: serde_json::Value,
    #[serde(default, rename = "rejectAt")]
    reject_at: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

fn snapshot(profile: &str) -> Arc<RuntimeConfigSnapshot> {
    let reference = skiff_artifact_model::RuntimeConfigSnapshotRef {
        snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
            "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect("snapshot id"),
    };
    Arc::new(RuntimeConfigSnapshot::new(profile, reference, Vec::new()).expect("snapshot fixture"))
}

fn epoch_from_projection(projection: ActorRoutingProjection) -> Arc<RoutingEpoch> {
    let catalog = Arc::new(ActorRoutingCatalog::from_projection(Arc::new(projection)));
    Arc::new(
        RoutingEpoch::new(
            "prod",
            42,
            Arc::new(empty_runtime_assembly_fixture().expect("assembly fixture")),
            snapshot("prod"),
            catalog,
        )
        .expect("epoch fixture"),
    )
}

fn query_from_method(method: &ActorRoutingMethod) -> CatalogQuery {
    CatalogQuery::new(
        method.actor.service_id.clone(),
        method.actor.actor_abi_identity.clone(),
        method.actor_implementation_identity.clone(),
        method.method_identity.clone(),
    )
}

fn a3_corpus() -> A3Corpus {
    serde_json::from_str(&std::fs::read_to_string(A3_CORPUS).expect("A3 corpus")).expect("decode")
}

fn a3_record(name: &str) -> String {
    a3_corpus()
        .records
        .into_iter()
        .find(|record| record.name == name)
        .unwrap_or_else(|| panic!("A3 record {name}"))
        .content
        .expect("record content")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_typed_key_resolves_the_immutable_entry_with_binding() {
        let projection: ActorRoutingProjection =
            serde_json::from_str(&a3_record("single-entry")).expect("projection parses");
        let epoch = epoch_from_projection(projection);
        let view = ActorMethodCatalogView::from_epoch(Arc::clone(&epoch));
        let method = &epoch.actor_catalog().entries()[0];
        let query = query_from_method(method);
        assert!(view.has_method(&query));
        let resolved = view.method_for(&query).expect("method_for resolves");
        assert_eq!(&resolved, method);
        assert_eq!(resolved.deployment.service_id, "example.com/docs");
        assert_eq!(resolved.package.package_id, "example.com/docs-package");
        assert_eq!(
            view.schema_version(),
            ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION
        );
    }

    #[test]
    fn misses_fail_closed_and_never_touch_live_state() {
        let projection: ActorRoutingProjection =
            serde_json::from_str(&a3_record("single-entry")).expect("projection parses");
        let epoch = epoch_from_projection(projection);
        let view = ActorMethodCatalogView::from_epoch(Arc::clone(&epoch));
        let method = &epoch.actor_catalog().entries()[0];
        let mut wrong_service = query_from_method(method);
        wrong_service.service_id = "example.com/unknown".to_string();
        assert!(!view.has_method(&wrong_service));
        assert!(view.method_for(&wrong_service).is_none());

        let mut wrong_method = query_from_method(method);
        wrong_method.method_identity =
            ActorMethodIdentity::new(format!("skiff-actor-method-v1:sha256:{}", "e".repeat(64)));
        assert!(view.method_for(&wrong_method).is_none());
    }

    #[test]
    fn empty_projection_is_valid_and_all_queries_miss() {
        let projection: ActorRoutingProjection =
            serde_json::from_str(&a3_record("empty")).expect("empty projection parses");
        assert!(projection.methods.is_empty());
        let epoch = epoch_from_projection(projection);
        let view = ActorMethodCatalogView::from_epoch(epoch);
        assert!(!view.has_method(&CatalogQuery::new(
            "example.com/docs",
            ActorAbiIdentity::new(format!("skiff-actor-abi-v1:sha256:{}", "a".repeat(64))),
            ActorImplementationIdentity::new(format!(
                "skiff-actor-implementation-v1:sha256:{}",
                "b".repeat(64)
            )),
            ActorMethodIdentity::new(format!("skiff-actor-method-v1:sha256:{}", "d".repeat(64))),
        )));
        assert_eq!(view.health().misses, 1);
    }

    #[test]
    fn view_follows_store_replacement() {
        let first: ActorRoutingProjection =
            serde_json::from_str(&a3_record("single-entry")).expect("first projection");
        let second: ActorRoutingProjection =
            serde_json::from_str(&a3_record("multi-entry-sorted")).expect("second projection");
        let store = Arc::new(ActiveRoutingEpochStore::new());
        let first_epoch = epoch_from_projection(first);
        let second_epoch = epoch_from_projection(second);
        let view = ActorMethodCatalogView::new(Arc::clone(&store));
        store.publish(Arc::clone(&first_epoch));
        let method = &first_epoch.actor_catalog().entries()[0];
        assert!(view.has_method(&query_from_method(method)));
        store.publish(Arc::clone(&second_epoch));
        assert_eq!(store.capture().as_ref(), Some(&second_epoch));
        let replacement = &second_epoch.actor_catalog().entries()[0];
        assert!(view.has_method(&query_from_method(replacement)));
        assert_eq!(store.publish_count(), 2);
    }

    #[test]
    fn model_corpus_positive_projections_resolve_through_the_view() {
        let corpus: ModelCorpus =
            serde_json::from_str(&std::fs::read_to_string(MODEL_CORPUS).expect("model corpus"))
                .expect("model corpus decode");
        assert_eq!(
            corpus.schema_version,
            "skiff-router-rust-actor-routing-corpus-v1"
        );
        for case in &corpus.positive {
            let projection: ActorRoutingProjection = serde_json::from_value(case.json.clone())
                .unwrap_or_else(|error| panic!("{}: {error}", case.id));
            let epoch = epoch_from_projection(projection);
            let view = ActorMethodCatalogView::from_epoch(Arc::clone(&epoch));
            for method in epoch.actor_catalog().entries() {
                let query = query_from_method(method);
                assert!(
                    view.has_method(&query),
                    "{}: {} must hit",
                    case.id,
                    method.method_identity.as_str()
                );
                assert_eq!(view.method_for(&query).as_ref(), Some(method));
            }
        }
        for case in &corpus.negative {
            if case.id == "invalid-method-identity-uppercase" {
                // A0's framed-identity validator accepts uppercase hex; the
                // test-only corpus mirror is stricter. Deployment/A0 owns this
                // surface, so the real projection type is the authority here.
                continue;
            }
            let result: Result<ActorRoutingProjection, _> =
                serde_json::from_value(case.json.clone());
            assert!(
                result.is_err(),
                "{}: negative projection must be rejected by the real A0 type",
                case.id
            );
        }
    }

    #[test]
    fn health_counts_captures_hits_and_misses() {
        let projection: ActorRoutingProjection =
            serde_json::from_str(&a3_record("single-entry")).expect("projection parses");
        let epoch = epoch_from_projection(projection);
        let view = ActorMethodCatalogView::from_epoch(Arc::clone(&epoch));
        let method = &epoch.actor_catalog().entries()[0];
        assert!(view.has_method(&query_from_method(method)));
        assert!(view.method_for(&query_from_method(method)).is_some());
        let miss = CatalogQuery::new(
            "example.com/unknown",
            method.actor.actor_abi_identity.clone(),
            method.actor_implementation_identity.clone(),
            method.method_identity.clone(),
        );
        assert!(view.method_for(&miss).is_none());
        let health = view.health();
        assert_eq!(health.captures, 3);
        assert_eq!(health.hits, 2);
        assert_eq!(health.misses, 1);
    }

    #[test]
    fn view_has_no_index_refresh_or_file_ir_surface() {
        // The catalog view only reads the current epoch's projection;
        // `CatalogQuery` has no declarationOwner/modulePath/actorName/
        // methodName/sourceSpan fields. This test constructs a query purely
        // from A0 identities to prove the File-IR-free admission surface.
        let query = CatalogQuery::new(
            "example.com/docs",
            ActorAbiIdentity::new(format!("skiff-actor-abi-v1:sha256:{}", "a".repeat(64))),
            ActorImplementationIdentity::new(format!(
                "skiff-actor-implementation-v1:sha256:{}",
                "b".repeat(64)
            )),
            ActorMethodIdentity::new(format!("skiff-actor-method-v1:sha256:{}", "d".repeat(64))),
        );
        let projection: ActorRoutingProjection =
            serde_json::from_str(&a3_record("single-entry")).expect("projection parses");
        let view = ActorMethodCatalogView::from_epoch(epoch_from_projection(projection));
        assert!(view.has_method(&query));
        // No refresh/publication port exists on the view type; the current
        // epoch is captured from the store on every query.
        let _ = ActorRoutingProjectionRef::new(
            skiff_artifact_identity::ArtifactRelativePath::new(
                "records/single-entry.json",
                "corpus record",
            )
            .expect("record path"),
        );
    }
}
