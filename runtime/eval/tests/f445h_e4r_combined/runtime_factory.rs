use super::{imports::*, stream_support::ProbeStreamRuntime};

#[derive(Clone)]
pub(super) struct HarnessRuntimeFactory {
    pub(super) stream: ProbeStreamRuntime,
}

impl EvalRuntimeFactoryApi for HarnessRuntimeFactory {
    fn stream_runtime(&self) -> StreamRuntime {
        StreamRuntime::new(self.stream.clone())
    }

    fn reusable_test_effect_doubles(
        &self,
        _doubles: HashMap<String, TestEffectDouble>,
        _stream_runtime: &StreamRuntime,
        _test_effects_enabled: bool,
    ) -> TestEffectDoubleContext {
        TestEffectDoubleContext::new(NoTestEffects)
    }

    fn one_shot_test_effect_double_sequences(
        &self,
        _doubles: HashMap<String, Vec<TestEffectDouble>>,
        _stream_runtime: &StreamRuntime,
        _test_effects_enabled: bool,
    ) -> TestEffectDoubleContext {
        TestEffectDoubleContext::new(NoTestEffects)
    }
}

struct NoTestEffects;

impl TestEffectDoubleContextApi for NoTestEffects {
    fn next_test_effect_double(&self, _target: &str) -> Option<TestEffectDouble> {
        None
    }

    fn dispatch_test_effect_double(
        &self,
        _target: &str,
        _input: Option<&Value>,
    ) -> Option<EvalResult<Value>> {
        None
    }

    fn dispatch_test_stable_target_double(
        &self,
        _target: &str,
        _return_plan: Option<&RuntimeTypePlan>,
        _heap: &mut RequestHeap,
    ) -> Option<EvalResult<RuntimeValue>> {
        None
    }

    fn dispatch_test_host_operation_double(
        &self,
        _target: &str,
        _input: Option<&RuntimeValue>,
        _arg_plan: Option<&RuntimeTypePlan>,
        _return_plan: Option<&RuntimeTypePlan>,
        _heap: &mut RequestHeap,
    ) -> Option<EvalResult<RuntimeValue>> {
        None
    }

    fn dispatch_test_http_effect_invocation_double(
        &self,
        _target: &str,
        _input: Option<&RuntimeValue>,
        _arg_plan: Option<&RuntimeTypePlan>,
        _return_plan: Option<&RuntimeTypePlan>,
        _heap: &mut RequestHeap,
    ) -> Option<EvalResult<RuntimeValue>> {
        None
    }
}
