use serde_json::{json, Value};

use crate::{
    error::ProjectionError,
    runtime_manifest_model::{
        JsonSchema, RuntimeOperationManifest, RuntimeWebSocketContextExpectationManifest,
        RuntimeWebSocketGatewayManifest, RuntimeWebSocketOperationManifest,
    },
    WebSocketContextProjectionConfig,
};
use skiff_artifact_model::TypeRefIr;
use skiff_compiler_core::json_utils::{sha256_hex, stable_json_string};
use skiff_compiler_projection_input::EntryTypeSpec;

use crate::contract::{
    ContractNamedTypeKey, ContractProjection, ContractProjectionIndex, ContractTypeKey,
};

use super::{
    entrypoints::{WebSocketContextArtifact, WebSocketGatewayArtifact},
    package_runtime_schema_for_type_spec,
};

pub fn build_websocket_manifest(
    service_id: &str,
    websocket: &WebSocketGatewayArtifact,
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    operations: &[RuntimeOperationManifest],
) -> Result<RuntimeWebSocketGatewayManifest, ProjectionError> {
    let config = &websocket.config;
    let context_type_identity = websocket_context_type_identity(websocket);
    let context = websocket
        .context_type
        .as_ref()
        .map(|context| websocket_context_schema(context, contract_projection, projection_index))
        .or_else(|| {
            config
                .context
                .as_ref()
                .map(|context| websocket_context_schema_from_config(context, contract_projection))
        });
    let connect = config
        .connect
        .as_ref()
        .map(|connect| RuntimeWebSocketOperationManifest {
            operation: connect.operation.clone(),
            operation_abi_id: String::new(),
            adapter_args: connect.adapter_args.clone(),
            service_operation_target: None,
            service_protocol_identity: None,
            gateway_entry_identity: None,
        });
    let receive = RuntimeWebSocketOperationManifest {
        operation: config.receive.operation.clone(),
        operation_abi_id: String::new(),
        adapter_args: config.receive.adapter_args.clone(),
        service_operation_target: None,
        service_protocol_identity: None,
        gateway_entry_identity: None,
    };
    let mut manifest = RuntimeWebSocketGatewayManifest {
        id: config.id.clone(),
        path: config.path.clone(),
        service_param: config.service_param.clone(),
        context,
        context_expectation: RuntimeWebSocketContextExpectationManifest::Null,
        connect,
        receive,
        gateway_entry_identity: None,
    };
    apply_websocket_gateway_identities(
        service_id,
        &mut manifest,
        operations,
        context_type_identity.as_deref(),
    )?;
    Ok(manifest)
}

fn websocket_context_type_identity(websocket: &WebSocketGatewayArtifact) -> Option<String> {
    if websocket.config.connect.is_none() {
        return None;
    }
    websocket
        .context_type
        .as_ref()
        .map(|context| {
            websocket_context_type_identity_from_parts(
                Some(context.source_module.as_str()),
                &context.ty,
            )
        })
        .or_else(|| {
            websocket.config.context.as_ref().map(|context| {
                let source_module = context.source_module.as_deref();
                let ty = EntryTypeSpec {
                    name: context.context_type.clone(),
                    ir: TypeRefIr::Native {
                        name: context.context_type.clone(),
                        args: Vec::new(),
                    },
                    local_type_names: Default::default(),
                };
                websocket_context_type_identity_from_parts(source_module, &ty)
            })
        })
}

fn websocket_context_type_identity_from_parts(
    source_module: Option<&str>,
    ty: &EntryTypeSpec,
) -> String {
    websocket_gateway_identity(json!({
        "contextSourceModule": source_module,
        "contextType": ty.name,
    }))
}

fn websocket_context_schema(
    context: &WebSocketContextArtifact,
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
) -> JsonSchema {
    package_runtime_schema_for_type_spec(
        contract_projection,
        projection_index,
        &context.source_module,
        &context.ty,
        &context.schema_types,
        &context.service_type_names,
    )
}

fn websocket_context_schema_from_config(
    context: &WebSocketContextProjectionConfig,
    contract_projection: &ContractProjection,
) -> JsonSchema {
    let normalized = normalize_websocket_context_type(&context.context_type);
    if normalized.is_empty() {
        return JsonSchema::any();
    }

    if contract_projection.types.contains_key(normalized)
        || contract_projection.aliases.contains_key(normalized)
        || contract_projection.interfaces.contains_key(normalized)
    {
        return contract_projection.schema_for_type_key(&ContractTypeKey::Named(
            ContractNamedTypeKey::Public {
                symbol: normalized.to_string(),
            },
        ));
    }

    JsonSchema::any()
}

fn normalize_websocket_context_type(context_type: &str) -> &str {
    context_type
        .trim()
        .strip_prefix("root.")
        .unwrap_or(context_type.trim())
}

fn apply_websocket_gateway_identities(
    service_id: &str,
    websocket: &mut RuntimeWebSocketGatewayManifest,
    operations: &[RuntimeOperationManifest],
    context_type_identity: Option<&str>,
) -> Result<(), ProjectionError> {
    let connect_identity = match &mut websocket.connect {
        Some(connect) => {
            let connect_operation =
                require_operation_manifest(operations, "websocket.connect", &connect.operation)?;
            connect.service_operation_target = Some(connect_operation.target.clone());
            connect.operation_abi_id = connect_operation.operation_abi_id.clone();
            connect.service_protocol_identity =
                Some(connect_operation.service_protocol_identity.clone());
            websocket.context_expectation = RuntimeWebSocketContextExpectationManifest::Typed {
                connect_operation_abi_id: connect.operation_abi_id.clone(),
                context_type_identity: context_type_identity
                    .unwrap_or("skiff-context-type-v1:unknown")
                    .to_string(),
            };
            let identity = websocket_gateway_identity(json!({
                "adapterArgs": connect.adapter_args,
                "connect": true,
                "contextExpectation": websocket.context_expectation,
                "serviceId": service_id,
                "serviceParam": websocket.service_param,
                "websocketId": websocket.id,
            }));
            connect.gateway_entry_identity = Some(identity);
            Some(connect_entry_identity_input(
                connect,
                &websocket.context_expectation,
            ))
        }
        None => {
            websocket.context_expectation = RuntimeWebSocketContextExpectationManifest::Null;
            None
        }
    };

    let receive_operation = require_operation_manifest(
        operations,
        "websocket.receive",
        &websocket.receive.operation,
    )?;
    let receive_identity = websocket_gateway_identity(json!({
        "adapterArgs": websocket.receive.adapter_args,
        "contextExpectation": websocket.context_expectation,
        "serviceId": service_id,
        "serviceParam": websocket.service_param,
        "websocketId": websocket.id,
    }));
    websocket.receive.service_operation_target = Some(receive_operation.target.clone());
    websocket.receive.operation_abi_id = receive_operation.operation_abi_id.clone();
    websocket.receive.service_protocol_identity =
        Some(receive_operation.service_protocol_identity.clone());
    websocket.receive.gateway_entry_identity = Some(receive_identity);

    let entry_body = if let Some(connect) = connect_identity {
        json!({
            "connect": connect,
            "contextExpectation": websocket.context_expectation,
            "receive": receive_entry_identity_input(&websocket.receive, &websocket.context_expectation),
            "routes": [],
            "serviceId": service_id,
            "serviceParam": websocket.service_param,
            "websocketId": websocket.id,
        })
    } else {
        json!({
            "connect": Value::Null,
            "contextExpectation": websocket.context_expectation,
            "receive": receive_entry_identity_input(&websocket.receive, &websocket.context_expectation),
            "routes": [],
            "serviceId": service_id,
            "serviceParam": websocket.service_param,
            "websocketId": websocket.id,
        })
    };
    websocket.gateway_entry_identity = Some(websocket_gateway_identity(entry_body));
    Ok(())
}

fn operation_manifest<'a>(
    operations: &'a [RuntimeOperationManifest],
    operation_name: &str,
) -> Option<&'a RuntimeOperationManifest> {
    operations
        .iter()
        .find(|operation| operation.operation == operation_name)
}

fn require_operation_manifest<'a>(
    operations: &'a [RuntimeOperationManifest],
    label: &str,
    operation_name: &str,
) -> Result<&'a RuntimeOperationManifest, ProjectionError> {
    let operation = operation_manifest(operations, operation_name).ok_or_else(|| {
        websocket_manifest_error(format!(
            "{label} operation {operation_name} has no runtime operation manifest"
        ))
    })?;
    if operation.operation_abi_id.is_empty() {
        return Err(websocket_manifest_error(format!(
            "{label} operation {operation_name} has empty operation ABI id"
        )));
    }
    Ok(operation)
}

fn websocket_manifest_error(message: String) -> ProjectionError {
    ProjectionError::ContractValidation { message }
}

fn connect_entry_identity_input(
    connect: &RuntimeWebSocketOperationManifest,
    context_expectation: &RuntimeWebSocketContextExpectationManifest,
) -> Value {
    json!({
        "adapterArgs": connect.adapter_args,
        "contextExpectation": context_expectation,
    })
}

fn receive_entry_identity_input(
    receive: &RuntimeWebSocketOperationManifest,
    context_expectation: &RuntimeWebSocketContextExpectationManifest,
) -> Value {
    json!({
        "adapterArgs": receive.adapter_args,
        "contextExpectation": context_expectation,
    })
}

fn websocket_gateway_identity(value: Value) -> String {
    let stable = stable_json_string(&value);
    format!("skiff-gateway-v1:sha256:{}", sha256_hex(stable.as_bytes()))
}
