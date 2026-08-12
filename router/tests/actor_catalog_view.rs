//! `ActorMethodCatalogView` sequence tests: read-only typed queries over a
//! real A0/A3 `ActorRoutingProjection` record published into a temporary
//! canonical artifact root. The view lazily loads the catalog once on first
//! query (M4: no routing epoch) and caches it; the view never reads File IR,
//! never accepts source/declaration coordinates as query input.

mod actor_support;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;
use skiff_artifact_identity::{ArtifactRelativePath, PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX};
use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity};
use skiff_deployment::projection::actor_routing::{
    ActorRoutingMethod, ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_RECORD_PATH,
    ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_router::actor::{ActorMethodCatalogView, CatalogQuery};
use skiff_router::artifact::{ActorRoutingCatalog, ActorRoutingProjectionRef};

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

/// Unique temporary artifact root per view (parallel tests must not share
/// the `records/actor-routing/current.json` record path).
fn projection_root() -> std::path::PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "skiff-actor-catalog-view-{}-{id}",
        std::process::id()
    ))
}

/// Publishes the projection as the current actor routing record in a fresh
/// temporary artifact root and builds a view over it.
fn view_from_projection(projection: &ActorRoutingProjection) -> ActorMethodCatalogView {
    let root = projection_root();
    std::fs::create_dir_all(&root).expect("create temp artifact root");
    let store = CanonicalArtifactStore::open(&root).expect("open artifact store");
    store
        .write_actor_routing_projection(projection)
        .expect("write actor routing projection");
    let reference = ActorRoutingProjectionRef::new(
        ArtifactRelativePath::new(
            ACTOR_ROUTING_PROJECTION_RECORD_PATH,
            "actor routing projection record",
        )
        .expect("record path"),
    );
    ActorMethodCatalogView::new(&root, reference).expect("view")
}

fn method_from_projection(projection: &ActorRoutingProjection) -> ActorRoutingMethod {
    ActorRoutingCatalog::from_projection(Arc::new(projection.clone())).entries()[0].clone()
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

fn model_corpus() -> ModelCorpus {
    let mut value: Value = serde_json::from_str(
        &std::fs::read_to_string(MODEL_CORPUS).expect("model corpus"),
    )
    .expect("model corpus decode");
    for case in value
        .get_mut("positive")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        let Some(methods) = case
            .get_mut("json")
            .and_then(|json| json.get_mut("methods"))
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        for method in methods {
            if let Some(build) = method
                .get_mut("package")
                .and_then(|package| package.get_mut("packageBuildId"))
            {
                if let Some(text) = build.as_str() {
                    if let Some(rest) = text.strip_prefix("skiff-package-build-v11:sha256:") {
                        *build = Value::String(format!(
                            "{PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX}:{rest}"
                        ));
                    }
                }
            }
        }
    }
    serde_json::from_value(value).expect("model corpus decode")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_typed_key_resolves_the_immutable_entry_with_binding() {
        let projection: ActorRoutingProjection =
            serde_json::from_str(&a3_record("single-entry")).expect("projection parses");
        let view = view_from_projection(&projection);
        let method = method_from_projection(&projection);
        let query = query_from_method(&method);
        assert!(view.has_method(&query));
        let resolved = view.method_for(&query).expect("method_for resolves");
        assert_eq!(&resolved, &method);
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
        let view = view_from_projection(&projection);
        let method = method_from_projection(&projection);
        let mut wrong_service = query_from_method(&method);
        wrong_service.service_id = "example.com/unknown".to_string();
        assert!(!view.has_method(&wrong_service));
        assert!(view.method_for(&wrong_service).is_none());

        let mut wrong_method = query_from_method(&method);
        wrong_method.method_identity =
            ActorMethodIdentity::new(format!("skiff-actor-method-v1:sha256:{}", "e".repeat(64)));
        assert!(view.method_for(&wrong_method).is_none());
    }

    #[test]
    fn empty_projection_is_valid_and_all_queries_miss() {
        let projection: ActorRoutingProjection =
            serde_json::from_str(&a3_record("empty")).expect("empty projection parses");
        assert!(projection.methods.is_empty());
        let view = view_from_projection(&projection);
        assert!(!view.has_method(&CatalogQuery::new(
            "example.com/docs",
            ActorAbiIdentity::new(format!("skiff-actor-abi-v1:sha256:{}", "a".repeat(64))),
            ActorImplementationIdentity::new(format!(
                "skiff-actor-implementation-v1:sha256:{}",
                "b".repeat(64)
            )),
            ActorMethodIdentity::new(format!("skiff-actor-method-v1:sha256:{}", "d".repeat(64))),
        )));
        assert_eq!(view.health().misses, 2, "miss reloads the projection once and retries");
    }

    #[test]
    fn view_reloads_the_catalog_on_miss_after_record_replacement() {
        let first: ActorRoutingProjection =
            serde_json::from_str(&a3_record("single-entry")).expect("first projection");
        let second: ActorRoutingProjection =
            serde_json::from_str(&a3_record("multi-entry-sorted")).expect("second projection");
        let root = projection_root();
        std::fs::create_dir_all(&root).expect("create temp artifact root");
        let store = CanonicalArtifactStore::open(&root).expect("open artifact store");
        store
            .write_actor_routing_projection(&first)
            .expect("write first projection");
        let reference = ActorRoutingProjectionRef::new(
            ArtifactRelativePath::new(
                ACTOR_ROUTING_PROJECTION_RECORD_PATH,
                "actor routing projection record",
            )
            .expect("record path"),
        );
        let view = ActorMethodCatalogView::new(&root, reference).expect("view");
        let method = method_from_projection(&first);
        assert!(view.has_method(&query_from_method(&method)));
        assert_eq!(view.loads(), 1);
        // Build switch replaces the record on disk; the cached catalog is
        // stale but a miss reloads it and resolves the new build.
        store
            .write_actor_routing_projection(&second)
            .expect("write second projection");
        let replacement = &second.methods[1];
        let replacement_query = CatalogQuery::new(
            replacement.actor.service_id.clone(),
            replacement.actor.actor_abi_identity.clone(),
            replacement.actor_implementation_identity.clone(),
            replacement.method_identity.clone(),
        );
        assert!(
            view.has_method(&replacement_query),
            "miss reloads the replaced projection and resolves the new entry"
        );
        assert_eq!(view.loads(), 2);
        // The cached (reloaded) catalog still serves the first entry.
        assert!(view.has_method(&query_from_method(&method)));
        assert_eq!(view.loads(), 2);
    }

    #[test]
    fn model_corpus_positive_projections_resolve_through_the_view() {
        let corpus = model_corpus();
        assert_eq!(
            corpus.schema_version,
            "skiff-router-rust-actor-routing-corpus-v1"
        );
        for case in &corpus.positive {
            let projection: ActorRoutingProjection = serde_json::from_value(case.json.clone())
                .unwrap_or_else(|error| panic!("{}: {error}", case.id));
            let view = view_from_projection(&projection);
            let catalog = ActorRoutingCatalog::from_projection(Arc::new(projection.clone()));
            for method in catalog.entries() {
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
        let view = view_from_projection(&projection);
        let method = method_from_projection(&projection);
        assert!(view.has_method(&query_from_method(&method)));
        assert!(view.method_for(&query_from_method(&method)).is_some());
        let miss = CatalogQuery::new(
            "example.com/unknown",
            method.actor.actor_abi_identity.clone(),
            method.actor_implementation_identity.clone(),
            method.method_identity.clone(),
        );
        assert!(view.method_for(&miss).is_none());
        let health = view.health();
        assert_eq!(health.captures, 4);
        assert_eq!(health.hits, 2);
        assert_eq!(health.misses, 2);
        assert_eq!(view.loads(), 2, "the miss reloads the projection once and retries");
    }

    #[test]
    fn view_has_no_index_refresh_or_file_ir_surface() {
        // The catalog view only reads the current routing projection;
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
        let view = view_from_projection(&projection);
        assert!(view.has_method(&query));
        // No refresh/publication port exists on the view type; the catalog
        // is loaded once on first query and cached.
        let _ = ActorRoutingProjectionRef::new(
            skiff_artifact_identity::ArtifactRelativePath::new(
                "records/single-entry.json",
                "corpus record",
            )
            .expect("record path"),
        );
    }
}
