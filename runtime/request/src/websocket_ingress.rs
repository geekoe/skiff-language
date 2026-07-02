use skiff_runtime_eval::{
    EvalRequestExecutor, EvalRequestWebSocketAdapterResult, EvalRequestWebSocketConnectResponse,
    EvalRequestWebSocketConnectResult, EvalRequestWebSocketContextCodec,
};

use crate::{
    ingress::RequestIngressContext, request_payload_context_from_request, BoundaryResponse,
    RequestError, RequestResult, WebSocketConnectResponse, WebSocketContextCodec,
};

#[cfg(test)]
mod tests;

pub(super) struct WebSocketIngressHandler<'ctx, 'req> {
    context: &'ctx RequestIngressContext<'req>,
    executor: &'ctx EvalRequestExecutor,
}

impl<'ctx, 'req> WebSocketIngressHandler<'ctx, 'req> {
    pub(super) fn new(
        context: &'ctx RequestIngressContext<'req>,
        executor: &'ctx EvalRequestExecutor,
    ) -> Self {
        Self { context, executor }
    }

    pub(super) async fn dispatch(&self) -> RequestResult<BoundaryResponse> {
        self.context
            .request
            .websocket_adapter
            .as_ref()
            .expect("websocket adapter checked by caller");
        if self.context.request.mode != "unary" {
            return Err(RequestError::Unsupported(
                "WebSocket adapter dispatch only supports unary request.start".to_string(),
            ));
        }
        let eval_invocation = self.context.build_eval_invocation()?;
        let execution_input = self
            .context
            .eval_execution_input(request_payload_context_from_request(self.context.request));
        Ok(self
            .executor
            .execute_websocket_adapter(execution_input, eval_invocation)
            .await
            .map(boundary_response_from_eval_websocket_adapter_result)?)
    }
}

fn boundary_response_from_eval_websocket_adapter_result(
    response: EvalRequestWebSocketAdapterResult,
) -> BoundaryResponse {
    BoundaryResponse::end(
        response.payload,
        None,
        response
            .response
            .map(request_websocket_connect_response_from_eval),
    )
}

fn request_websocket_connect_response_from_eval(
    response: EvalRequestWebSocketConnectResponse,
) -> WebSocketConnectResponse {
    WebSocketConnectResponse {
        result: match response.result {
            EvalRequestWebSocketConnectResult::Accept => "accept".to_string(),
            EvalRequestWebSocketConnectResult::Reject => "reject".to_string(),
        },
        business_identity: response.business_identity,
        connection_policy: response.connection_policy,
        context_codec: response
            .context_codec
            .map(request_websocket_context_codec_from_eval),
        context_payload_present: response.context_payload_present,
        code: response.code,
        reason: response.reason,
    }
}

fn request_websocket_context_codec_from_eval(
    codec: EvalRequestWebSocketContextCodec,
) -> WebSocketContextCodec {
    WebSocketContextCodec {
        operation_abi_id: codec.operation_abi_id,
        context_type_identity: codec.context_type_identity,
    }
}
