use super::{EffectDispatchContext, HttpEffectContext, TestEffectDoubleContext};
use skiff_runtime_capability_context::{HttpRuntimeOptions, StreamRuntime};

#[derive(Clone)]
pub struct HttpClientCapabilityContext {
    effects: EffectDispatchContext,
    http_options: HttpRuntimeOptions,
    stream_runtime: Option<StreamRuntime>,
    test_effect_doubles: Option<TestEffectDoubleContext>,
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
            stream_runtime: Some(stream_runtime),
            test_effect_doubles: Some(test_effect_doubles),
        }
    }

    /// Constructs the production HTTP context used by the bytecode provider.
    ///
    /// A request effect does not need a stream runtime, and the exact stream
    /// call installs the request's ResourceTable-backed runtime with
    /// [`Self::with_stream_runtime`]. Keeping both fields absent here makes the
    /// legacy host stream registry and test-double registry unreachable from
    /// the bytecode path rather than merely leaving them empty.
    pub(crate) fn production(
        effects: EffectDispatchContext,
        http_options: HttpRuntimeOptions,
    ) -> Self {
        Self {
            effects,
            http_options,
            stream_runtime: None,
            test_effect_doubles: None,
        }
    }

    pub fn http(&self) -> &HttpEffectContext {
        self.effects.http()
    }

    pub fn http_options(&self) -> HttpRuntimeOptions {
        self.http_options.clone()
    }

    pub fn stream_runtime(&self) -> Option<&StreamRuntime> {
        self.stream_runtime.as_ref()
    }

    pub fn with_stream_runtime(&self, stream_runtime: StreamRuntime) -> Self {
        Self {
            effects: self.effects.clone(),
            http_options: self.http_options.clone(),
            stream_runtime: Some(stream_runtime),
            test_effect_doubles: self.test_effect_doubles.clone(),
        }
    }

    pub fn test_effect_double_context(&self) -> Option<TestEffectDoubleContext> {
        self.test_effect_doubles.clone()
    }
}
