use super::*;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "runtime-blob-store-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&path).expect("temp dir should be created");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn local_blob_store_puts_and_gets_without_overwrite() {
    let temp = TempDir::new("put-get");
    let store = LocalBlobStore::new(temp.path.join("blobs")).expect("store should create");

    store
        .put_if_absent(PutBlobRequest {
            key: "abcdef-5".to_string(),
            body: BlobBody::Bytes(b"first".to_vec()),
            metadata: BlobMetadata::default(),
        })
        .expect("first put should succeed");
    store
        .put_if_absent(PutBlobRequest {
            key: "abcdef-5".to_string(),
            body: BlobBody::Bytes(b"second".to_vec()),
            metadata: BlobMetadata::default(),
        })
        .expect("duplicate put should be idempotent");

    assert_eq!(store.get("abcdef-5").expect("blob should read"), b"first");
}

#[test]
fn local_blob_store_rejects_escaping_keys() {
    let temp = TempDir::new("escape");
    let store = LocalBlobStore::new(temp.path.join("blobs")).expect("store should create");

    let error = store
        .put_if_absent(PutBlobRequest {
            key: "../escape".to_string(),
            body: BlobBody::Bytes(Vec::new()),
            metadata: BlobMetadata::default(),
        })
        .expect_err("escaping key should be rejected");

    assert!(error.to_string().contains("must not contain .."));
}

#[test]
fn blob_store_from_control_prefers_local_over_oss() {
    let temp = TempDir::new("prefer-local");
    let store = blob_store_from_control(&FileBackendControlConfig {
        local: Some(FileBackendLocalConfig {
            root: temp.path.join("blobs"),
        }),
        oss: Some(FileBackendOssConfig {
            endpoint: "https://oss.example.com".to_string(),
            bucket: "bucket".to_string(),
            region: None,
            access_key_id: None,
            access_key_secret: None,
            access_key_id_env: Some("OSS_ACCESS_KEY_ID".to_string()),
            access_key_secret_env: Some("OSS_ACCESS_KEY_SECRET".to_string()),
        }),
    })
    .expect("local backend should win");

    store
        .put_if_absent(PutBlobRequest {
            key: "abcdef-5".to_string(),
            body: BlobBody::Bytes(b"local".to_vec()),
            metadata: BlobMetadata::default(),
        })
        .expect("local store should be usable");
    assert_eq!(
        fs::read(temp.path.join("blobs/abcdef-5")).expect("blob file should exist"),
        b"local"
    );
    assert!(
        !temp.path.join("blobs/blob").exists(),
        "flat blob key should not create the old nested key root"
    );
    assert!(
        !temp.path.join("blobs/sha256").exists(),
        "flat blob key should not create a sha256 prefix directory"
    );
}
