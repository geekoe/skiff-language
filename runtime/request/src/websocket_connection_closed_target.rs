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
    RuntimeAssemblyEvalTarget, RuntimeWebSocketConnectionClosedCallable,
    RuntimeWebSocketConnectionClosedExecutionTarget,
};
use skiff_runtime_linked_program::{ExecutableAddr, ExecutableKind, SharedPackageCode};
use skiff_runtime_linker::LinkedGatewayEntry;

#[derive(Debug, Clone)]
pub struct RuntimeAssemblyWebSocketConnectionClosedTarget {
    eval: RuntimeAssemblyEvalTarget,
    selector: IngressSelector,
    entry: Arc<LinkedGatewayEntry>,
    websocket_entry_id: WebSocketEntryId,
    handler_addr: ExecutableAddr,
}

impl RuntimeAssemblyWebSocketConnectionClosedTarget {
    pub fn new(
        eval: RuntimeAssemblyEvalTarget,
        selector: IngressSelector,
        entry: Arc<LinkedGatewayEntry>,
    ) -> Result<Self, RuntimeAssemblyWebSocketConnectionClosedTargetError> {
        if !matches!(
            entry.protocol_surface().protocol,
            GatewayProtocolSurface::WebSocketConnect(_)
        ) || entry
            .close_adapter_plan()
            .is_none_or(|plan| plan.kind != GatewayAdapterKind::WebSocketConnectionClosed)
        {
            return Err(RuntimeAssemblyWebSocketConnectionClosedTargetError::PlanSurfaceMismatch);
        }
        let handler = entry
            .close_handler()
            .ok_or(RuntimeAssemblyWebSocketConnectionClosedTargetError::HandlerRequired)?;
        if entry.owner() != &eval.activation_context().identity().deployment {
            return Err(RuntimeAssemblyWebSocketConnectionClosedTargetError::OwnerMismatch);
        }
        if !Arc::ptr_eq(
            eval.activation_context(),
            eval.request_activation().receiver(),
        ) {
            return Err(
                RuntimeAssemblyWebSocketConnectionClosedTargetError::ActivationOwnerMismatch,
            );
        }
        eval.ensure_execution_ready().map_err(|error| {
            RuntimeAssemblyWebSocketConnectionClosedTargetError::ExecutionImage {
                detail: error.to_string(),
            }
        })?;
        validate_entry_facts(&entry)?;
        let websocket_entry_id =
            websocket_entry_id(&entry.owner().service_id, entry.gateway_entry_key()).map_err(
                |error| RuntimeAssemblyWebSocketConnectionClosedTargetError::InvalidEntryIdentity {
                    detail: error.to_string(),
                },
            )?;
        if !eval.activation_context().websocket_entry_matches(
            &selector,
            entry.gateway_entry_key(),
            entry.gateway_entry_identity(),
            &websocket_entry_id,
        ) {
            return Err(
                RuntimeAssemblyWebSocketConnectionClosedTargetError::ActivationEntryMismatch,
            );
        }
        let implementation = eval
            .execution_image()
            .shared_packages()
            .code_by_build(eval.activation_context().implementation_package_build_id())
            .ok_or(RuntimeAssemblyWebSocketConnectionClosedTargetError::ImplementationNotLinked)?;
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

    pub fn close_adapter_plan(&self) -> &GatewayAdapterPlan {
        self.entry
            .close_adapter_plan()
            .expect("target construction requires a WebSocket connection closed adapter plan")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeAssemblyWebSocketConnectionClosedTargetError {
    #[error(
        "WebSocket connection close gateway owner does not match the pinned request activation"
    )]
    OwnerMismatch,
    #[error("WebSocket connection close eval target does not pin the request receiver activation")]
    ActivationOwnerMismatch,
    #[error("WebSocket connection close activation record does not match the exact linked entry")]
    ActivationEntryMismatch,
    #[error("WebSocket connection close execution image is not ready: {detail}")]
    ExecutionImage { detail: String },
    #[error(
        "WebSocket connection close implementation package is absent from the execution image"
    )]
    ImplementationNotLinked,
    #[error("WebSocket connection close entry key is not compiler-owned")]
    InvalidKey,
    #[error("WebSocket connection close gateway identity is not canonical")]
    InvalidGatewayIdentity,
    #[error("WebSocket connection close internal entry identity is invalid: {detail}")]
    InvalidEntryIdentity { detail: String },
    #[error("WebSocket connection close protocol surface is invalid: {detail}")]
    InvalidProtocolSurface { detail: String },
    #[error("WebSocket connection close adapter plan is invalid: {detail}")]
    InvalidAdapterPlan { detail: String },
    #[error("WebSocket connection close adapter plan does not match its protocol surface")]
    PlanSurfaceMismatch,
    #[error("WebSocket connection close dispatch requires a real close handler")]
    HandlerRequired,
    #[error("WebSocket connection close handler signature and adapter plan do not match")]
    HandlerPlanMismatch,
    #[error("WebSocket connection close handler is not an exact private linked target: {detail}")]
    CallableMismatch { detail: String },
}

fn validate_entry_facts(
    entry: &LinkedGatewayEntry,
) -> Result<(), RuntimeAssemblyWebSocketConnectionClosedTargetError> {
    validate_entry_fact_view(WebSocketConnectionClosedEntryValidationFacts {
        key: entry.gateway_entry_key(),
        identity: entry.gateway_entry_identity(),
        surface: entry.protocol_surface(),
        plan: entry.close_adapter_plan(),
        handler_signature: entry.close_handler().map(|handler| handler.signature()),
        has_pre: entry.pre().is_some(),
        has_guard: entry.guard().is_some(),
    })
}

struct WebSocketConnectionClosedEntryValidationFacts<'a> {
    key: &'a GatewayEntryKey,
    identity: &'a GatewayEntryIdentity,
    surface: &'a GatewayEntryProtocolSurface,
    plan: Option<&'a GatewayAdapterPlan>,
    handler_signature: Option<&'a PackageCallableSignature>,
    has_pre: bool,
    has_guard: bool,
}

fn validate_entry_fact_view(
    facts: WebSocketConnectionClosedEntryValidationFacts<'_>,
) -> Result<(), RuntimeAssemblyWebSocketConnectionClosedTargetError> {
    let handler_signature = facts
        .handler_signature
        .ok_or(RuntimeAssemblyWebSocketConnectionClosedTargetError::HandlerRequired)?;
    let plan = facts
        .plan
        .ok_or(RuntimeAssemblyWebSocketConnectionClosedTargetError::PlanSurfaceMismatch)?;
    if facts.key.as_str() != WEBSOCKET_GATEWAY_ENTRY_KEY {
        return Err(RuntimeAssemblyWebSocketConnectionClosedTargetError::InvalidKey);
    }
    validate_gateway_entry_protocol_surface(facts.surface).map_err(|error| {
        RuntimeAssemblyWebSocketConnectionClosedTargetError::InvalidProtocolSurface {
            detail: error.to_string(),
        }
    })?;
    let expected_identity = gateway_entry_identity(facts.surface).map_err(|error| {
        RuntimeAssemblyWebSocketConnectionClosedTargetError::InvalidProtocolSurface {
            detail: error.to_string(),
        }
    })?;
    if facts.identity != &expected_identity {
        return Err(RuntimeAssemblyWebSocketConnectionClosedTargetError::InvalidGatewayIdentity);
    }
    if facts.has_pre
        || facts.has_guard
        || !matches!(
            facts.surface.protocol,
            GatewayProtocolSurface::WebSocketConnect(_)
        )
        || plan.kind != GatewayAdapterKind::WebSocketConnectionClosed
    {
        return Err(RuntimeAssemblyWebSocketConnectionClosedTargetError::PlanSurfaceMismatch);
    }
    validate_gateway_adapter_args(plan.kind, false, &plan.args).map_err(|error| {
        RuntimeAssemblyWebSocketConnectionClosedTargetError::InvalidAdapterPlan {
            detail: error.to_string(),
        }
    })?;
    if !handler_signature.type_params.is_empty() {
        return Err(RuntimeAssemblyWebSocketConnectionClosedTargetError::HandlerPlanMismatch);
    }
    let formal_names = handler_signature
        .parameters
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect::<BTreeSet<_>>();
    let actual_names = plan
        .args
        .iter()
        .map(|arg| arg.param.as_str())
        .collect::<BTreeSet<_>>();
    if formal_names.len() != handler_signature.parameters.len()
        || actual_names.len() != plan.args.len()
        || formal_names != actual_names
    {
        return Err(RuntimeAssemblyWebSocketConnectionClosedTargetError::HandlerPlanMismatch);
    }
    Ok(())
}

fn validate_callable(
    eval: &RuntimeAssemblyEvalTarget,
    implementation: &SharedPackageCode,
    callable_id: &PackageCallableId,
    target: &skiff_artifact_model::OperationTargetRef,
    signature: &PackageCallableSignature,
) -> Result<ExecutableAddr, RuntimeAssemblyWebSocketConnectionClosedTargetError> {
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

fn callable_mismatch(
    detail: impl Into<String>,
) -> RuntimeAssemblyWebSocketConnectionClosedTargetError {
    RuntimeAssemblyWebSocketConnectionClosedTargetError::CallableMismatch {
        detail: detail.into(),
    }
}

impl RuntimeWebSocketConnectionClosedExecutionTarget
    for RuntimeAssemblyWebSocketConnectionClosedTarget
{
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

    fn close_adapter_plan(&self) -> &GatewayAdapterPlan {
        self.entry
            .close_adapter_plan()
            .expect("target construction requires a WebSocket connection closed adapter plan")
    }

    fn close_handler(&self) -> RuntimeWebSocketConnectionClosedCallable<'_> {
        let handler = self
            .entry
            .close_handler()
            .expect("target construction requires a WebSocket connection closed close handler");
        RuntimeWebSocketConnectionClosedCallable {
            callable_id: handler.callable_id(),
            signature: handler.signature(),
            addr: &self.handler_addr,
        }
    }
}
