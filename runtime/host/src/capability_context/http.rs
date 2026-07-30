use super::{
    EffectDispatchContext, HttpEffectContext, PreparedTestHttpSelfIngress, TestEffectDoubleContext,
};
use skiff_runtime_capability_context::{HttpRuntimeOptions, StreamRuntime};

#[derive(Clone)]
pub struct HttpClientCapabilityContext {
    effects: EffectDispatchContext,
    http_options: HttpRuntimeOptions,
    stream_runtime: StreamRuntime,
    test_effect_doubles: TestEffectDoubleContext,
}

impl HttpClientCapabilityContext {
    pub fn new(
        effects: EffectDispatchContext,
        http_options: HttpRuntimeOptions,
        stream_runtime: StreamRuntime,
        test_effect_doubles: TestEffectDoubleContext,
    ) -> Self {
        Self {
            effects,
            http_options,
            stream_runtime,
            test_effect_doubles,
        }
    }

    pub fn http(&self) -> &HttpEffectContext {
        self.effects.http()
    }

    pub fn http_options(&self) -> HttpRuntimeOptions {
        self.http_options.clone()
    }

    pub fn stream_runtime(&self) -> &StreamRuntime {
        &self.stream_runtime
    }

    pub fn with_stream_runtime(&self, stream_runtime: StreamRuntime) -> Self {
        Self {
            effects: self.effects.clone(),
            http_options: self.http_options.clone(),
            stream_runtime,
            test_effect_doubles: self.test_effect_doubles.clone(),
        }
    }

    pub fn test_effect_double_context(&self) -> TestEffectDoubleContext {
        self.test_effect_doubles.clone()
    }

    pub(crate) fn prepare_test_http_self_ingress(
        &self,
        input: &serde_json::Value,
    ) -> crate::error::Result<Option<PreparedTestHttpSelfIngress>> {
        self.effects
            .test_http_self_ingress()
            .map_or(Ok(None), |context| context.prepare(input))
    }

    pub(crate) fn is_test_http_self_ingress(&self, input: &serde_json::Value) -> bool {
        self.effects
            .test_http_self_ingress()
            .is_some_and(|context| context.matches(input))
    }
}
