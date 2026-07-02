use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use skiff_runtime_activation::RuntimeActivation;
use skiff_runtime_capability_context::{
    DbCapabilitySource, DbProviderBuildInput, DbProviderConfig, DbProviderSource,
};
use skiff_runtime_linked_program::{
    ExecutableAddr, LinkedProgramImageResolverExt, OperationIngressKind,
};
use skiff_runtime_request::{RuntimeOperation, RuntimeOperationParameter};
use tracing::info;

use crate::error::{Result, RuntimeError};

use super::{
    service_context::{ServiceOperationContext, ServiceRuntimeContext},
    BuildOperationAbiRouteKey, BuildSelectorRouteKey, RuntimeHost, RuntimeServiceConfig,
    ServiceRouteState,
};

impl RuntimeHost {
    #[allow(dead_code)]
    pub(crate) fn replace_services(
        &self,
        services: Vec<RuntimeServiceConfig>,
    ) -> anyhow::Result<()> {
        let services =
            apply_default_http_response_limits(services, self.default_http_response_max_bytes);
        let state = build_service_route_state(
            services,
            self.default_http_response_max_bytes,
            &self.db_provider,
        )?;
        let service_count = state.services.len();
        *self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("runtime service route state lock is poisoned"))? = state;
        let build_ids = self
            .state
            .read()
            .map_err(|_| anyhow::anyhow!("runtime service route state lock is poisoned"))?
            .build_ids();
        self.loaded_builds.replace_builds(build_ids);
        info!(event = "runtime.services_replaced", service_count);
        Ok(())
    }

    pub(super) fn add_services(
        &self,
        services: Vec<RuntimeServiceConfig>,
    ) -> anyhow::Result<Vec<Arc<ServiceRuntimeContext>>> {
        let services =
            apply_default_http_response_limits(services, self.default_http_response_max_bytes);
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("runtime service route state lock is poisoned"))?;
        let mut contexts = state.services.iter().cloned().collect::<Vec<_>>();
        let mut operation_routes = (*state.route_by_build_and_operation_abi_id).clone();
        let mut selector_routes = (*state.operation_abi_id_by_build_and_selector).clone();
        let mut runtime_ids = contexts
            .iter()
            .map(|service| service.runtime_id.clone())
            .collect::<HashSet<_>>();
        let mut added = Vec::new();

        for service in services {
            let context = service_context_from_config(service, &self.db_provider)?;
            if !runtime_ids.insert(context.runtime_id.clone()) {
                continue;
            }
            insert_service_context_routes(&mut operation_routes, &mut selector_routes, &context)?;
            contexts.push(context.clone());
            added.push(context);
        }
        validate_route_activation_uniqueness(&operation_routes)?;

        state.services = Arc::new(contexts);
        state.route_by_build_and_operation_abi_id = Arc::new(operation_routes);
        state.operation_abi_id_by_build_and_selector = Arc::new(selector_routes);
        self.loaded_builds
            .upsert_builds(added.iter().map(|service| service.build_id.clone()));
        if !added.is_empty() {
            info!(
                event = "runtime.services_added",
                service_count = added.len()
            );
        }
        Ok(added)
    }
}

pub(super) fn build_service_route_state(
    services: Vec<RuntimeServiceConfig>,
    default_http_response_max_bytes: usize,
    db_provider: &DbProviderSource,
) -> anyhow::Result<ServiceRouteState> {
    let services = apply_default_http_response_limits(services, default_http_response_max_bytes);

    let mut contexts = Vec::new();
    let mut route_by_build_and_operation_abi_id = HashMap::new();
    let mut operation_abi_id_by_build_and_selector = HashMap::new();
    let mut runtime_ids = HashSet::new();

    for service in services {
        let context = service_context_from_config(service, db_provider)?;
        if !runtime_ids.insert(context.runtime_id.clone()) {
            anyhow::bail!("duplicate runtime id {}", context.runtime_id);
        }
        insert_service_context_routes(
            &mut route_by_build_and_operation_abi_id,
            &mut operation_abi_id_by_build_and_selector,
            &context,
        )?;
        contexts.push(context);
    }

    validate_route_activation_uniqueness(&route_by_build_and_operation_abi_id)?;

    Ok(ServiceRouteState {
        services: Arc::new(contexts),
        route_by_build_and_operation_abi_id: Arc::new(route_by_build_and_operation_abi_id),
        operation_abi_id_by_build_and_selector: Arc::new(operation_abi_id_by_build_and_selector),
    })
}

fn service_context_from_config(
    service: RuntimeServiceConfig,
    db_provider: &DbProviderSource,
) -> anyhow::Result<Arc<ServiceRuntimeContext>> {
    let (service_id, revision_id) = runtime_service_identity(&service)?;
    let selector_build_id = runtime_service_selector_build_id(&service).to_string();
    if !is_service_build_id(&selector_build_id) {
        anyhow::bail!(
            "runtime service buildId must be skiff-service-build-v1:sha256:<64 lowercase hex>, got {}",
            selector_build_id
        );
    }
    let runtime_program_identity = service.runtime_program_identity.clone();
    let linked_image = service.linked_image.clone();
    let runtime_activation = service.runtime_activation.clone();
    let service_db = build_control_service_db_source(
        service.service_db.clone(),
        runtime_activation.as_ref(),
        db_provider,
    )?;
    Ok(Arc::new(ServiceRuntimeContext::new(
        service_id,
        service.http_response_max_bytes,
        service.activation_identity,
        service.resolved_config_identity,
        linked_image,
        runtime_program_identity,
        runtime_activation,
        revision_id,
        service.runtime_id,
        service.contract_identity,
        service.implementation_identity,
        service.artifact_identity,
        selector_build_id,
        service.config,
        service.package_configs,
        service_db,
    )))
}

fn insert_service_context_routes(
    route_by_build_and_operation_abi_id: &mut HashMap<
        BuildOperationAbiRouteKey,
        Vec<ServiceOperationContext>,
    >,
    operation_abi_id_by_build_and_selector: &mut HashMap<BuildSelectorRouteKey, String>,
    context: &Arc<ServiceRuntimeContext>,
) -> anyhow::Result<()> {
    let routes = program_routes_for_service(context)?;
    let mut registered_operation_abi_ids = HashSet::new();
    let mut operation_abi_ids = Vec::new();
    for route in routes {
        let Some(operation_abi_id) = route.operation.operation_abi_id.as_deref() else {
            anyhow::bail!(
                "runtime service route {} for buildId {} is missing operationAbiId",
                route.operation.target,
                context.build_id
            );
        };
        if registered_operation_abi_ids.insert(operation_abi_id.to_string()) {
            operation_abi_ids.push(operation_abi_id.to_string());
        }
        let route_key = BuildOperationAbiRouteKey {
            build_id: context.build_id.clone(),
            operation_abi_id: operation_abi_id.to_string(),
        };
        route_by_build_and_operation_abi_id
            .entry(route_key)
            .or_insert_with(Vec::new)
            .push(route.clone());
    }
    let mut explicit_selectors = HashSet::new();
    for binding in &context.runtime_activation.operation_route_bindings {
        if !explicit_selectors.insert(binding.selector.clone()) {
            anyhow::bail!(
                "duplicate route binding selector {} for buildId {} operationAbiId {}",
                binding.selector,
                context.build_id,
                binding.operation_abi_id
            );
        }
        register_selector_route(
            operation_abi_id_by_build_and_selector,
            &context.build_id,
            &binding.selector,
            &binding.operation_abi_id,
            binding.ingress_kind,
            &registered_operation_abi_ids,
        )?;
    }
    for operation_abi_id in operation_abi_ids {
        let selector = format!("operation:{operation_abi_id}");
        register_implicit_service_call_selector_route(
            operation_abi_id_by_build_and_selector,
            &context.build_id,
            &selector,
            &operation_abi_id,
            &registered_operation_abi_ids,
        )?;
    }
    Ok(())
}

fn validate_route_activation_uniqueness(
    route_by_build_and_operation_abi_id: &HashMap<
        BuildOperationAbiRouteKey,
        Vec<ServiceOperationContext>,
    >,
) -> anyhow::Result<()> {
    for (route_key, routes) in route_by_build_and_operation_abi_id {
        let mut activation_identities = HashSet::new();
        for route in routes {
            if let Some(activation_identity) = route.service.activation_identity.as_deref() {
                if !activation_identities.insert(activation_identity.to_string()) {
                    anyhow::bail!(
                        "duplicate activationIdentity {} for buildId {} operationAbiId {}",
                        activation_identity,
                        route_key.build_id,
                        route_key.operation_abi_id
                    );
                }
            }
        }
    }
    Ok(())
}

fn insert_selector_route(
    operation_abi_id_by_build_and_selector: &mut HashMap<BuildSelectorRouteKey, String>,
    build_id: &str,
    selector: &str,
    operation_abi_id: &str,
) -> anyhow::Result<()> {
    let key = BuildSelectorRouteKey {
        build_id: build_id.to_string(),
        selector: selector.to_string(),
    };
    if let Some(existing) = operation_abi_id_by_build_and_selector.get(&key) {
        anyhow::bail!(
            "duplicate selector {} for buildId {} maps to operationAbiId {} and {}",
            selector,
            build_id,
            existing,
            operation_abi_id
        );
    }
    operation_abi_id_by_build_and_selector.insert(key, operation_abi_id.to_string());
    Ok(())
}

fn register_selector_route(
    operation_abi_id_by_build_and_selector: &mut HashMap<BuildSelectorRouteKey, String>,
    build_id: &str,
    selector: &str,
    operation_abi_id: &str,
    ingress_kind: OperationIngressKind,
    registered_operation_abi_ids: &HashSet<String>,
) -> anyhow::Result<()> {
    validate_selector_route_binding(
        build_id,
        selector,
        operation_abi_id,
        ingress_kind,
        registered_operation_abi_ids,
    )?;
    if selector_route_already_maps_to_operation(
        operation_abi_id_by_build_and_selector,
        build_id,
        selector,
        operation_abi_id,
    ) {
        return Ok(());
    }
    insert_selector_route(
        operation_abi_id_by_build_and_selector,
        build_id,
        selector,
        operation_abi_id,
    )
}

fn register_implicit_service_call_selector_route(
    operation_abi_id_by_build_and_selector: &mut HashMap<BuildSelectorRouteKey, String>,
    build_id: &str,
    selector: &str,
    operation_abi_id: &str,
    registered_operation_abi_ids: &HashSet<String>,
) -> anyhow::Result<()> {
    validate_selector_route_binding(
        build_id,
        selector,
        operation_abi_id,
        OperationIngressKind::ServiceCall,
        registered_operation_abi_ids,
    )?;
    if selector_route_already_maps_to_operation(
        operation_abi_id_by_build_and_selector,
        build_id,
        selector,
        operation_abi_id,
    ) {
        return Ok(());
    }
    insert_selector_route(
        operation_abi_id_by_build_and_selector,
        build_id,
        selector,
        operation_abi_id,
    )
}

fn validate_selector_route_binding(
    build_id: &str,
    selector: &str,
    operation_abi_id: &str,
    ingress_kind: OperationIngressKind,
    registered_operation_abi_ids: &HashSet<String>,
) -> anyhow::Result<()> {
    if operation_abi_id.is_empty() {
        anyhow::bail!(
            "route binding for buildId {} selector {} has empty operationAbiId",
            build_id,
            selector
        );
    }
    if selector.is_empty() {
        anyhow::bail!(
            "route binding for buildId {} operationAbiId {} has empty selector",
            build_id,
            operation_abi_id
        );
    }
    if !registered_operation_abi_ids.contains(operation_abi_id) {
        anyhow::bail!(
            "route binding selector {} for buildId {} references unknown operationAbiId {}",
            selector,
            build_id,
            operation_abi_id
        );
    }
    if ingress_kind == OperationIngressKind::ServiceCall {
        let expected_selector = format!("operation:{operation_abi_id}");
        if selector != expected_selector {
            anyhow::bail!(
                "service-call route binding selector {} for buildId {} does not match operationAbiId {}; expected {}",
                selector,
                build_id,
                operation_abi_id,
                expected_selector
            );
        }
    }
    Ok(())
}

fn selector_route_already_maps_to_operation(
    operation_abi_id_by_build_and_selector: &HashMap<BuildSelectorRouteKey, String>,
    build_id: &str,
    selector: &str,
    operation_abi_id: &str,
) -> bool {
    let key = BuildSelectorRouteKey {
        build_id: build_id.to_string(),
        selector: selector.to_string(),
    };
    operation_abi_id_by_build_and_selector
        .get(&key)
        .map(String::as_str)
        == Some(operation_abi_id)
}

pub(super) fn runtime_service_identity(
    service: &RuntimeServiceConfig,
) -> anyhow::Result<(String, String)> {
    let service_meta = &service.runtime_activation.service;
    if service_meta.id.is_empty() {
        anyhow::bail!("runtime program service metadata id is missing");
    }
    if !is_bare_sha256(&service.revision_id) {
        anyhow::bail!(
            "runtime service revisionId must be 64 lowercase hex, got {}",
            service.revision_id
        );
    }
    Ok((service_meta.id.clone(), service.revision_id.clone()))
}

pub(super) fn runtime_service_selector_build_id(service: &RuntimeServiceConfig) -> &str {
    service.runtime_program_identity.dynamic_build_id.as_str()
}

pub(super) fn package_test_revision_id(test_build_identity: &str) -> String {
    test_build_identity
        .strip_prefix("skiff-package-test-build-v1:sha256:")
        .filter(|hash| is_bare_sha256(hash))
        .unwrap_or("0000000000000000000000000000000000000000000000000000000000000000")
        .to_string()
}

fn is_bare_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn program_routes_for_service(
    service: &Arc<ServiceRuntimeContext>,
) -> anyhow::Result<Vec<ServiceOperationContext>> {
    let mut routes = Vec::new();
    for (operation_abi_id, addr) in &service.linked_image.operations {
        if operation_abi_id.is_empty() {
            anyhow::bail!(
                "runtime program operation for buildId {} has empty operationAbiId",
                service.build_id
            );
        }
        let target = route_target_for_operation_addr(&service.linked_image.routes, addr)
            .unwrap_or_else(|| format!("operation:{operation_abi_id}"));
        let executable = service
            .linked_image
            .resolve_executable(addr)
            .map_err(|error| {
                anyhow::anyhow!(
                    "runtime program route {target} references invalid executable: {error}"
                )
            })?;
        let operation = RuntimeOperation {
            operation_abi_id: Some(operation_abi_id.clone()),
            operation: executable.executable.symbol.clone(),
            target,
            mode: "unary".to_string(),
            parameters: executable
                .executable
                .params
                .iter()
                .map(|parameter| RuntimeOperationParameter {
                    name: parameter.name.clone(),
                    extra: serde_json::Map::new(),
                })
                .collect(),
            service_protocol_identity: Some(service.contract_identity.clone()),
            extra: serde_json::Map::new(),
        };
        routes.push(ServiceOperationContext::new(
            service.clone(),
            operation,
            addr.clone(),
        ));
    }
    Ok(routes)
}

fn route_target_for_operation_addr(
    routes: &HashMap<String, ExecutableAddr>,
    addr: &ExecutableAddr,
) -> Option<String> {
    let mut targets = routes
        .iter()
        .filter_map(|(target, route_addr)| (route_addr == addr).then(|| target.clone()))
        .collect::<Vec<_>>();
    targets.sort();
    targets
        .iter()
        .find(|target| !target.starts_with("function:"))
        .cloned()
        .or_else(|| targets.into_iter().next())
}

pub(super) fn apply_default_http_response_limits(
    mut services: Vec<RuntimeServiceConfig>,
    default_http_response_max_bytes: usize,
) -> Vec<RuntimeServiceConfig> {
    for service in &mut services {
        if service.use_runtime_default_http_response_max_bytes {
            service.http_response_max_bytes = default_http_response_max_bytes;
            service.use_runtime_default_http_response_max_bytes = false;
        }
    }
    services
}

pub(super) fn build_service_db_source(
    storage_service_id: String,
    service_db: Option<DbProviderConfig>,
    runtime_activation: &RuntimeActivation,
    db_provider: &DbProviderSource,
) -> anyhow::Result<DbCapabilitySource> {
    let Some(service_db) = service_db else {
        return Ok(DbCapabilitySource::unavailable());
    };
    db_provider
        .build(DbProviderBuildInput {
            service_id: storage_service_id,
            config: service_db,
            runtime_program_db: runtime_activation.db.clone(),
        })
        .map_err(anyhow::Error::new)
}

fn build_control_service_db_source(
    service_db: Option<DbProviderConfig>,
    runtime_activation: &RuntimeActivation,
    db_provider: &DbProviderSource,
) -> anyhow::Result<DbCapabilitySource> {
    let Some(service_db) = service_db else {
        return Ok(DbCapabilitySource::unavailable());
    };
    let storage_service_id = control_service_db_storage_service_id(&service_db)?;
    let provider_config = control_service_db_provider_config(service_db)?;
    build_service_db_source(
        storage_service_id,
        Some(provider_config),
        runtime_activation,
        db_provider,
    )
}

fn control_service_db_storage_service_id(service_db: &DbProviderConfig) -> anyhow::Result<String> {
    service_db
        .as_value()
        .get("storageServiceId")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| anyhow::anyhow!("runtime serviceDb.storageServiceId is required"))
}

fn control_service_db_provider_config(
    service_db: DbProviderConfig,
) -> anyhow::Result<DbProviderConfig> {
    let value = service_db.into_value();
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("runtime serviceDb must be a JSON object"))?;
    let mongo_url = object
        .get("mongoUrl")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("runtime serviceDb.mongoUrl is required"))?;
    Ok(DbProviderConfig::opaque(serde_json::json!({
        "mongoUrl": mongo_url,
    })))
}

pub(super) fn is_service_build_id(value: &str) -> bool {
    let Some((prefix, hash)) = value.rsplit_once(":sha256:") else {
        return false;
    };
    prefix == "skiff-service-build-v1"
        && hash.len() == 64
        && hash
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

pub(super) fn lookup_operation_by_build_id(
    state: &ServiceRouteState,
    request: &skiff_runtime_request::RequestEnvelope,
    build_id: &str,
) -> Result<ServiceOperationContext> {
    if !is_service_build_id(build_id) {
        return Err(RuntimeError::Unsupported(format!(
            "request.start buildId must be skiff-service-build-v1:sha256:<64 lowercase hex>, got {}",
            build_id
        )));
    }
    let Some(operation_abi_id) = request.operation_abi_id.as_deref() else {
        return Err(RuntimeError::Unsupported(format!(
            "request.start operationAbiId is required for buildId {} target {}",
            build_id, request.target
        )));
    };
    validate_request_selector(state, build_id, operation_abi_id, request)?;
    let route_key = BuildOperationAbiRouteKey {
        build_id: build_id.to_string(),
        operation_abi_id: operation_abi_id.to_string(),
    };
    let routes = state
        .route_by_build_and_operation_abi_id
        .get(&route_key)
        .ok_or_else(|| {
            RuntimeError::Unsupported(format!(
                "no registered route supports buildId {} operationAbiId {}",
                build_id, operation_abi_id
            ))
        })?;
    if let Some(activation_identity) = request.activation_identity.as_deref() {
        return routes
            .iter()
            .find(|route| route.service.activation_identity.as_deref() == Some(activation_identity))
            .cloned()
            .ok_or_else(|| {
                RuntimeError::Unsupported(format!(
                    "no activationIdentity {} supports buildId {} target {}",
                    activation_identity, build_id, request.target
                ))
            });
    }
    if routes.len() == 1 {
        return Ok(routes[0].clone());
    }
    Err(RuntimeError::Unsupported(format!(
        "request.start activationIdentity is required because buildId {} operation/target {} has {} active service contexts",
        build_id,
        operation_abi_id,
        routes.len()
    )))
}

fn validate_request_selector(
    state: &ServiceRouteState,
    build_id: &str,
    operation_abi_id: &str,
    request: &skiff_runtime_request::RequestEnvelope,
) -> Result<()> {
    let Some(selector) = request.selector.as_deref() else {
        if request_allows_omitted_selector(request) {
            return Ok(());
        }
        return Err(RuntimeError::Unsupported(format!(
            "request.start selector is required for buildId {} operationAbiId {}",
            build_id, operation_abi_id
        )));
    };
    if selector.is_empty() {
        return Err(RuntimeError::Unsupported(format!(
            "request.start selector must be non-empty for buildId {} operationAbiId {}",
            build_id, operation_abi_id
        )));
    };
    let key = BuildSelectorRouteKey {
        build_id: build_id.to_string(),
        selector: selector.to_string(),
    };
    let Some(mapped_operation_abi_id) = state.operation_abi_id_by_build_and_selector.get(&key)
    else {
        return Err(RuntimeError::Unsupported(format!(
            "request.start selector {} is not registered for buildId {}",
            selector, build_id
        )));
    };
    if mapped_operation_abi_id != operation_abi_id {
        return Err(RuntimeError::Unsupported(format!(
            "request.start selector {} maps to operationAbiId {}, not {}",
            selector, mapped_operation_abi_id, operation_abi_id
        )));
    }
    Ok(())
}

fn request_allows_omitted_selector(request: &skiff_runtime_request::RequestEnvelope) -> bool {
    request
        .extra
        .get("caller")
        .and_then(|caller| caller.get("kind"))
        .and_then(|kind| kind.as_str())
        == Some("service")
}
