use std::{error::Error, fmt, future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;
use serde_json::json;
use serde_json::Value;
use skiff_runtime_boundary::file::{FileCreateOptions, ImmutableFileRef};
use skiff_runtime_model::error::{RuntimeErrorPayload, TypeIdentity, WirePayload};

use crate::{
    DbCapabilityContext, ExecutionControl, ExecutionControlError, StreamRuntime, StreamRuntimeError,
};

#[derive(Debug)]
pub enum FileCapabilityError {
    Decode(String),
    File(String),
    Opaque(Box<dyn WirePayload>),
    ProviderUnavailable {
        target: String,
        reason: String,
    },
    ResourceLimitExceeded {
        resource: String,
        reason: String,
        limit: usize,
        current: usize,
        requested_delta: usize,
    },
    Stream(StreamRuntimeError),
    Execution(ExecutionControlError),
}

impl FileCapabilityError {
    pub fn decode(message: impl Into<String>) -> Self {
        Self::Decode(message.into())
    }

    pub fn file(message: impl Into<String>) -> Self {
        Self::File(message.into())
    }

    pub fn opaque(error: impl WirePayload) -> Self {
        Self::Opaque(Box::new(error))
    }

    pub fn provider_unavailable(target: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::ProviderUnavailable {
            target: target.into(),
            reason: reason.into(),
        }
    }

    pub fn resource_limit_exceeded(
        resource: impl Into<String>,
        reason: impl Into<String>,
        limit: usize,
        current: usize,
        requested_delta: usize,
    ) -> Self {
        Self::ResourceLimitExceeded {
            resource: resource.into(),
            reason: reason.into(),
            limit,
            current,
            requested_delta,
        }
    }
}

impl fmt::Display for FileCapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(message) | Self::File(message) => formatter.write_str(message),
            Self::Opaque(error) => error.fmt(formatter),
            Self::ProviderUnavailable { target, reason } => {
                write!(formatter, "provider unavailable for {target}: {reason}")
            }
            Self::ResourceLimitExceeded {
                resource, reason, ..
            } => write!(
                formatter,
                "resource limit exceeded for {resource}: {reason}"
            ),
            Self::Stream(error) => error.fmt(formatter),
            Self::Execution(error) => error.fmt(formatter),
        }
    }
}

impl Error for FileCapabilityError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Stream(error) => Some(error),
            Self::Execution(error) => Some(error),
            Self::Opaque(error) => Some(error.as_ref()),
            Self::Decode(_)
            | Self::File(_)
            | Self::ProviderUnavailable { .. }
            | Self::ResourceLimitExceeded { .. } => None,
        }
    }
}

impl WirePayload for FileCapabilityError {
    fn payload(&self) -> RuntimeErrorPayload {
        match self {
            Self::Decode(message) => RuntimeErrorPayload {
                code: "InternalError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            Self::File(message) => RuntimeErrorPayload {
                code: "std.file.FileError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            Self::Opaque(error) => error.payload(),
            Self::ProviderUnavailable { target, reason } => RuntimeErrorPayload {
                code: "std.service.ProviderUnavailableError".to_string(),
                message: reason.clone(),
                status: None,
                details: Some(json!({
                    "target": target,
                    "reason": reason,
                })),
            },
            Self::ResourceLimitExceeded {
                resource,
                reason,
                limit,
                current,
                requested_delta,
            } => RuntimeErrorPayload {
                code: "ResourceLimitExceeded".to_string(),
                message: format!("resource limit exceeded for {resource}: {reason}"),
                status: None,
                details: Some(json!({
                    "resource": resource,
                    "reason": reason,
                    "limit": limit,
                    "current": current,
                    "requestedDelta": requested_delta,
                })),
            },
            Self::Stream(error) => error.payload(),
            Self::Execution(error) => error.payload(),
        }
    }

    fn catch_projection(&self) -> Option<(TypeIdentity, serde_json::Value)> {
        match self {
            Self::File(message) => Some((
                TypeIdentity::builtin("std.file.FileError"),
                json!({
                    "message": message,
                }),
            )),
            Self::Opaque(error) => error.catch_projection(),
            Self::ProviderUnavailable { target, reason } => Some((
                TypeIdentity::builtin("std.service.ProviderUnavailableError"),
                json!({
                    "target": target,
                    "reason": reason,
                }),
            )),
            Self::Stream(error) => error.catch_projection(),
            Self::Execution(error) => error.catch_projection(),
            Self::Decode(_) | Self::ResourceLimitExceeded { .. } => None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl From<StreamRuntimeError> for FileCapabilityError {
    fn from(error: StreamRuntimeError) -> Self {
        Self::Stream(error)
    }
}

impl From<ExecutionControlError> for FileCapabilityError {
    fn from(error: ExecutionControlError) -> Self {
        Self::Execution(error)
    }
}

pub type FileCapabilityResult<T> = Result<T, FileCapabilityError>;
pub type FileCapabilityFuture<'a, T> =
    Pin<Box<dyn Future<Output = FileCapabilityResult<T>> + Send + 'a>>;
pub type FileChunkFuture<'a> =
    Pin<Box<dyn Future<Output = FileCapabilityResult<Option<Bytes>>> + Send + 'a>>;
pub type FileChunkSource<'a> = Box<dyn FnMut() -> FileChunkFuture<'a> + Send + 'a>;

pub trait FileCapabilitySourceApi: Send + Sync {
    fn context_for_request(&self, db_context: DbCapabilityContext) -> FileCapabilityContext;
}

#[derive(Clone)]
pub struct FileCapabilitySource {
    inner: Arc<dyn FileCapabilitySourceApi>,
}

impl FileCapabilitySource {
    pub fn new<T>(inner: T) -> Self
    where
        T: FileCapabilitySourceApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn context_for_request(&self, db_context: DbCapabilityContext) -> FileCapabilityContext {
        self.inner.context_for_request(db_context)
    }
}

pub trait FileCapabilityApi: Send + Sync {
    fn source(&self) -> FileCapabilitySource;

    fn create_file<'a>(
        &'a self,
        target: &'a str,
        input: Bytes,
        options: FileCreateOptions,
    ) -> FileCapabilityFuture<'a, Value>;

    fn read_file_wire<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> FileCapabilityFuture<'a, Value>;

    fn read_text_file<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> FileCapabilityFuture<'a, Value>;

    fn file_info<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> FileCapabilityFuture<'a, Value>;

    fn delete_file<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> FileCapabilityFuture<'a, ()>;

    fn create_file_from_chunks<'a>(
        &'a self,
        target: &'a str,
        options: FileCreateOptions,
        next_chunk: FileChunkSource<'a>,
    ) -> FileCapabilityFuture<'a, Value>;
}

#[derive(Clone)]
pub struct FileCapabilityContext {
    inner: Arc<dyn FileCapabilityApi>,
}

impl FileCapabilityContext {
    pub fn new<T>(inner: T) -> Self
    where
        T: FileCapabilityApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn source(&self) -> FileCapabilitySource {
        self.inner.source()
    }

    pub async fn create_file(
        &self,
        target: &str,
        input: Bytes,
        options: FileCreateOptions,
    ) -> FileCapabilityResult<Value> {
        self.inner.create_file(target, input, options).await
    }

    pub async fn read_file_wire(
        &self,
        target: &str,
        file: &ImmutableFileRef,
    ) -> FileCapabilityResult<Value> {
        self.inner.read_file_wire(target, file).await
    }

    pub async fn read_text_file(
        &self,
        target: &str,
        file: &ImmutableFileRef,
    ) -> FileCapabilityResult<Value> {
        self.inner.read_text_file(target, file).await
    }

    pub async fn file_info(
        &self,
        target: &str,
        file: &ImmutableFileRef,
    ) -> FileCapabilityResult<Value> {
        self.inner.file_info(target, file).await
    }

    pub async fn delete_file(
        &self,
        target: &str,
        file: &ImmutableFileRef,
    ) -> FileCapabilityResult<()> {
        self.inner.delete_file(target, file).await
    }

    pub async fn create_file_from_chunks<'a>(
        &'a self,
        target: &'a str,
        options: FileCreateOptions,
        next_chunk: FileChunkSource<'a>,
    ) -> FileCapabilityResult<Value> {
        self.inner
            .create_file_from_chunks(target, options, next_chunk)
            .await
    }
}

pub trait FileSourceStreamApi: Send + Sync {
    fn stream_runtime_handle(&self) -> StreamRuntime;
    fn next_file_source_stream_item<'a>(
        &'a self,
        stream: &'a Value,
    ) -> FileCapabilityFuture<'a, Option<Value>>;
}

#[derive(Clone)]
pub struct FileSourceStreamContext<'a> {
    inner: Arc<dyn FileSourceStreamApi + 'a>,
}

impl FileSourceStreamContext<'static> {
    pub fn new(stream_runtime: StreamRuntime, execution: ExecutionControl<'_>) -> Self {
        execution.file_source_stream_context(stream_runtime)
    }
}

impl<'a> FileSourceStreamContext<'a> {
    pub fn from_api<T>(inner: T) -> Self
    where
        T: FileSourceStreamApi + 'a,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn stream_runtime_handle(&self) -> StreamRuntime {
        self.inner.stream_runtime_handle()
    }

    pub fn next_file_source_stream_item<'b>(
        &'b self,
        stream: &'b Value,
    ) -> FileCapabilityFuture<'b, Option<Value>> {
        self.inner.next_file_source_stream_item(stream)
    }
}
