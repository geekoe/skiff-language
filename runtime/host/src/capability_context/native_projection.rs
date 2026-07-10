//! Runtime concrete context constructors for promoted native capability traits.

use super::{
    ActorCapabilityContext, ActorClient, ConfigCapabilityContext, FileCapabilityContext,
    FileSourceStreamContext, HttpClientCapabilityContext, HttpResponseStreamCapabilityContext,
    TelemetryCapabilityContext, TimeCapabilityContext, WebsocketCapabilityContext,
};
use crate::error as runtime_error;
use bytes::Bytes;
use promoted_runtime::{
    capability::{
        NativeActorCapability, NativeCapabilityFuture, NativeConfigCapability,
        NativeFileCapability, NativeFileCapabilityBundle, NativeFileChunkSource,
        NativeFileSourceStreamCapability, NativeHttpClientCapability,
        NativeHttpResponseStreamCapability, NativeTelemetryCapability, NativeTimeCapability,
        NativeWebsocketCapability,
    },
    error::{BudgetReason as NativeBudgetReason, Result, RuntimeError},
};
use serde_json::Value;
use skiff_runtime_boundary::file::{FileCreateOptions, ImmutableFileRef};
use skiff_runtime_capability_context::{
    ActorFindControlRequest, ActorPutControlRequest, ActorRemoveControlRequest,
    FileCapabilityFuture, NativeFileCapabilityContext as ContractNativeFileCapabilityContext,
    NativeHttpClientCapabilityContext as ContractNativeHttpClientCapabilityContext,
    NativeHttpResponseStreamCapabilityContext as ContractNativeHttpResponseStreamCapabilityContext,
    NativeTelemetryCapabilityContext as ContractNativeTelemetryCapabilityContext,
};
use skiff_runtime_model::{
    addr::ExecutableAddr,
    error::WirePayload,
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{ActorRef, RuntimeValue},
    type_plan::RuntimeTypePlan,
};
use skiff_runtime_native as promoted_runtime;

pub struct RuntimeNativeFileCapabilityContext<'execution>(
    ContractNativeFileCapabilityContext<
        FileCapabilityContext,
        FileSourceStreamContext<'execution>,
        RequestHeapLimits,
    >,
);

impl<'execution> RuntimeNativeFileCapabilityContext<'execution> {
    pub fn new(
        file_context: FileCapabilityContext,
        file_source_stream_context: FileSourceStreamContext<'execution>,
        request_heap_limits: RequestHeapLimits,
    ) -> Self {
        Self(ContractNativeFileCapabilityContext::new(
            file_context,
            file_source_stream_context,
            request_heap_limits,
        ))
    }
}

pub struct RuntimeNativeTimeCapabilityContext<'execution>(TimeCapabilityContext<'execution>);

impl<'execution> RuntimeNativeTimeCapabilityContext<'execution> {
    pub fn new(context: TimeCapabilityContext<'execution>) -> Self {
        Self(context)
    }
}

#[derive(Clone)]
pub struct RuntimeNativeHttpClientCapabilityContext(
    ContractNativeHttpClientCapabilityContext<HttpClientCapabilityContext>,
);

impl RuntimeNativeHttpClientCapabilityContext {
    pub fn new(context: HttpClientCapabilityContext) -> Self {
        Self(ContractNativeHttpClientCapabilityContext::new(context))
    }
}

#[derive(Clone)]
pub struct RuntimeNativeHttpResponseStreamCapabilityContext<'execution>(
    ContractNativeHttpResponseStreamCapabilityContext<
        HttpResponseStreamCapabilityContext<'execution>,
    >,
);

impl<'execution> RuntimeNativeHttpResponseStreamCapabilityContext<'execution> {
    pub fn new(context: HttpResponseStreamCapabilityContext<'execution>) -> Self {
        Self(ContractNativeHttpResponseStreamCapabilityContext::new(
            context,
        ))
    }
}

#[derive(Clone)]
pub struct RuntimeNativeTelemetryCapabilityContext(
    ContractNativeTelemetryCapabilityContext<TelemetryCapabilityContext>,
);

impl RuntimeNativeTelemetryCapabilityContext {
    pub fn new(context: TelemetryCapabilityContext) -> Self {
        Self(ContractNativeTelemetryCapabilityContext::new(context))
    }
}

trait IntoNativeResult<T> {
    fn into_native_result(self) -> Result<T>;
}

impl<T> IntoNativeResult<T> for runtime_error::Result<T> {
    fn into_native_result(self) -> Result<T> {
        self.map_err(runtime_error_to_native)
    }
}

impl<T> IntoNativeResult<T> for skiff_runtime_capability_context::ExecutionControlResult<T> {
    fn into_native_result(self) -> Result<T> {
        self.map_err(execution_control_error_to_native)
    }
}

fn execution_control_error_to_native(
    error: skiff_runtime_capability_context::ExecutionControlError,
) -> RuntimeError {
    match error {
        skiff_runtime_capability_context::ExecutionControlError::Cancelled => {
            RuntimeError::Cancelled
        }
        skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(failure) => {
            if failure.reason == skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled
            {
                RuntimeError::Cancelled
            } else {
                RuntimeError::ExecutionBudgetExceeded {
                    reason: native_capability_budget_reason(failure.reason),
                    instruction_count: failure.instruction_count,
                    limit: failure.limit,
                    elapsed_ms: failure.elapsed_ms,
                }
            }
        }
    }
}

fn native_capability_budget_reason(
    reason: skiff_runtime_capability_context::ExecutionBudgetReason,
) -> NativeBudgetReason {
    match reason {
        skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled => {
            NativeBudgetReason::Cancelled
        }
        skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded => {
            NativeBudgetReason::DeadlineExceeded
        }
        skiff_runtime_capability_context::ExecutionBudgetReason::InstructionLimitExceeded => {
            NativeBudgetReason::InstructionLimitExceeded
        }
    }
}

fn runtime_error_to_native(error: runtime_error::RuntimeError) -> RuntimeError {
    match error {
        runtime_error::RuntimeError::Opaque(error) => {
            if let Some(control) = wire_payload_to_native_control(error.as_ref()) {
                control
            } else {
                RuntimeError::Opaque(error)
            }
        }
        runtime_error::RuntimeError::Diagnosed(diagnosed) => {
            RuntimeError::Opaque(Box::new(diagnosed))
        }
        error => RuntimeError::Opaque(Box::new(error)),
    }
}

fn wire_payload_to_native_control(error: &dyn WirePayload) -> Option<RuntimeError> {
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::ExecutionControlError>()
    {
        return Some(execution_control_error_to_native(*error));
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::StreamRuntimeError>()
    {
        return stream_error_to_native_control(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_request::RequestError>()
    {
        return request_error_to_native_control(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_eval::error::RuntimeError>()
    {
        return eval_error_to_native_control(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_native::error::RuntimeError>()
    {
        return native_error_to_native_control(error);
    }
    None
}

fn stream_error_to_native_control(
    error: &skiff_runtime_capability_context::StreamRuntimeError,
) -> Option<RuntimeError> {
    match error {
        skiff_runtime_capability_context::StreamRuntimeError::Cancelled => {
            Some(RuntimeError::Cancelled)
        }
        skiff_runtime_capability_context::StreamRuntimeError::Producer(error) => {
            wire_payload_to_native_control(error.as_ref())
        }
        skiff_runtime_capability_context::StreamRuntimeError::Decode(_) => None,
    }
}

fn request_error_to_native_control(
    error: &skiff_runtime_request::RequestError,
) -> Option<RuntimeError> {
    match error {
        skiff_runtime_request::RequestError::Cancelled => Some(RuntimeError::Cancelled),
        skiff_runtime_request::RequestError::ExecutionBudgetExceeded {
            reason,
            instruction_count,
            limit,
            elapsed_ms,
        } => {
            if *reason == skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled {
                Some(RuntimeError::Cancelled)
            } else {
                Some(RuntimeError::ExecutionBudgetExceeded {
                    reason: native_capability_budget_reason(*reason),
                    instruction_count: *instruction_count,
                    limit: *limit,
                    elapsed_ms: *elapsed_ms,
                })
            }
        }
        skiff_runtime_request::RequestError::Eval(error) => eval_error_to_native_control(error),
        _ => None,
    }
}

fn eval_error_to_native_control(
    error: &skiff_runtime_eval::error::RuntimeError,
) -> Option<RuntimeError> {
    match error {
        skiff_runtime_eval::error::RuntimeError::Cancelled => Some(RuntimeError::Cancelled),
        skiff_runtime_eval::error::RuntimeError::ExecutionBudgetExceeded {
            reason,
            instruction_count,
            limit,
            elapsed_ms,
        } => {
            if *reason == skiff_runtime_eval::error::BudgetReason::Cancelled {
                Some(RuntimeError::Cancelled)
            } else {
                Some(RuntimeError::ExecutionBudgetExceeded {
                    reason: eval_budget_reason_to_native(*reason),
                    instruction_count: *instruction_count,
                    limit: *limit,
                    elapsed_ms: *elapsed_ms,
                })
            }
        }
        skiff_runtime_eval::error::RuntimeError::Opaque(error) => {
            wire_payload_to_native_control(error.as_ref())
        }
        _ => None,
    }
}

fn native_error_to_native_control(
    error: &skiff_runtime_native::error::RuntimeError,
) -> Option<RuntimeError> {
    match error {
        skiff_runtime_native::error::RuntimeError::Cancelled => Some(RuntimeError::Cancelled),
        skiff_runtime_native::error::RuntimeError::ExecutionBudgetExceeded {
            reason,
            instruction_count,
            limit,
            elapsed_ms,
        } => Some(RuntimeError::ExecutionBudgetExceeded {
            reason: *reason,
            instruction_count: *instruction_count,
            limit: *limit,
            elapsed_ms: *elapsed_ms,
        }),
        skiff_runtime_native::error::RuntimeError::Opaque(error) => {
            wire_payload_to_native_control(error.as_ref())
        }
        _ => None,
    }
}

fn eval_budget_reason_to_native(
    reason: skiff_runtime_eval::error::BudgetReason,
) -> NativeBudgetReason {
    match reason {
        skiff_runtime_eval::error::BudgetReason::Cancelled => NativeBudgetReason::Cancelled,
        skiff_runtime_eval::error::BudgetReason::DeadlineExceeded => {
            NativeBudgetReason::DeadlineExceeded
        }
        skiff_runtime_eval::error::BudgetReason::InstructionLimitExceeded => {
            NativeBudgetReason::InstructionLimitExceeded
        }
    }
}

impl<'execution> NativeConfigCapability for ConfigCapabilityContext<'execution> {
    fn read_config_target(
        &self,
        current_addr: &ExecutableAddr,
        target: &str,
        args: &[Value],
        type_arg: Option<&RuntimeTypePlan>,
    ) -> Result<Value> {
        ConfigCapabilityContext::read_config_target(self, current_addr, target, args, type_arg)
            .into_native_result()
    }
}

impl<'execution> NativeActorCapability for ActorCapabilityContext<'execution> {
    fn service_id(&self) -> &str {
        self.service_id()
    }

    fn put_actor<'a>(
        &'a self,
        request: ActorPutControlRequest,
        object_payload: Vec<u8>,
    ) -> NativeCapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            ActorClient::new(self.clone())
                .put(request, object_payload)
                .await
                .into_native_result()
        })
    }

    fn find_actor<'a>(
        &'a self,
        request: ActorFindControlRequest,
    ) -> NativeCapabilityFuture<'a, Option<ActorRef>> {
        Box::pin(async move {
            ActorClient::new(self.clone())
                .find(request)
                .await
                .into_native_result()
        })
    }

    fn remove_actor<'a>(
        &'a self,
        request: ActorRemoveControlRequest,
    ) -> NativeCapabilityFuture<'a, bool> {
        Box::pin(async move {
            ActorClient::new(self.clone())
                .remove(request)
                .await
                .into_native_result()
        })
    }
}

impl<'execution> NativeFileCapabilityBundle for RuntimeNativeFileCapabilityContext<'execution> {
    type File = FileCapabilityContext;
    type FileSourceStream = FileSourceStreamContext<'execution>;

    fn into_native_file_parts(self) -> (Self::File, Self::FileSourceStream, RequestHeapLimits) {
        self.0.into_parts()
    }
}

impl NativeFileCapability for FileCapabilityContext {
    fn create_file<'a>(
        &'a self,
        target: &'a str,
        input: Bytes,
        options: FileCreateOptions,
    ) -> NativeCapabilityFuture<'a, Value> {
        Box::pin(async move {
            FileCapabilityContext::create_file(self, target, input, options)
                .await
                .into_native_result()
        })
    }

    fn read_file_wire<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> NativeCapabilityFuture<'a, Value> {
        Box::pin(async move {
            FileCapabilityContext::read_file_wire(self, target, file)
                .await
                .into_native_result()
        })
    }

    fn read_text_file<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> NativeCapabilityFuture<'a, Value> {
        Box::pin(async move {
            FileCapabilityContext::read_text_file(self, target, file)
                .await
                .into_native_result()
        })
    }

    fn file_info<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> NativeCapabilityFuture<'a, Value> {
        Box::pin(async move {
            FileCapabilityContext::file_info(self, target, file)
                .await
                .into_native_result()
        })
    }

    fn delete_file<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> NativeCapabilityFuture<'a, ()> {
        Box::pin(async move {
            FileCapabilityContext::delete_file(self, target, file)
                .await
                .into_native_result()
        })
    }

    fn create_file_from_chunks<'a>(
        &'a self,
        target: &'a str,
        options: FileCreateOptions,
        mut next_chunk: NativeFileChunkSource<'a>,
    ) -> NativeCapabilityFuture<'a, Value> {
        Box::pin(async move {
            FileCapabilityContext::create_file_from_chunks(self, target, options, move || {
                next_chunk()
            })
            .await
            .into_native_result()
        })
    }
}

impl<'execution> NativeFileSourceStreamCapability for FileSourceStreamContext<'execution> {
    fn next_file_source_stream_item<'a>(
        &'a self,
        stream: &'a Value,
    ) -> FileCapabilityFuture<'a, Option<Value>> {
        Box::pin(async move {
            FileSourceStreamContext::next_file_source_stream_item(self, stream).await
        })
    }
}

impl<'execution> NativeTimeCapability for RuntimeNativeTimeCapabilityContext<'execution> {
    fn poll_execution_budget(&self) -> Result<()> {
        self.0
            .execution_control()
            .poll_execution_budget()
            .into_native_result()
    }
}

impl NativeHttpClientCapability for HttpClientCapabilityContext {
    fn dispatch_test_http_effect_invocation_double(
        &self,
        target: &str,
        input: Option<&RuntimeValue>,
        arg_plan: Option<&RuntimeTypePlan>,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        HttpClientCapabilityContext::dispatch_test_http_effect_invocation_double(
            self,
            target,
            input,
            arg_plan,
            return_plan,
            heap,
        )
        .map(IntoNativeResult::into_native_result)
    }

    fn dispatch_http_request<'a>(&'a self, input: &'a Value) -> NativeCapabilityFuture<'a, Value> {
        Box::pin(async move {
            HttpClientCapabilityContext::dispatch_http_request(self, input)
                .await
                .into_native_result()
        })
    }

    fn dispatch_http_stream<'a>(
        &'a self,
        input: &'a Value,
        expected_body_item_type: Option<&'a RuntimeTypePlan>,
    ) -> NativeCapabilityFuture<'a, Value> {
        Box::pin(async move {
            HttpClientCapabilityContext::dispatch_http_stream(self, input, expected_body_item_type)
                .await
                .into_native_result()
        })
    }

    fn dispatch_http_sse<'a>(
        &'a self,
        input: &'a Value,
        expected_item_type: Option<&'a RuntimeTypePlan>,
    ) -> NativeCapabilityFuture<'a, Value> {
        Box::pin(async move {
            HttpClientCapabilityContext::dispatch_http_sse(self, input, expected_item_type)
                .await
                .into_native_result()
        })
    }
}

impl NativeHttpClientCapability for RuntimeNativeHttpClientCapabilityContext {
    fn dispatch_test_http_effect_invocation_double(
        &self,
        target: &str,
        input: Option<&RuntimeValue>,
        arg_plan: Option<&RuntimeTypePlan>,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        self.0
            .clone()
            .into_effect_context()
            .dispatch_test_http_effect_invocation_double(target, input, arg_plan, return_plan, heap)
            .map(IntoNativeResult::into_native_result)
    }

    fn dispatch_http_request<'a>(&'a self, input: &'a Value) -> NativeCapabilityFuture<'a, Value> {
        let context = self.0.clone().into_effect_context();
        Box::pin(async move {
            context
                .dispatch_http_request(input)
                .await
                .into_native_result()
        })
    }

    fn dispatch_http_stream<'a>(
        &'a self,
        input: &'a Value,
        expected_body_item_type: Option<&'a RuntimeTypePlan>,
    ) -> NativeCapabilityFuture<'a, Value> {
        let context = self.0.clone().into_effect_context();
        Box::pin(async move {
            context
                .dispatch_http_stream(input, expected_body_item_type)
                .await
                .into_native_result()
        })
    }

    fn dispatch_http_sse<'a>(
        &'a self,
        input: &'a Value,
        expected_item_type: Option<&'a RuntimeTypePlan>,
    ) -> NativeCapabilityFuture<'a, Value> {
        let context = self.0.clone().into_effect_context();
        Box::pin(async move {
            context
                .dispatch_http_sse(input, expected_item_type)
                .await
                .into_native_result()
        })
    }
}

impl<'execution> NativeHttpResponseStreamCapability
    for HttpResponseStreamCapabilityContext<'execution>
{
    fn response_item_type(&self, target: &str) -> Result<RuntimeTypePlan> {
        HttpResponseStreamCapabilityContext::response_item_type(self, target)
            .cloned()
            .into_native_result()
    }

    fn send_response_event<'a>(
        &'a self,
        target: &'a str,
        event: Value,
    ) -> NativeCapabilityFuture<'a, ()> {
        Box::pin(async move {
            HttpResponseStreamCapabilityContext::send_response_event(self, target, event)
                .await
                .into_native_result()
        })
    }
}

impl<'execution> NativeHttpResponseStreamCapability
    for RuntimeNativeHttpResponseStreamCapabilityContext<'execution>
{
    fn response_item_type(&self, target: &str) -> Result<RuntimeTypePlan> {
        self.0
            .clone()
            .into_execution_context()
            .response_item_type(target)
            .cloned()
            .into_native_result()
    }

    fn send_response_event<'a>(
        &'a self,
        target: &'a str,
        event: Value,
    ) -> NativeCapabilityFuture<'a, ()> {
        let context = self.0.clone().into_execution_context();
        Box::pin(async move {
            context
                .send_response_event(target, event)
                .await
                .into_native_result()
        })
    }
}

impl<'execution> NativeWebsocketCapability for WebsocketCapabilityContext<'execution> {
    fn send_connection_text_to_business_identity(
        &self,
        business_identity: String,
        text: String,
    ) -> Result<()> {
        WebsocketCapabilityContext::send_connection_text_to_business_identity(
            self,
            business_identity,
            text,
        )
        .into_native_result()
    }

    fn send_connection_binary_to_business_identity(
        &self,
        business_identity: String,
        payload: Vec<u8>,
    ) -> Result<()> {
        WebsocketCapabilityContext::send_connection_binary_to_business_identity(
            self,
            business_identity,
            payload,
        )
        .into_native_result()
    }

    fn send_connection_text_to_connection(
        &self,
        connection_id: String,
        text: String,
    ) -> Result<()> {
        WebsocketCapabilityContext::send_connection_text_to_connection(self, connection_id, text)
            .into_native_result()
    }

    fn send_connection_binary_to_connection(
        &self,
        connection_id: String,
        payload: Vec<u8>,
    ) -> Result<()> {
        WebsocketCapabilityContext::send_connection_binary_to_connection(
            self,
            connection_id,
            payload,
        )
        .into_native_result()
    }
}

impl NativeTelemetryCapability for TelemetryCapabilityContext {
    fn emit_native(&self, target: &str, args: &[Value]) -> Result<Value> {
        TelemetryCapabilityContext::emit_native(self, target, args).into_native_result()
    }
}

impl NativeTelemetryCapability for RuntimeNativeTelemetryCapabilityContext {
    fn emit_native(&self, target: &str, args: &[Value]) -> Result<Value> {
        self.0
            .clone()
            .into_effect_context()
            .emit_native(target, args)
            .into_native_result()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn diagnosed_db_decode_projects_to_native_opaque_with_frames() {
        let error = runtime_error::RuntimeError::Opaque(Box::new(
            skiff_runtime_service_db::ServiceDbError::db_decode(
                "std.db",
                "db value missing key field id",
            ),
        ))
        .with_source(7, json!({ "sourceId": 7 }));

        let native = runtime_error_to_native(error);

        match native {
            RuntimeError::Opaque(error) => {
                let payload = error.payload();
                assert_eq!(payload.code, "std.db.DecodeError");
                assert_eq!(payload.message, "db value missing key field id");
                assert_eq!(
                    payload.details.expect("diagnostic details should exist")["sourceId"].as_u64(),
                    Some(7)
                );
            }
            error => panic!("expected native Opaque, got {error:?}"),
        }
    }

    #[test]
    fn diagnosed_lease_lost_projects_to_native_opaque_with_frames() {
        let error = runtime_error::RuntimeError::Opaque(Box::new(
            skiff_runtime_service_db::ServiceDbError::LeaseLost("lease abc was lost".to_string()),
        ))
        .with_diagnostic_frame(json!({ "sourceId": 7 }));

        let native = runtime_error_to_native(error);

        match native {
            RuntimeError::Opaque(error) => {
                let payload = error.payload();
                assert_eq!(payload.code, "LeaseLost");
                assert_eq!(payload.message, "lease abc was lost");
                assert_eq!(
                    payload.details.expect("diagnostic details should exist")["frames"][0]
                        ["sourceId"]
                        .as_u64(),
                    Some(7)
                );
            }
            error => panic!("expected native Opaque, got {error:?}"),
        }
    }

    #[test]
    fn host_small_root_projects_to_native_opaque() {
        let native = runtime_error_to_native(runtime_error::RuntimeError::Decode(
            "internal invariant failed".to_string(),
        ));

        match native {
            RuntimeError::Opaque(error) => {
                let payload = error.payload();
                assert_eq!(payload.code, "InternalError");
                assert_eq!(payload.message, "internal invariant failed");
            }
            error => panic!("expected native Opaque, got {error:?}"),
        }
    }
}
