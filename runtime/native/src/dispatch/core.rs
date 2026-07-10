use super::{
    actor::ActorNativeDispatch, bytes::BytesNativeDispatch, external::ExternalNativeDispatch,
    file::FileNativeDispatch, http::HttpNativeDispatch, invocation::RuntimeNativeInvocation,
    json::JsonNativeDispatch, resource::ResourceNativeDispatch, telemetry::TelemetryNativeDispatch,
    time::TimeNativeDispatch, websocket::WebsocketNativeDispatch,
};
use crate::error::{Result, RuntimeError};
use crate::{
    capability::{
        NativeActorCapability, NativeFileCapabilityBundle, NativeHttpClientCapability,
        NativeHttpResponseStreamCapability, NativeTelemetryCapability, NativeTimeCapability,
        NativeWebsocketCapability, NativeResourceCapability,
    },
    registry::NativeRegistry,
    runtime_value_facade::{RequestHeap, RuntimeValue},
};
use skiff_runtime_capability_context::NativeCapabilityContexts;
use skiff_runtime_native_contract::{NativeRequiredContext, NativeSignatureRegistry};

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
    NativeRegistry,
    ReceiverMethod,
}

pub fn runtime_shared_native_route(target: &str) -> Option<RuntimeNativeRoute> {
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
    if is_runtime_receiver_native_binding_key(target) {
        return Some(RuntimeNativeRoute::ReceiverMethod);
    }
    NativeRegistry
        .is_registered(target)
        .then_some(RuntimeNativeRoute::NativeRegistry)
}

fn is_runtime_receiver_native_binding_key(binding_key: &str) -> bool {
    NativeSignatureRegistry::builtins()
        .signature(binding_key)
        .is_some_and(|signature| {
            matches!(
                signature.target,
                "Date.toEpochMilliseconds"
                    | "Date.toISOString"
                    | "Date.addMilliseconds"
                    | "Date.diffMilliseconds"
                    | "Date.compare"
                    | "Date.isBefore"
                    | "Date.isAfter"
                    | "Duration.toMilliseconds"
            )
        })
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
    ActorContext: NativeActorCapability,
    FileContext: NativeFileCapabilityBundle,
    TimeContext: NativeTimeCapability,
    HttpClientContext: NativeHttpClientCapability,
    HttpResponseStreamContext: NativeHttpResponseStreamCapability,
    WebsocketContext: NativeWebsocketCapability,
    TelemetryContext: NativeTelemetryCapability,
    ResourceContext: NativeResourceCapability,
{
    let binding_key = invocation.binding_key();
    let diagnostic_target = invocation.target_name();
    if BytesNativeDispatch::matches(binding_key) {
        ensure_native_capability_context(
            binding_key,
            NativeRequiredContext::None,
            native_capability_context.required_context(),
        )?;
        return BytesNativeDispatch::dispatch_native_call(
            &invocation,
            diagnostic_target,
            args,
            heap,
        );
    }
    if JsonNativeDispatch::matches(binding_key) {
        ensure_native_capability_context(
            binding_key,
            NativeRequiredContext::None,
            native_capability_context.required_context(),
        )?;
        return JsonNativeDispatch::dispatch(&invocation, diagnostic_target, args, heap);
    }
    if TimeNativeDispatch::matches(binding_key) {
        let time_context = match native_capability_context {
            NativeCapabilityContexts::Time(time_context) => time_context,
            other => {
                return Err(native_capability_route_mismatch(
                    binding_key,
                    NativeRequiredContext::Time,
                    other.required_context(),
                ));
            }
        };
        return TimeNativeDispatch::dispatch(
            &time_context,
            &invocation,
            diagnostic_target,
            args,
            heap,
        )
        .await;
    }
    if FileNativeDispatch::matches(binding_key) {
        let (file_context, file_source_stream_context, request_heap_limits) =
            match native_capability_context {
                NativeCapabilityContexts::File(file_context) => {
                    file_context.into_native_file_parts()
                }
                other => {
                    return Err(native_capability_route_mismatch(
                        binding_key,
                        NativeRequiredContext::File,
                        other.required_context(),
                    ));
                }
            };
        return FileNativeDispatch::dispatch(
            &file_context,
            &file_source_stream_context,
            request_heap_limits,
            &invocation,
            diagnostic_target,
            args,
            heap,
        )
        .await;
    }
    if HttpNativeDispatch::matches(binding_key) {
        return HttpNativeDispatch::new()
            .dispatch(
                native_capability_context,
                &invocation,
                diagnostic_target,
                args,
                heap,
            )
            .await;
    }
    if WebsocketNativeDispatch::matches(binding_key) {
        let websocket_context = match native_capability_context {
            NativeCapabilityContexts::Websocket(websocket_context) => websocket_context,
            other => {
                return Err(native_capability_route_mismatch(
                    binding_key,
                    NativeRequiredContext::Websocket,
                    other.required_context(),
                ));
            }
        };
        return WebsocketNativeDispatch::dispatch(
            &websocket_context,
            &invocation,
            diagnostic_target,
            args,
            heap,
        );
    }
    if TelemetryNativeDispatch::matches(binding_key) {
        let telemetry_context = match native_capability_context {
            NativeCapabilityContexts::Telemetry(telemetry_context) => telemetry_context,
            other => {
                return Err(native_capability_route_mismatch(
                    binding_key,
                    NativeRequiredContext::Telemetry,
                    other.required_context(),
                ));
            }
        };
        return TelemetryNativeDispatch::dispatch(
            &telemetry_context,
            &invocation,
            diagnostic_target,
            args,
            heap,
        );
    }
    if ResourceNativeDispatch::matches(binding_key) {
        let resource_context = match native_capability_context {
            NativeCapabilityContexts::Resource(resource_context) => resource_context,
            other => {
                return Err(native_capability_route_mismatch(
                    binding_key,
                    NativeRequiredContext::Resource,
                    other.required_context(),
                ));
            }
        };
        return ResourceNativeDispatch::dispatch(
            &resource_context,
            &invocation,
            diagnostic_target,
            args,
            heap,
        );
    }
    if ActorNativeDispatch::matches(binding_key) {
        let actor_context = match native_capability_context {
            NativeCapabilityContexts::Actor(actor_context) => actor_context,
            other => {
                return Err(native_capability_route_mismatch(
                    binding_key,
                    NativeRequiredContext::Actor,
                    other.required_context(),
                ));
            }
        };
        return ActorNativeDispatch::dispatch(
            &actor_context,
            &invocation,
            diagnostic_target,
            args,
            heap,
        )
        .await;
    }

    ExternalNativeDispatch::dispatch_native_call(&invocation, diagnostic_target, args, heap)
}
