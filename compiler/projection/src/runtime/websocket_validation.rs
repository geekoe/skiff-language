use crate::{
    error::ProjectionError,
    {contract::ContractProjection, WebSocketGatewayProjectionConfig},
};

use super::{
    is_projection_gateway_connect_result_type, is_projection_null_or_void_type,
    projection_gateway_connect_result_context_type, projection_type_matches_text,
    validate_operation_adapter_args, validate_receive_adapter_args,
    validate_websocket_adapter_sources,
};

pub fn validate_websocket_gateway(
    websocket: Option<&WebSocketGatewayProjectionConfig>,
    contract: &ContractProjection,
) -> Result<(), ProjectionError> {
    let Some(websocket) = websocket else {
        return Ok(());
    };

    let mut violations = Vec::new();
    if let Some(connect) = &websocket.connect {
        validate_websocket_adapter_sources(
            &connect.adapter_args,
            "connect",
            websocket.context.is_some(),
            &mut violations,
        );
        match contract.operation(&connect.operation) {
            Some(operation) => {
                validate_operation_adapter_args(
                    operation,
                    &connect.adapter_args,
                    &format!(
                        "gateway.websocket.connect.adapterArgs for {}",
                        connect.operation
                    ),
                    &mut violations,
                );
                if let Some(context) = &websocket.context {
                    let expected = format!("WebSocketConnectResult<{}>", context.context_type);
                    if !projection_gateway_connect_result_context_type(&operation.return_type)
                        .is_some_and(|actual_context| {
                            projection_type_matches_text(actual_context, &context.context_type)
                        })
                    {
                        violations.push(format!(
                            "gateway.websocket.connect operation {} must return {expected}",
                            connect.operation
                        ));
                    }
                } else if !is_projection_gateway_connect_result_type(&operation.return_type) {
                    violations.push(format!(
                        "gateway.websocket.connect operation {} must return WebSocketConnectResult<T>",
                        connect.operation
                    ));
                }
            }
            None => violations.push(format!(
                "gateway.websocket.connect references unknown service operation {}",
                connect.operation
            )),
        }
    }

    validate_websocket_adapter_sources(
        &websocket.receive.adapter_args,
        "receive",
        websocket.context.is_some(),
        &mut violations,
    );
    match contract.operation(&websocket.receive.operation) {
        Some(operation) => {
            validate_operation_adapter_args(
                operation,
                &websocket.receive.adapter_args,
                &format!(
                    "gateway.websocket.receive.adapterArgs for {}",
                    websocket.receive.operation
                ),
                &mut violations,
            );
            validate_receive_adapter_args(
                operation,
                &websocket.receive.adapter_args,
                websocket
                    .context
                    .as_ref()
                    .map(|context| context.context_type.as_str()),
                &mut violations,
            );
            if !is_projection_null_or_void_type(&operation.return_type) {
                violations.push(format!(
                    "gateway.websocket.receive operation {} must return null or void",
                    websocket.receive.operation
                ));
            }
        }
        None => violations.push(format!(
            "gateway.websocket.receive references unknown service operation {}",
            websocket.receive.operation
        )),
    }

    if violations.is_empty() {
        return Ok(());
    }
    violations.sort();
    violations.dedup();
    Err(ProjectionError::ContractValidation {
        message: violations
            .into_iter()
            .map(|violation| format!("- {violation}"))
            .collect::<Vec<_>>()
            .join("\n"),
    })
}
