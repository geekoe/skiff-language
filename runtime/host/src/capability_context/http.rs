use super::{EffectDispatchContext, HttpEffectContext, StreamRuntime, TestEffectDoubleContext};
use skiff_runtime_capability_context::HttpRuntimeOptions;

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

    pub fn test_effect_double_context(&self) -> TestEffectDoubleContext {
        self.test_effect_doubles.clone()
    }
}
