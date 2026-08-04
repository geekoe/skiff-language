//! W-bootstrap strict loader real-boundary tests: canonical artifact root +
//! snapshot store + A3 actor routing projection record, with the full
//! fail-closed matrix (C-bootstrap §2.3, C-model-artifact §3).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use skiff_artifact_identity::{runtime_assembly_ref, ArtifactRelativePath};
use skiff_canonical_json::canonical_json_bytes;
use skiff_deployment::fixtures::empty_runtime_assembly_fixture;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_router::artifact::ActorRoutingProjectionRef;
use skiff_router::bootstrap::{BootstrapLoadFailure, BootstrapStrictLoader};
use skiff_runtime_config_snapshot::{RuntimeConfigSnapshot, RuntimeConfigSnapshotStore};

static TEMP_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot {
    parent: PathBuf,
    root: PathBuf,
}

impl TestRoot {
    fn new() -> Self {
        let sequence = TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let parent = std::env::temp_dir().join(format!(
            "skiff-router-w-bootstrap-loader-{}-{sequence}",
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

fn snapshot_ref() -> skiff_artifact_model::RuntimeConfigSnapshotRef {
    skiff_artifact_model::RuntimeConfigSnapshotRef {
        snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
            "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect("snapshot id"),
    }
}

fn actor_projection_ref(root: &Path) -> ActorRoutingProjectionRef {
    let directory = root.join("records/actor-routing");
    fs::create_dir_all(&directory).expect("create actor routing records directory");
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection");
    let bytes = canonical_json_bytes(&projection).expect("canonical projection bytes");
    fs::write(directory.join("empty.json"), bytes).expect("write projection record");
    ActorRoutingProjectionRef::new(
        ArtifactRelativePath::new("records/actor-routing/empty.json", "test record")
            .expect("record path"),
    )
}

fn materialize(
    profile: &str,
) -> (
    TestRoot,
    Arc<BootstrapStrictLoader>,
    ActorRoutingProjectionRef,
) {
    let root = TestRoot::new();
    let snapshot_store =
        RuntimeConfigSnapshotStore::create(root.path()).expect("create snapshot store");
    let snapshot =
        RuntimeConfigSnapshot::new(profile, snapshot_ref(), Vec::new()).expect("snapshot fixture");
    snapshot_store.publish(&snapshot).expect("publish snapshot");
    let artifact_store =
        CanonicalArtifactStore::create(root.path()).expect("create artifact store");
    let assembly = empty_runtime_assembly_fixture().expect("assembly fixture");
    artifact_store
        .write_runtime_assembly(&assembly)
        .expect("write assembly");
    let actor_ref = actor_projection_ref(root.path());
    let loader = Arc::new(
        BootstrapStrictLoader::open(root.path(), root.path()).expect("open strict loader"),
    );
    (root, loader, actor_ref)
}

fn committed_refs() -> skiff_router::bootstrap::CommittedBootstrapRefs {
    let assembly = empty_runtime_assembly_fixture().expect("assembly fixture");
    let assembly_ref = runtime_assembly_ref(&assembly).expect("assembly ref");
    skiff_router::bootstrap::CommittedBootstrapRefs {
        generation: 7,
        assembly: assembly_ref,
        config_snapshot: snapshot_ref(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_loader_builds_complete_epoch_from_real_records() {
        let (_root, loader, actor_ref) = materialize("prod");
        let refs = committed_refs();
        let epoch = loader
            .load_epoch(
                "prod",
                refs.generation,
                &refs.assembly,
                &refs.config_snapshot,
                &actor_ref,
            )
            .expect("epoch must load");
        assert_eq!(epoch.profile(), "prod");
        assert_eq!(epoch.assembly_generation(), 7);
        assert_eq!(
            epoch.assembly_identity(),
            refs.assembly.assembly_identity.as_str()
        );
        assert_eq!(
            epoch.config_snapshot_id(),
            refs.config_snapshot.snapshot_id.as_str()
        );
        assert!(epoch.ingress_projection().is_empty());
        assert!(epoch.deployment_projection().is_empty());
        assert!(epoch.actor_catalog().is_empty());
    }

    #[test]
    fn missing_assembly_fails_closed() {
        let (_root, loader, actor_ref) = materialize("prod");
        let mut refs = committed_refs();
        refs.assembly = skiff_artifact_model::RuntimeAssemblyRef {
            assembly_identity: skiff_artifact_model::AssemblyIdentity::new(
                "skiff-runtime-assembly-v3:sha256:".to_string() + &"b".repeat(64),
            ),
        };
        let error = loader
            .load_epoch(
                "prod",
                refs.generation,
                &refs.assembly,
                &refs.config_snapshot,
                &actor_ref,
            )
            .expect_err("missing assembly must fail closed");
        assert!(
            matches!(error, BootstrapLoadFailure::Assembly(_)),
            "{error}"
        );
    }

    #[test]
    fn missing_snapshot_fails_closed() {
        let (_root, loader, actor_ref) = materialize("prod");
        let mut refs = committed_refs();
        refs.config_snapshot = skiff_artifact_model::RuntimeConfigSnapshotRef {
            snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
                "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )
            .expect("snapshot id"),
        };
        let error = loader
            .load_epoch(
                "prod",
                refs.generation,
                &refs.assembly,
                &refs.config_snapshot,
                &actor_ref,
            )
            .expect_err("missing snapshot must fail closed");
        assert!(
            matches!(error, BootstrapLoadFailure::Snapshot(_)),
            "{error}"
        );
    }

    #[test]
    fn snapshot_profile_mismatch_fails_closed() {
        let (_root, loader, actor_ref) = materialize("stage");
        let refs = committed_refs();
        let error = loader
            .load_epoch(
                "prod",
                refs.generation,
                &refs.assembly,
                &refs.config_snapshot,
                &actor_ref,
            )
            .expect_err("snapshot profile mismatch must fail closed");
        assert!(
            matches!(error, BootstrapLoadFailure::ProfileMismatch { .. }),
            "{error}"
        );
    }

    #[test]
    fn missing_actor_projection_fails_closed() {
        let (_root, loader, _actor_ref) = materialize("prod");
        let refs = committed_refs();
        let missing_ref = ActorRoutingProjectionRef::new(
            ArtifactRelativePath::new("records/actor-routing/missing.json", "test record")
                .expect("record path"),
        );
        let error = loader
            .load_epoch(
                "prod",
                refs.generation,
                &refs.assembly,
                &refs.config_snapshot,
                &missing_ref,
            )
            .expect_err("missing actor projection must fail closed");
        assert!(
            matches!(error, BootstrapLoadFailure::ActorProjection(_)),
            "{error}"
        );
    }

    #[test]
    fn malformed_actor_projection_fails_closed() {
        let (_root, loader, _actor_ref) = materialize("prod");
        let directory = loader.artifact_root().join("records/actor-routing");
        fs::create_dir_all(&directory).expect("records directory");
        fs::write(directory.join("malformed.json"), "{not json").expect("write malformed record");
        let malformed_ref = ActorRoutingProjectionRef::new(
            ArtifactRelativePath::new("records/actor-routing/malformed.json", "test record")
                .expect("record path"),
        );
        let refs = committed_refs();
        let error = loader
            .load_epoch(
                "prod",
                refs.generation,
                &refs.assembly,
                &refs.config_snapshot,
                &malformed_ref,
            )
            .expect_err("malformed actor projection must fail closed");
        assert!(
            matches!(error, BootstrapLoadFailure::ActorProjection(_)),
            "{error}"
        );
    }

    #[test]
    fn opening_invalid_root_fails_closed() {
        let missing = std::env::temp_dir().join(format!(
            "skiff-router-w-bootstrap-missing-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&missing);
        let error = BootstrapStrictLoader::open(&missing, &missing)
            .expect_err("missing root must fail closed");
        assert!(matches!(error, BootstrapLoadFailure::Open(_)), "{error}");
    }
}
