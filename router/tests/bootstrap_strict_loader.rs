//! W-bootstrap strict loader tests (M4): `BootstrapStrictLoader` opens the
//! actor routing projection store and loads the catalog on demand, with the
//! fail-closed matrix for the A3 actor routing record.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use skiff_artifact_identity::ArtifactRelativePath;
use skiff_canonical_json::canonical_json_bytes;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_router::artifact::ActorRoutingProjectionRef;
use skiff_router::bootstrap::{
    BootstrapLoadFailure, BootstrapStrictLoader, ACTOR_ROUTING_PROJECTION_RECORD_PATH,
};

static TEMP_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot {
    parent: PathBuf,
    root: PathBuf,
}

impl TestRoot {
    fn new() -> Self {
        let sequence = TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let parent = std::env::temp_dir().join(format!(
            "skiff-router-w-bootstrap-strict-{}-{sequence}",
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

fn write_projection(root: &Path, bytes: &[u8]) {
    let path = root.join(ACTOR_ROUTING_PROJECTION_RECORD_PATH);
    fs::create_dir_all(path.parent().expect("projection parent"))
        .expect("create projection dirs");
    fs::write(path, bytes).expect("write projection");
}

fn canonical_projection_bytes() -> Vec<u8> {
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection");
    canonical_json_bytes(&projection).expect("canonical projection")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_loader_opens_and_loads_the_actor_catalog() {
        let root = TestRoot::new();
        fs::create_dir_all(root.path()).expect("create root");
        CanonicalArtifactStore::create(root.path()).expect("create artifact store");
        write_projection(root.path(), &canonical_projection_bytes());
        let loader = BootstrapStrictLoader::open(root.path()).expect("loader opens");
        let catalog = loader
            .load_actor_catalog(&projection_ref())
            .expect("catalog loads");
        assert!(catalog.is_empty());
    }

    #[test]
    fn strict_loader_open_fails_closed_on_missing_root() {
        let root = TestRoot::new();
        let error = BootstrapStrictLoader::open(root.path())
            .expect_err("missing root must fail closed");
        assert!(matches!(error, BootstrapLoadFailure::Open(_)));
    }

    #[test]
    fn strict_loader_load_fails_closed_on_missing_projection() {
        let root = TestRoot::new();
        fs::create_dir_all(root.path()).expect("create root");
        CanonicalArtifactStore::create(root.path()).expect("create artifact store");
        let loader = BootstrapStrictLoader::open(root.path()).expect("loader opens");
        let error = loader
            .load_actor_catalog(&projection_ref())
            .expect_err("missing projection must fail closed");
        assert!(matches!(error, BootstrapLoadFailure::ActorProjection(_)));
    }

    #[test]
    fn strict_loader_load_fails_closed_on_malformed_projection() {
        let root = TestRoot::new();
        fs::create_dir_all(root.path()).expect("create root");
        CanonicalArtifactStore::create(root.path()).expect("create artifact store");
        write_projection(root.path(), b"not json");
        let loader = BootstrapStrictLoader::open(root.path()).expect("loader opens");
        let error = loader
            .load_actor_catalog(&projection_ref())
            .expect_err("malformed projection must fail closed");
        assert!(matches!(error, BootstrapLoadFailure::ActorProjection(_)));
    }
}
