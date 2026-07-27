use std::{collections::BTreeSet, sync::Arc};

use skiff_artifact_identity::{gateway_entry_identity, validate_gateway_entry_protocol_surface};
use skiff_artifact_model::{
    validate_gateway_adapter_args, GatewayAdapterKind, GatewayAdapterPlan, GatewayAdapterSource,
    GatewayDispatchMode, GatewayEntryIdentity, GatewayEntryKey, GatewayEntryProtocolSurface,
    GatewayProtocolSurface, OperationCallableKind, PackageCallableId, PackageCallableSignature,
    PackageLocalAbiSymbol, ServiceDeploymentRef,
};
use skiff_runtime_eval::{
    RuntimeAssemblyEvalTarget, RuntimeHttpGatewayCallable, RuntimeHttpGatewayExecutionTarget,
};
use skiff_runtime_linked_program::{ExecutableAddr, ExecutableKind, SharedPackageCode};
use skiff_runtime_linker::LinkedGatewayEntry;

/// Request-owned HTTP gateway target pinned to one exact linked entry and eval activation.
#[derive(Debug, Clone)]
pub struct RuntimeAssemblyHttpGatewayTarget {
    eval: RuntimeAssemblyEvalTarget,
    entry: Arc<LinkedGatewayEntry>,
    handler_addr: ExecutableAddr,
    pre_addr: Option<ExecutableAddr>,
    guard_addr: Option<ExecutableAddr>,
}

impl RuntimeAssemblyHttpGatewayTarget {
    pub fn new(
        eval: RuntimeAssemblyEvalTarget,
        entry: Arc<LinkedGatewayEntry>,
    ) -> Result<Self, RuntimeAssemblyHttpGatewayTargetError> {
        if !matches!(
            entry.protocol_surface().protocol,
            GatewayProtocolSurface::Http(_)
        ) {
            return Err(RuntimeAssemblyHttpGatewayTargetError::PlanSurfaceMismatch);
        }
        let handler = entry
            .optional_handler()
            .ok_or(RuntimeAssemblyHttpGatewayTargetError::HandlerPlanMismatch)?;
        if !gateway_owner_matches(
            entry.owner(),
            &eval.activation_context().identity().deployment,
        ) {
            return Err(RuntimeAssemblyHttpGatewayTargetError::OwnerMismatch);
        }
        if !Arc::ptr_eq(
            eval.activation_context(),
            eval.request_activation().receiver(),
        ) {
            return Err(RuntimeAssemblyHttpGatewayTargetError::ActivationOwnerMismatch);
        }
        eval.ensure_execution_ready().map_err(|error| {
            RuntimeAssemblyHttpGatewayTargetError::ExecutionImage {
                detail: error.to_string(),
            }
        })?;
        validate_entry_facts(&entry)?;
        let implementation = eval
            .execution_image()
            .shared_packages()
            .code_by_build(eval.activation_context().implementation_package_build_id())
            .ok_or(RuntimeAssemblyHttpGatewayTargetError::ImplementationNotLinked)?;
        let handler_addr = validate_callable(
            &eval,
            implementation,
            "handler",
            handler.callable_id(),
            handler.target(),
            handler.signature(),
        )?;
        let pre_addr = entry
            .pre()
            .map(|callable| {
                validate_callable(
                    &eval,
                    implementation,
                    "pre",
                    callable.callable_id(),
                    callable.target(),
                    callable.signature(),
                )
            })
            .transpose()?;
        let guard_addr = entry
            .guard()
            .map(|callable| {
                validate_callable(
                    &eval,
                    implementation,
                    "guard",
                    callable.callable_id(),
                    callable.target(),
                    callable.signature(),
                )
            })
            .transpose()?;
        Ok(Self {
            eval,
            entry,
            handler_addr,
            pre_addr,
            guard_addr,
        })
    }

    pub fn eval(&self) -> &RuntimeAssemblyEvalTarget {
        &self.eval
    }

    pub fn entry(&self) -> &Arc<LinkedGatewayEntry> {
        &self.entry
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

    pub fn protocol_surface(&self) -> &GatewayEntryProtocolSurface {
        self.entry.protocol_surface()
    }

    pub fn adapter_plan(&self) -> &GatewayAdapterPlan {
        self.entry.adapter_plan()
    }

    pub fn handler_addr(&self) -> &ExecutableAddr {
        &self.handler_addr
    }

    pub fn pre_addr(&self) -> Option<&ExecutableAddr> {
        self.pre_addr.as_ref()
    }

    pub fn guard_addr(&self) -> Option<&ExecutableAddr> {
        self.guard_addr.as_ref()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeAssemblyHttpGatewayTargetError {
    #[error("HTTP gateway owner does not match the pinned request activation")]
    OwnerMismatch,
    #[error("HTTP gateway eval target does not pin the request receiver activation")]
    ActivationOwnerMismatch,
    #[error("HTTP gateway execution image is not ready: {detail}")]
    ExecutionImage { detail: String },
    #[error("HTTP gateway implementation package is absent from the pinned execution image")]
    ImplementationNotLinked,
    #[error("HTTP gateway entry key is not its strict canonical value")]
    InvalidKey,
    #[error("HTTP gateway entry identity is not its strict canonical value")]
    InvalidIdentity,
    #[error("HTTP gateway protocol surface is invalid: {detail}")]
    InvalidProtocolSurface { detail: String },
    #[error("HTTP gateway adapter plan is invalid: {detail}")]
    InvalidAdapterPlan { detail: String },
    #[error("HTTP gateway adapter plan does not match its protocol surface")]
    PlanSurfaceMismatch,
    #[error("HTTP gateway handler signature and adapter plan do not match")]
    HandlerPlanMismatch,
    #[error("HTTP gateway {role} callable is not an exact private linked target: {detail}")]
    CallableMismatch { role: &'static str, detail: String },
}

fn validate_entry_facts(
    entry: &LinkedGatewayEntry,
) -> Result<(), RuntimeAssemblyHttpGatewayTargetError> {
    let handler = entry
        .optional_handler()
        .ok_or(RuntimeAssemblyHttpGatewayTargetError::HandlerPlanMismatch)?;
    validate_entry_fact_view(GatewayEntryValidationFacts {
        key: entry.gateway_entry_key(),
        identity: entry.gateway_entry_identity(),
        surface: entry.protocol_surface(),
        plan: entry.adapter_plan(),
        handler_signature: handler.signature(),
        pre_signature: entry.pre().map(|callable| callable.signature()),
        guard_signature: entry.guard().map(|callable| callable.signature()),
    })
}

struct GatewayEntryValidationFacts<'a> {
    key: &'a GatewayEntryKey,
    identity: &'a GatewayEntryIdentity,
    surface: &'a GatewayEntryProtocolSurface,
    plan: &'a GatewayAdapterPlan,
    handler_signature: &'a PackageCallableSignature,
    pre_signature: Option<&'a PackageCallableSignature>,
    guard_signature: Option<&'a PackageCallableSignature>,
}

fn validate_entry_fact_view(
    facts: GatewayEntryValidationFacts<'_>,
) -> Result<(), RuntimeAssemblyHttpGatewayTargetError> {
    let parsed_key = GatewayEntryKey::parse(facts.key.as_str())
        .map_err(|_| RuntimeAssemblyHttpGatewayTargetError::InvalidKey)?;
    if &parsed_key != facts.key {
        return Err(RuntimeAssemblyHttpGatewayTargetError::InvalidKey);
    }
    let parsed_identity = GatewayEntryIdentity::parse(facts.identity.as_str())
        .map_err(|_| RuntimeAssemblyHttpGatewayTargetError::InvalidIdentity)?;
    if &parsed_identity != facts.identity
        || gateway_entry_identity(facts.surface).map_err(|error| {
            RuntimeAssemblyHttpGatewayTargetError::InvalidProtocolSurface {
                detail: error.to_string(),
            }
        })? != *facts.identity
    {
        return Err(RuntimeAssemblyHttpGatewayTargetError::InvalidIdentity);
    }
    validate_gateway_entry_protocol_surface(facts.surface).map_err(|error| {
        RuntimeAssemblyHttpGatewayTargetError::InvalidProtocolSurface {
            detail: error.to_string(),
        }
    })?;
    validate_gateway_adapter_args(
        facts.plan.kind,
        facts.pre_signature.is_some(),
        &facts.plan.args,
    )
    .map_err(
        |error| RuntimeAssemblyHttpGatewayTargetError::InvalidAdapterPlan {
            detail: error.to_string(),
        },
    )?;

    let GatewayProtocolSurface::Http(http) = &facts.surface.protocol else {
        return Err(RuntimeAssemblyHttpGatewayTargetError::PlanSurfaceMismatch);
    };
    if http.adapter_kind != facts.plan.kind
        || !adapter_mode_is_supported(http.adapter_kind, http.dispatch_mode)
    {
        return Err(RuntimeAssemblyHttpGatewayTargetError::PlanSurfaceMismatch);
    }
    let mut external_sources = facts
        .plan
        .args
        .iter()
        .map(|arg| arg.source)
        .filter(|source| source.is_external_protocol_source())
        .collect::<Vec<_>>();
    external_sources.sort_by_key(|source| source.wire_name());
    external_sources.dedup();
    if external_sources != http.external_sources {
        return Err(RuntimeAssemblyHttpGatewayTargetError::PlanSurfaceMismatch);
    }
    validate_handler_plan(&facts)?;
    Ok(())
}

fn gateway_owner_matches(
    entry_owner: &ServiceDeploymentRef,
    activation_owner: &ServiceDeploymentRef,
) -> bool {
    entry_owner == activation_owner
}

fn adapter_mode_is_supported(kind: GatewayAdapterKind, mode: GatewayDispatchMode) -> bool {
    matches!(
        (kind, mode),
        (GatewayAdapterKind::TypedJson, GatewayDispatchMode::Unary)
            | (GatewayAdapterKind::RawHttp, GatewayDispatchMode::Unary)
            | (
                GatewayAdapterKind::RawHttp,
                GatewayDispatchMode::ServerStream
            )
    )
}

fn validate_handler_plan(
    facts: &GatewayEntryValidationFacts<'_>,
) -> Result<(), RuntimeAssemblyHttpGatewayTargetError> {
    let signature = facts.handler_signature;
    if !signature.type_params.is_empty() {
        return Err(RuntimeAssemblyHttpGatewayTargetError::HandlerPlanMismatch);
    }
    let formal_names = signature
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
    if formal_names.len() != signature.parameters.len()
        || actual_names.len() != facts.plan.args.len()
        || formal_names != actual_names
    {
        return Err(RuntimeAssemblyHttpGatewayTargetError::HandlerPlanMismatch);
    }
    if let Some(pre) = facts.pre_signature {
        if !pre.type_params.is_empty() || pre.parameters.len() != 1 {
            return Err(RuntimeAssemblyHttpGatewayTargetError::HandlerPlanMismatch);
        }
        for arg in facts
            .plan
            .args
            .iter()
            .filter(|arg| arg.source == GatewayAdapterSource::HttpContext)
        {
            let formal = signature
                .parameters
                .iter()
                .find(|parameter| parameter.name == arg.param)
                .ok_or(RuntimeAssemblyHttpGatewayTargetError::HandlerPlanMismatch)?;
            if formal.ty != pre.return_type {
                return Err(RuntimeAssemblyHttpGatewayTargetError::HandlerPlanMismatch);
            }
        }
    }
    if facts
        .guard_signature
        .is_some_and(|guard| !guard.type_params.is_empty() || guard.parameters.len() != 1)
    {
        return Err(RuntimeAssemblyHttpGatewayTargetError::HandlerPlanMismatch);
    }
    Ok(())
}

fn validate_callable(
    eval: &RuntimeAssemblyEvalTarget,
    implementation: &SharedPackageCode,
    role: &'static str,
    callable_id: &PackageCallableId,
    target: &skiff_artifact_model::OperationTargetRef,
    signature: &PackageCallableSignature,
) -> Result<ExecutableAddr, RuntimeAssemblyHttpGatewayTargetError> {
    if target.callable_abi_id != callable_id.as_str()
        || target.callable_kind != OperationCallableKind::InternalFunction
        || implementation.callable_target(callable_id) != Some(target)
    {
        return Err(callable_mismatch(role, "callable target is not exact"));
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
            role,
            "implementation signature is absent, ambiguous, or mismatched",
        ));
    }
    let executable = eval
        .execution_image()
        .entry_executable(implementation.package_build_id(), target)
        .map_err(|error| callable_mismatch(role, error.to_string()))?;
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
            role,
            "execution image signature does not match the linked callable signature",
        ));
    }
    Ok(executable.addr().clone())
}

fn callable_mismatch(
    role: &'static str,
    detail: impl Into<String>,
) -> RuntimeAssemblyHttpGatewayTargetError {
    RuntimeAssemblyHttpGatewayTargetError::CallableMismatch {
        role,
        detail: detail.into(),
    }
}

impl RuntimeHttpGatewayExecutionTarget for RuntimeAssemblyHttpGatewayTarget {
    fn eval_target(&self) -> &RuntimeAssemblyEvalTarget {
        &self.eval
    }

    fn gateway_entry_key(&self) -> &GatewayEntryKey {
        self.entry.gateway_entry_key()
    }

    fn gateway_entry_identity(&self) -> &GatewayEntryIdentity {
        self.entry.gateway_entry_identity()
    }

    fn protocol_surface(&self) -> &GatewayEntryProtocolSurface {
        self.entry.protocol_surface()
    }

    fn adapter_plan(&self) -> &GatewayAdapterPlan {
        self.entry.adapter_plan()
    }

    fn handler(&self) -> RuntimeHttpGatewayCallable<'_> {
        RuntimeHttpGatewayCallable {
            callable_id: self.entry.handler().callable_id(),
            signature: self.entry.handler().signature(),
            addr: &self.handler_addr,
        }
    }

    fn pre(&self) -> Option<RuntimeHttpGatewayCallable<'_>> {
        self.entry
            .pre()
            .zip(self.pre_addr.as_ref())
            .map(|(callable, addr)| RuntimeHttpGatewayCallable {
                callable_id: callable.callable_id(),
                signature: callable.signature(),
                addr,
            })
    }

    fn guard(&self) -> Option<RuntimeHttpGatewayCallable<'_>> {
        self.entry
            .guard()
            .zip(self.guard_addr.as_ref())
            .map(|(callable, addr)| RuntimeHttpGatewayCallable {
                callable_id: callable.callable_id(),
                signature: callable.signature(),
                addr,
            })
    }
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{
        DeploymentRevision, GatewayAdapterArg, GatewayExternalErrorProjection,
        GatewayExternalSchema, GatewayHttpProtocolSurface, GatewayWebSocketConnectProtocolSurface,
        GatewayWebSocketDownlinkFrame, GatewayWebSocketShapeVersion, PackageCallableParameter,
        PackageTypeRef, TypeRefIr,
    };

    use super::*;

    #[test]
    fn runtime_http_gateway_target_fact_mutations_fail_closed() {
        assert!(
            GatewayEntryKey::parse("http typed").is_err(),
            "a non-canonical gateway key cannot enter the typed target"
        );
        let key = GatewayEntryKey::parse("http:typed").unwrap();
        let surface = typed_surface();
        let identity = gateway_entry_identity(&surface).unwrap();
        let plan = GatewayAdapterPlan {
            kind: GatewayAdapterKind::TypedJson,
            args: vec![GatewayAdapterArg {
                param: "body".to_string(),
                source: GatewayAdapterSource::HttpBody,
            }],
        };
        let signature = unary_signature("body");

        validate_entry_fact_view(GatewayEntryValidationFacts {
            key: &key,
            identity: &identity,
            surface: &surface,
            plan: &plan,
            handler_signature: &signature,
            pre_signature: None,
            guard_signature: None,
        })
        .expect("canonical gateway facts");

        let wrong_identity = GatewayEntryIdentity::parse(format!(
            "skiff-gateway-entry-v1:sha256:{}",
            "0".repeat(64)
        ))
        .unwrap();
        assert!(matches!(
            validate_entry_fact_view(GatewayEntryValidationFacts {
                key: &key,
                identity: &wrong_identity,
                surface: &surface,
                plan: &plan,
                handler_signature: &signature,
                pre_signature: None,
                guard_signature: None,
            }),
            Err(RuntimeAssemblyHttpGatewayTargetError::InvalidIdentity)
        ));

        let mut wrong_mode = surface.clone();
        let GatewayProtocolSurface::Http(http) = &mut wrong_mode.protocol else {
            panic!("typed fixture must use an HTTP surface");
        };
        http.dispatch_mode = GatewayDispatchMode::ServerStream;
        assert!(matches!(
            validate_entry_fact_view(GatewayEntryValidationFacts {
                key: &key,
                identity: &identity,
                surface: &wrong_mode,
                plan: &plan,
                handler_signature: &signature,
                pre_signature: None,
                guard_signature: None,
            }),
            Err(RuntimeAssemblyHttpGatewayTargetError::InvalidProtocolSurface { .. })
                | Err(RuntimeAssemblyHttpGatewayTargetError::InvalidIdentity)
                | Err(RuntimeAssemblyHttpGatewayTargetError::PlanSurfaceMismatch)
        ));

        let mut wrong_kind = plan.clone();
        wrong_kind.kind = GatewayAdapterKind::RawHttp;
        assert!(matches!(
            validate_entry_fact_view(GatewayEntryValidationFacts {
                key: &key,
                identity: &identity,
                surface: &surface,
                plan: &wrong_kind,
                handler_signature: &signature,
                pre_signature: None,
                guard_signature: None,
            }),
            Err(RuntimeAssemblyHttpGatewayTargetError::PlanSurfaceMismatch)
                | Err(RuntimeAssemblyHttpGatewayTargetError::InvalidAdapterPlan { .. })
        ));

        let wrong_signature = unary_signature("other");
        assert!(matches!(
            validate_entry_fact_view(GatewayEntryValidationFacts {
                key: &key,
                identity: &identity,
                surface: &surface,
                plan: &plan,
                handler_signature: &wrong_signature,
                pre_signature: None,
                guard_signature: None,
            }),
            Err(RuntimeAssemblyHttpGatewayTargetError::HandlerPlanMismatch)
        ));
    }

    #[test]
    fn runtime_http_gateway_target_owner_is_exact_deployment_ref() {
        let owner = deployment_ref("revision:a", "identity:a");
        assert!(gateway_owner_matches(&owner, &owner));
        assert!(!gateway_owner_matches(
            &owner,
            &deployment_ref("revision:b", "identity:b")
        ));
    }

    #[test]
    fn runtime_http_gateway_target_refuses_websocket_connect_surface() {
        let key = GatewayEntryKey::parse("websocket").unwrap();
        let surface = websocket_surface();
        let identity = gateway_entry_identity(&surface).unwrap();
        let plan = GatewayAdapterPlan {
            kind: GatewayAdapterKind::WebSocketConnect,
            args: vec![GatewayAdapterArg {
                param: "request".to_string(),
                source: GatewayAdapterSource::WebSocketConnectRequest,
            }],
        };
        assert!(matches!(
            validate_entry_fact_view(GatewayEntryValidationFacts {
                key: &key,
                identity: &identity,
                surface: &surface,
                plan: &plan,
                handler_signature: &unary_signature("request"),
                pre_signature: None,
                guard_signature: None,
            }),
            Err(RuntimeAssemblyHttpGatewayTargetError::PlanSurfaceMismatch)
        ));
    }

    fn typed_surface() -> GatewayEntryProtocolSurface {
        GatewayEntryProtocolSurface {
            protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
                adapter_kind: GatewayAdapterKind::TypedJson,
                dispatch_mode: GatewayDispatchMode::Unary,
                external_sources: vec![GatewayAdapterSource::HttpBody],
                request_body_schema: Some(GatewayExternalSchema::String),
                response_schema: Some(GatewayExternalSchema::String),
                stream_item_schema: None,
            }),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        }
    }

    fn websocket_surface() -> GatewayEntryProtocolSurface {
        GatewayEntryProtocolSurface {
            protocol: GatewayProtocolSurface::WebSocketConnect(
                GatewayWebSocketConnectProtocolSurface {
                    connect_request_shape: GatewayWebSocketShapeVersion::V1,
                    connect_result_shape: GatewayWebSocketShapeVersion::V1,
                    connection_policy_shape: GatewayWebSocketShapeVersion::V1,
                    external_sources: vec![
                        GatewayAdapterSource::WebSocketConnectRequest,
                        GatewayAdapterSource::WebSocketConnectionId,
                    ],
                    downlink_frames: vec![
                        GatewayWebSocketDownlinkFrame::Binary,
                        GatewayWebSocketDownlinkFrame::Text,
                    ],
                },
            ),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        }
    }

    fn unary_signature(parameter: &str) -> PackageCallableSignature {
        PackageCallableSignature {
            type_params: Vec::new(),
            parameters: vec![PackageCallableParameter {
                name: parameter.to_string(),
                ty: PackageTypeRef::Local {
                    local_type: TypeRefIr::builtin("string"),
                },
            }],
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            },
            may_suspend: false,
        }
    }

    fn deployment_ref(revision: &str, identity: &str) -> ServiceDeploymentRef {
        ServiceDeploymentRef {
            service_id: "example.com/gateway".to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: DeploymentRevision::new(revision),
            deployment_artifact_identity: skiff_artifact_model::DeploymentArtifactIdentity::new(
                identity,
            ),
        }
    }
}
