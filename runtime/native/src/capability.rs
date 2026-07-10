use std::{future::Future, pin::Pin, sync::OnceLock};

use bytes::Bytes;
use serde_json::Value;
use skiff_runtime_boundary::file::{FileCreateOptions, ImmutableFileRef};
use skiff_runtime_capability_context::{
    ActorFindControlRequest, ActorPutControlRequest, ActorRemoveControlRequest,
    FileCapabilityFuture, FileChunkFuture, FileChunkSource,
};
use skiff_runtime_model::addr::ExecutableAddr;
use skiff_runtime_model::{PublicationResourceTable, RuntimeProgramResourceView};

use crate::error::Result;
use crate::runtime_value_facade::{
    ActorRef, RequestHeap, RequestHeapLimits, RuntimeTypePlan, RuntimeValue,
};

pub type NativeCapabilityFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
pub type NativeFileChunkFuture<'a> = FileChunkFuture<'a>;
pub type NativeFileChunkSource<'a> = FileChunkSource<'a>;

pub trait NativeConfigCapability {
    fn read_config_target(
        &self,
        current_addr: &ExecutableAddr,
        target: &str,
        args: &[Value],
        type_arg: Option<&RuntimeTypePlan>,
    ) -> Result<Value>;
}

pub trait NativeActorCapability {
    fn service_id(&self) -> &str;

    fn put_actor<'a>(
        &'a self,
        request: ActorPutControlRequest,
        object_payload: Vec<u8>,
    ) -> NativeCapabilityFuture<'a, ActorRef>;

    fn find_actor<'a>(
        &'a self,
        request: ActorFindControlRequest,
    ) -> NativeCapabilityFuture<'a, Option<ActorRef>>;

    fn remove_actor<'a>(
        &'a self,
        request: ActorRemoveControlRequest,
    ) -> NativeCapabilityFuture<'a, bool>;
}

pub trait NativeFileCapability: Clone + Send {
    fn create_file<'a>(
        &'a self,
        target: &'a str,
        input: Bytes,
        options: FileCreateOptions,
    ) -> NativeCapabilityFuture<'a, Value>;

    fn read_file_wire<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> NativeCapabilityFuture<'a, Value>;

    fn read_text_file<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> NativeCapabilityFuture<'a, Value>;

    fn file_info<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> NativeCapabilityFuture<'a, Value>;

    fn delete_file<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> NativeCapabilityFuture<'a, ()>;

    fn create_file_from_chunks<'a>(
        &'a self,
        target: &'a str,
        options: FileCreateOptions,
        next_chunk: NativeFileChunkSource<'a>,
    ) -> NativeCapabilityFuture<'a, Value>;
}

pub trait NativeFileSourceStreamCapability: Clone + Send {
    fn next_file_source_stream_item<'a>(
        &'a self,
        stream: &'a Value,
    ) -> FileCapabilityFuture<'a, Option<Value>>;
}

pub trait NativeFileCapabilityBundle {
    type File: NativeFileCapability;
    type FileSourceStream: NativeFileSourceStreamCapability;

    fn into_native_file_parts(self) -> (Self::File, Self::FileSourceStream, RequestHeapLimits);
}

pub trait NativeTimeCapability {
    fn poll_execution_budget(&self) -> Result<()>;
}

pub trait NativeHttpClientCapability {
    fn dispatch_test_http_effect_invocation_double(
        &self,
        target: &str,
        input: Option<&RuntimeValue>,
        arg_plan: Option<&RuntimeTypePlan>,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>>;

    fn dispatch_http_request<'a>(&'a self, input: &'a Value) -> NativeCapabilityFuture<'a, Value>;

    fn dispatch_http_stream<'a>(
        &'a self,
        input: &'a Value,
        expected_body_item_type: Option<&'a RuntimeTypePlan>,
    ) -> NativeCapabilityFuture<'a, Value>;

    fn dispatch_http_sse<'a>(
        &'a self,
        input: &'a Value,
        expected_item_type: Option<&'a RuntimeTypePlan>,
    ) -> NativeCapabilityFuture<'a, Value>;
}

pub trait NativeHttpResponseStreamCapability {
    fn response_item_type(&self, target: &str) -> Result<RuntimeTypePlan>;

    fn send_response_event<'a>(
        &'a self,
        target: &'a str,
        event: Value,
    ) -> NativeCapabilityFuture<'a, ()>;
}

pub trait NativeWebsocketCapability {
    fn send_connection_text_to_business_identity(
        &self,
        business_identity: String,
        text: String,
    ) -> Result<()>;

    fn send_connection_binary_to_business_identity(
        &self,
        business_identity: String,
        payload: Vec<u8>,
    ) -> Result<()>;

    fn send_connection_text_to_connection(&self, connection_id: String, text: String)
        -> Result<()>;

    fn send_connection_binary_to_connection(
        &self,
        connection_id: String,
        payload: Vec<u8>,
    ) -> Result<()>;
}

pub trait NativeTelemetryCapability {
    fn emit_native(&self, target: &str, args: &[Value]) -> Result<Value>;
}

pub trait NativeResourceCapability {
    fn resources(&self) -> RuntimeProgramResourceView<'_>;
}

impl NativeResourceCapability for () {
    fn resources(&self) -> RuntimeProgramResourceView<'_> {
        static EMPTY: OnceLock<(PublicationResourceTable, Vec<PublicationResourceTable>)> =
            OnceLock::new();
        let (service_resources, package_resources) =
            EMPTY.get_or_init(|| (PublicationResourceTable::default(), Vec::new()));
        RuntimeProgramResourceView::new(service_resources, package_resources)
    }
}
