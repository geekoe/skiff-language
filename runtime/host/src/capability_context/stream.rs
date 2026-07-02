use serde_json::Value;

use skiff_runtime_model::type_plan::RuntimeTypePlan;
use skiff_runtime_request::ExecutionControl;

use super::StreamSink;
use crate::error::{Result, RuntimeError};

#[derive(Clone, Debug)]
pub struct TypedStreamSink {
    pub sink: StreamSink,
    pub item_type: RuntimeTypePlan,
}

#[derive(Clone, Debug, Default)]
pub struct StreamCapabilityContext {
    current_stream_sink: Option<StreamSink>,
    response_stream_sink: Option<TypedStreamSink>,
}

impl StreamCapabilityContext {
    pub fn new(
        current_stream_sink: Option<StreamSink>,
        response_stream_sink: Option<TypedStreamSink>,
    ) -> Self {
        Self {
            current_stream_sink,
            response_stream_sink,
        }
    }
}

#[derive(Clone)]
pub struct HttpResponseStreamCapabilityContext<'execution> {
    execution: ExecutionControl<'execution>,
    stream_context: StreamCapabilityContext,
}

impl<'execution> HttpResponseStreamCapabilityContext<'execution> {
    pub fn new(
        execution: ExecutionControl<'execution>,
        stream_context: StreamCapabilityContext,
    ) -> Self {
        Self {
            execution,
            stream_context,
        }
    }

    pub fn response_item_type(&self, target: &str) -> Result<&RuntimeTypePlan> {
        Ok(&self.response_stream_sink(target)?.item_type)
    }

    pub async fn send_response_event(&self, target: &str, event: Value) -> Result<()> {
        let typed_sink = self.response_stream_sink(target)?;
        let mut cancel_flags = vec![self.execution.cancel_flag()];
        if let Some(inner_sink) = self.stream_context.current_stream_sink.as_ref() {
            if !inner_sink.is_same_stream(&typed_sink.sink) {
                cancel_flags.push(inner_sink.cancel_flag());
            }
        }
        Ok(typed_sink
            .sink
            .send_with_cancel(event, &cancel_flags)
            .await?)
    }

    fn response_stream_sink(&self, target: &str) -> Result<&TypedStreamSink> {
        self.stream_context
            .response_stream_sink
            .as_ref()
            .ok_or_else(|| {
                RuntimeError::Decode(format!(
                    "{target} used outside a raw HTTP streaming response context"
                ))
            })
    }
}
