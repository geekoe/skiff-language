use std::collections::BTreeSet;

use crate::{
    contract::ContractInterfaceOperationProjection,
    runtime_manifest_model::{
        RuntimeGatewayAdapterArgManifest, RuntimeGatewayAdapterSourceManifest,
    },
};

use super::{
    is_projection_connection_message_type, is_projection_string_type,
    is_projection_websocket_connection_type, is_projection_websocket_receive_event_type,
    projection_type_matches_text,
};

pub fn validate_websocket_adapter_sources(
    adapter_args: &[RuntimeGatewayAdapterArgManifest],
    kind: &str,
    has_context: bool,
    violations: &mut Vec<String>,
) {
    for arg in adapter_args {
        if kind == "connect" {
            if arg.source == RuntimeGatewayAdapterSourceManifest::WebSocketConnectRequest {
                continue;
            }
            violations.push(format!(
                "gateway.websocket.connect.adapterArgs parameter {} uses unsupported source {}",
                arg.param,
                adapter_source_kind(arg.source)
            ));
            continue;
        }

        if kind == "receive" {
            if arg.source == RuntimeGatewayAdapterSourceManifest::WebSocketConnectionContext
                && !has_context
            {
                violations.push(format!(
                    "gateway.websocket.receive.adapterArgs parameter {} uses websocket.connectionContext without connection context",
                    arg.param
                ));
                continue;
            }
            if matches!(
                arg.source,
                RuntimeGatewayAdapterSourceManifest::WebSocketReceiveEvent
                    | RuntimeGatewayAdapterSourceManifest::WebSocketConnection
                    | RuntimeGatewayAdapterSourceManifest::WebSocketConnectionContext
                    | RuntimeGatewayAdapterSourceManifest::WebSocketMessage
                    | RuntimeGatewayAdapterSourceManifest::WebSocketMessageBody
                    | RuntimeGatewayAdapterSourceManifest::WebSocketConnectionId
                    | RuntimeGatewayAdapterSourceManifest::WebSocketBusinessIdentity
            ) {
                continue;
            }
            violations.push(format!(
                "gateway.websocket.receive.adapterArgs parameter {} uses unsupported source {}",
                arg.param,
                adapter_source_kind(arg.source)
            ));
            continue;
        }

        violations.push(format!(
            "gateway.websocket.{kind}.adapterArgs parameter {} uses unsupported source {}",
            arg.param,
            adapter_source_kind(arg.source)
        ));
    }
}

pub fn validate_operation_adapter_args(
    operation: &ContractInterfaceOperationProjection,
    adapter_args: &[RuntimeGatewayAdapterArgManifest],
    label: &str,
    violations: &mut Vec<String>,
) {
    let params = operation
        .params
        .iter()
        .map(|param| param.name.as_str())
        .collect::<BTreeSet<_>>();
    let args = adapter_args
        .iter()
        .map(|arg| arg.param.as_str())
        .collect::<BTreeSet<_>>();
    for param in &operation.params {
        if !args.contains(param.name.as_str()) {
            violations.push(format!(
                "{label} is missing operation parameter {}",
                param.name
            ));
        }
    }
    for arg in adapter_args {
        if !params.contains(arg.param.as_str()) {
            violations.push(format!(
                "{label} references unknown operation parameter {}",
                arg.param
            ));
        }
    }
    if args.len() != adapter_args.len() {
        violations.push(format!("{label} declares duplicate adapterArgs parameters"));
    }
}

pub fn validate_receive_adapter_args(
    operation: &ContractInterfaceOperationProjection,
    adapter_args: &[RuntimeGatewayAdapterArgManifest],
    context_type: Option<&str>,
    violations: &mut Vec<String>,
) {
    let mut saw_message_source = false;
    for arg in adapter_args {
        let Some(param) = operation
            .params
            .iter()
            .find(|param| param.name == arg.param)
        else {
            continue;
        };
        match arg.source {
            RuntimeGatewayAdapterSourceManifest::WebSocketReceiveEvent => {
                saw_message_source = true;
                if !is_projection_websocket_receive_event_type(&param.ty) {
                    violations.push(format!(
                        "gateway.websocket.receive parameter {} bound to websocket.receiveEvent must have type WebSocketReceiveEvent<C>",
                        param.name
                    ));
                }
            }
            RuntimeGatewayAdapterSourceManifest::WebSocketConnection => {
                if !is_projection_websocket_connection_type(&param.ty) {
                    violations.push(format!(
                        "gateway.websocket.receive parameter {} bound to websocket.connection must have type WebSocketConnection<C>",
                        param.name
                    ));
                }
            }
            RuntimeGatewayAdapterSourceManifest::WebSocketConnectionContext => {
                if let Some(context_type) = context_type {
                    if !projection_type_matches_text(&param.ty, context_type) {
                        violations.push(format!(
                            "gateway.websocket.receive parameter {} bound to websocket.connectionContext must have context type {}",
                            param.name, context_type
                        ));
                    }
                }
            }
            RuntimeGatewayAdapterSourceManifest::WebSocketMessage => {
                saw_message_source = true;
                if !is_projection_connection_message_type(&param.ty) {
                    violations.push(format!(
                        "gateway.websocket.receive parameter {} bound to websocket.message must have type ConnectionMessage",
                        param.name
                    ));
                }
            }
            RuntimeGatewayAdapterSourceManifest::WebSocketMessageBody => {
                saw_message_source = true;
            }
            RuntimeGatewayAdapterSourceManifest::WebSocketConnectionId => {
                if !is_projection_string_type(&param.ty) {
                    violations.push(format!(
                        "gateway.websocket.receive parameter {} bound to websocket.connectionId must have type string",
                        param.name
                    ));
                }
            }
            RuntimeGatewayAdapterSourceManifest::WebSocketBusinessIdentity => {
                if !is_projection_string_type(&param.ty) {
                    violations.push(format!(
                        "gateway.websocket.receive parameter {} bound to websocket.businessIdentity must have type string",
                        param.name
                    ));
                }
            }
            RuntimeGatewayAdapterSourceManifest::WebSocketConnectRequest
            | RuntimeGatewayAdapterSourceManifest::HttpRequest
            | RuntimeGatewayAdapterSourceManifest::HttpBody
            | RuntimeGatewayAdapterSourceManifest::HttpContext => {}
        }
    }
    if !saw_message_source {
        violations.push(format!(
            "gateway.websocket.receive operation {} must bind at least one parameter to websocket.message, websocket.messageBody, or websocket.receiveEvent",
            operation.name
        ));
    }
}

fn adapter_source_kind(source: RuntimeGatewayAdapterSourceManifest) -> &'static str {
    match source {
        RuntimeGatewayAdapterSourceManifest::HttpRequest => "http.request",
        RuntimeGatewayAdapterSourceManifest::HttpBody => "http.body",
        RuntimeGatewayAdapterSourceManifest::HttpContext => "http.context",
        RuntimeGatewayAdapterSourceManifest::WebSocketConnectRequest => "websocket.connectRequest",
        RuntimeGatewayAdapterSourceManifest::WebSocketReceiveEvent => "websocket.receiveEvent",
        RuntimeGatewayAdapterSourceManifest::WebSocketConnection => "websocket.connection",
        RuntimeGatewayAdapterSourceManifest::WebSocketConnectionContext => {
            "websocket.connectionContext"
        }
        RuntimeGatewayAdapterSourceManifest::WebSocketMessage => "websocket.message",
        RuntimeGatewayAdapterSourceManifest::WebSocketMessageBody => "websocket.messageBody",
        RuntimeGatewayAdapterSourceManifest::WebSocketConnectionId => "websocket.connectionId",
        RuntimeGatewayAdapterSourceManifest::WebSocketBusinessIdentity => {
            "websocket.businessIdentity"
        }
    }
}
