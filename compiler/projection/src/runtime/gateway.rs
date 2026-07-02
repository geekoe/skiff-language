use serde::Serialize;

use std::collections::BTreeMap;

use crate::runtime_manifest_model::{
    JsonSchema, RuntimeGatewayAdapterArgManifest, RuntimeHttpRouteGatewayManifest,
    RuntimeWebSocketContextExpectationManifest, SkiffRuntimeManifest,
};

/// Typed gateway entry projected into the service assembly. Optional members
/// are skipped when absent (matching the former conditional `json!` maps), and
/// `http` / `websocket` are `null` when the service declares no such gateway.
#[derive(Debug, Clone, Serialize)]
pub struct GatewayEntry {
    pub http: Option<HttpGatewayEntry>,
    pub websocket: Option<WebSocketGatewayEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HttpGatewayEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    raw: Option<HttpRawEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    routes: Vec<RuntimeHttpRouteGatewayManifest>,
}

#[derive(Debug, Clone, Serialize)]
struct HttpRawEntry {
    operation: String,
    target: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSocketGatewayEntry {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_param: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gateway_entry_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<WebSocketContextEntry>,
    context_expectation: RuntimeWebSocketContextExpectationManifest,
    #[serde(skip_serializing_if = "Option::is_none")]
    connect: Option<WebSocketChannelEntry>,
    receive: WebSocketChannelEntry,
}

#[derive(Debug, Clone, Serialize)]
struct WebSocketContextEntry {
    #[serde(rename = "type")]
    ty: &'static str,
    schema: JsonSchema,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebSocketChannelEntry {
    operation: String,
    operation_abi_id: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    adapter_args: Vec<RuntimeGatewayAdapterArgManifest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_operation_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_protocol_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gateway_entry_identity: Option<String>,
}

impl GatewayEntry {
    /// HTTP routes (path, method, operation) for service-unit gateway projection.
    pub fn http_routes(&self) -> &[RuntimeHttpRouteGatewayManifest] {
        self.http.as_ref().map_or(&[], |http| &http.routes)
    }

    /// The websocket entry's path, receive operation, and connect operation,
    /// if a websocket gateway is present with a receive operation.
    pub fn websocket_default(&self) -> Option<WebSocketDefault<'_>> {
        let websocket = self.websocket.as_ref()?;
        Some(WebSocketDefault {
            path: websocket.path.as_deref(),
            receive_operation: websocket.receive.operation.as_str(),
            receive_operation_abi_id: websocket.receive.operation_abi_id.as_str(),
            connect_operation: websocket
                .connect
                .as_ref()
                .map(|connect| connect.operation.as_str()),
            connect_operation_abi_id: websocket
                .connect
                .as_ref()
                .map(|connect| connect.operation_abi_id.as_str()),
        })
    }
}

pub struct WebSocketDefault<'a> {
    pub path: Option<&'a str>,
    pub receive_operation: &'a str,
    pub receive_operation_abi_id: &'a str,
    pub connect_operation: Option<&'a str>,
    pub connect_operation_abi_id: Option<&'a str>,
}

pub fn gateway_entry(manifest: &SkiffRuntimeManifest) -> GatewayEntry {
    GatewayEntry {
        http: http_entry(manifest),
        websocket: websocket_entry(manifest),
    }
}

pub fn websocket_entry(manifest: &SkiffRuntimeManifest) -> Option<WebSocketGatewayEntry> {
    let websocket = manifest.gateway.as_ref()?.websocket.as_ref()?;
    Some(WebSocketGatewayEntry {
        id: websocket.id.clone(),
        path: websocket.path.clone(),
        service_param: websocket.service_param.clone(),
        gateway_entry_identity: websocket.gateway_entry_identity.clone(),
        context: websocket
            .context
            .as_ref()
            .map(|schema| WebSocketContextEntry {
                ty: "ConnectionContext",
                schema: schema.clone(),
            }),
        context_expectation: websocket.context_expectation.clone(),
        connect: websocket
            .connect
            .as_ref()
            .map(|connect| WebSocketChannelEntry {
                operation: connect.operation.clone(),
                operation_abi_id: connect.operation_abi_id.clone(),
                adapter_args: connect.adapter_args.clone(),
                service_operation_target: connect.service_operation_target.clone(),
                service_protocol_identity: connect.service_protocol_identity.clone(),
                gateway_entry_identity: connect.gateway_entry_identity.clone(),
            }),
        receive: WebSocketChannelEntry {
            operation: websocket.receive.operation.clone(),
            operation_abi_id: websocket.receive.operation_abi_id.clone(),
            adapter_args: websocket.receive.adapter_args.clone(),
            service_operation_target: websocket.receive.service_operation_target.clone(),
            service_protocol_identity: websocket.receive.service_protocol_identity.clone(),
            gateway_entry_identity: websocket.receive.gateway_entry_identity.clone(),
        },
    })
}

pub fn http_entry(manifest: &SkiffRuntimeManifest) -> Option<HttpGatewayEntry> {
    let http = manifest.gateway.as_ref()?.http.as_ref()?;
    Some(HttpGatewayEntry {
        raw: http.raw.as_ref().map(|raw| HttpRawEntry {
            operation: raw.operation.clone(),
            target: raw.target.clone(),
        }),
        routes: http.routes.clone(),
    })
}

/// Service operation timeout configuration projected into the assembly.
// Mirrors the former `json!` which always emitted both fields (defaultMs as
// null when absent, methods as {} when empty), so no skip_serializing_if here.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeoutEntry {
    pub default_ms: Option<u64>,
    pub methods: BTreeMap<String, u64>,
}

pub fn timeout_entry(manifest: &SkiffRuntimeManifest) -> Option<TimeoutEntry> {
    manifest.timeout.as_ref().map(|timeout| TimeoutEntry {
        default_ms: timeout.default_ms,
        methods: timeout.methods.clone(),
    })
}
