use std::{collections::BTreeSet, sync::Arc};

use skiff_artifact_identity::{gateway_entry_identity, validate_gateway_entry_protocol_surface};
use skiff_artifact_model::{
    AssemblyIdentity, GatewayAdapterKind, GatewayAdapterPlan, GatewayAdapterSource,
    GatewayEntryIdentity, GatewayEntryKey, GatewayEntryProtocolSurface, GatewayProtocolSurface,
    GatewayWebSocketRpcProfile, IngressProtocol, IngressSelector, OperationCallableKind,
    PackageBuildId, PackageCallableId, PackageCallableSignature, PackageLocalAbiSymbol,
    ServiceDeploymentRef, WebSocketEntryId, WEBSOCKET_GATEWAY_ENTRY_KEY,
};
use skiff_runtime_eval::RuntimeAssemblyEvalTarget;
use skiff_runtime_linked_program::{ExecutableAddr, ExecutableKind, SharedPackageCode};
use skiff_runtime_linker::LinkedGatewayEntry;

/// Exact physical WebSocket facts retained beside a method target.
///
/// The method entry cannot derive these facts from its own key. Host admission constructs this
/// value only from the compiler-owned physical entry in the same immutable assembly candidate.
#[derive(Debug, Clone)]
pub struct RuntimeAssemblyWebSocketJsonRpcPhysicalRoute {
    selector: IngressSelector,
    gateway_entry_key: GatewayEntryKey,
    gateway_entry_identity: GatewayEntryIdentity,
    websocket_entry_id: WebSocketEntryId,
}

impl RuntimeAssemblyWebSocketJsonRpcPhysicalRoute {
    pub fn new(
        selector: IngressSelector,
        gateway_entry_key: GatewayEntryKey,
        gateway_entry_identity: GatewayEntryIdentity,
        websocket_entry_id: WebSocketEntryId,
    ) -> Self {
        Self {
            selector,
            gateway_entry_key,
            gateway_entry_identity,
            websocket_entry_id,
        }
    }

    pub fn selector(&self) -> &IngressSelector {
        &self.selector
    }

    pub fn gateway_entry_key(&self) -> &GatewayEntryKey {
        &self.gateway_entry_key
    }

    pub fn gateway_entry_identity(&self) -> &GatewayEntryIdentity {
        &self.gateway_entry_identity
    }

    pub fn websocket_entry_id(&self) -> &WebSocketEntryId {
        &self.websocket_entry_id
    }
}

/// Generation-pinned, handler-ready target for one WebSocket JSON-RPC method.
///
/// Construction resolves no current assembly state. The target owns the exact linked method
/// entry, callable address, signature owner and adapter source plan from the already-pinned
/// execution image. E0b may consume these facts, but this type does not decode or execute values.
#[derive(Debug, Clone)]
pub struct RuntimeAssemblyWebSocketJsonRpcTarget {
    eval: RuntimeAssemblyEvalTarget,
    selector: IngressSelector,
    physical: RuntimeAssemblyWebSocketJsonRpcPhysicalRoute,
    entry: Arc<LinkedGatewayEntry>,
    profile: GatewayWebSocketRpcProfile,
    handler_addr: ExecutableAddr,
}

impl RuntimeAssemblyWebSocketJsonRpcTarget {
    pub fn new(
        eval: RuntimeAssemblyEvalTarget,
        selector: IngressSelector,
        physical: RuntimeAssemblyWebSocketJsonRpcPhysicalRoute,
        entry: Arc<LinkedGatewayEntry>,
    ) -> Result<Self, RuntimeAssemblyWebSocketJsonRpcTargetError> {
        let profile = validate_entry_facts(&selector, &physical, &entry)?;
        if entry.owner() != &eval.activation_context().identity().deployment {
            return Err(RuntimeAssemblyWebSocketJsonRpcTargetError::OwnerMismatch);
        }
        if !Arc::ptr_eq(
            eval.activation_context(),
            eval.request_activation().receiver(),
        ) {
            return Err(RuntimeAssemblyWebSocketJsonRpcTargetError::ActivationOwnerMismatch);
        }
        if !eval.activation_context().websocket_entry_matches(
            physical.selector(),
            physical.gateway_entry_key(),
            physical.gateway_entry_identity(),
            physical.websocket_entry_id(),
        ) {
            return Err(RuntimeAssemblyWebSocketJsonRpcTargetError::PhysicalEntryMismatch);
        }
        eval.ensure_execution_ready().map_err(|error| {
            RuntimeAssemblyWebSocketJsonRpcTargetError::ExecutionImage {
                detail: error.to_string(),
            }
        })?;
        let implementation = eval
            .execution_image()
            .shared_packages()
            .code_by_build(eval.activation_context().implementation_package_build_id())
            .ok_or(RuntimeAssemblyWebSocketJsonRpcTargetError::ImplementationNotLinked)?;
        let handler = entry
            .optional_handler()
            .ok_or(RuntimeAssemblyWebSocketJsonRpcTargetError::HandlerRequired)?;
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
            physical,
            entry,
            profile,
            handler_addr,
        })
    }

    pub fn eval(&self) -> &RuntimeAssemblyEvalTarget {
        &self.eval
    }

    pub fn assembly_identity(&self) -> &AssemblyIdentity {
        &self.eval.activation_context().identity().assembly_identity
    }

    pub fn assembly_generation(&self) -> u64 {
        self.eval
            .activation_context()
            .identity()
            .assembly_generation
    }

    pub fn owner(&self) -> &ServiceDeploymentRef {
        self.entry.owner()
    }

    pub fn implementation_package_build_id(&self) -> &PackageBuildId {
        self.eval
            .activation_context()
            .implementation_package_build_id()
    }

    pub fn selector(&self) -> &IngressSelector {
        &self.selector
    }

    pub fn method(&self) -> &str {
        self.selector
            .method
            .as_deref()
            .expect("target construction requires an exact method selector")
    }

    pub fn gateway_entry_key(&self) -> &GatewayEntryKey {
        self.entry.gateway_entry_key()
    }

    pub fn gateway_entry_identity(&self) -> &GatewayEntryIdentity {
        self.entry.gateway_entry_identity()
    }

    pub fn physical_route(&self) -> &RuntimeAssemblyWebSocketJsonRpcPhysicalRoute {
        &self.physical
    }

    pub fn websocket_entry_id(&self) -> &WebSocketEntryId {
        self.physical.websocket_entry_id()
    }

    pub fn profile(&self) -> GatewayWebSocketRpcProfile {
        self.profile
    }

    pub fn protocol_surface(&self) -> &GatewayEntryProtocolSurface {
        self.entry.protocol_surface()
    }

    pub fn adapter_plan(&self) -> &GatewayAdapterPlan {
        self.entry.adapter_plan()
    }

    pub fn handler_callable_id(&self) -> &PackageCallableId {
        self.entry
            .optional_handler()
            .expect("target construction requires a method handler")
            .callable_id()
    }

    pub fn handler_signature(&self) -> &PackageCallableSignature {
        self.entry
            .optional_handler()
            .expect("target construction requires a method handler")
            .signature()
    }

    pub fn handler_addr(&self) -> &ExecutableAddr {
        &self.handler_addr
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeAssemblyWebSocketJsonRpcTargetError {
    #[error("WebSocket JSON-RPC method owner does not match the pinned request activation")]
    OwnerMismatch,
    #[error("WebSocket JSON-RPC eval target does not pin the request receiver activation")]
    ActivationOwnerMismatch,
    #[error("WebSocket JSON-RPC physical route does not match the pinned activation")]
    PhysicalEntryMismatch,
    #[error("WebSocket JSON-RPC execution image is not ready: {detail}")]
    ExecutionImage { detail: String },
    #[error("WebSocket JSON-RPC implementation package is absent from the execution image")]
    ImplementationNotLinked,
    #[error("WebSocket JSON-RPC method key cannot use the compiler-owned physical key")]
    InvalidKey,
    #[error("WebSocket JSON-RPC method selector is not an exact sibling of the physical route")]
    SelectorMismatch,
    #[error("WebSocket JSON-RPC method gateway identity is not canonical")]
    InvalidGatewayIdentity,
    #[error("WebSocket JSON-RPC method protocol surface is invalid: {detail}")]
    InvalidProtocolSurface { detail: String },
    #[error("WebSocket JSON-RPC adapter plan is invalid: {detail}")]
    InvalidAdapterPlan { detail: String },
    #[error("WebSocket JSON-RPC adapter plan does not match its protocol surface")]
    PlanSurfaceMismatch,
    #[error("WebSocket JSON-RPC dispatch requires a real method handler")]
    HandlerRequired,
    #[error("WebSocket JSON-RPC handler signature and adapter plan do not match")]
    HandlerPlanMismatch,
    #[error("WebSocket JSON-RPC handler is not an exact private linked target: {detail}")]
    CallableMismatch { detail: String },
}

fn validate_entry_facts(
    selector: &IngressSelector,
    physical: &RuntimeAssemblyWebSocketJsonRpcPhysicalRoute,
    entry: &LinkedGatewayEntry,
) -> Result<GatewayWebSocketRpcProfile, RuntimeAssemblyWebSocketJsonRpcTargetError> {
    validate_entry_fact_view(WebSocketJsonRpcEntryValidationFacts {
        key: entry.gateway_entry_key(),
        identity: entry.gateway_entry_identity(),
        selector,
        physical_selector: physical.selector(),
        surface: entry.protocol_surface(),
        plan: entry.adapter_plan(),
        handler_signature: entry.optional_handler().map(|handler| handler.signature()),
        has_pre: entry.pre().is_some(),
        has_guard: entry.guard().is_some(),
    })
}

struct WebSocketJsonRpcEntryValidationFacts<'a> {
    key: &'a GatewayEntryKey,
    identity: &'a GatewayEntryIdentity,
    selector: &'a IngressSelector,
    physical_selector: &'a IngressSelector,
    surface: &'a GatewayEntryProtocolSurface,
    plan: &'a GatewayAdapterPlan,
    handler_signature: Option<&'a PackageCallableSignature>,
    has_pre: bool,
    has_guard: bool,
}

fn validate_entry_fact_view(
    facts: WebSocketJsonRpcEntryValidationFacts<'_>,
) -> Result<GatewayWebSocketRpcProfile, RuntimeAssemblyWebSocketJsonRpcTargetError> {
    if facts.key.as_str() == WEBSOCKET_GATEWAY_ENTRY_KEY {
        return Err(RuntimeAssemblyWebSocketJsonRpcTargetError::InvalidKey);
    }
    if facts.selector.protocol != IngressProtocol::WebSocket
        || facts.selector.method.as_deref().is_none_or(str::is_empty)
        || facts.physical_selector.protocol != IngressProtocol::WebSocket
        || facts.physical_selector.method.is_some()
        || facts.selector.path != facts.physical_selector.path
    {
        return Err(RuntimeAssemblyWebSocketJsonRpcTargetError::SelectorMismatch);
    }
    validate_gateway_entry_protocol_surface(facts.surface).map_err(|error| {
        RuntimeAssemblyWebSocketJsonRpcTargetError::InvalidProtocolSurface {
            detail: error.to_string(),
        }
    })?;
    let expected_identity = gateway_entry_identity(facts.surface).map_err(|error| {
        RuntimeAssemblyWebSocketJsonRpcTargetError::InvalidProtocolSurface {
            detail: error.to_string(),
        }
    })?;
    if facts.identity != &expected_identity {
        return Err(RuntimeAssemblyWebSocketJsonRpcTargetError::InvalidGatewayIdentity);
    }
    let GatewayProtocolSurface::WebSocketJsonRpc(surface) = &facts.surface.protocol else {
        return Err(RuntimeAssemblyWebSocketJsonRpcTargetError::PlanSurfaceMismatch);
    };
    if facts.has_pre
        || facts.has_guard
        || facts.plan.kind != GatewayAdapterKind::WebSocketJsonRpc
        || surface.dispatch_mode != skiff_artifact_model::GatewayDispatchMode::Unary
    {
        return Err(RuntimeAssemblyWebSocketJsonRpcTargetError::PlanSurfaceMismatch);
    }
    skiff_artifact_model::validate_gateway_adapter_args(facts.plan.kind, false, &facts.plan.args)
        .map_err(
        |error| RuntimeAssemblyWebSocketJsonRpcTargetError::InvalidAdapterPlan {
            detail: error.to_string(),
        },
    )?;
    if facts
        .plan
        .args
        .iter()
        .filter(|arg| arg.source == GatewayAdapterSource::WebSocketJsonRpcParams)
        .count()
        != 1
    {
        return Err(
            RuntimeAssemblyWebSocketJsonRpcTargetError::InvalidAdapterPlan {
                detail: "method adapter must contain exactly one websocket.jsonRpcParams source"
                    .to_string(),
            },
        );
    }
    let handler_signature = facts
        .handler_signature
        .ok_or(RuntimeAssemblyWebSocketJsonRpcTargetError::HandlerRequired)?;
    if !handler_signature.type_params.is_empty() {
        return Err(RuntimeAssemblyWebSocketJsonRpcTargetError::HandlerPlanMismatch);
    }
    let formal_names = handler_signature
        .parameters
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect::<BTreeSet<_>>();
    let actual_names = facts
        .plan
        .args
        .iter()
        .map(|arg| arg.param.as_str())
        .collect::<BTreeSet<_>>();
    if formal_names.len() != handler_signature.parameters.len()
        || actual_names.len() != facts.plan.args.len()
        || formal_names != actual_names
    {
        return Err(RuntimeAssemblyWebSocketJsonRpcTargetError::HandlerPlanMismatch);
    }
    Ok(surface.profile)
}

fn validate_callable(
    eval: &RuntimeAssemblyEvalTarget,
    implementation: &SharedPackageCode,
    callable_id: &PackageCallableId,
    target: &skiff_artifact_model::OperationTargetRef,
    signature: &PackageCallableSignature,
) -> Result<ExecutableAddr, RuntimeAssemblyWebSocketJsonRpcTargetError> {
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

fn callable_mismatch(detail: impl Into<String>) -> RuntimeAssemblyWebSocketJsonRpcTargetError {
    RuntimeAssemblyWebSocketJsonRpcTargetError::CallableMismatch {
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use skiff_artifact_model::{
        GatewayAdapterArg, GatewayDispatchMode, GatewayExternalErrorProjection,
        GatewayExternalSchema, GatewayWebSocketJsonRpcProtocolSurface, PackageCallableParameter,
        PackageTypeRef, TypeRefIr,
    };

    fn surface() -> GatewayEntryProtocolSurface {
        GatewayEntryProtocolSurface {
            protocol: GatewayProtocolSurface::WebSocketJsonRpc(
                GatewayWebSocketJsonRpcProtocolSurface {
                    profile: GatewayWebSocketRpcProfile::JsonRpc2_0Text,
                    dispatch_mode: GatewayDispatchMode::Unary,
                    external_sources: vec![GatewayAdapterSource::WebSocketJsonRpcParams],
                    params_schema: GatewayExternalSchema::Record {
                        fields: BTreeMap::new(),
                        required: Vec::new(),
                    },
                    result_schema: GatewayExternalSchema::Null,
                },
            ),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        }
    }

    fn signature(parameter: &str) -> PackageCallableSignature {
        PackageCallableSignature {
            type_params: Vec::new(),
            parameters: vec![PackageCallableParameter {
                name: parameter.to_string(),
                ty: PackageTypeRef::Local {
                    local_type: TypeRefIr::builtin("string"),
                },
            }],
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("void"),
            },
            may_suspend: false,
        }
    }

    fn selector(method: Option<&str>) -> IngressSelector {
        IngressSelector {
            protocol: IngressProtocol::WebSocket,
            method: method.map(str::to_string),
            path: "/socket".to_string(),
        }
    }

    #[test]
    fn websocket_jsonrpc_target_requires_exact_sibling_surface_handler_and_plan() {
        let key = GatewayEntryKey::parse("status").unwrap();
        let surface = surface();
        let identity = gateway_entry_identity(&surface).unwrap();
        let plan = GatewayAdapterPlan {
            kind: GatewayAdapterKind::WebSocketJsonRpc,
            args: vec![GatewayAdapterArg {
                param: "params".to_string(),
                source: GatewayAdapterSource::WebSocketJsonRpcParams,
            }],
        };
        let canonical_signature = signature("params");

        assert_eq!(
            validate_entry_fact_view(WebSocketJsonRpcEntryValidationFacts {
                key: &key,
                identity: &identity,
                selector: &selector(Some("status.get")),
                physical_selector: &selector(None),
                surface: &surface,
                plan: &plan,
                handler_signature: Some(&canonical_signature),
                has_pre: false,
                has_guard: false,
            })
            .unwrap(),
            GatewayWebSocketRpcProfile::JsonRpc2_0Text
        );

        assert!(matches!(
            validate_entry_fact_view(WebSocketJsonRpcEntryValidationFacts {
                key: &key,
                identity: &identity,
                selector: &IngressSelector {
                    path: "/other".to_string(),
                    ..selector(Some("status.get"))
                },
                physical_selector: &selector(None),
                surface: &surface,
                plan: &plan,
                handler_signature: Some(&canonical_signature),
                has_pre: false,
                has_guard: false,
            }),
            Err(RuntimeAssemblyWebSocketJsonRpcTargetError::SelectorMismatch)
        ));

        assert!(matches!(
            validate_entry_fact_view(WebSocketJsonRpcEntryValidationFacts {
                key: &key,
                identity: &identity,
                selector: &selector(Some("status.get")),
                physical_selector: &selector(None),
                surface: &surface,
                plan: &plan,
                handler_signature: None,
                has_pre: false,
                has_guard: false,
            }),
            Err(RuntimeAssemblyWebSocketJsonRpcTargetError::HandlerRequired)
        ));
    }
}
