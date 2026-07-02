use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

use skiff_runtime_transport::protocol::{
    FileBackendControlConfig, FileBackendLocalConfig, FileBackendOssConfig,
};

pub const SKIFF_FILE_CACHE_CONTROL: &str = "private, max-age=31536000, immutable";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobMetadata {
    pub content_type: Option<String>,
    pub cache_control: String,
}

impl Default for BlobMetadata {
    fn default() -> Self {
        Self {
            content_type: None,
            cache_control: SKIFF_FILE_CACHE_CONTROL.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutBlobRequest {
    pub key: String,
    pub body: BlobBody,
    pub metadata: BlobMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobBody {
    Bytes(Vec<u8>),
    TempFile(PathBuf),
}

pub trait BlobStore: Send + Sync {
    fn put_if_absent(&self, request: PutBlobRequest) -> anyhow::Result<()>;
    fn get(&self, key: &str) -> anyhow::Result<Vec<u8>>;
}

pub fn blob_store_from_control(
    config: &FileBackendControlConfig,
) -> anyhow::Result<Box<dyn BlobStore>> {
    config.validate().map_err(|error| anyhow::anyhow!(error))?;
    if let Some(local) = &config.local {
        return Ok(Box::new(LocalBlobStore::new(local.root.clone())?));
    }
    if let Some(oss) = &config.oss {
        return Ok(Box::new(OssBlobStore::new(oss.clone())?));
    }
    anyhow::bail!("file backend must configure local or oss")
}

#[derive(Debug, Clone)]
pub struct LocalBlobStore {
    root: PathBuf,
}

impl LocalBlobStore {
    pub fn new(root: PathBuf) -> anyhow::Result<Self> {
        if root.as_os_str().is_empty() {
            anyhow::bail!("local blob store root must not be empty");
        }
        fs::create_dir_all(&root)
            .map_err(|error| anyhow::anyhow!("failed to create {}: {error}", root.display()))?;
        let root = fs::canonicalize(&root).map_err(|error| {
            anyhow::anyhow!("failed to canonicalize {}: {error}", root.display())
        })?;
        Ok(Self { root })
    }

    pub fn from_config(config: &FileBackendLocalConfig) -> anyhow::Result<Self> {
        Self::new(config.root.clone())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path_for_key(&self, key: &str) -> anyhow::Result<PathBuf> {
        let relative = safe_relative_key(key)?;
        Ok(self.root.join(relative))
    }
}

impl BlobStore for LocalBlobStore {
    fn put_if_absent(&self, mut request: PutBlobRequest) -> anyhow::Result<()> {
        request.metadata.cache_control = SKIFF_FILE_CACHE_CONTROL.to_string();
        let path = self.path_for_key(&request.key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                anyhow::anyhow!(
                    "failed to create blob directory {}: {error}",
                    parent.display()
                )
            })?;
        }

        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Ok(());
            }
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "failed to create blob {}: {error}",
                    path.display()
                ));
            }
        };

        match request.body {
            BlobBody::Bytes(bytes) => {
                file.write_all(&bytes).map_err(|error| {
                    anyhow::anyhow!("failed to write blob {}: {error}", path.display())
                })?;
            }
            BlobBody::TempFile(source) => {
                let mut source_file = fs::File::open(&source).map_err(|error| {
                    anyhow::anyhow!("failed to open temp file {}: {error}", source.display())
                })?;
                std::io::copy(&mut source_file, &mut file).map_err(|error| {
                    anyhow::anyhow!(
                        "failed to copy temp file into blob {}: {error}",
                        path.display()
                    )
                })?;
            }
        }
        file.sync_all()
            .map_err(|error| anyhow::anyhow!("failed to sync blob {}: {error}", path.display()))?;
        Ok(())
    }

    fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        let path = self.path_for_key(key)?;
        fs::read(&path)
            .map_err(|error| anyhow::anyhow!("failed to read blob {}: {error}", path.display()))
    }
}

#[derive(Debug, Clone)]
pub struct OssBlobStore {
    config: FileBackendOssConfig,
}

impl OssBlobStore {
    pub fn new(config: FileBackendOssConfig) -> anyhow::Result<Self> {
        config.validate().map_err(|error| anyhow::anyhow!(error))?;
        Ok(Self { config })
    }

    pub fn config(&self) -> &FileBackendOssConfig {
        &self.config
    }
}

impl BlobStore for OssBlobStore {
    fn put_if_absent(&self, _request: PutBlobRequest) -> anyhow::Result<()> {
        anyhow::bail!(
            "OSS blob store is not implemented yet; local file backend is required for runtime blob writes"
        )
    }

    fn get(&self, _key: &str) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!(
            "OSS blob store is not implemented yet; local file backend is required for runtime blob reads"
        )
    }
}

fn safe_relative_key(key: &str) -> anyhow::Result<PathBuf> {
    if key.is_empty() {
        anyhow::bail!("blob key must not be empty");
    }
    let path = Path::new(key);
    if path.is_absolute() {
        anyhow::bail!("blob key must be relative");
    }

    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => relative.push(segment),
            Component::CurDir => {}
            Component::ParentDir => anyhow::bail!("blob key must not contain .."),
            Component::RootDir | Component::Prefix(_) => anyhow::bail!("blob key must be relative"),
        }
    }
    if relative.as_os_str().is_empty() {
        anyhow::bail!("blob key must not be empty");
    }
    Ok(relative)
}

#[cfg(test)]
mod tests {
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
}
