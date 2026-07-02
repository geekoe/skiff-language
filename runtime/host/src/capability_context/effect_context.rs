use std::sync::{atomic::AtomicBool, Arc};

use super::{HttpRuntimeOptions, TelemetryCapabilityContext};

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
    request_cancelled: Arc<AtomicBool>,
}

impl HttpEffectContext {
    pub fn new(
        deadline_ms: Option<u64>,
        response_max_bytes: usize,
        request_cancelled: Arc<AtomicBool>,
    ) -> Self {
        Self {
            deadline_ms,
            response_max_bytes,
            request_cancelled,
        }
    }

    pub fn deadline_ms(&self) -> Option<u64> {
        self.deadline_ms
    }

    pub fn response_max_bytes(&self) -> usize {
        self.response_max_bytes
    }

    pub fn request_cancelled(&self) -> Arc<AtomicBool> {
        self.request_cancelled.clone()
    }
}
