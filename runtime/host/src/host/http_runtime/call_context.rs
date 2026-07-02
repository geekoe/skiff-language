use std::sync::atomic::AtomicBool;

use serde_json::Value;
use skiff_runtime_capability_context::CancellationSignals;

use super::cancel::borrowed_cancel_signals;
use crate::capability_context::HttpRuntimeOptions;

pub(super) struct HttpCallContext<'input, 'cancel> {
    input: &'input Value,
    frame_deadline_ms: Option<u64>,
    service_max_response_bytes: usize,
    cancel_signals: CancellationSignals<'cancel>,
    options: HttpRuntimeOptions,
    target: &'static str,
}

impl<'input, 'cancel> HttpCallContext<'input, 'cancel> {
    pub(super) fn borrowed(
        input: &'input Value,
        frame_deadline_ms: Option<u64>,
        service_max_response_bytes: usize,
        cancelled: Option<&'cancel AtomicBool>,
        options: HttpRuntimeOptions,
        target: &'static str,
    ) -> Self {
        Self::new(
            input,
            frame_deadline_ms,
            service_max_response_bytes,
            borrowed_cancel_signals(cancelled),
            options,
            target,
        )
    }

    pub(super) fn new(
        input: &'input Value,
        frame_deadline_ms: Option<u64>,
        service_max_response_bytes: usize,
        cancel_signals: CancellationSignals<'cancel>,
        options: HttpRuntimeOptions,
        target: &'static str,
    ) -> Self {
        Self {
            input,
            frame_deadline_ms,
            service_max_response_bytes,
            cancel_signals,
            options,
            target,
        }
    }

    pub(super) fn input(&self) -> &Value {
        self.input
    }

    pub(super) fn frame_deadline_ms(&self) -> Option<u64> {
        self.frame_deadline_ms
    }

    pub(super) fn service_max_response_bytes(&self) -> usize {
        self.service_max_response_bytes
    }

    pub(super) fn cancel_signals(&self) -> &CancellationSignals<'cancel> {
        &self.cancel_signals
    }

    pub(super) fn options(&self) -> HttpRuntimeOptions {
        self.options.clone()
    }

    pub(super) fn target(&self) -> &'static str {
        self.target
    }

    pub(super) fn into_cancel_signals(self) -> CancellationSignals<'cancel> {
        self.cancel_signals
    }
}

impl<'input> HttpCallContext<'input, 'static> {
    pub(super) fn owned(
        input: &'input Value,
        frame_deadline_ms: Option<u64>,
        service_max_response_bytes: usize,
        cancel_signals: CancellationSignals<'static>,
        options: HttpRuntimeOptions,
        target: &'static str,
    ) -> Self {
        Self::new(
            input,
            frame_deadline_ms,
            service_max_response_bytes,
            cancel_signals,
            options,
            target,
        )
    }
}
