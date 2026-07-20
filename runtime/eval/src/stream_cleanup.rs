use serde_json::Value;

use crate::capabilities::StreamRuntime;

/// Owns the cancellation obligation after a consumer has obtained a stream.
/// Only the runtime's natural `End` transition disarms it; every other return
/// path cancels synchronously during stack unwinding.
pub(crate) struct StreamConsumerCleanup {
    runtime: StreamRuntime,
    stream_value: Value,
    reached_end: bool,
}

impl StreamConsumerCleanup {
    pub(crate) fn new(runtime: StreamRuntime, stream_value: &Value) -> Self {
        Self {
            runtime,
            stream_value: stream_value.clone(),
            reached_end: false,
        }
    }

    pub(crate) fn reached_end(&mut self) {
        self.reached_end = true;
    }
}

impl Drop for StreamConsumerCleanup {
    fn drop(&mut self) {
        if !self.reached_end {
            self.runtime.cancel(&self.stream_value);
        }
    }
}
