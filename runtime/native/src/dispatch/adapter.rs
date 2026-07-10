use super::{
    builtin::BuiltinDispatch, bytes::BytesNativeDispatch, config::ConfigNativeDispatch, core,
    external::ExternalNativeDispatch, invocation::RuntimeNativeInvocation,
};
use crate::error::{Result, RuntimeError};
use crate::{
    capability::{
        NativeActorCapability, NativeConfigCapability, NativeFileCapabilityBundle,
        NativeHttpClientCapability, NativeHttpResponseStreamCapability, NativeResourceCapability,
        NativeTelemetryCapability, NativeTimeCapability, NativeWebsocketCapability,
    },
    runtime_value_facade::{RequestHeap, RuntimeTypePlan, RuntimeValue},
};
use skiff_runtime_capability_context::NativeCapabilityContexts;
use skiff_runtime_model::addr::ExecutableAddr;

pub struct NativeDispatch;

impl NativeDispatch {
    pub fn new() -> Self {
        Self
    }

    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_builtin(
        &self,
        config_context: &impl NativeConfigCapability,
        current_addr: &ExecutableAddr,
        op: &str,
        config_type_arg_plan: Option<RuntimeTypePlan>,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        match op {
            target if ConfigNativeDispatch::matches(target) => {
                ConfigNativeDispatch::dispatch_builtin(
                    config_context,
                    current_addr,
                    target,
                    config_type_arg_plan,
                    &args,
                    heap,
                )
            }
            target if BuiltinDispatch::matches(target) => {
                BuiltinDispatch::dispatch(target, args, heap)
            }
            target if BytesNativeDispatch::matches(target) => {
                // Builtin dispatch can run without caller context, so keep its legacy
                // direct path. RuntimeProgram native calls go through dispatch_native_call.
                BytesNativeDispatch::dispatch(target, args, heap)
            }
            target if ExternalNativeDispatch::is_registered(target) => {
                ExternalNativeDispatch::dispatch(target, args, heap)
            }
            other => Err(RuntimeError::Unsupported(format!(
                "unsupported RuntimeProgram builtin {other}"
            ))),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn dispatch_resolved_native_call<
        ActorContext,
        FileContext,
        TimeContext,
        HttpClientContext,
        HttpResponseStreamContext,
        WebsocketContext,
        TelemetryContext,
        ResourceContext,
    >(
        &self,
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
        core::dispatch_resolved_native_call(native_capability_context, invocation, args, heap).await
    }
}
