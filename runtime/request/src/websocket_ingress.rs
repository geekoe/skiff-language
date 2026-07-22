use skiff_runtime_eval::{
    EvalRequestExecutor, EvalRequestWebSocketAdapterResult, EvalRequestWebSocketConnectAccept,
    EvalRequestWebSocketConnectContext, EvalRequestWebSocketConnectReject,
    EvalRequestWebSocketContextCodec,
};

use crate::{
    ingress::RequestIngressContext, request_payload_context_from_request, BoundaryResponse,
    RequestError, RequestResult, WebSocketAdapterKind, WebSocketConnectAccept,
    WebSocketConnectContext, WebSocketConnectReject, WebSocketContextCodec, WebSocketResponse,
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
        let adapter = self
            .context
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
        let response = self
            .executor
            .execute_websocket_adapter(execution_input, eval_invocation)
            .await
            .map_err(RequestError::from)?;
        boundary_response_from_eval_websocket_adapter_result(adapter.kind, response)
    }
}

pub(crate) fn boundary_response_from_eval_websocket_adapter_result(
    phase: WebSocketAdapterKind,
    response: EvalRequestWebSocketAdapterResult,
) -> RequestResult<BoundaryResponse> {
    let response = match (phase, response) {
        (
            WebSocketAdapterKind::Connect,
            EvalRequestWebSocketAdapterResult::ConnectAccept(value),
        ) => WebSocketResponse::ConnectAccept(request_websocket_connect_accept_from_eval(value)),
        (
            WebSocketAdapterKind::Connect,
            EvalRequestWebSocketAdapterResult::ConnectReject(value),
        ) => WebSocketResponse::ConnectReject(request_websocket_connect_reject_from_eval(value)),
        (WebSocketAdapterKind::Receive, EvalRequestWebSocketAdapterResult::Receive) => {
            WebSocketResponse::Receive
        }
        _ => {
            return Err(RequestError::protocol(
                "websocket",
                "WebSocket response variant does not match the admitted request phase",
            ))
        }
    };
    Ok(BoundaryResponse::websocket(response))
}

fn request_websocket_connect_accept_from_eval(
    response: EvalRequestWebSocketConnectAccept,
) -> WebSocketConnectAccept {
    WebSocketConnectAccept {
        business_identity: response.business_identity,
        connection_policy: response.connection_policy,
        context: match response.context {
            EvalRequestWebSocketConnectContext::Null => WebSocketConnectContext::Null,
            EvalRequestWebSocketConnectContext::Typed { payload, codec } => {
                WebSocketConnectContext::Typed {
                    payload,
                    codec: request_websocket_context_codec_from_eval(codec),
                }
            }
        },
    }
}

fn request_websocket_connect_reject_from_eval(
    response: EvalRequestWebSocketConnectReject,
) -> WebSocketConnectReject {
    WebSocketConnectReject {
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
