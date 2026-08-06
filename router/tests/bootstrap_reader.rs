//! W-bootstrap assembly tests (M4): `RouterBootstrapAssembly` opens the
//! canonical artifact store and validates the profile; fail-closed is exactly
//! store-open failure + invalid profile. No committed/pending state, no Mongo
//! repository, no routing epoch.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use skiff_router::bootstrap::RouterBootstrapAssembly;
use skiff_router::config::{RouterConfig, ServiceDbConfig};

static TEMP_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot {
    parent: PathBuf,
    root: PathBuf,
}

impl TestRoot {
    fn new() -> Self {
        let sequence = TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let parent = std::env::temp_dir().join(format!(
            "skiff-router-w-bootstrap-{}-{sequence}",
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

fn config(profile: &str, artifacts_path: &Path) -> RouterConfig {
    RouterConfig {
        artifacts_path: artifacts_path.to_path_buf(),
        dev_reload: None,
        host: "127.0.0.1".to_string(),
        http_max_request_bytes: 1,
        http_max_response_bytes: 8_388_608,
        http_port: 4000,
        manifests: vec![],
        profile: profile.to_string(),
        release_mode: None,
        request_timeout_ms: 20_000,
        rewrite: vec![],
        runtime_path: "/runtime".to_string(),
        runtime_port: 4001,
        runtime_max_concurrency: 4,
        file_backend: None,
        service_db: ServiceDbConfig {
            mongo_url: "mongodb://127.0.0.1:27017/?replicaSet=rs0".to_string(),
        },
        telemetry: None,
        websocket_path: "/ws".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn assemble_opens_the_canonical_artifact_store() {
        let root = TestRoot::new();
        fs::create_dir_all(root.path()).expect("create artifact root");
        skiff_deployment::storage::CanonicalArtifactStore::create(root.path())
            .expect("create artifact store");
        let assembly = RouterBootstrapAssembly::assemble(&config("prod", root.path()))
            .await
            .expect("assembly must succeed");
        assert_eq!(assembly.profile(), "prod");
        // The store canonicalizes its root (macOS maps `/var` to
        // `/private/var`), so compare against the canonicalized path.
        let expected_root = fs::canonicalize(root.path()).expect("canonicalize artifact root");
        assert_eq!(assembly.store().root(), expected_root);
        assembly.shutdown().await;
    }

    #[tokio::test]
    async fn assemble_fails_closed_when_the_artifact_root_is_missing() {
        let root = TestRoot::new();
        let error = RouterBootstrapAssembly::assemble(&config("prod", root.path()))
            .await
            .expect_err("missing artifact root must fail closed");
        assert!(
            error
                .to_string()
                .contains("canonical artifact store open failed"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn assemble_fails_closed_when_the_artifact_root_is_a_file() {
        let root = TestRoot::new();
        fs::write(root.path(), b"not a directory").expect("write file");
        let error = RouterBootstrapAssembly::assemble(&config("prod", root.path()))
            .await
            .expect_err("file artifact root must fail closed");
        assert!(
            error
                .to_string()
                .contains("canonical artifact store open failed"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn assemble_fails_closed_on_invalid_profile() {
        let root = TestRoot::new();
        fs::create_dir_all(root.path()).expect("create artifact root");
        skiff_deployment::storage::CanonicalArtifactStore::create(root.path())
            .expect("create artifact store");
        for invalid in ["prod env", "", "..", "prod/"] {
            let error = RouterBootstrapAssembly::assemble(&config(invalid, root.path()))
                .await
                .expect_err("invalid profile must fail closed");
            assert!(
                error.to_string().contains("profile is invalid"),
                "profile {invalid:?} unexpected error: {error}"
            );
        }
    }

    #[tokio::test]
    async fn assemble_exposes_the_blocking_loader_and_actor_projection() {
        let root = TestRoot::new();
        fs::create_dir_all(root.path()).expect("create artifact root");
        skiff_deployment::storage::CanonicalArtifactStore::create(root.path())
            .expect("create artifact store");
        let assembly = RouterBootstrapAssembly::assemble(&config("prod", root.path()))
            .await
            .expect("assembly must succeed");
        assert_eq!(
            assembly.actor_projection().record_path.as_str(),
            skiff_router::bootstrap::ACTOR_ROUTING_PROJECTION_RECORD_PATH
        );
        let loader = assembly.loader();
        assert_eq!(loader.health().shutdown, false);
        assembly.shutdown().await;
        assert_eq!(loader.health().shutdown, true);
    }

    #[tokio::test]
    async fn shutdown_is_idempotent() {
        let root = TestRoot::new();
        fs::create_dir_all(root.path()).expect("create artifact root");
        skiff_deployment::storage::CanonicalArtifactStore::create(root.path())
            .expect("create artifact store");
        let assembly = RouterBootstrapAssembly::assemble(&config("prod", root.path()))
            .await
            .expect("assembly must succeed");
        assembly.shutdown().await;
        assembly.shutdown().await;
        let _ = Arc::new(assembly);
    }
}
