//! W-bootstrap runner tests (M4): the bootstrap chain is exactly
//! store-open + profile validation with no committed/pending matrix. This
//! file covers the runner-shaped assembly over a real artifact root plus the
//! profile/store fail-closed matrix; `bootstrap_reader.rs` covers the same
//! matrix through the production entry point.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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
            "skiff-router-w-bootstrap-runner-{}-{sequence}",
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
    use skiff_deployment::storage::CanonicalArtifactStore;

    #[tokio::test]
    async fn runner_chain_succeeds_over_a_canonical_root() {
        let root = TestRoot::new();
        fs::create_dir_all(root.path()).expect("create artifact root");
        CanonicalArtifactStore::create(root.path()).expect("create artifact store");
        let assembly =
            RouterBootstrapAssembly::assemble(&config("dev", root.path()))
                .await
                .expect("bootstrap must succeed");
        assert_eq!(assembly.profile(), "dev");
        assembly.shutdown().await;
    }

    #[tokio::test]
    async fn runner_chain_fails_closed_on_missing_store() {
        let root = TestRoot::new();
        let error = RouterBootstrapAssembly::assemble(&config("dev", root.path()))
            .await
            .expect_err("missing store must fail closed");
        assert!(error.to_string().contains("artifact store open failed"));
    }

    #[tokio::test]
    async fn runner_chain_fails_closed_on_invalid_profile() {
        let root = TestRoot::new();
        fs::create_dir_all(root.path()).expect("create artifact root");
        CanonicalArtifactStore::create(root.path()).expect("create artifact store");
        let error = RouterBootstrapAssembly::assemble(&config("not a profile", root.path()))
            .await
            .expect_err("invalid profile must fail closed");
        assert!(error.to_string().contains("profile is invalid"));
    }
}
