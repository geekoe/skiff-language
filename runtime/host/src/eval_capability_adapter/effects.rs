use super::*;
use skiff_runtime_model::type_plan::RuntimeTypePlan;

pub(super) struct RuntimeEffectDispatchContext(pub(super) concrete::EffectDispatchContext);

impl eval_capabilities::EffectDispatchApi for RuntimeEffectDispatchContext {
    fn telemetry_context(&self) -> eval_capabilities::TelemetryCapabilityContext {
        eval_capabilities::TelemetryCapabilityContext::new(RuntimeTelemetryCapabilityContext(
            self.0.telemetry_context(),
        ))
    }

    fn http_client_context(
        &self,
        http_options: eval_capabilities::HttpRuntimeOptions,
        stream_runtime: eval_capabilities::StreamRuntime,
        test_effect_doubles: eval_capabilities::TestEffectDoubleContext,
    ) -> eval_capabilities::HttpClientCapabilityContext {
        eval_capabilities::HttpClientCapabilityContext::new(RuntimeHttpClientCapabilityContext(
            concrete::HttpClientCapabilityContext::new(
                self.0.clone(),
                self.0
                    .http_options()
                    .with_allow_unsafe_targets(http_options.allow_unsafe_targets()),
                concrete_stream_runtime(&stream_runtime).clone(),
                concrete_test_effect_double_context(&test_effect_doubles).clone(),
            ),
        ))
    }
}

#[derive(Clone)]
pub(super) struct RuntimeTestEffectDoubleContext(pub(super) concrete::TestEffectDoubleContext);

impl eval_capabilities::TestEffectDoubleContextApi for RuntimeTestEffectDoubleContext {
    fn next_test_effect_double(&self, target: &str) -> Option<eval_capabilities::TestEffectDouble> {
        self.0.next_test_effect_double(target).map(eval_test_double)
    }

    fn dispatch_test_effect_double(
        &self,
        target: &str,
        input: Option<&Value>,
    ) -> Option<Result<Value>> {
        self.0
            .dispatch_test_effect_double(target, input)
            .map(IntoEvalResult::into_eval_result)
    }

    fn dispatch_test_stable_target_double(
        &self,
        target: &str,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        self.0
            .dispatch_test_stable_target_double(target, return_plan, heap)
            .map(IntoEvalResult::into_eval_result)
    }

    fn dispatch_test_host_operation_double(
        &self,
        target: &str,
        input: Option<&RuntimeValue>,
        arg_plan: Option<&RuntimeTypePlan>,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        self.0
            .dispatch_test_host_operation_double(target, input, arg_plan, return_plan, heap)
            .map(IntoEvalResult::into_eval_result)
    }

    fn dispatch_test_http_effect_invocation_double(
        &self,
        target: &str,
        input: Option<&RuntimeValue>,
        arg_plan: Option<&RuntimeTypePlan>,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        self.0
            .dispatch_test_http_effect_invocation_double(target, input, arg_plan, return_plan, heap)
            .map(IntoEvalResult::into_eval_result)
    }
}
