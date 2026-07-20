use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use serde_json::Value;

use crate::StreamRuntime;

#[derive(Clone, Debug, Default)]
pub struct StreamConsumerEndMarker {
    reached_end: Arc<AtomicBool>,
}

impl StreamConsumerEndMarker {
    pub fn mark_reached_end(&self) {
        self.reached_end.store(true, Ordering::Release);
    }

    pub fn has_reached_end(&self) -> bool {
        self.reached_end.load(Ordering::Acquire)
    }
}

/// Owns the cancellation obligation after a consumer has obtained a stream.
/// Only a completed operation that observed the runtime's natural `End`
/// transition may disarm it; every other return cancels synchronously.
pub struct StreamConsumerCleanup {
    cancel: Box<dyn Fn(&Value) + Send + Sync>,
    stream_value: Value,
    end_marker: StreamConsumerEndMarker,
    disarmed: bool,
}

impl StreamConsumerCleanup {
    pub fn new(runtime: StreamRuntime, stream_value: &Value) -> Self {
        Self::from_cancel(stream_value, move |value| runtime.cancel(value))
    }

    pub fn from_cancel(
        stream_value: &Value,
        cancel: impl Fn(&Value) + Send + Sync + 'static,
    ) -> Self {
        Self {
            cancel: Box::new(cancel),
            stream_value: stream_value.clone(),
            end_marker: StreamConsumerEndMarker::default(),
            disarmed: false,
        }
    }

    pub fn end_marker(&self) -> StreamConsumerEndMarker {
        self.end_marker.clone()
    }

    pub fn reached_end(&mut self) {
        self.end_marker.mark_reached_end();
        self.disarm_after_end();
    }

    pub fn disarm_after_end(&mut self) {
        debug_assert!(
            self.end_marker.has_reached_end(),
            "stream cleanup can only disarm after natural End"
        );
        if self.end_marker.has_reached_end() {
            self.disarmed = true;
        }
    }
}

impl Drop for StreamConsumerCleanup {
    fn drop(&mut self) {
        if !self.disarmed {
            (self.cancel)(&self.stream_value);
        }
    }
}
