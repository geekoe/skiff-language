use std::{future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;
use serde_json::Value;
use skiff_runtime_boundary::file::{
    bytes_wire, file_decode_error, FileCreateOptions, ImmutableFileRef,
};
use skiff_runtime_capability_context::{
    DbCapabilityContext, DbCapabilityStore, DbCapabilityStoreApi, FileCapabilityResult,
    FileChunkSource, StreamPoll,
};

use crate::error::Result;
use skiff_runtime_request::ExecutionControl;

use super::StreamRuntime;

pub type HostCapabilityFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub trait FileCapabilityRuntime: Send + Sync {
    fn create_file<'a>(
        &'a self,
        store: &'a dyn DbCapabilityStoreApi,
        input: Bytes,
        options: FileCreateOptions,
    ) -> HostCapabilityFuture<'a, Value>;

    fn read_file<'a>(
        &'a self,
        store: &'a dyn DbCapabilityStoreApi,
        file: &'a ImmutableFileRef,
    ) -> HostCapabilityFuture<'a, Bytes>;

    fn file_info<'a>(
        &'a self,
        store: &'a dyn DbCapabilityStoreApi,
        file: &'a ImmutableFileRef,
    ) -> HostCapabilityFuture<'a, Value>;

    fn delete_file<'a>(
        &'a self,
        store: &'a dyn DbCapabilityStoreApi,
        file: &'a ImmutableFileRef,
    ) -> HostCapabilityFuture<'a, ()>;

    fn create_file_from_chunks<'a>(
        &'a self,
        store: &'a dyn DbCapabilityStoreApi,
        options: FileCreateOptions,
        next_chunk: FileChunkSource<'a>,
    ) -> HostCapabilityFuture<'a, Value>;
}

#[derive(Clone)]
pub struct FileCapabilitySource {
    file_runtime: Arc<dyn FileCapabilityRuntime>,
}

impl FileCapabilitySource {
    pub fn new<T>(file_runtime: Arc<T>) -> Self
    where
        T: FileCapabilityRuntime + 'static,
    {
        Self::from_runtime(file_runtime)
    }

    fn from_runtime(file_runtime: Arc<dyn FileCapabilityRuntime>) -> Self {
        Self { file_runtime }
    }

    pub fn context_for_request(&self, db_context: DbCapabilityContext) -> FileCapabilityContext {
        FileCapabilityContext::from_runtime(self.file_runtime.clone(), db_context)
    }
}

#[derive(Clone)]
pub struct FileCapabilityContext {
    file_runtime: Arc<dyn FileCapabilityRuntime>,
    db_context: DbCapabilityContext,
}

impl FileCapabilityContext {
    #[cfg(any(test, feature = "test-support"))]
    pub fn new<T>(file_runtime: Arc<T>, db_context: DbCapabilityContext) -> Self
    where
        T: FileCapabilityRuntime + 'static,
    {
        Self::from_runtime(file_runtime, db_context)
    }

    fn from_runtime(
        file_runtime: Arc<dyn FileCapabilityRuntime>,
        db_context: DbCapabilityContext,
    ) -> Self {
        Self {
            file_runtime,
            db_context,
        }
    }

    pub fn source(&self) -> FileCapabilitySource {
        FileCapabilitySource::from_runtime(self.file_runtime.clone())
    }

    fn require_store(&self, target: &str, unavailable_reason: &str) -> Result<DbCapabilityStore> {
        self.db_context
            .require_store(target, unavailable_reason)
            .map_err(Into::into)
    }

    pub async fn create_file(
        &self,
        target: &str,
        input: Bytes,
        options: FileCreateOptions,
    ) -> Result<Value> {
        let store = self.require_file_store(target)?;
        self.file_runtime
            .create_file(store.as_api(), input, options)
            .await
    }

    pub async fn read_file_wire(&self, target: &str, file: &ImmutableFileRef) -> Result<Value> {
        let bytes = self.read_file_bytes(target, file).await?;
        Ok(bytes_wire(&bytes))
    }

    pub async fn read_text_file(&self, target: &str, file: &ImmutableFileRef) -> Result<Value> {
        let bytes = self.read_file_bytes(target, file).await?;
        let text = String::from_utf8(bytes.to_vec()).map_err(|error| {
            file_decode_error(format!("readText payload is not UTF-8: {error}"))
        })?;
        Ok(Value::String(text))
    }

    pub async fn file_info(&self, target: &str, file: &ImmutableFileRef) -> Result<Value> {
        let store = self.require_file_store(target)?;
        self.file_runtime.file_info(store.as_api(), file).await
    }

    pub async fn delete_file(&self, target: &str, file: &ImmutableFileRef) -> Result<()> {
        let store = self.require_file_store(target)?;
        self.file_runtime.delete_file(store.as_api(), file).await
    }

    pub async fn create_file_from_chunks<'a, I, F>(
        &self,
        target: &str,
        options: FileCreateOptions,
        next_chunk: I,
    ) -> Result<Value>
    where
        I: FnMut() -> F + Send + 'a,
        F: Future<Output = FileCapabilityResult<Option<Bytes>>> + Send + 'a,
    {
        let store = self.require_file_store(target)?;
        let mut next_chunk = next_chunk;
        self.file_runtime
            .create_file_from_chunks(
                store.as_api(),
                options,
                Box::new(move || Box::pin(next_chunk())),
            )
            .await
    }

    async fn read_file_bytes(&self, target: &str, file: &ImmutableFileRef) -> Result<Bytes> {
        let store = self.require_file_store(target)?;
        self.file_runtime.read_file(store.as_api(), file).await
    }

    fn require_file_store(&self, target: &str) -> Result<DbCapabilityStore> {
        self.require_store(
            target,
            "serviceDb is not configured for std.file operations",
        )
    }
}

#[derive(Clone)]
pub struct FileSourceStreamContext<'a> {
    source_stream: StreamRuntime,
    source_execution: ExecutionControl<'a>,
}

impl<'a> FileSourceStreamContext<'a> {
    pub fn new(source_stream: StreamRuntime, source_execution: ExecutionControl<'a>) -> Self {
        Self {
            source_stream,
            source_execution,
        }
    }

    pub fn stream_runtime_handle(&self) -> StreamRuntime {
        self.source_stream.clone()
    }

    pub async fn next_file_source_stream_item(
        &self,
        stream: &Value,
    ) -> FileCapabilityResult<Option<Value>> {
        let cancellation =
            skiff_runtime_capability_context::CancellationSignals::from_tokens([self
                .source_execution
                .cancellation_token()]);
        match self
            .source_stream
            .next_with_cancellation(stream, &[], &cancellation)
            .await?
        {
            StreamPoll::End => Ok(None),
            StreamPoll::Item(item) => Ok(Some(item)),
        }
    }
}
