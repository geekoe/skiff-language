use skiff_runtime_eval::{
    EvalRequestInvocation, EvalRequestInvocationArg, EvalRequestInvocationArgFrom,
    EvalRequestInvocationCallable, EvalRequestInvocationHttpAdapter, EvalRequestInvocationHttpKind,
    EvalRequestInvocationInput, EvalRequestInvocationMode, EvalRequestInvocationWebSocketAdapter,
    EvalRequestInvocationWebSocketConnectRequest, EvalRequestInvocationWebSocketContextCodec,
    EvalRequestInvocationWebSocketContextExpectation, EvalRequestInvocationWebSocketKind,
    EvalRequestInvocationWebSocketMessage, EvalRequestInvocationWebSocketMessageEncoding,
    EvalRequestInvocationWebSocketMessageTag, EvalRequestInvocationWebSocketNameValue,
    EvalRequestInvocationWebSocketPayloadSegment, EvalRequestInvocationWebSocketPayloadSegmentKind,
    EvalRequestInvocationWebSocketReceiveRequest, EvalRuntimeProgram,
};
use skiff_runtime_linked_program::ExecutableAddr;

use crate::{
    request_payload_context_from_request, GatewayAdapterArg, GatewayAdapterSource, HttpAdapter,
    HttpAdapterCallable, HttpAdapterKind, HttpNameValue, RequestEnvelope, RequestResult,
    WebSocketAdapter, WebSocketAdapterKind, WebSocketConnectRequest, WebSocketContextCodec,
    WebSocketContextExpectation, WebSocketMessage, WebSocketMessageEncoding, WebSocketMessageTag,
    WebSocketPayloadSegment, WebSocketPayloadSegmentKind, WebSocketReceiveRequest,
};

pub(crate) fn build_eval_invocation<'a>(
    request: &'a RequestEnvelope,
    operation: &'a str,
    addr: &'a ExecutableAddr,
    program: &'a EvalRuntimeProgram,
) -> RequestResult<EvalRequestInvocation<'a>> {
    Ok(program.build_invocation(eval_invocation_build_input(request), operation, addr)?)
}

fn eval_invocation_build_input<'a>(request: &'a RequestEnvelope) -> EvalRequestInvocationInput<'a> {
    EvalRequestInvocationInput {
        request: request_payload_context_from_request(request),
        target: request.target.clone(),
        mode: match request.mode.as_str() {
            "serverStream" => EvalRequestInvocationMode::ServerStream,
            _ => EvalRequestInvocationMode::Unary,
        },
        has_binary_http: request.binary_http.is_some(),
        has_retired_actor_call_metadata: request.extra.contains_key("actorCall"),
        http_adapter: request.http_adapter.as_ref().map(eval_http_adapter),
        websocket_adapter: request
            .websocket_adapter
            .as_ref()
            .map(eval_websocket_adapter),
    }
}

fn eval_http_adapter(adapter: &HttpAdapter) -> EvalRequestInvocationHttpAdapter {
    EvalRequestInvocationHttpAdapter {
        kind: match adapter.kind {
            HttpAdapterKind::TypedJson => EvalRequestInvocationHttpKind::TypedJson,
            HttpAdapterKind::RawHttp => EvalRequestInvocationHttpKind::RawHttp,
        },
        handler: eval_callable(&adapter.handler),
        guard: adapter.guard.as_ref().map(eval_callable),
        pre: adapter.pre.as_ref().map(eval_callable),
        args: eval_args(&adapter.adapter_args),
    }
}

fn eval_websocket_adapter(adapter: &WebSocketAdapter) -> EvalRequestInvocationWebSocketAdapter {
    EvalRequestInvocationWebSocketAdapter {
        kind: match adapter.kind {
            WebSocketAdapterKind::Connect => EvalRequestInvocationWebSocketKind::Connect,
            WebSocketAdapterKind::Receive => EvalRequestInvocationWebSocketKind::Receive,
        },
        args: eval_args(&adapter.adapter_args),
        context_expectation: adapter
            .context_expectation
            .as_ref()
            .map(eval_websocket_context_expectation),
        connect_request: adapter
            .connect_request
            .as_ref()
            .map(eval_websocket_connect_request),
        receive_request: adapter
            .receive_request
            .as_ref()
            .map(eval_websocket_receive_request),
    }
}

fn eval_callable(callable: &HttpAdapterCallable) -> EvalRequestInvocationCallable {
    match callable {
        HttpAdapterCallable::ServiceFunction {
            module_path,
            symbol,
        } => EvalRequestInvocationCallable::ServiceFunction {
            module_path: module_path.clone(),
            symbol: symbol.clone(),
        },
        HttpAdapterCallable::PackageFunction {
            package_id,
            symbol_path,
        } => EvalRequestInvocationCallable::PackageFunction {
            package_id: package_id.clone(),
            symbol_path: symbol_path.clone(),
        },
    }
}

fn eval_args(args: &[GatewayAdapterArg]) -> Vec<EvalRequestInvocationArg> {
    args.iter()
        .map(|arg| EvalRequestInvocationArg {
            param: arg.param.clone(),
            from: eval_arg_from(arg.source),
        })
        .collect()
}

fn eval_arg_from(source: GatewayAdapterSource) -> EvalRequestInvocationArgFrom {
    match source {
        GatewayAdapterSource::HttpRequest => EvalRequestInvocationArgFrom::HttpRequest,
        GatewayAdapterSource::HttpBody => EvalRequestInvocationArgFrom::HttpBody,
        GatewayAdapterSource::HttpContext => EvalRequestInvocationArgFrom::HttpContext,
        GatewayAdapterSource::WebSocketConnectRequest => {
            EvalRequestInvocationArgFrom::WebSocketConnectRequest
        }
        GatewayAdapterSource::WebSocketReceiveEvent => {
            EvalRequestInvocationArgFrom::WebSocketReceiveEvent
        }
        GatewayAdapterSource::WebSocketConnection => {
            EvalRequestInvocationArgFrom::WebSocketConnection
        }
        GatewayAdapterSource::WebSocketConnectionContext => {
            EvalRequestInvocationArgFrom::WebSocketConnectionContext
        }
        GatewayAdapterSource::WebSocketMessage => EvalRequestInvocationArgFrom::WebSocketMessage,
        GatewayAdapterSource::WebSocketMessageBody => {
            EvalRequestInvocationArgFrom::WebSocketMessageBody
        }
        GatewayAdapterSource::WebSocketConnectionId => {
            EvalRequestInvocationArgFrom::WebSocketConnectionId
        }
        GatewayAdapterSource::WebSocketBusinessIdentity => {
            EvalRequestInvocationArgFrom::WebSocketBusinessIdentity
        }
    }
}

fn eval_websocket_context_expectation(
    expectation: &WebSocketContextExpectation,
) -> EvalRequestInvocationWebSocketContextExpectation {
    match expectation {
        WebSocketContextExpectation::Null => EvalRequestInvocationWebSocketContextExpectation::Null,
        WebSocketContextExpectation::Typed {
            connect_operation_abi_id,
            context_type_identity,
        } => EvalRequestInvocationWebSocketContextExpectation::Typed {
            connect_operation_abi_id: connect_operation_abi_id.clone(),
            context_type_identity: context_type_identity.clone(),
        },
    }
}

fn eval_websocket_context_codec(
    codec: &WebSocketContextCodec,
) -> EvalRequestInvocationWebSocketContextCodec {
    EvalRequestInvocationWebSocketContextCodec {
        operation_abi_id: codec.operation_abi_id.clone(),
        context_type_identity: codec.context_type_identity.clone(),
    }
}

fn eval_websocket_connect_request(
    request: &WebSocketConnectRequest,
) -> EvalRequestInvocationWebSocketConnectRequest {
    EvalRequestInvocationWebSocketConnectRequest {
        connection_id: request.connection_id.clone(),
        url: request.url.clone(),
        query: eval_websocket_name_values(&request.query),
        headers: eval_websocket_name_values(&request.headers),
        cookies: eval_websocket_name_values(&request.cookies),
        version: request.version.clone(),
    }
}

fn eval_websocket_receive_request(
    request: &WebSocketReceiveRequest,
) -> EvalRequestInvocationWebSocketReceiveRequest {
    EvalRequestInvocationWebSocketReceiveRequest {
        connection_id: request.connection_id.clone(),
        business_identity: request.business_identity.clone(),
        message: eval_websocket_message(&request.message),
        context_codec: request
            .context_codec
            .as_ref()
            .map(eval_websocket_context_codec),
        payload_segments: request
            .payload_segments
            .iter()
            .map(eval_websocket_payload_segment)
            .collect(),
    }
}

fn eval_websocket_name_values(
    items: &[HttpNameValue],
) -> Vec<EvalRequestInvocationWebSocketNameValue> {
    items
        .iter()
        .map(|item| EvalRequestInvocationWebSocketNameValue {
            name: item.name.clone(),
            value: item.value.clone(),
        })
        .collect()
}

fn eval_websocket_message(message: &WebSocketMessage) -> EvalRequestInvocationWebSocketMessage {
    EvalRequestInvocationWebSocketMessage {
        tag: match message.tag {
            WebSocketMessageTag::Text => EvalRequestInvocationWebSocketMessageTag::Text,
            WebSocketMessageTag::Binary => EvalRequestInvocationWebSocketMessageTag::Binary,
        },
        encoding: match message.encoding {
            WebSocketMessageEncoding::Utf8 => EvalRequestInvocationWebSocketMessageEncoding::Utf8,
            WebSocketMessageEncoding::Raw => EvalRequestInvocationWebSocketMessageEncoding::Raw,
        },
    }
}

fn eval_websocket_payload_segment(
    segment: &WebSocketPayloadSegment,
) -> EvalRequestInvocationWebSocketPayloadSegment {
    EvalRequestInvocationWebSocketPayloadSegment {
        kind: match segment.kind {
            WebSocketPayloadSegmentKind::Context => {
                EvalRequestInvocationWebSocketPayloadSegmentKind::Context
            }
            WebSocketPayloadSegmentKind::Message => {
                EvalRequestInvocationWebSocketPayloadSegmentKind::Message
            }
        },
        offset: segment.offset,
        length: segment.length,
    }
}
