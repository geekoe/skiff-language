use std::{collections::BTreeSet, sync::Arc};

use skiff_artifact_identity::{
    gateway_entry_identity, validate_gateway_entry_protocol_surface, websocket_entry_id,
};
use skiff_artifact_model::{
    validate_gateway_adapter_args, GatewayAdapterKind, GatewayAdapterPlan, GatewayEntryIdentity,
    GatewayEntryKey, GatewayEntryProtocolSurface, GatewayProtocolSurface, IngressSelector,
    OperationCallableKind, PackageCallableId, PackageCallableSignature, PackageLocalAbiSymbol,
    ServiceDeploymentRef, WebSocketEntryId, WEBSOCKET_GATEWAY_ENTRY_KEY,
};
use skiff_runtime_eval::{
    RuntimeAssemblyEvalTarget, RuntimeWebSocketConnectCallable,
    RuntimeWebSocketConnectExecutionTarget,
};
use skiff_runtime_linked_program::{ExecutableAddr, ExecutableKind, SharedPackageCode};
use skiff_runtime_linker::LinkedGatewayEntry;

#[derive(Debug, Clone)]
pub struct RuntimeAssemblyWebSocketConnectTarget {
    eval: RuntimeAssemblyEvalTarget,
    selector: IngressSelector,
    entry: Arc<LinkedGatewayEntry>,
    websocket_entry_id: WebSocketEntryId,
    handler_addr: ExecutableAddr,
}

impl RuntimeAssemblyWebSocketConnectTarget {
    pub fn new(
        eval: RuntimeAssemblyEvalTarget,
        selector: IngressSelector,
        entry: Arc<LinkedGatewayEntry>,
    ) -> Result<Self, RuntimeAssemblyWebSocketConnectTargetError> {
        if !matches!(
            entry.protocol_surface().protocol,
            GatewayProtocolSurface::WebSocketConnect(_)
        ) || entry.adapter_plan().kind != GatewayAdapterKind::WebSocketConnect
        {
            return Err(RuntimeAssemblyWebSocketConnectTargetError::PlanSurfaceMismatch);
        }
        let handler = entry
            .optional_handler()
            .ok_or(RuntimeAssemblyWebSocketConnectTargetError::HandlerRequired)?;
        if entry.owner() != &eval.activation_context().identity().deployment {
            return Err(RuntimeAssemblyWebSocketConnectTargetError::OwnerMismatch);
        }
        if !Arc::ptr_eq(
            eval.activation_context(),
            eval.request_activation().receiver(),
        ) {
            return Err(RuntimeAssemblyWebSocketConnectTargetError::ActivationOwnerMismatch);
        }
        eval.ensure_execution_ready().map_err(|error| {
            RuntimeAssemblyWebSocketConnectTargetError::ExecutionImage {
                detail: error.to_string(),
            }
        })?;
        validate_entry_facts(&entry)?;
        let websocket_entry_id =
            websocket_entry_id(&entry.owner().service_id, entry.gateway_entry_key()).map_err(
                |error| RuntimeAssemblyWebSocketConnectTargetError::InvalidEntryIdentity {
                    detail: error.to_string(),
                },
            )?;
        if !eval.activation_context().websocket_entry_matches(
            &selector,
            entry.gateway_entry_key(),
            entry.gateway_entry_identity(),
            &websocket_entry_id,
        ) {
            return Err(RuntimeAssemblyWebSocketConnectTargetError::ActivationEntryMismatch);
        }
        let implementation = eval
            .execution_image()
            .shared_packages()
            .code_by_build(eval.activation_context().implementation_package_build_id())
            .ok_or(RuntimeAssemblyWebSocketConnectTargetError::ImplementationNotLinked)?;
        let handler_addr = validate_callable(
            &eval,
            implementation,
            handler.callable_id(),
            handler.target(),
            handler.signature(),
        )?;
        Ok(Self {
            eval,
            selector,
            entry,
            websocket_entry_id,
            handler_addr,
        })
    }

    pub fn eval(&self) -> &RuntimeAssemblyEvalTarget {
        &self.eval
    }

    pub fn selector(&self) -> &IngressSelector {
        &self.selector
    }

    pub fn owner(&self) -> &ServiceDeploymentRef {
        self.entry.owner()
    }

    pub fn gateway_entry_key(&self) -> &GatewayEntryKey {
        self.entry.gateway_entry_key()
    }

    pub fn gateway_entry_identity(&self) -> &GatewayEntryIdentity {
        self.entry.gateway_entry_identity()
    }

    pub fn websocket_entry_id(&self) -> &WebSocketEntryId {
        &self.websocket_entry_id
    }

    pub fn protocol_surface(&self) -> &GatewayEntryProtocolSurface {
        self.entry.protocol_surface()
    }

    pub fn adapter_plan(&self) -> &GatewayAdapterPlan {
        self.entry.adapter_plan()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeAssemblyWebSocketConnectTargetError {
    #[error("WebSocket connect gateway owner does not match the pinned request activation")]
    OwnerMismatch,
    #[error("WebSocket connect eval target does not pin the request receiver activation")]
    ActivationOwnerMismatch,
    #[error("WebSocket connect activation record does not match the exact linked entry")]
    ActivationEntryMismatch,
    #[error("WebSocket connect execution image is not ready: {detail}")]
    ExecutionImage { detail: String },
    #[error("WebSocket connect implementation package is absent from the execution image")]
    ImplementationNotLinked,
    #[error("WebSocket connect entry key is not compiler-owned")]
    InvalidKey,
    #[error("WebSocket connect gateway identity is not canonical")]
    InvalidGatewayIdentity,
    #[error("WebSocket connect internal entry identity is invalid: {detail}")]
    InvalidEntryIdentity { detail: String },
    #[error("WebSocket connect protocol surface is invalid: {detail}")]
    InvalidProtocolSurface { detail: String },
    #[error("WebSocket connect adapter plan is invalid: {detail}")]
    InvalidAdapterPlan { detail: String },
    #[error("WebSocket connect adapter plan does not match its protocol surface")]
    PlanSurfaceMismatch,
    #[error("WebSocket connect dispatch requires a real handler")]
    HandlerRequired,
    #[error("WebSocket connect handler signature and adapter plan do not match")]
    HandlerPlanMismatch,
    #[error("WebSocket connect handler is not an exact private linked target: {detail}")]
    CallableMismatch { detail: String },
}

fn validate_entry_facts(
    entry: &LinkedGatewayEntry,
) -> Result<(), RuntimeAssemblyWebSocketConnectTargetError> {
    let handler = entry
        .optional_handler()
        .ok_or(RuntimeAssemblyWebSocketConnectTargetError::HandlerRequired)?;
    if entry.gateway_entry_key().as_str() != WEBSOCKET_GATEWAY_ENTRY_KEY {
        return Err(RuntimeAssemblyWebSocketConnectTargetError::InvalidKey);
    }
    validate_gateway_entry_protocol_surface(entry.protocol_surface()).map_err(|error| {
        RuntimeAssemblyWebSocketConnectTargetError::InvalidProtocolSurface {
            detail: error.to_string(),
        }
    })?;
    let expected_identity = gateway_entry_identity(entry.protocol_surface()).map_err(|error| {
        RuntimeAssemblyWebSocketConnectTargetError::InvalidProtocolSurface {
            detail: error.to_string(),
        }
    })?;
    if entry.gateway_entry_identity() != &expected_identity {
        return Err(RuntimeAssemblyWebSocketConnectTargetError::InvalidGatewayIdentity);
    }
    if entry.pre().is_some()
        || entry.guard().is_some()
        || !matches!(
            entry.protocol_surface().protocol,
            GatewayProtocolSurface::WebSocketConnect(_)
        )
        || entry.adapter_plan().kind != GatewayAdapterKind::WebSocketConnect
    {
        return Err(RuntimeAssemblyWebSocketConnectTargetError::PlanSurfaceMismatch);
    }
    validate_gateway_adapter_args(entry.adapter_plan().kind, false, &entry.adapter_plan().args)
        .map_err(
            |error| RuntimeAssemblyWebSocketConnectTargetError::InvalidAdapterPlan {
                detail: error.to_string(),
            },
        )?;
    if !handler.signature().type_params.is_empty() {
        return Err(RuntimeAssemblyWebSocketConnectTargetError::HandlerPlanMismatch);
    }
    let formal_names = handler
        .signature()
        .parameters
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect::<BTreeSet<_>>();
    let actual_names = entry
        .adapter_plan()
        .args
        .iter()
        .map(|arg| arg.param.as_str())
        .collect::<BTreeSet<_>>();
    if formal_names.len() != handler.signature().parameters.len()
        || actual_names.len() != entry.adapter_plan().args.len()
        || formal_names != actual_names
    {
        return Err(RuntimeAssemblyWebSocketConnectTargetError::HandlerPlanMismatch);
    }
    Ok(())
}

fn validate_callable(
    eval: &RuntimeAssemblyEvalTarget,
    implementation: &SharedPackageCode,
    callable_id: &PackageCallableId,
    target: &skiff_artifact_model::OperationTargetRef,
    signature: &PackageCallableSignature,
) -> Result<ExecutableAddr, RuntimeAssemblyWebSocketConnectTargetError> {
    if target.callable_abi_id != callable_id.as_str()
        || target.callable_kind != OperationCallableKind::InternalFunction
        || implementation.callable_target(callable_id) != Some(target)
    {
        return Err(callable_mismatch("callable target is not exact"));
    }
    let exact_signatures = implementation
        .artifact()
        .package_local_abi
        .implementation_symbols
        .values()
        .filter_map(|symbol| match symbol {
            PackageLocalAbiSymbol::Callable {
                callable_id: candidate,
                signature,
            } if candidate == callable_id => Some(signature),
            _ => None,
        })
        .collect::<Vec<_>>();
    if exact_signatures.as_slice() != [signature] {
        return Err(callable_mismatch(
            "implementation signature is absent, ambiguous, or mismatched",
        ));
    }
    let executable = eval
        .execution_image()
        .entry_executable(implementation.package_build_id(), target)
        .map_err(|error| callable_mismatch(error.to_string()))?;
    let linked = executable.executable();
    if linked.kind != ExecutableKind::Function
        || linked.self_type.is_some()
        || linked.return_type.is_none()
        || linked.type_params != signature.type_params
        || linked.may_suspend != signature.may_suspend
        || linked.params.len() != signature.parameters.len()
        || linked
            .params
            .iter()
            .zip(&signature.parameters)
            .any(|(linked, declared)| linked.name != declared.name)
    {
        return Err(callable_mismatch(
            "execution image signature does not match the linked callable signature",
        ));
    }
    Ok(executable.addr().clone())
}

fn callable_mismatch(detail: impl Into<String>) -> RuntimeAssemblyWebSocketConnectTargetError {
    RuntimeAssemblyWebSocketConnectTargetError::CallableMismatch {
        detail: detail.into(),
    }
}

impl RuntimeWebSocketConnectExecutionTarget for RuntimeAssemblyWebSocketConnectTarget {
    fn eval_target(&self) -> &RuntimeAssemblyEvalTarget {
        &self.eval
    }

    fn gateway_entry_key(&self) -> &GatewayEntryKey {
        self.entry.gateway_entry_key()
    }

    fn gateway_entry_identity(&self) -> &GatewayEntryIdentity {
        self.entry.gateway_entry_identity()
    }

    fn websocket_entry_id(&self) -> &WebSocketEntryId {
        &self.websocket_entry_id
    }

    fn protocol_surface(&self) -> &GatewayEntryProtocolSurface {
        self.entry.protocol_surface()
    }

    fn adapter_plan(&self) -> &GatewayAdapterPlan {
        self.entry.adapter_plan()
    }

    fn handler(&self) -> RuntimeWebSocketConnectCallable<'_> {
        let handler = self
            .entry
            .optional_handler()
            .expect("target construction requires a WebSocket connect handler");
        RuntimeWebSocketConnectCallable {
            callable_id: handler.callable_id(),
            signature: handler.signature(),
            addr: &self.handler_addr,
        }
    }
}
