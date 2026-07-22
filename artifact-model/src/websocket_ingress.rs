use std::fmt;

use crate::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryErrorContract,
    BoundaryStreamContract, ContractOperationId, ContractTypeId, ContractTypeRef, ServiceContract,
};

pub const WEBSOCKET_INGRESS_OPERATION_NAME: &str = "websocket";
pub const WEBSOCKET_INGRESS_EVENT_TYPE: &str = "std.websocket.WebSocketIngressEvent";
pub const WEBSOCKET_CONNECT_RESULT_TYPE: &str = "std.websocket.WebSocketConnectResult";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSocketIngressContext {
    Null,
    Contract(ContractTypeId),
}

impl WebSocketIngressContext {
    pub fn contract_type_id(&self) -> Option<&ContractTypeId> {
        match self {
            Self::Null => None,
            Self::Contract(contract_type_id) => Some(contract_type_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketIngressContractError {
    message: String,
}

impl WebSocketIngressContractError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for WebSocketIngressContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for WebSocketIngressContractError {}

/// Validates the single canonical WebSocket ingress ABI and returns its contract-owned Context.
pub fn websocket_ingress_context(
    contract: &ServiceContract,
    operation_id: &ContractOperationId,
) -> Result<WebSocketIngressContext, WebSocketIngressContractError> {
    let descriptor = contract.operations.get(operation_id).ok_or_else(|| {
        WebSocketIngressContractError::new(format!(
            "contract has no WebSocket ingress operation {operation_id}"
        ))
    })?;
    if descriptor.stable_key != WEBSOCKET_INGRESS_OPERATION_NAME {
        return Err(WebSocketIngressContractError::new(format!(
            "WebSocket ingress operation must be named {WEBSOCKET_INGRESS_OPERATION_NAME}"
        )));
    }
    let operation = &descriptor.contract;
    let [parameter] = operation.parameters.as_slice() else {
        return Err(WebSocketIngressContractError::new(
            "WebSocket ingress operation must declare exactly one event parameter",
        ));
    };
    if parameter.name != "event" {
        return Err(WebSocketIngressContractError::new(
            "WebSocket ingress parameter must be named event",
        ));
    }
    let context =
        generic_argument(&parameter.ty, WEBSOCKET_INGRESS_EVENT_TYPE).ok_or_else(|| {
            WebSocketIngressContractError::new(format!(
                "WebSocket ingress event must be {WEBSOCKET_INGRESS_EVENT_TYPE}<Context>"
            ))
        })?;
    let context = match context {
        ContractTypeRef::Builtin { name, arguments } if name == "null" && arguments.is_empty() => {
            WebSocketIngressContext::Null
        }
        ContractTypeRef::Contract { contract_type_id }
            if contract.boundary_schema.contains_key(contract_type_id) =>
        {
            WebSocketIngressContext::Contract(contract_type_id.clone())
        }
        ContractTypeRef::Contract { .. } => {
            return Err(WebSocketIngressContractError::new(
                "WebSocket ingress Context must be owned by the same ServiceContract",
            ))
        }
        _ => {
            return Err(WebSocketIngressContractError::new(
                "WebSocket ingress Context must be null or a contract-owned nominal type",
            ))
        }
    };
    let ContractTypeRef::Nullable { inner } = &operation.return_value.ty else {
        return Err(WebSocketIngressContractError::new(format!(
            "WebSocket ingress return must be {WEBSOCKET_CONNECT_RESULT_TYPE}<Context>?"
        )));
    };
    let return_context =
        generic_argument(inner, WEBSOCKET_CONNECT_RESULT_TYPE).ok_or_else(|| {
            WebSocketIngressContractError::new(format!(
                "WebSocket ingress return must be {WEBSOCKET_CONNECT_RESULT_TYPE}<Context>?"
            ))
        })?;
    if return_context != &generic_context_ref(&context) {
        return Err(WebSocketIngressContractError::new(
            "WebSocket ingress event and result Context must be identical",
        ));
    }
    if !matches!(operation.errors, BoundaryErrorContract::None) {
        return Err(WebSocketIngressContractError::new(
            "WebSocket ingress operation must not declare throws",
        ));
    }
    if !matches!(operation.stream, BoundaryStreamContract::Unary) {
        return Err(WebSocketIngressContractError::new(
            "WebSocket ingress operation must be unary",
        ));
    }
    if operation.may_suspend {
        return Err(WebSocketIngressContractError::new(
            "WebSocket ingress operation must not suspend",
        ));
    }
    if !matches!(
        operation.cancellation,
        BoundaryCancellationContract::NotCancellable
    ) || !matches!(operation.callbacks, BoundaryCallbackContract::None)
    {
        return Err(WebSocketIngressContractError::new(
            "WebSocket ingress operation must not declare cancellation or callbacks",
        ));
    }
    Ok(context)
}

fn generic_argument<'a>(ty: &'a ContractTypeRef, name: &str) -> Option<&'a ContractTypeRef> {
    let ContractTypeRef::Builtin {
        name: actual,
        arguments,
    } = ty
    else {
        return None;
    };
    let [argument] = arguments.as_slice() else {
        return None;
    };
    (actual == name).then_some(argument)
}

fn generic_context_ref(context: &WebSocketIngressContext) -> ContractTypeRef {
    match context {
        WebSocketIngressContext::Null => ContractTypeRef::builtin("null"),
        WebSocketIngressContext::Contract(contract_type_id) => {
            ContractTypeRef::contract(contract_type_id.clone())
        }
    }
}
