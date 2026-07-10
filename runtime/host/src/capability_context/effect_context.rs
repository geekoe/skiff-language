use super::{HttpRuntimeOptions, TelemetryCapabilityContext};
use skiff_runtime_capability_context::CancellationToken;

#[derive(Clone)]
pub struct EffectDispatchContext {
    http: HttpEffectContext,
    telemetry: TelemetryCapabilityContext,
    http_options: HttpRuntimeOptions,
}

impl EffectDispatchContext {
    pub fn new(
        http: HttpEffectContext,
        telemetry: TelemetryCapabilityContext,
        http_options: HttpRuntimeOptions,
    ) -> Self {
        Self {
            http,
            telemetry,
            http_options,
        }
    }

    pub fn http(&self) -> &HttpEffectContext {
        &self.http
    }

    pub fn telemetry_context(&self) -> TelemetryCapabilityContext {
        self.telemetry.clone()
    }

    pub fn http_options(&self) -> HttpRuntimeOptions {
        self.http_options.clone()
    }
}

#[derive(Clone)]
pub struct HttpEffectContext {
    deadline_ms: Option<u64>,
    response_max_bytes: usize,
    cancellation: CancellationToken,
}

impl HttpEffectContext {
    pub fn new(
        deadline_ms: Option<u64>,
        response_max_bytes: usize,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            deadline_ms,
            response_max_bytes,
            cancellation,
        }
    }

    pub fn deadline_ms(&self) -> Option<u64> {
        self.deadline_ms
    }

    pub fn response_max_bytes(&self) -> usize {
        self.response_max_bytes
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}
