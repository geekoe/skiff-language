use skiff_runtime_boundary::http::HttpBoundaryNameValue;
use skiff_runtime_eval::{stream_callback::EvalStreamExecutionError, EvalRequestExecutor};

use crate::{
    ingress::RequestIngressContext, request_payload_context_from_request, response_stream_writer,
    BoundaryResponse, HttpAdapterKind, HttpNameValue, HttpResponseMetadata, RequestError,
    RequestResult,
};

pub(super) struct BinaryHttpIngressHandler<'ctx, 'req> {
    context: &'ctx RequestIngressContext<'req>,
    executor: &'ctx EvalRequestExecutor,
}

impl<'ctx, 'req> BinaryHttpIngressHandler<'ctx, 'req> {
    pub(super) fn new(
        context: &'ctx RequestIngressContext<'req>,
        executor: &'ctx EvalRequestExecutor,
    ) -> Self {
        Self { context, executor }
    }

    pub(super) async fn dispatch(&self) -> RequestResult<BoundaryResponse> {
        if let Some(adapter) = self.context.request.http_adapter.as_ref() {
            return self.dispatch_http_adapter(adapter).await;
        }
        if self.context.request.mode == "serverStream" {
            self.dispatch_binary_http_response_stream().await
        } else {
            self.dispatch_binary_http_unary().await
        }
    }

    async fn dispatch_http_adapter(
        &self,
        adapter: &crate::HttpAdapter,
    ) -> RequestResult<BoundaryResponse> {
        if adapter.kind == HttpAdapterKind::TypedJson && self.context.request.mode != "unary" {
            Err(RequestError::Unsupported(
                "HTTP adapter dispatch only supports unary request.start".to_string(),
            ))
        } else if self.context.request.mode == "serverStream" {
            let eval_invocation = self.context.build_eval_invocation()?;
            let execution_input = self
                .context
                .eval_execution_input(request_payload_context_from_request(self.context.request));
            self.context.require_router_sender(
                "binary HTTP serverStream request is missing router sender",
            )?;
            let mut stream_writer = response_stream_writer::ResponseStreamWriter::new(
                self.context.request.request_id.clone(),
                self.context.handles.response_events.clone(),
            );
            self.executor
                .execute_http_raw_adapter_response_stream(
                    execution_input,
                    eval_invocation,
                    |event| stream_writer.send_binary_http_event(event),
                )
                .await
                .map_err(stream_execution_error_into_request_error)?;
            stream_writer.finish()?;
            Ok(BoundaryResponse::StreamSent)
        } else {
            let eval_invocation = self.context.build_eval_invocation()?;
            let execution_input = self
                .context
                .eval_execution_input(request_payload_context_from_request(self.context.request));
            Ok(self
                .executor
                .execute_http_adapter(execution_input, eval_invocation)
                .await
                .map(|response| {
                    BoundaryResponse::end(
                        response.body,
                        Some(http_response_metadata_from_boundary(
                            response.status,
                            response.headers,
                        )),
                        None,
                    )
                })?)
        }
    }

    async fn dispatch_binary_http_response_stream(&self) -> RequestResult<BoundaryResponse> {
        self.context
            .require_router_sender("binary HTTP serverStream request is missing router sender")?;
        let eval_invocation = self.context.build_eval_invocation()?;
        let execution_input = self
            .context
            .eval_execution_input(request_payload_context_from_request(self.context.request));
        let mut stream_writer = response_stream_writer::ResponseStreamWriter::new(
            self.context.request.request_id.clone(),
            self.context.handles.response_events.clone(),
        );
        self.executor
            .execute_binary_http_response_stream(execution_input, eval_invocation, |event| {
                stream_writer.send_binary_http_event(event)
            })
            .await
            .map_err(stream_execution_error_into_request_error)?;
        stream_writer.finish()?;
        Ok(BoundaryResponse::StreamSent)
    }

    async fn dispatch_binary_http_unary(&self) -> RequestResult<BoundaryResponse> {
        let eval_invocation = self.context.build_eval_invocation()?;
        let execution_input = self
            .context
            .eval_execution_input(request_payload_context_from_request(self.context.request));
        Ok(self
            .executor
            .execute_binary_http(execution_input, eval_invocation)
            .await
            .map(|response| {
                BoundaryResponse::end(
                    response.body,
                    Some(http_response_metadata_from_boundary(
                        response.status,
                        response.headers,
                    )),
                    None,
                )
            })?)
    }
}

fn http_response_metadata_from_boundary(
    status: u16,
    headers: Vec<HttpBoundaryNameValue>,
) -> HttpResponseMetadata {
    HttpResponseMetadata::new(
        status,
        headers
            .into_iter()
            .map(|header| HttpNameValue {
                name: header.name,
                value: header.value,
            })
            .collect(),
    )
}

fn stream_execution_error_into_request_error(
    error: EvalStreamExecutionError<RequestError>,
) -> RequestError {
    match error {
        EvalStreamExecutionError::Eval(error) => error.into(),
        EvalStreamExecutionError::Callback(error) => error,
    }
}
