use super::{
    actor::ActorNativeDispatch, bytes::BytesNativeDispatch, external::ExternalNativeDispatch,
    file::FileNativeDispatch, http::HttpNativeDispatch, invocation::RuntimeNativeInvocation,
    json::JsonNativeDispatch, prepared::run_prepared_native_call, resource::ResourceNativeDispatch,
    task::TaskControlNativeDispatch, telemetry::TelemetryNativeDispatch, time::TimeNativeDispatch,
    websocket::WebsocketNativeDispatch, PreparedNativeCall,
};
use crate::error::{Result, RuntimeError};
use crate::{
    capability::{
        NativeActorCapability, NativeFileCapabilityBundle, NativeHttpClientCapability,
        NativeHttpResponseStreamCapability, NativeResourceCapability, NativeTelemetryCapability,
        NativeTimeCapability, NativeWebsocketCapability,
    },
    registry::NativeRegistry,
    runtime_value_facade::{RequestHeap, RuntimeValue},
};
use skiff_runtime_capability_context::NativeCapabilityContexts;
use skiff_runtime_native_contract::NativeRequiredContext;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RuntimeNativeRoute {
    Actor,
    Bytes,
    File,
    Json,
    Time,
    Http,
    Websocket,
    Telemetry,
    Resource,
    TaskControl,
    NativeRegistry,
    ReceiverMethod,
}

pub fn runtime_shared_native_route(target: &str) -> Option<RuntimeNativeRoute> {
    runtime_shared_native_route_for_validation(target, NativeRegistry.is_registered(target))
}

pub(crate) fn runtime_shared_native_route_for_validation(
    target: &str,
    native_registry_registered: bool,
) -> Option<RuntimeNativeRoute> {
    if ActorNativeDispatch::matches(target) {
        return Some(RuntimeNativeRoute::Actor);
    }
    if BytesNativeDispatch::matches(target) {
        return Some(RuntimeNativeRoute::Bytes);
    }
    if FileNativeDispatch::matches(target) {
        return Some(RuntimeNativeRoute::File);
    }
    if JsonNativeDispatch::matches(target) {
        return Some(RuntimeNativeRoute::Json);
    }
    if TimeNativeDispatch::matches(target) {
        return Some(RuntimeNativeRoute::Time);
    }
    if HttpNativeDispatch::matches(target) {
        return Some(RuntimeNativeRoute::Http);
    }
    if WebsocketNativeDispatch::matches(target) {
        return Some(RuntimeNativeRoute::Websocket);
    }
    if TelemetryNativeDispatch::matches(target) {
        return Some(RuntimeNativeRoute::Telemetry);
    }
    if ResourceNativeDispatch::matches(target) {
        return Some(RuntimeNativeRoute::Resource);
    }
    if TaskControlNativeDispatch::matches(target) {
        return Some(RuntimeNativeRoute::TaskControl);
    }
    if skiff_artifact_model::is_runtime_receiver_native_binding_key(target) {
        return Some(RuntimeNativeRoute::ReceiverMethod);
    }
    native_registry_registered.then_some(RuntimeNativeRoute::NativeRegistry)
}

pub(super) fn native_capability_route_mismatch(
    binding_key: &str,
    expected_context: NativeRequiredContext,
    actual_context: NativeRequiredContext,
) -> RuntimeError {
    RuntimeError::InvalidArtifact(format!(
        "native binding {binding_key} routed with {actual_context:?} capability context, expected {expected_context:?}"
    ))
}

pub(super) fn ensure_native_capability_context(
    binding_key: &str,
    expected_context: NativeRequiredContext,
    actual_context: NativeRequiredContext,
) -> Result<()> {
    if actual_context == expected_context {
        Ok(())
    } else {
        Err(native_capability_route_mismatch(
            binding_key,
            expected_context,
            actual_context,
        ))
    }
}

pub(super) fn unsupported_native_target(target_or_callee: &str) -> RuntimeError {
    RuntimeError::Unsupported(format!("unsupported native target {target_or_callee}"))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_resolved_native_call<
    'a,
    ActorContext,
    FileContext,
    TimeContext,
    HttpClientContext,
    HttpResponseStreamContext,
    WebsocketContext,
    TelemetryContext,
    ResourceContext,
>(
    native_capability_context: NativeCapabilityContexts<
        ActorContext,
        FileContext,
        TimeContext,
        HttpClientContext,
        HttpResponseStreamContext,
        WebsocketContext,
        TelemetryContext,
        ResourceContext,
    >,
    invocation: RuntimeNativeInvocation,
    args: Vec<RuntimeValue>,
    heap: &mut RequestHeap,
) -> Result<PreparedNativeCall<'a>>
where
    ActorContext: NativeActorCapability + Send + 'a,
    FileContext: NativeFileCapabilityBundle,
    <FileContext as NativeFileCapabilityBundle>::File: 'a,
    <FileContext as NativeFileCapabilityBundle>::FileSourceStream: 'a,
    TimeContext: NativeTimeCapability + Send + 'a,
    HttpClientContext: NativeHttpClientCapability + Send + 'a,
    HttpResponseStreamContext: NativeHttpResponseStreamCapability + Send + 'a,
    WebsocketContext: NativeWebsocketCapability + Send + 'a,
    TelemetryContext: NativeTelemetryCapability,
    ResourceContext: NativeResourceCapability,
{
    let binding_key = invocation.binding_key().to_string();
    let diagnostic_target = invocation.target_name().to_string();
    if BytesNativeDispatch::matches(&binding_key) {
        ensure_native_capability_context(
            &binding_key,
            NativeRequiredContext::None,
            native_capability_context.required_context(),
        )?;
        let value =
            BytesNativeDispatch::dispatch_native_call(&invocation, &diagnostic_target, args, heap)?;
        return Ok(PreparedNativeCall::Ready(value));
    }
    if JsonNativeDispatch::matches(&binding_key) {
        ensure_native_capability_context(
            &binding_key,
            NativeRequiredContext::None,
            native_capability_context.required_context(),
        )?;
        let value = JsonNativeDispatch::dispatch(&invocation, &diagnostic_target, args, heap)?;
        return Ok(PreparedNativeCall::Ready(value));
    }
    if TimeNativeDispatch::matches(&binding_key) {
        let time_context = match native_capability_context {
            NativeCapabilityContexts::Time(time_context) => time_context,
            other => {
                return Err(native_capability_route_mismatch(
                    &binding_key,
                    NativeRequiredContext::Time,
                    other.required_context(),
                ));
            }
        };
        return TimeNativeDispatch::prepare(
            time_context,
            invocation,
            diagnostic_target,
            args,
            heap,
        );
    }
    if FileNativeDispatch::matches(&binding_key) {
        let (file_context, file_source_stream_context, request_heap_limits) =
            match native_capability_context {
                NativeCapabilityContexts::File(file_context) => {
                    file_context.into_native_file_parts()
                }
                other => {
                    return Err(native_capability_route_mismatch(
                        &binding_key,
                        NativeRequiredContext::File,
                        other.required_context(),
                    ));
                }
            };
        return FileNativeDispatch::prepare(
            file_context,
            file_source_stream_context,
            request_heap_limits,
            invocation,
            diagnostic_target,
            args,
            heap,
        );
    }
    if HttpNativeDispatch::matches(&binding_key) {
        return HttpNativeDispatch::new().prepare(
            native_capability_context,
            invocation,
            diagnostic_target,
            args,
            heap,
        );
    }
    if WebsocketNativeDispatch::matches(&binding_key) {
        let websocket_context = match native_capability_context {
            NativeCapabilityContexts::Websocket(websocket_context) => websocket_context,
            other => {
                return Err(native_capability_route_mismatch(
                    &binding_key,
                    NativeRequiredContext::Websocket,
                    other.required_context(),
                ));
            }
        };
        return WebsocketNativeDispatch::prepare(
            websocket_context,
            invocation,
            diagnostic_target,
            args,
            heap,
        );
    }
    if TelemetryNativeDispatch::matches(&binding_key) {
        let telemetry_context = match native_capability_context {
            NativeCapabilityContexts::Telemetry(telemetry_context) => telemetry_context,
            other => {
                return Err(native_capability_route_mismatch(
                    &binding_key,
                    NativeRequiredContext::Telemetry,
                    other.required_context(),
                ));
            }
        };
        let value = TelemetryNativeDispatch::dispatch(
            &telemetry_context,
            &invocation,
            &diagnostic_target,
            args,
            heap,
        )?;
        return Ok(PreparedNativeCall::Ready(value));
    }
    if ResourceNativeDispatch::matches(&binding_key) {
        let resource_context = match native_capability_context {
            NativeCapabilityContexts::Resource(resource_context) => resource_context,
            other => {
                return Err(native_capability_route_mismatch(
                    &binding_key,
                    NativeRequiredContext::Resource,
                    other.required_context(),
                ));
            }
        };
        let value = ResourceNativeDispatch::dispatch(
            &resource_context,
            &invocation,
            &diagnostic_target,
            args,
            heap,
        )?;
        return Ok(PreparedNativeCall::Ready(value));
    }
    if ActorNativeDispatch::matches(&binding_key) {
        let actor_context = match native_capability_context {
            NativeCapabilityContexts::Actor(actor_context) => actor_context,
            other => {
                return Err(native_capability_route_mismatch(
                    &binding_key,
                    NativeRequiredContext::Actor,
                    other.required_context(),
                ));
            }
        };
        return ActorNativeDispatch::prepare(
            actor_context,
            invocation,
            diagnostic_target,
            args,
            heap,
        );
    }

    let value =
        ExternalNativeDispatch::dispatch_native_call(&invocation, &diagnostic_target, args, heap)?;
    Ok(PreparedNativeCall::Ready(value))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn dispatch_resolved_native_call<
    ActorContext,
    FileContext,
    TimeContext,
    HttpClientContext,
    HttpResponseStreamContext,
    WebsocketContext,
    TelemetryContext,
    ResourceContext,
>(
    native_capability_context: NativeCapabilityContexts<
        ActorContext,
        FileContext,
        TimeContext,
        HttpClientContext,
        HttpResponseStreamContext,
        WebsocketContext,
        TelemetryContext,
        ResourceContext,
    >,
    invocation: RuntimeNativeInvocation,
    args: Vec<RuntimeValue>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue>
where
    ActorContext: NativeActorCapability + Send,
    FileContext: NativeFileCapabilityBundle,
    TimeContext: NativeTimeCapability + Send,
    HttpClientContext: NativeHttpClientCapability + Send,
    HttpResponseStreamContext: NativeHttpResponseStreamCapability + Send,
    WebsocketContext: NativeWebsocketCapability + Send,
    TelemetryContext: NativeTelemetryCapability,
    ResourceContext: NativeResourceCapability,
{
    let prepared = prepare_resolved_native_call(native_capability_context, invocation, args, heap)?;
    run_prepared_native_call(prepared, heap).await
}
