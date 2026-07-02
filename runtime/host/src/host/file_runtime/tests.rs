use std::{path::PathBuf, sync::Arc};

use bytes::Bytes;

use super::*;
use crate::error::RuntimeError;
use skiff_runtime_service_db::{
    DbRequestState, ServiceDbCapabilityStore, ServiceDbConfig, ServiceDbRuntime, ServiceDbStore,
};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "runtime-file-runtime-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("temp dir should be created");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[tokio::test]
async fn file_runtime_create_from_chunks_rejects_over_limit_before_persist() {
    let temp = TempDir::new("limit");
    let runtime = FileRuntime::new(None, temp.path.join("tmp"));
    let store = unused_store();
    let mut chunks = vec![
        Bytes::from(vec![0_u8; FILE_GUARD_LIMIT_BYTES]),
        Bytes::from_static(b"x"),
    ]
    .into_iter();

    let error = runtime
        .create_from_chunks(&store, FileCreateOptions::default(), || {
            let next = chunks.next();
            async move { Ok(next) }
        })
        .await
        .expect_err("over-limit stream should fail");
    let payload = error.payload();
    assert_eq!(payload.code, "ResourceLimitExceeded");
    assert_eq!(
        payload
            .details
            .as_ref()
            .and_then(|details| details["resource"].as_str()),
        Some("std.file")
    );
}

#[tokio::test]
async fn file_runtime_storage_unavailable_is_clear() {
    let temp = TempDir::new("unavailable");
    let runtime = FileRuntime::new(None, temp.path.join("tmp"));
    let store = unused_store();

    let error = runtime
        .create(
            &store,
            Bytes::from_static(b"hello"),
            FileCreateOptions::default(),
        )
        .await
        .expect_err("missing blob store should fail");
    assert!(matches!(error, RuntimeError::ProviderUnavailable { .. }));
    assert!(error.to_string().contains("blob store is not configured"));
}

#[test]
fn cas_blob_key_is_sha256_size_filename() {
    let key = cas_blob_key(
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        5,
    );
    assert_eq!(
        key,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824-5"
    );
    assert!(!key.contains('/'), "blob key should not create directories");
}

fn unused_store() -> ServiceDbCapabilityStore {
    let service_db = Arc::new(
        ServiceDbRuntime::new_with_config(
            format!("example.com/file_{}", uuid::Uuid::new_v4().simple()),
            ServiceDbConfig {
                mongo_url: "mongodb://127.0.0.1:1/?directConnection=true".to_string(),
            },
            &[],
        )
        .expect("service db metadata should build without connecting"),
    );
    ServiceDbCapabilityStore::new(ServiceDbStore::new(
        service_db,
        Arc::new(tokio::sync::Mutex::new(DbRequestState::default())),
    ))
}
