//! W-bootstrap epoch tests (M4): the `RoutingEpoch` is retired; the actor
//! routing catalog is now loaded lazily on demand through
//! `ActorMethodCatalogView` (artifact store projection record, cached for the
//! process lifetime, fail-closed on missing/malformed records).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use skiff_artifact_identity::{ArtifactRelativePath, PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX};
use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity, DeploymentArtifactIdentity,
    DeploymentRevision, PackageArtifactRef, PackageBuildId, PackageLocalAbiIdentity,
    ServiceDeploymentRef,
};
use skiff_canonical_json::canonical_json_bytes;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingMethod, ActorRoutingProjection, ActorRoutingRef,
    ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_router::actor::{ActorMethodCatalogView, CatalogQuery};
use skiff_router::artifact::ActorRoutingProjectionRef;
use skiff_router::bootstrap::ACTOR_ROUTING_PROJECTION_RECORD_PATH;

static TEMP_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot {
    parent: PathBuf,
    root: PathBuf,
}

impl TestRoot {
    fn new() -> Self {
        let sequence = TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let parent = std::env::temp_dir().join(format!(
            "skiff-router-w-bootstrap-epoch-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&parent).expect("create temp parent");
        Self {
            parent: parent.clone(),
            root: parent.join("root"),
        }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.parent);
    }
}

fn projection_ref() -> ActorRoutingProjectionRef {
    ActorRoutingProjectionRef::new(
        ArtifactRelativePath::new(
            ACTOR_ROUTING_PROJECTION_RECORD_PATH,
            "actor routing projection record",
        )
        .expect("projection path"),
    )
}

fn projection_path(root: &Path) -> PathBuf {
    root.join(ACTOR_ROUTING_PROJECTION_RECORD_PATH)
}

fn write_projection(root: &Path, projection: &ActorRoutingProjection) {
    let path = projection_path(root);
    fs::create_dir_all(path.parent().expect("projection parent")).expect("create projection dirs");
    let bytes = canonical_json_bytes(projection).expect("canonical projection");
    fs::write(path, bytes).expect("write projection");
}

fn abi(byte: char) -> ActorAbiIdentity {
    ActorAbiIdentity::new(format!(
        "skiff-actor-abi-v1:sha256:{}",
        byte.to_string().repeat(64)
    ))
}

fn implementation(byte: char) -> ActorImplementationIdentity {
    ActorImplementationIdentity::new(format!(
        "skiff-actor-implementation-v1:sha256:{}",
        byte.to_string().repeat(64)
    ))
}

fn method(byte: char) -> ActorMethodIdentity {
    ActorMethodIdentity::new(format!(
        "skiff-actor-method-v1:sha256:{}",
        byte.to_string().repeat(64)
    ))
}

fn empty_projection() -> ActorRoutingProjection {
    ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_view_loads_lazily_and_caches() {
        let root = TestRoot::new();
        fs::create_dir_all(root.path()).expect("create root");
        CanonicalArtifactStore::create(root.path()).expect("create artifact store");
        write_projection(root.path(), &empty_projection());
        let view = ActorMethodCatalogView::new(root.path(), projection_ref()).expect("view opens");
        assert_eq!(view.loads(), 0);
        let query = CatalogQuery::new(
            "example.com/service-1".to_string(),
            abi('a'),
            implementation('b'),
            method('c'),
        );
        assert!(!view.has_method(&query), "empty projection has no methods");
        assert_eq!(view.loads(), 2, "miss retries a fresh projection load once");
        let _ = view.has_method(&query);
        assert_eq!(
            view.loads(),
            3,
            "each miss reloads once; the reloaded catalog is cached"
        );
        assert_eq!(
            view.schema_version(),
            ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION
        );
    }

    #[test]
    fn catalog_view_reloads_projection_after_build_switch() {
        let root = TestRoot::new();
        fs::create_dir_all(root.path()).expect("create root");
        CanonicalArtifactStore::create(root.path()).expect("create artifact store");
        let old_abi = abi('a');
        let new_abi = abi('c');
        let impl_id = implementation('b');
        let method_id = method('c');
        let service = "example.com/service-1".to_string();
        let entry = |abi: &ActorAbiIdentity, build: &str| ActorRoutingMethod {
            actor: skiff_deployment::projection::actor_routing::ActorRoutingRef {
                service_id: service.clone(),
                actor_abi_identity: abi.clone(),
            },
            actor_implementation_identity: impl_id.clone(),
            method_identity: method_id.clone(),
            deployment: ServiceDeploymentRef {
                service_id: service.clone(),
                contract_version: "0.1.0".to_string(),
                deployment_revision: DeploymentRevision::new(format!("rev-{build}")),
                deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
                    "skiff-deployment-artifact-v4:sha256:{build}"
                )),
            },
            package: PackageArtifactRef {
                package_id: service.clone(),
                package_version: "0.1.0".to_string(),
                package_build_id: PackageBuildId::new(format!(
                    "{PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX}:{build}"
                )),
                package_local_abi_identity: PackageLocalAbiIdentity::new(format!(
                    "skiff-package-local-abi-v7:sha256:{build}"
                )),
            },
        };
        let old_build = "a".repeat(64);
        let new_build = "b".repeat(64);
        write_projection(
            root.path(),
            &ActorRoutingProjection::new(
                ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
                vec![entry(&old_abi, &old_build)],
            )
            .expect("old projection"),
        );
        let view = ActorMethodCatalogView::new(root.path(), projection_ref()).expect("view opens");
        let old_query = CatalogQuery::new(
            service.clone(),
            old_abi.clone(),
            impl_id.clone(),
            method_id.clone(),
        );
        assert!(
            view.has_method(&old_query),
            "first query loads the old projection and hits"
        );
        // Build switch replaces the projection record with a new actor ABI.
        write_projection(
            root.path(),
            &ActorRoutingProjection::new(
                ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
                vec![entry(&new_abi, &new_build)],
            )
            .expect("new projection"),
        );
        let stale_query =
            CatalogQuery::new(service.clone(), old_abi, impl_id.clone(), method_id.clone());
        let new_query = CatalogQuery::new(service, new_abi.clone(), impl_id, method_id);
        assert!(
            view.has_method(&stale_query),
            "the stale cached catalog still serves old identities until a miss"
        );
        assert!(
            view.has_method(&new_query),
            "miss reloads the projection and resolves the new build"
        );
        assert_eq!(
            view.loads(),
            2,
            "initial load + one reload on the stale miss"
        );
    }

    #[test]
    fn catalog_view_fails_closed_on_missing_projection() {
        let root = TestRoot::new();
        fs::create_dir_all(root.path()).expect("create root");
        CanonicalArtifactStore::create(root.path()).expect("create artifact store");
        let view = ActorMethodCatalogView::new(root.path(), projection_ref()).expect("view opens");
        let query = CatalogQuery::new(
            "example.com/service-1".to_string(),
            abi('a'),
            implementation('b'),
            method('c'),
        );
        assert!(
            !view.has_method(&query),
            "missing projection must fail closed"
        );
    }

    #[test]
    fn catalog_view_fails_closed_on_malformed_projection() {
        let root = TestRoot::new();
        fs::create_dir_all(root.path()).expect("create root");
        CanonicalArtifactStore::create(root.path()).expect("create artifact store");
        let path = projection_path(root.path());
        fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        fs::write(path, b"not json").expect("write malformed projection");
        let view = ActorMethodCatalogView::new(root.path(), projection_ref()).expect("view opens");
        let query = CatalogQuery::new(
            "example.com/service-1".to_string(),
            abi('a'),
            implementation('b'),
            method('c'),
        );
        assert!(
            !view.has_method(&query),
            "malformed projection must fail closed"
        );
    }
}
