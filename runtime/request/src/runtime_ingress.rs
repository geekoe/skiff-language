use skiff_runtime_boundary::{
    binary::encode_payload_plan,
    payload::{PayloadBoundary, PayloadBoundaryKind, PayloadServiceRef},
};
use skiff_runtime_eval::{stream_callback::EvalStreamExecutionError, EvalRequestExecutor};

use crate::{
    ingress::RequestIngressContext, request_payload_context_from_request, response_stream_writer,
    BoundaryResponse, RequestError, RequestResult,
};

pub(super) struct RuntimeIngressHandler<'ctx, 'req> {
    context: &'ctx RequestIngressContext<'req>,
    executor: &'ctx EvalRequestExecutor,
}

impl<'ctx, 'req> RuntimeIngressHandler<'ctx, 'req> {
    pub(super) fn new(
        context: &'ctx RequestIngressContext<'req>,
        executor: &'ctx EvalRequestExecutor,
    ) -> Self {
        Self { context, executor }
    }

    pub(super) async fn dispatch_server_stream(&self) -> RequestResult<BoundaryResponse> {
        self.context
            .require_router_sender("serverStream request is missing router sender")?;
        let eval_invocation = self.context.build_eval_invocation()?;
        let execution_input = self
            .context
            .eval_execution_input(request_payload_context_from_request(self.context.request));
        let mut writer = response_stream_writer::ResponseStreamWriter::new(
            self.context.request.request_id.clone(),
            self.context.handles.response_events.clone(),
        );
        writer.start_runtime_stream()?;
        self.executor
            .execute_runtime_response_stream_payloads(execution_input, eval_invocation, |payload| {
                writer.send_chunk(payload)
            })
            .await
            .map_err(stream_execution_error_into_request_error)?;
        writer.finish()?;
        Ok(BoundaryResponse::StreamSent)
    }

    pub(super) async fn dispatch_unary(&self) -> RequestResult<BoundaryResponse> {
        let eval_invocation = self.context.build_eval_invocation()?;
        let execution_input = self
            .context
            .eval_execution_input(request_payload_context_from_request(self.context.request));
        let (value, expected_plan, heap) = self
            .executor
            .execute_runtime_value(execution_input, eval_invocation)
            .await?;
        let boundary = PayloadBoundary::external_untrusted(PayloadBoundaryKind::ServiceResponse)
            .with_origin_service(
                PayloadServiceRef::new(self.context.metadata.service_id.clone())
                    .with_version(self.context.metadata.service_version.clone())
                    .with_build_id(self.context.metadata.build_id.clone()),
            );
        Ok(BoundaryResponse::end(
            encode_payload_plan(&value, &expected_plan, &boundary, &heap)?,
            None,
            None,
        ))
    }
}

fn stream_execution_error_into_request_error(
    error: EvalStreamExecutionError<RequestError>,
) -> RequestError {
    match error {
        EvalStreamExecutionError::Eval(error) => error.into(),
        EvalStreamExecutionError::Callback(error) => error,
    }
}
