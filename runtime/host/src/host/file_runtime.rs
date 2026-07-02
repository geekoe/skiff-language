//! Runtime implementation for `std.file` immutable file host operations.

use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
};

use bytes::Bytes;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use skiff_runtime_boundary::file::{immutable_file_wire, FileCreateOptions, ImmutableFileRef};
use skiff_runtime_capability_context::{
    DbCapabilityStoreApi, FileCapabilityResult, FileChunkSource,
};
use tokio::{
    fs::{self, File},
    io::AsyncWriteExt,
};

use super::blob_store::{
    BlobBody, BlobMetadata, BlobStore, PutBlobRequest, SKIFF_FILE_CACHE_CONTROL,
};
use crate::{
    capability_context::{FileCapabilityRecord, FileCapabilityRuntime, HostCapabilityFuture},
    error::{Result, RuntimeError},
};

pub const FILE_MEMORY_FAST_PATH_BYTES: usize = 1024 * 1024;
pub const FILE_GUARD_LIMIT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone)]
pub struct FileRuntime {
    blob_store: Option<Arc<dyn BlobStore>>,
    tmp_root: PathBuf,
}

impl FileRuntime {
    pub fn new(blob_store: Option<Arc<dyn BlobStore>>, tmp_root: PathBuf) -> Self {
        Self {
            blob_store,
            tmp_root,
        }
    }

    pub async fn create(
        &self,
        store: &dyn DbCapabilityStoreApi,
        input: Bytes,
        options: FileCreateOptions,
    ) -> Result<Value> {
        let mut ingest = FileIngest::new(&self.tmp_root);
        ingest.push(&input).await?;
        let staged = ingest.finish().await?;
        self.persist_staged_file(store, staged, options).await
    }

    pub async fn create_from_chunks<I, F>(
        &self,
        store: &dyn DbCapabilityStoreApi,
        options: FileCreateOptions,
        mut next_chunk: I,
    ) -> Result<Value>
    where
        I: FnMut() -> F,
        F: Future<Output = FileCapabilityResult<Option<Bytes>>>,
    {
        let mut ingest = FileIngest::new(&self.tmp_root);
        loop {
            match next_chunk().await {
                Ok(Some(chunk)) => ingest.push(&chunk).await?,
                Ok(None) => break,
                Err(error) => {
                    ingest.cleanup().await;
                    return Err(RuntimeError::from(error));
                }
            }
        }
        let staged = ingest.finish().await?;
        self.persist_staged_file(store, staged, options).await
    }

    pub async fn read(
        &self,
        store: &dyn DbCapabilityStoreApi,
        file: &ImmutableFileRef,
    ) -> Result<Bytes> {
        let record = self.validated_record(store, file).await?;
        let blob_store = self.require_blob_store()?;
        let key = record.blob_key.clone();
        let bytes = tokio::task::spawn_blocking(move || blob_store.get(&key))
            .await
            .map_err(|error| storage_unavailable(format!("blob read task failed: {error}")))?
            .map_err(|error| {
                storage_unavailable(format!("read blob {} failed: {error}", record.blob_key))
            })?;
        Ok(Bytes::from(bytes))
    }

    pub async fn info(
        &self,
        store: &dyn DbCapabilityStoreApi,
        file: &ImmutableFileRef,
    ) -> Result<Value> {
        let record = self.validated_record(store, file).await?;
        Ok(json!({
            "id": record.id,
            "size": record.size,
            "sha256": record.sha256,
            "contentType": record.content_type,
            "purpose": record.purpose,
            "createdAt": record.created_at,
        }))
    }

    pub async fn delete(
        &self,
        store: &dyn DbCapabilityStoreApi,
        file: &ImmutableFileRef,
    ) -> Result<()> {
        store.delete_skiff_file_by_id(&file.id).await?;
        Ok(())
    }

    async fn persist_staged_file(
        &self,
        store: &dyn DbCapabilityStoreApi,
        staged: StagedFile,
        options: FileCreateOptions,
    ) -> Result<Value> {
        let key = cas_blob_key(&staged.sha256, staged.size);
        let blob_store = self.require_blob_store()?;
        let source = staged.blob_body();
        let content_type = options.content_type.clone();
        let put_key = key.clone();
        let put_result = tokio::task::spawn_blocking(move || {
            blob_store.put_if_absent(PutBlobRequest {
                key: put_key,
                body: source,
                metadata: BlobMetadata {
                    content_type,
                    cache_control: SKIFF_FILE_CACHE_CONTROL.to_string(),
                },
            })
        })
        .await
        .map_err(|error| storage_unavailable(format!("blob put task failed: {error}")))?
        .map_err(|error| storage_unavailable(format!("put blob {key} failed: {error}")));
        staged.cleanup().await;
        put_result?;

        let id = uuid::Uuid::new_v4().to_string();
        let record = FileCapabilityRecord {
            id: id.clone(),
            sha256: staged.sha256.clone(),
            size: i64::try_from(staged.size).map_err(|_| {
                RuntimeError::Decode("std.file size does not fit runtime integer".to_string())
            })?,
            content_type: options.content_type.clone(),
            purpose: options.purpose,
            blob_key: key,
            created_at: time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .map_err(|error| {
                    RuntimeError::Decode(format!("createdAt encode failed: {error}"))
                })?,
        };
        store.insert_skiff_file_record(record).await?;
        Ok(immutable_file_wire(ImmutableFileRef {
            id,
            size: i64::try_from(staged.size).map_err(|_| {
                RuntimeError::Decode("std.file size does not fit runtime integer".to_string())
            })?,
            sha256: staged.sha256.clone(),
            content_type: options.content_type,
        }))
    }

    async fn validated_record(
        &self,
        store: &dyn DbCapabilityStoreApi,
        file: &ImmutableFileRef,
    ) -> Result<FileCapabilityRecord> {
        let record = store
            .find_skiff_file_by_id(&file.id)
            .await?
            .ok_or_else(|| file_not_found(&file.id))?;
        if record.sha256 != file.sha256 || record.size != file.size {
            return Err(storage_corruption(format!(
                "std.file {} fat ref identity does not match _skiff_file record",
                file.id
            )));
        }
        Ok(record)
    }

    fn require_blob_store(&self) -> Result<Arc<dyn BlobStore>> {
        self.blob_store
            .clone()
            .ok_or_else(|| storage_unavailable("blob store is not configured for this runtime"))
    }
}

impl FileCapabilityRuntime for FileRuntime {
    fn create_file<'a>(
        &'a self,
        store: &'a dyn DbCapabilityStoreApi,
        input: Bytes,
        options: FileCreateOptions,
    ) -> HostCapabilityFuture<'a, Value> {
        Box::pin(FileRuntime::create(self, store, input, options))
    }

    fn read_file<'a>(
        &'a self,
        store: &'a dyn DbCapabilityStoreApi,
        file: &'a ImmutableFileRef,
    ) -> HostCapabilityFuture<'a, Bytes> {
        Box::pin(FileRuntime::read(self, store, file))
    }

    fn file_info<'a>(
        &'a self,
        store: &'a dyn DbCapabilityStoreApi,
        file: &'a ImmutableFileRef,
    ) -> HostCapabilityFuture<'a, Value> {
        Box::pin(FileRuntime::info(self, store, file))
    }

    fn delete_file<'a>(
        &'a self,
        store: &'a dyn DbCapabilityStoreApi,
        file: &'a ImmutableFileRef,
    ) -> HostCapabilityFuture<'a, ()> {
        Box::pin(FileRuntime::delete(self, store, file))
    }

    fn create_file_from_chunks<'a>(
        &'a self,
        store: &'a dyn DbCapabilityStoreApi,
        options: FileCreateOptions,
        mut next_chunk: FileChunkSource<'a>,
    ) -> HostCapabilityFuture<'a, Value> {
        Box::pin(async move {
            FileRuntime::create_from_chunks(self, store, options, move || next_chunk()).await
        })
    }
}

pub fn file_not_found(id: &str) -> RuntimeError {
    RuntimeError::file_error(format!("std.file not found: {id}"))
}

pub fn storage_unavailable(reason: impl Into<String>) -> RuntimeError {
    RuntimeError::ProviderUnavailable {
        target: "std.file.storage".to_string(),
        reason: reason.into(),
    }
}

pub fn storage_corruption(message: impl Into<String>) -> RuntimeError {
    RuntimeError::Decode(format!("std.file storage corruption: {}", message.into()))
}

fn cas_blob_key(sha256: &str, size: u64) -> String {
    format!("{sha256}-{size}")
}

struct FileIngest<'a> {
    tmp_root: &'a Path,
    hasher: Sha256,
    size: u64,
    memory: Vec<u8>,
    tmp_path: Option<PathBuf>,
    tmp_file: Option<File>,
}

impl<'a> FileIngest<'a> {
    fn new(tmp_root: &'a Path) -> Self {
        Self {
            tmp_root,
            hasher: Sha256::new(),
            size: 0,
            memory: Vec::new(),
            tmp_path: None,
            tmp_file: None,
        }
    }

    async fn push(&mut self, chunk: &[u8]) -> Result<()> {
        let chunk_len = u64::try_from(chunk.len()).map_err(|_| {
            RuntimeError::resource_limit_exceeded(
                "std.file",
                "file chunk length does not fit u64",
                FILE_GUARD_LIMIT_BYTES,
                usize::MAX,
                0,
            )
        })?;
        let next_size = self.size.checked_add(chunk_len).ok_or_else(|| {
            RuntimeError::resource_limit_exceeded(
                "std.file",
                "file size overflow",
                FILE_GUARD_LIMIT_BYTES,
                usize::MAX,
                0,
            )
        })?;
        if next_size > FILE_GUARD_LIMIT_BYTES as u64 {
            self.cleanup().await;
            return Err(RuntimeError::resource_limit_exceeded(
                "std.file",
                "file body exceeds 64 MiB guard limit",
                FILE_GUARD_LIMIT_BYTES,
                self.size.min(usize::MAX as u64) as usize,
                chunk.len(),
            ));
        }

        self.hasher.update(chunk);
        self.size = next_size;
        if let Some(file) = self.tmp_file.as_mut() {
            file.write_all(chunk).await.map_err(tmp_error)?;
            return Ok(());
        }
        if self.memory.len().saturating_add(chunk.len()) <= FILE_MEMORY_FAST_PATH_BYTES {
            self.memory.extend_from_slice(chunk);
            return Ok(());
        }

        self.open_tmp().await?;
        let file = self
            .tmp_file
            .as_mut()
            .ok_or_else(|| storage_unavailable("runtime tmp file was not opened"))?;
        if !self.memory.is_empty() {
            file.write_all(&self.memory).await.map_err(tmp_error)?;
            self.memory.clear();
        }
        file.write_all(chunk).await.map_err(tmp_error)?;
        Ok(())
    }

    async fn finish(mut self) -> Result<StagedFile> {
        if let Some(file) = self.tmp_file.as_mut() {
            file.flush().await.map_err(tmp_error)?;
            file.sync_all().await.map_err(tmp_error)?;
        }
        let tmp_path = self.tmp_path.take();
        Ok(StagedFile {
            sha256: hex::encode(self.hasher.finalize()),
            size: self.size,
            memory: if tmp_path.is_none() {
                Some(Bytes::from(std::mem::take(&mut self.memory)))
            } else {
                None
            },
            tmp_path,
        })
    }

    async fn open_tmp(&mut self) -> Result<()> {
        fs::create_dir_all(self.tmp_root).await.map_err(tmp_error)?;
        let path = self
            .tmp_root
            .join(format!("skiff-file-{}", uuid::Uuid::new_v4()));
        let file = File::create_new(&path).await.map_err(tmp_error)?;
        self.tmp_path = Some(path);
        self.tmp_file = Some(file);
        Ok(())
    }

    async fn cleanup(&mut self) {
        self.tmp_file.take();
        if let Some(path) = self.tmp_path.take() {
            let _ = fs::remove_file(path).await;
        }
    }
}

struct StagedFile {
    sha256: String,
    size: u64,
    memory: Option<Bytes>,
    tmp_path: Option<PathBuf>,
}

impl StagedFile {
    fn blob_body(&self) -> BlobBody {
        match (&self.memory, &self.tmp_path) {
            (Some(bytes), _) => BlobBody::Bytes(bytes.to_vec()),
            (None, Some(path)) => BlobBody::TempFile(path.clone()),
            (None, None) => BlobBody::Bytes(Vec::new()),
        }
    }

    async fn cleanup(&self) {
        if let Some(path) = &self.tmp_path {
            let _ = fs::remove_file(path).await;
        }
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        if let Some(path) = self.tmp_path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn tmp_error(error: std::io::Error) -> RuntimeError {
    storage_unavailable(format!("runtime tmp file operation failed: {error}"))
}

#[cfg(test)]
mod tests;
