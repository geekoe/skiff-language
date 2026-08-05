use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use anyhow::Context;
use skiff_artifact_identity::{gateway_entry_identity, websocket_entry_id};
use skiff_artifact_model::{
    AssemblyActivationServiceDb, ContractOperationId, DbMetadataIndexIr, DbMetadataIr,
    DeploymentGatewayEntry, DeploymentIngressBinding, GatewayAdapterKind, GatewayEntryIdentity,
    GatewayEntryKey, GatewayProtocolSurface, GatewayWebSocketRpcProfile, IngressProtocol,
    OperationTargetRef, PackageBuildId, ServiceContract, ServiceContractRef, ServiceDeployment,
    ServiceDeploymentRef, WebSocketEntryId, WEBSOCKET_GATEWAY_ENTRY_KEY,
};
use skiff_runtime_activation::{ActivationContext, ActivationId};
use skiff_runtime_capability_context::{
    DbCapabilityError, DbCapabilitySource, DbCapabilityTarget, DbCapabilityTargetId,
    DbProviderBuildInput, DbProviderConfig, DbProviderSource, DbProviderTargetMetadata,
};
use skiff_runtime_eval::{AdmittedPackageSchemaRecords, RuntimeAssemblyEvalResolver};
use skiff_runtime_linker::{AssemblyLinkedCandidate, LinkedGatewayEntry};

use super::config_snapshot::{materialize_snapshot_config, ActivationConfigViews};

/// Converts a capability/provider error into an anyhow error without dropping the root
/// cause. Opaque wire payloads can Display as an empty string; fall back to `Debug` so
/// whole-assembly admission failures always name the underlying provider rejection.
fn provider_error(error: DbCapabilityError) -> anyhow::Error {
    let message = error.to_string();
    if message.trim().is_empty() {
        anyhow::anyhow!("{error:?}")
    } else {
        anyhow::anyhow!(message)
    }
}

/// Immutable activation owners and canonical target facts published with one assembly generation.
#[derive(Debug)]
pub(crate) struct ActiveAssemblyContextSet {
    activations: BTreeMap<ActivationId, Arc<ActivationContext>>,
    activations_by_deployment: BTreeMap<ServiceDeploymentRef, Arc<ActivationContext>>,
    contracts_by_deployment: BTreeMap<ServiceDeploymentRef, ServiceContractRef>,
    contracts: BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    schema_records: BTreeMap<ServiceContractRef, AdmittedPackageSchemaRecords>,
    operation_targets: BTreeMap<(ActivationId, ContractOperationId), OperationTargetRef>,
    config_views: BTreeMap<ServiceDeploymentRef, Arc<ActivationConfigViews>>,
    db_sources: BTreeMap<ActivationId, DbCapabilitySource>,
    websocket_entries: BTreeMap<ServiceDeploymentRef, AdmittedWebSocketEntry>,
}

impl ActiveAssemblyContextSet {
    pub(crate) async fn from_candidate(
        candidate: &AssemblyLinkedCandidate,
        generation: u64,
        runtime_replica_id: &str,
        db_provider: &DbProviderSource,
        service_db: Option<&AssemblyActivationServiceDb>,
        profile: Option<&str>,
        config_snapshot: Option<&skiff_runtime_config_snapshot::RuntimeConfigSnapshot>,
    ) -> anyhow::Result<Self> {
        if runtime_replica_id.trim().is_empty() {
            anyhow::bail!("runtime replica id must be non-empty for activation construction");
        }
        let mut activations = BTreeMap::new();
        let mut activations_by_deployment = BTreeMap::new();
        let mut contracts_by_deployment = BTreeMap::new();
        let mut operation_targets = BTreeMap::new();
        let config_views = match config_snapshot {
            Some(snapshot) => materialize_snapshot_config(candidate, snapshot)?,
            #[cfg(test)]
            None => super::config_snapshot::materialize_empty_config_for_test(candidate)?,
            #[cfg(not(test))]
            None => anyhow::bail!("production activation requires a RuntimeConfigSnapshot"),
        }
        .into_iter()
        .map(|(deployment, views)| (deployment, Arc::new(views)))
        .collect();
        let runtime_program_db_by_deployment = candidate_db_metadata(candidate)?;
        validate_db_contract_implementations(candidate)?;
        let db_inputs = candidate_db_provider_inputs(
            candidate,
            &runtime_program_db_by_deployment,
            service_db,
            profile,
        )?;
        // This is intentionally one provider call for the whole candidate. Services can have
        // multiple exact deployments in one generation while sharing one system database, so
        // per-activation reconciliation would miss cross-version collection/index conflicts.
        if !db_inputs.is_empty() {
            db_provider
                .provision(db_inputs)
                .await
                .map_err(provider_error)
                .context("whole-assembly service DB index provisioning failed")?;
        }
        let mut db_sources = BTreeMap::new();
        let mut websocket_entries = BTreeMap::new();
        for (deployment, linked) in candidate.activations() {
            let binding_template = candidate
                .assembly()
                .service_binding_templates
                .iter()
                .find(|template| &template.activation == deployment)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "admitted activation {:?} has no service binding template",
                        deployment
                    )
                })?;
            let websocket_entry = admitted_websocket_entry(candidate, deployment)?;
            let activation_websocket_entry = websocket_entry
                .as_ref()
                .map(AdmittedWebSocketEntry::activation_parts);
            let activation = ActivationContext::from_assembly_templates_with_websocket_entry(
                candidate.assembly().assembly_identity.clone(),
                generation,
                runtime_replica_id,
                linked.source(),
                binding_template,
                activation_websocket_entry,
            )
            .with_context(|| {
                format!(
                    "failed to construct activation context for deployment {:?}",
                    deployment
                )
            })?;
            let runtime_program_db = runtime_program_db_by_deployment
                .get(deployment)
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "activation {:?} has no whole-candidate DB metadata projection",
                        deployment
                    )
                })?;
            let db_source = if runtime_program_db.is_empty() {
                DbCapabilitySource::unavailable()
            } else {
                let profile = profile.ok_or_else(|| {
                    anyhow::anyhow!(
                        "activation {:?} with DB metadata requires a trusted profile",
                        deployment
                    )
                })?;
                skiff_artifact_model::validate_activation_profile(profile)
                    .map_err(anyhow::Error::msg)?;
                let provider = service_db.ok_or_else(|| {
                    anyhow::anyhow!(
                        "activation {:?} requires Router-supplied serviceDb",
                        deployment
                    )
                })?;
                db_provider
                    .build(DbProviderBuildInput {
                        environment: profile.to_string(),
                        service_id: deployment.service_id.clone(),
                        config: DbProviderConfig::mongo(provider.mongo_url.as_str())
                            .map_err(provider_error)?,
                        runtime_program_db,
                    })
                    .map_err(provider_error)?
            };
            for (operation, linked_operation) in linked.operations() {
                if operation_targets
                    .insert(
                        (activation.activation_id().clone(), operation.clone()),
                        linked_operation.target().clone(),
                    )
                    .is_some()
                {
                    anyhow::bail!(
                        "activation {} repeats canonical operation {}",
                        activation.activation_id().as_str(),
                        operation
                    );
                }
            }
            if activations
                .insert(activation.activation_id().clone(), Arc::clone(&activation))
                .is_some()
            {
                anyhow::bail!(
                    "activation id {} is duplicated within generation {generation}",
                    activation.activation_id().as_str()
                );
            }
            db_sources.insert(activation.activation_id().clone(), db_source);
            if let Some(websocket_entry) = websocket_entry {
                if websocket_entries
                    .insert(deployment.clone(), websocket_entry)
                    .is_some()
                {
                    anyhow::bail!(
                        "deployment {:?} has duplicate admitted WebSocket route sets",
                        deployment
                    );
                }
            }
            if activations_by_deployment
                .insert(deployment.clone(), activation)
                .is_some()
            {
                anyhow::bail!(
                    "deployment {:?} has duplicate activation contexts",
                    deployment
                );
            }
            if contracts_by_deployment
                .insert(deployment.clone(), linked.contract().clone())
                .is_some()
            {
                anyhow::bail!(
                    "deployment {:?} has duplicate contract owner facts",
                    deployment
                );
            }
        }
        if activations.len() != candidate.activations().len() {
            anyhow::bail!("activation context set does not exactly match linked activations");
        }
        let contracts: BTreeMap<ServiceContractRef, Arc<ServiceContract>> = candidate
            .contract_store()
            .contracts()
            .map(|(reference, contract)| (reference.clone(), Arc::clone(contract)))
            .collect();
        let mut schema_records = BTreeMap::new();
        for contract_ref in contracts.keys() {
            let schema = candidate
                .contract_store()
                .resolved_schema(contract_ref)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "admitted contract {:?} has no resolved Package schema",
                        contract_ref
                    )
                })?;
            if schema.contract() != contract_ref {
                anyhow::bail!(
                    "admitted schema contract {:?} does not match store key {:?}",
                    schema.contract(),
                    contract_ref
                );
            }
            let records = schema
                .records()
                .map(|(type_id, record)| (type_id.clone(), Arc::clone(record)))
                .collect();
            schema_records.insert(contract_ref.clone(), Arc::new(records));
        }
        Ok(Self {
            activations,
            activations_by_deployment,
            contracts_by_deployment,
            contracts,
            schema_records,
            operation_targets,
            config_views,
            db_sources,
            websocket_entries,
        })
    }

    pub(crate) fn activation_for_deployment(
        &self,
        deployment: &ServiceDeploymentRef,
    ) -> Option<Arc<ActivationContext>> {
        self.activations_by_deployment.get(deployment).cloned()
    }

    pub(crate) fn contract_for_deployment(
        &self,
        deployment: &ServiceDeploymentRef,
    ) -> Option<&ServiceContractRef> {
        self.contracts_by_deployment.get(deployment)
    }

    pub(crate) fn db_source(&self, activation_id: &ActivationId) -> Option<DbCapabilitySource> {
        self.db_sources.get(activation_id).cloned()
    }

    pub(crate) fn config_views(
        &self,
        deployment: &ServiceDeploymentRef,
    ) -> Option<Arc<ActivationConfigViews>> {
        self.config_views.get(deployment).cloned()
    }

    pub(crate) fn websocket_entry(
        &self,
        deployment: &ServiceDeploymentRef,
    ) -> Option<&AdmittedWebSocketEntry> {
        self.websocket_entries.get(deployment)
    }

    #[cfg(test)]
    pub(crate) fn admitted_schema_records(
        &self,
        contract: &ServiceContractRef,
    ) -> Option<AdmittedPackageSchemaRecords> {
        self.schema_records.get(contract).cloned()
    }
}

fn candidate_db_provider_inputs(
    candidate: &AssemblyLinkedCandidate,
    runtime_program_db_by_deployment: &BTreeMap<
        ServiceDeploymentRef,
        Vec<DbProviderTargetMetadata>,
    >,
    service_db: Option<&AssemblyActivationServiceDb>,
    profile: Option<&str>,
) -> anyhow::Result<Vec<DbProviderBuildInput>> {
    let mut inputs = Vec::new();
    for (deployment, _) in candidate.activations() {
        let runtime_program_db = runtime_program_db_by_deployment
            .get(deployment)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "activation {:?} has no whole-candidate DB metadata projection",
                    deployment
                )
            })?;
        if runtime_program_db.is_empty() {
            continue;
        }
        let profile = profile.ok_or_else(|| {
            anyhow::anyhow!(
                "activation {:?} with DB metadata requires a trusted profile",
                deployment
            )
        })?;
        skiff_artifact_model::validate_activation_profile(profile).map_err(anyhow::Error::msg)?;
        let provider = service_db.ok_or_else(|| {
            anyhow::anyhow!(
                "activation {:?} requires Router-supplied serviceDb",
                deployment
            )
        })?;
        inputs.push(DbProviderBuildInput {
            environment: profile.to_string(),
            service_id: deployment.service_id.clone(),
            config: DbProviderConfig::mongo(provider.mongo_url.as_str()).map_err(provider_error)?,
            runtime_program_db,
        });
    }
    Ok(inputs)
}

fn candidate_db_metadata(
    candidate: &AssemblyLinkedCandidate,
) -> anyhow::Result<BTreeMap<ServiceDeploymentRef, Vec<DbProviderTargetMetadata>>> {
    candidate
        .activations()
        .map(|(deployment, linked)| {
            activation_db_metadata(candidate, linked.implementation_package_build_id())
                .map(|metadata| (deployment.clone(), metadata))
        })
        .collect()
}

#[derive(Debug, Clone)]
pub(crate) struct AdmittedWebSocketEntry {
    pub(crate) selector: skiff_artifact_model::IngressSelector,
    pub(crate) gateway_entry_key: skiff_artifact_model::GatewayEntryKey,
    pub(crate) gateway_entry_identity: skiff_artifact_model::GatewayEntryIdentity,
    pub(crate) websocket_entry_id: skiff_artifact_model::WebSocketEntryId,
    pub(crate) linked_entry: Arc<LinkedGatewayEntry>,
    methods: BTreeMap<String, AdmittedWebSocketMethodEntry>,
}

impl AdmittedWebSocketEntry {
    fn activation_parts(
        &self,
    ) -> (
        skiff_artifact_model::IngressSelector,
        skiff_artifact_model::GatewayEntryKey,
        skiff_artifact_model::GatewayEntryIdentity,
        skiff_artifact_model::WebSocketEntryId,
    ) {
        (
            self.selector.clone(),
            self.gateway_entry_key.clone(),
            self.gateway_entry_identity.clone(),
            self.websocket_entry_id.clone(),
        )
    }

    pub(crate) fn has_methods(&self) -> bool {
        !self.methods.is_empty()
    }

    pub(crate) fn method(&self, method: &str) -> Option<&AdmittedWebSocketMethodEntry> {
        self.methods.get(method)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AdmittedWebSocketMethodEntry {
    pub(crate) selector: skiff_artifact_model::IngressSelector,
    pub(crate) gateway_entry_key: GatewayEntryKey,
    pub(crate) gateway_entry_identity: GatewayEntryIdentity,
    pub(crate) profile: GatewayWebSocketRpcProfile,
    pub(crate) linked_entry: Arc<LinkedGatewayEntry>,
}

pub(crate) fn admitted_websocket_entry(
    candidate: &AssemblyLinkedCandidate,
    owner: &ServiceDeploymentRef,
) -> anyhow::Result<Option<AdmittedWebSocketEntry>> {
    let activation = candidate.activation(owner).ok_or_else(|| {
        anyhow::anyhow!("WebSocket admission owner {owner:?} has no linked activation")
    })?;
    let deployment = activation.deployment();
    let Some(deployment_facts) = deployment_websocket_entry(deployment, owner)? else {
        return Ok(None);
    };
    let entry_key = deployment_facts.entry_key;
    let deployment_entry = deployment_facts.entry;
    let binding = deployment_facts.binding;

    let linked_entry = candidate
        .gateway_entry(owner, entry_key)
        .ok_or_else(|| anyhow::anyhow!("activation {owner:?} WebSocket entry is not linked"))?;
    let selected_entry = candidate
        .ingress(&skiff_artifact_model::ServiceIngressKey {
            deployment: owner.clone(),
            selector: binding.selector.clone(),
        })
        .ok_or_else(|| anyhow::anyhow!("activation {owner:?} WebSocket selector is not linked"))?;
    if !Arc::ptr_eq(linked_entry, selected_entry)
        || linked_entry.owner() != owner
        || linked_entry.gateway_entry_key() != entry_key
        || linked_entry.gateway_entry_identity() != &deployment_entry.gateway_entry_identity
        || linked_entry.protocol_surface() != &deployment_entry.protocol_surface
        || linked_entry.adapter_plan() != &deployment_entry.adapter_plan
        || linked_entry
            .optional_handler()
            .map(|handler| handler.callable_id())
            != deployment_entry.handler.as_ref()
        || linked_entry.pre().is_some()
        || linked_entry.guard().is_some()
    {
        anyhow::bail!(
            "activation {owner:?} WebSocket selector, entry, identity, surface or handler join is not exact"
        );
    }

    let mut methods = BTreeMap::new();
    for method in deployment_facts.methods {
        let linked_method = candidate
            .gateway_entry(owner, method.entry_key)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "activation {owner:?} WebSocket JSON-RPC method {} is not linked",
                    method.method
                )
            })?;
        let selected_method = candidate
            .ingress(&skiff_artifact_model::ServiceIngressKey {
                deployment: owner.clone(),
                selector: method.binding.selector.clone(),
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "activation {owner:?} WebSocket JSON-RPC selector {:?} is not linked",
                    method.binding.selector
                )
            })?;
        if !Arc::ptr_eq(linked_method, selected_method)
            || linked_method.owner() != owner
            || linked_method.gateway_entry_key() != method.entry_key
            || linked_method.gateway_entry_identity() != &method.entry.gateway_entry_identity
            || linked_method.protocol_surface() != &method.entry.protocol_surface
            || linked_method.adapter_plan() != &method.entry.adapter_plan
            || linked_method
                .optional_handler()
                .map(|handler| handler.callable_id())
                != method.entry.handler.as_ref()
            || linked_method.pre().is_some()
            || linked_method.guard().is_some()
        {
            anyhow::bail!(
                "activation {owner:?} WebSocket JSON-RPC method {} selector, entry, identity, surface or handler join is not exact",
                method.method
            );
        }
        if methods
            .insert(
                method.method.to_string(),
                AdmittedWebSocketMethodEntry {
                    selector: method.binding.selector.clone(),
                    gateway_entry_key: method.entry_key.clone(),
                    gateway_entry_identity: method.entry.gateway_entry_identity.clone(),
                    profile: method.profile,
                    linked_entry: Arc::clone(linked_method),
                },
            )
            .is_some()
        {
            anyhow::bail!(
                "activation {owner:?} repeats WebSocket JSON-RPC method {:?}",
                method.method
            );
        }
    }

    Ok(Some(AdmittedWebSocketEntry {
        selector: binding.selector.clone(),
        gateway_entry_key: entry_key.clone(),
        gateway_entry_identity: deployment_entry.gateway_entry_identity.clone(),
        websocket_entry_id: deployment_facts.websocket_entry_id,
        linked_entry: Arc::clone(linked_entry),
        methods,
    }))
}

struct DeploymentWebSocketEntry<'a> {
    entry_key: &'a GatewayEntryKey,
    entry: &'a DeploymentGatewayEntry,
    binding: &'a DeploymentIngressBinding,
    websocket_entry_id: WebSocketEntryId,
    methods: Vec<DeploymentWebSocketMethodEntry<'a>>,
}

struct DeploymentWebSocketMethodEntry<'a> {
    method: &'a str,
    entry_key: &'a GatewayEntryKey,
    entry: &'a DeploymentGatewayEntry,
    binding: &'a DeploymentIngressBinding,
    profile: GatewayWebSocketRpcProfile,
}

fn deployment_websocket_entry<'a>(
    deployment: &'a ServiceDeployment,
    owner: &ServiceDeploymentRef,
) -> anyhow::Result<Option<DeploymentWebSocketEntry<'a>>> {
    if &skiff_artifact_identity::service_deployment_ref(deployment) != owner {
        anyhow::bail!("WebSocket admission deployment owner is not exact");
    }
    let physical_entries = deployment
        .gateway_entries
        .iter()
        .filter(|(_, entry)| {
            matches!(
                entry.protocol_surface.protocol,
                GatewayProtocolSurface::WebSocketConnect(_)
            )
        })
        .collect::<Vec<_>>();
    if physical_entries.len() > 1 {
        anyhow::bail!(
            "activation {owner:?} declares more than one physical WebSocket gateway entry"
        );
    }

    let method_entries = deployment
        .gateway_entries
        .iter()
        .filter(|(_, entry)| {
            matches!(
                entry.protocol_surface.protocol,
                GatewayProtocolSurface::WebSocketJsonRpc(_)
            )
        })
        .collect::<Vec<_>>();
    let websocket_bindings = deployment
        .ingress
        .iter()
        .filter(|binding| binding.selector.protocol == IngressProtocol::WebSocket)
        .collect::<Vec<_>>();

    let (entry_key, entry) = match physical_entries.first() {
        None if method_entries.is_empty() && websocket_bindings.is_empty() => return Ok(None),
        None if !method_entries.is_empty() => {
            anyhow::bail!(
                "activation {owner:?} declares orphan WebSocket JSON-RPC methods without a physical entry"
            )
        }
        None => anyhow::bail!(
            "activation {owner:?} WebSocket selectors have no physical WebSocket gateway entry"
        ),
        Some((entry_key, entry)) => (*entry_key, *entry),
    };

    let physical_bindings = websocket_bindings
        .iter()
        .copied()
        .filter(|binding| binding.gateway_entry_key == *entry_key)
        .collect::<Vec<_>>();
    if physical_bindings.len() != 1 {
        anyhow::bail!(
            "activation {owner:?} physical WebSocket gateway entry must have exactly one selector"
        );
    }
    let binding = physical_bindings[0];
    if entry_key.as_str() != WEBSOCKET_GATEWAY_ENTRY_KEY || binding.gateway_entry_key != *entry_key
    {
        anyhow::bail!(
            "activation {owner:?} WebSocket selector does not join the compiler-owned {WEBSOCKET_GATEWAY_ENTRY_KEY:?} entry"
        );
    }
    if binding.selector.method.is_some() || binding.selector.path.trim().is_empty() {
        anyhow::bail!(
            "activation {owner:?} WebSocket selector {:?} is not canonical",
            binding.selector
        );
    }
    if entry.adapter_plan.kind != GatewayAdapterKind::WebSocketConnect {
        anyhow::bail!("activation {owner:?} WebSocket entry has a non-connect adapter plan");
    }
    if entry.handler.is_none() && !entry.adapter_plan.args.is_empty() {
        anyhow::bail!("activation {owner:?} handler-free WebSocket entry has adapter arguments");
    }

    skiff_artifact_identity::validate_gateway_entry_protocol_surface(&entry.protocol_surface)
        .context("WebSocket gateway protocol surface is not canonical")?;
    let expected_gateway_identity = gateway_entry_identity(&entry.protocol_surface)
        .context("failed to compute canonical WebSocket gateway identity")?;
    if entry.gateway_entry_identity != expected_gateway_identity {
        anyhow::bail!(
            "activation {owner:?} WebSocket gateway identity does not match its protocol surface"
        );
    }

    let websocket_entry_id = websocket_entry_id(&owner.service_id, entry_key)
        .context("failed to compute canonical WebSocket entry id")?;
    let GatewayProtocolSurface::WebSocketConnect(connect_surface) =
        &entry.protocol_surface.protocol
    else {
        unreachable!("physical entries were filtered by the WebSocket connect surface")
    };
    let mut methods = Vec::with_capacity(method_entries.len());
    let mut method_names = BTreeSet::new();
    let mut method_binding_keys = BTreeSet::new();
    for (method_key, method_entry) in method_entries {
        if method_key.as_str() == WEBSOCKET_GATEWAY_ENTRY_KEY {
            anyhow::bail!(
                "activation {owner:?} WebSocket JSON-RPC method uses the physical entry key"
            );
        }
        let matching_bindings = websocket_bindings
            .iter()
            .copied()
            .filter(|candidate| candidate.gateway_entry_key == *method_key)
            .collect::<Vec<_>>();
        if matching_bindings.len() != 1 {
            anyhow::bail!(
                "activation {owner:?} WebSocket JSON-RPC entry {method_key} must have exactly one selector"
            );
        }
        let method_binding = matching_bindings[0];
        let Some(method) = method_binding.selector.method.as_deref() else {
            anyhow::bail!(
                "activation {owner:?} WebSocket JSON-RPC entry {method_key} has no method selector"
            );
        };
        if method.is_empty()
            || method_binding.selector.protocol != IngressProtocol::WebSocket
            || method_binding.selector.path != binding.selector.path
        {
            anyhow::bail!(
                "activation {owner:?} WebSocket JSON-RPC selector {:?} is not an exact physical sibling",
                method_binding.selector
            );
        }
        if !method_names.insert(method) {
            anyhow::bail!("activation {owner:?} repeats WebSocket JSON-RPC method {method:?}");
        }
        method_binding_keys.insert(method_key.clone());
        if method_entry.handler.is_none()
            || method_entry.pre.is_some()
            || method_entry.guard.is_some()
            || method_entry.adapter_plan.kind != GatewayAdapterKind::WebSocketJsonRpc
        {
            anyhow::bail!(
                "activation {owner:?} WebSocket JSON-RPC entry {method_key} has invalid handler or adapter facts"
            );
        }
        skiff_artifact_identity::validate_gateway_entry_protocol_surface(
            &method_entry.protocol_surface,
        )
        .context("WebSocket JSON-RPC protocol surface is not canonical")?;
        let expected_method_identity = gateway_entry_identity(&method_entry.protocol_surface)
            .context("failed to compute canonical WebSocket JSON-RPC gateway identity")?;
        if method_entry.gateway_entry_identity != expected_method_identity {
            anyhow::bail!(
                "activation {owner:?} WebSocket JSON-RPC entry {method_key} identity does not match its protocol surface"
            );
        }
        let GatewayProtocolSurface::WebSocketJsonRpc(method_surface) =
            &method_entry.protocol_surface.protocol
        else {
            unreachable!("method entries were filtered by the WebSocket JSON-RPC surface")
        };
        if !connect_surface
            .rpc_profiles
            .contains(&method_surface.profile)
        {
            anyhow::bail!(
                "activation {owner:?} WebSocket JSON-RPC entry {method_key} profile is not supported by the physical entry"
            );
        }
        methods.push(DeploymentWebSocketMethodEntry {
            method,
            entry_key: method_key,
            entry: method_entry,
            binding: method_binding,
            profile: method_surface.profile,
        });
    }
    if websocket_bindings.len() != methods.len() + 1
        || websocket_bindings.iter().any(|candidate| {
            candidate.gateway_entry_key != *entry_key
                && !method_binding_keys.contains(&candidate.gateway_entry_key)
        })
    {
        anyhow::bail!(
            "activation {owner:?} has WebSocket selectors outside its physical/method route set"
        );
    }

    Ok(Some(DeploymentWebSocketEntry {
        entry_key,
        entry,
        binding,
        websocket_entry_id,
        methods,
    }))
}

fn activation_db_metadata(
    candidate: &AssemblyLinkedCandidate,
    root: &PackageBuildId,
) -> anyhow::Result<Vec<DbProviderTargetMetadata>> {
    let image = candidate.execution_image().shared_packages();
    let mut pending = vec![(root.clone(), true)];
    let mut visited = BTreeSet::new();
    let mut builds_by_package_id = BTreeMap::new();
    let mut metadata = Vec::new();
    while let Some((build_id, is_root)) = pending.pop() {
        let code = image.code_by_build(&build_id).ok_or_else(|| {
            anyhow::anyhow!("activation DB metadata package {build_id} is not loaded")
        })?;
        if let Some(first_build) =
            builds_by_package_id.insert(code.artifact().package_id.as_str(), build_id.clone())
        {
            if first_build != build_id {
                anyhow::bail!(
                    "activation DB metadata resolves package ID {} to different builds {} and {}",
                    code.artifact().package_id,
                    first_build,
                    build_id
                );
            }
        }
        if !visited.insert(build_id.clone()) {
            continue;
        }
        for file in code.files() {
            let mut file_refs = code.artifact().files.iter().filter(|reference| {
                reference.file_ir_identity == file.file_ir_identity
                    && reference.module_path == file.module_path
                    && reference
                        .source_ast_hash
                        .as_deref()
                        .is_none_or(|hash| hash == file.source_ast_hash)
            });
            let file_ir_ref = file_refs.next().cloned().ok_or_else(|| {
                anyhow::anyhow!(
                    "activation DB metadata package {build_id} file {} has no exact artifact File IR reference",
                    file.file_ir_identity
                )
            })?;
            if file_refs.next().is_some() {
                anyhow::bail!(
                    "activation DB metadata package {build_id} file {} has multiple exact artifact File IR references",
                    file.file_ir_identity
                );
            }
            for (symbol, declaration) in &file.declarations.db {
                if declaration.kind == skiff_artifact_model::DbObjectKindIr::Contract {
                    continue;
                }
                let collection_name = declaration.collection_name.as_deref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "activation DB metadata package {build_id} file {} declaration {symbol} is a db object without a physical collection",
                        file.file_ir_identity
                    )
                })?;
                let declaration_symbol = match &declaration.type_ref {
                    skiff_artifact_model::TypeRefIr::LocalType { .. } => symbol.as_str(),
                    skiff_artifact_model::TypeRefIr::DbObjectSymbol { symbol: db_symbol }
                        if db_symbol.module_path == file.module_path
                            && db_symbol.symbol == *symbol =>
                    {
                        db_symbol.symbol.as_str()
                    }
                    _ => {
                        anyhow::bail!(
                            "activation DB metadata package {build_id} file {} declaration {symbol} does not name its exact local DB type",
                            file.file_ir_identity
                        );
                    }
                };
                let Some(type_declaration) = file.declarations.types.get(declaration_symbol) else {
                    anyhow::bail!(
                        "activation DB metadata package {build_id} file {} declaration {symbol} has no exact local type declaration",
                        file.file_ir_identity
                    );
                };
                let type_index = type_declaration.type_index as usize;
                let declared_type_index = match &declaration.type_ref {
                    skiff_artifact_model::TypeRefIr::LocalType { type_index } => {
                        Some(*type_index as usize)
                    }
                    _ => None,
                };
                if declared_type_index.is_some_and(|declared| declared != type_index)
                    || file
                        .type_table
                        .get(type_index)
                        .is_none_or(|ty| ty.name != declaration_symbol)
                {
                    anyhow::bail!(
                        "activation DB metadata package {build_id} file {} declaration {symbol} has an inconsistent type index",
                        file.file_ir_identity
                    );
                }
                let target = DbCapabilityTarget::new(
                    DbCapabilityTargetId {
                        package_artifact_ref: code.artifact_ref().clone(),
                        file_ir_ref: file_ir_ref.clone(),
                        type_index,
                    },
                    declaration.type_name.clone(),
                );
                metadata.push(DbProviderTargetMetadata {
                    target,
                    metadata: DbMetadataIr {
                        module_path: file.module_path.clone(),
                        source_role: if is_root { "service" } else { "package" }.to_string(),
                        package_id: (!is_root).then(|| code.artifact().package_id.clone()),
                        package_version: (!is_root)
                            .then(|| code.artifact().package_version.clone()),
                        file_ir_identity: Some(file.file_ir_identity.clone()),
                        kind: declaration.kind,
                        ty: declaration.type_ref.clone(),
                        type_name: declaration.type_name.clone(),
                        collection_name: Some(collection_name.to_string()),
                        key: Some(declaration.key.clone()),
                        fields: declaration.fields.clone(),
                        retention: declaration.retention.clone(),
                        leases: declaration.leases.clone(),
                        indexes: declaration
                            .indexes
                            .iter()
                            .map(|index| DbMetadataIndexIr {
                                name: index.name.clone(),
                                unique: index.unique,
                                fields: index.fields.clone(),
                            })
                            .collect(),
                    },
                });
            }
        }
        for link in &candidate.assembly().package_link_plan.package_links {
            if link.key.caller_package_build_id == build_id {
                pending.push((link.package.package_build_id.clone(), false));
            }
        }
    }
    metadata.sort_by(|left, right| {
        let left_id = &left.target.target_id;
        let right_id = &right.target.target_id;
        (
            &left_id.package_artifact_ref.package_id,
            &left_id.package_artifact_ref.package_version,
            left_id.package_artifact_ref.package_build_id.as_str(),
            left_id
                .package_artifact_ref
                .package_local_abi_identity
                .as_str(),
            &left_id.file_ir_ref.file_ir_identity,
            &left_id.file_ir_ref.module_path,
            left_id.type_index,
        )
            .cmp(&(
                &right_id.package_artifact_ref.package_id,
                &right_id.package_artifact_ref.package_version,
                right_id.package_artifact_ref.package_build_id.as_str(),
                right_id
                    .package_artifact_ref
                    .package_local_abi_identity
                    .as_str(),
                &right_id.file_ir_ref.file_ir_identity,
                &right_id.file_ir_ref.module_path,
                right_id.type_index,
            ))
    });
    Ok(metadata)
}

/// One `db contract` declaration fact for whole-assembly coverage validation.
#[derive(Debug, Clone)]
struct CandidateDbContractFacts {
    build_id: PackageBuildId,
    package_id: String,
    file_ir_ref: skiff_artifact_model::FileIrRef,
    type_index: u32,
    symbol: String,
    indexes: Vec<skiff_artifact_model::DbIndexIr>,
}

/// One `db object ... implements` declaration fact for whole-assembly coverage
/// validation.
#[derive(Debug, Clone)]
struct CandidateDbImplementationFacts {
    build_id: PackageBuildId,
    package_id: String,
    symbol: String,
    collection_name: String,
    implements: skiff_artifact_model::TypeRefIr,
    indexes: Vec<skiff_artifact_model::DbIndexIr>,
}

/// Activation-time contract coverage: every `db contract` declaration in the
/// whole candidate must be implemented by exactly one `db object ... implements`
/// declaration, and every required contract index must be covered by the
/// implementing collection's canonical managed index spec. All failures bail
/// through the existing admission fail-closed chain before DB provisioning.
fn validate_db_contract_implementations(candidate: &AssemblyLinkedCandidate) -> anyhow::Result<()> {
    let image = candidate.execution_image().shared_packages();
    let mut contracts: Vec<CandidateDbContractFacts> = Vec::new();
    let mut implementations: Vec<CandidateDbImplementationFacts> = Vec::new();
    let mut visited = BTreeSet::new();
    let mut pending = candidate
        .activations()
        .map(|(_, linked)| linked.implementation_package_build_id().clone())
        .collect::<Vec<_>>();
    while let Some(build_id) = pending.pop() {
        if !visited.insert(build_id.clone()) {
            continue;
        }
        let code = image.code_by_build(&build_id).ok_or_else(|| {
            anyhow::anyhow!("contract implementation validation package {build_id} is not loaded")
        })?;
        for file in code.files() {
            let mut file_refs = code.artifact().files.iter().filter(|reference| {
                reference.file_ir_identity == file.file_ir_identity
                    && reference.module_path == file.module_path
                    && reference
                        .source_ast_hash
                        .as_deref()
                        .is_none_or(|hash| hash == file.source_ast_hash)
            });
            let file_ir_ref = file_refs.next().cloned().ok_or_else(|| {
                anyhow::anyhow!(
                    "contract implementation validation package {build_id} file {} has no exact artifact File IR reference",
                    file.file_ir_identity
                )
            })?;
            if file_refs.next().is_some() {
                anyhow::bail!(
                    "contract implementation validation package {build_id} file {} has multiple exact artifact File IR references",
                    file.file_ir_identity
                );
            }
            for (symbol, declaration) in &file.declarations.db {
                let type_index = contract_declaration_type_index(file, symbol, declaration)?;
                match declaration.kind {
                    skiff_artifact_model::DbObjectKindIr::Contract => {
                        contracts.push(CandidateDbContractFacts {
                            build_id: build_id.clone(),
                            package_id: code.artifact().package_id.clone(),
                            file_ir_ref: file_ir_ref.clone(),
                            type_index,
                            symbol: symbol.clone(),
                            indexes: declaration.indexes.clone(),
                        });
                    }
                    skiff_artifact_model::DbObjectKindIr::Object => {
                        let Some(implements) = &declaration.implements else {
                            continue;
                        };
                        let collection_name = declaration.collection_name.as_deref().ok_or_else(|| {
                            anyhow::anyhow!(
                                "contract implementation validation package {build_id} file {} declaration {symbol} implements a contract without a physical collection",
                                file.file_ir_identity
                            )
                        })?;
                        implementations.push(CandidateDbImplementationFacts {
                            build_id: build_id.clone(),
                            package_id: code.artifact().package_id.clone(),
                            symbol: symbol.clone(),
                            collection_name: collection_name.to_string(),
                            implements: implements.clone(),
                            indexes: declaration.indexes.clone(),
                        });
                    }
                }
            }
        }
        for link in &candidate.assembly().package_link_plan.package_links {
            if link.key.caller_package_build_id == build_id {
                pending.push(link.package.package_build_id.clone());
            }
        }
    }

    let mut build_owners: BTreeMap<PackageBuildId, BTreeSet<String>> = BTreeMap::new();
    for (deployment, linked) in candidate.activations() {
        build_owners
            .entry(linked.implementation_package_build_id().clone())
            .or_default()
            .insert(deployment.service_id.clone());
    }
    let mut owners_by_contract: BTreeMap<CandidateContractKey, BTreeMap<String, Vec<usize>>> =
        BTreeMap::new();
    for (index, implementation) in implementations.iter().enumerate() {
        let (dependency_ref, symbol_path) = match &implementation.implements {
            skiff_artifact_model::TypeRefIr::PackageSymbol { symbol } => {
                match &symbol.package {
                    skiff_artifact_model::PackageRefIr::Dependency { dependency_ref } => {
                        (dependency_ref.as_str(), symbol.symbol_path.as_str())
                    }
                    _ => {
                        anyhow::bail!(
                            "db object {} in package {} implements {:?} which is not a dependency package contract reference",
                            implementation.symbol,
                            implementation.package_id,
                            implementation.implements
                        );
                    }
                }
            }
            _ => {
                anyhow::bail!(
                    "db object {} in package {} implements {:?} which is not a dependency package contract reference",
                    implementation.symbol,
                    implementation.package_id,
                    implementation.implements
                );
            }
        };
        let mut binding_matches = image.package_link_plan().package_links.iter().filter(
            |binding| {
                binding.key.caller_package_build_id == implementation.build_id
                    && binding.key.package_requirement_alias == dependency_ref
            },
        );
        let binding = binding_matches.next().ok_or_else(|| {
            anyhow::anyhow!(
                "db object {} in package {} implements contract target {dependency_ref} which is not a linked dependency",
                implementation.symbol, implementation.package_id
            )
        })?;
        if binding_matches.next().is_some() {
            anyhow::bail!(
                "db object {} in package {} implements ambiguous dependency {dependency_ref}",
                implementation.symbol,
                implementation.package_id
            );
        }
        let target_build = &binding.package.package_build_id;
        let target_code = image.code_by_build(target_build).ok_or_else(|| {
            anyhow::anyhow!(
                "db object {} in package {} implements {dependency_ref}/{symbol_path} which is not loaded",
                implementation.symbol,
                implementation.package_id
            )
        })?;
        let export = target_code
            .artifact()
            .implementation_links
            .types
            .get(symbol_path)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "db object {} in package {} implements {dependency_ref}/{symbol_path} which is not a db contract declaration",
                    implementation.symbol,
                    implementation.package_id
                )
            })?;
        if export.is_interface {
            anyhow::bail!(
                "db object {} in package {} implements {dependency_ref}/{symbol_path} which is an interface, not a db contract declaration",
                implementation.symbol,
                implementation.package_id
            );
        }
        let contract = contracts
            .iter()
            .find(|contract| {
                contract.build_id == *target_build
                    && contract.file_ir_ref.file_ir_identity == export.file.file_ir_identity
                    && contract.type_index == export.type_index
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "db object {} in package {} implements {dependency_ref}/{symbol_path} which is not a db contract declaration in the assembly",
                    implementation.symbol,
                    implementation.package_id
                )
            })?;
        let contract_key = CandidateContractKey {
            build_id: contract.build_id.clone(),
            file_ir_identity: contract.file_ir_ref.file_ir_identity.clone(),
            type_index: contract.type_index,
        };
        let owners = build_owners
            .get(&implementation.build_id)
            .cloned()
            .unwrap_or_else(|| BTreeSet::from([format!("build:{}", implementation.build_id)]));
        let by_owner = owners_by_contract.entry(contract_key).or_default();
        for owner in owners {
            by_owner.entry(owner).or_default().push(index);
        }
    }
    for contract in &contracts {
        let contract_key = CandidateContractKey {
            build_id: contract.build_id.clone(),
            file_ir_identity: contract.file_ir_ref.file_ir_identity.clone(),
            type_index: contract.type_index,
        };
        let by_owner = owners_by_contract.get(&contract_key);
        let Some(implementers) = by_owner.and_then(|by_owner| by_owner.iter().next().map(|(_, v)| v)) else {
            anyhow::bail!(
                "db contract {}/{} has no implementing db object declaration in the assembly",
                contract.package_id,
                contract.symbol
            );
        };
        if by_owner.is_some_and(|by_owner| by_owner.len() > 1) {
            anyhow::bail!(
                "db contract {}/{} is implemented by multiple services: {}",
                contract.package_id,
                contract.symbol,
                by_owner
                    .map(|by_owner| by_owner.keys().cloned().collect::<Vec<_>>().join(", "))
                    .unwrap_or_default()
            );
        }
        let mut implementers_by_build: BTreeMap<&PackageBuildId, usize> = BTreeMap::new();
        for index in implementers {
            *implementers_by_build
                .entry(&implementations[*index].build_id)
                .or_default() += 1;
        }
        if let Some((build_id, count)) = implementers_by_build.iter().find(|(_, count)| **count > 1) {
            anyhow::bail!(
                "db contract {}/{} is implemented more than once inside package build {build_id}",
                contract.package_id,
                contract.symbol
            );
        }
        let implementation_specs = implementers
            .iter()
            .flat_map(|index| {
                let implementation = &implementations[*index];
                implementation
                    .indexes
                    .iter()
                    .map(|index| {
                        managed_index_spec(
                            &implementation.package_id,
                            &implementation.collection_name,
                            index,
                        )
                    })
                    .collect::<anyhow::Result<Vec<_>>>()
            })
            .collect::<anyhow::Result<BTreeSet<_>>>()?;
        for index in &contract.indexes {
            let required = managed_index_spec(
                &implementations[implementers[0]].package_id,
                &implementations[implementers[0]].collection_name,
                index,
            )?;
            if !implementation_specs.contains(&required) {
                anyhow::bail!(
                    "db contract {}/{} requires managed index {} which no implementing db object of service {} declares",
                    contract.package_id,
                    contract.symbol,
                    index.name,
                    by_owner
                        .and_then(|by_owner| by_owner.keys().next().cloned())
                        .unwrap_or_default()
                );
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CandidateContractKey {
    build_id: PackageBuildId,
    file_ir_identity: String,
    type_index: u32,
}

fn contract_declaration_type_index(
    file: &skiff_artifact_model::FileIrUnit,
    symbol: &str,
    declaration: &skiff_artifact_model::DbDeclarationIr,
) -> anyhow::Result<u32> {
    let type_index = match &declaration.type_ref {
        skiff_artifact_model::TypeRefIr::LocalType { type_index } => Some(*type_index),
        skiff_artifact_model::TypeRefIr::DbObjectSymbol { symbol: db_symbol }
            if db_symbol.module_path == file.module_path && db_symbol.symbol == symbol =>
        {
            None
        }
        _ => {
            anyhow::bail!(
                "activation DB declaration {symbol} in {} does not name its exact local DB type",
                file.file_ir_identity
            );
        }
    };
    let Some(type_declaration) = file.declarations.types.get(symbol) else {
        anyhow::bail!(
            "activation DB declaration {symbol} in {} has no exact local type declaration",
            file.file_ir_identity
        );
    };
    let declared_index = type_index.unwrap_or(type_declaration.type_index);
    if declared_index != type_declaration.type_index {
        anyhow::bail!(
            "activation DB declaration {symbol} in {} has an inconsistent type index",
            file.file_ir_identity
        );
    }
    Ok(declared_index)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CanonicalManagedIndex {
    name: String,
    keys: Vec<(String, i32)>,
    unique: bool,
}

fn managed_index_spec(
    package_id: &str,
    collection_name: &str,
    index: &skiff_artifact_model::DbIndexIr,
) -> anyhow::Result<CanonicalManagedIndex> {
    let keys = index
        .fields
        .iter()
        .map(|field| {
            let direction = match field.direction {
                skiff_artifact_model::DbIndexDirectionIr::Asc => 1,
                skiff_artifact_model::DbIndexDirectionIr::Desc => -1,
            };
            Ok((field.field.text.clone(), direction))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    if keys.is_empty() {
        anyhow::bail!(
            "managed index {} must declare at least one physical key",
            index.name
        );
    }
    Ok(CanonicalManagedIndex {
        name: skiff_runtime_service_db::managed_index_name(
            package_id,
            collection_name,
            &index.name,
        ),
        keys,
        unique: index.unique,
    })
}

impl RuntimeAssemblyEvalResolver for ActiveAssemblyContextSet {
    fn activation(&self, activation_id: &ActivationId) -> Option<Arc<ActivationContext>> {
        self.activations.get(activation_id).cloned()
    }

    fn activation_by_opaque_id(&self, activation_id: &str) -> Option<Arc<ActivationContext>> {
        self.activations
            .values()
            .find(|activation| activation.activation_id().as_str() == activation_id)
            .cloned()
    }

    fn contract(&self, contract: &ServiceContractRef) -> Option<Arc<ServiceContract>> {
        self.contracts.get(contract).cloned()
    }

    fn admitted_schema_records(
        &self,
        contract: &ServiceContractRef,
    ) -> Option<AdmittedPackageSchemaRecords> {
        self.schema_records.get(contract).cloned()
    }

    fn operation_target(
        &self,
        activation_id: &ActivationId,
        operation: &ContractOperationId,
    ) -> Option<OperationTargetRef> {
        self.operation_targets
            .get(&(activation_id.clone(), operation.clone()))
            .cloned()
    }
}

#[cfg(test)]
mod websocket_admission_tests {
    use super::*;
    use skiff_artifact_model::{
        DeploymentArtifactIdentity, DeploymentDiagnosticText, DeploymentRevision,
        GatewayAdapterPlan, GatewayAdapterSource, GatewayDispatchMode, GatewayEntryProtocolSurface,
        GatewayExternalErrorProjection, GatewayExternalSchema,
        GatewayWebSocketConnectProtocolSurface, GatewayWebSocketDownlinkFrame,
        GatewayWebSocketJsonRpcProtocolSurface, GatewayWebSocketRpcProfile,
        GatewayWebSocketShapeVersion, PackageArtifactRef, PackageLocalAbiIdentity,
        ServiceProtocolIdentity, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
    };

    fn owner() -> ServiceDeploymentRef {
        ServiceDeploymentRef {
            service_id: "service:websocket".to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: DeploymentRevision::new("revision:one"),
            deployment_artifact_identity: DeploymentArtifactIdentity::new("deployment:websocket"),
        }
    }

    fn selector(path: &str) -> skiff_artifact_model::IngressSelector {
        skiff_artifact_model::IngressSelector {
            protocol: IngressProtocol::WebSocket,
            method: None,
            path: path.to_string(),
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
                    rpc_profiles: vec![GatewayWebSocketRpcProfile::JsonRpc2_0Text],
                },
            ),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        }
    }

    fn websocket_jsonrpc_surface() -> GatewayEntryProtocolSurface {
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

    fn deployment() -> ServiceDeployment {
        let key = GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap();
        let protocol_surface = websocket_surface();
        let gateway_entry_identity = gateway_entry_identity(&protocol_surface).unwrap();
        ServiceDeployment {
            schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
            contract: ServiceContractRef {
                service_id: "service:websocket".to_string(),
                contract_version: "1.0.0".to_string(),
                service_protocol_identity: ServiceProtocolIdentity::new("protocol:websocket"),
            },
            deployment_revision: DeploymentRevision::new("revision:one"),
            deployment_artifact_identity: DeploymentArtifactIdentity::new("deployment:websocket"),
            implementation: PackageArtifactRef {
                package_id: "package:websocket".to_string(),
                package_version: "1.0.0".to_string(),
                package_build_id: PackageBuildId::new("build:websocket"),
                package_local_abi_identity: PackageLocalAbiIdentity::new("abi:websocket"),
            },
            operation_bindings: Vec::new(),
            package_bindings: Vec::new(),
            service_selectors: Vec::new(),
            gateway_entries: BTreeMap::from([(
                key.clone(),
                DeploymentGatewayEntry {
                    gateway_entry_identity,
                    protocol_surface,
                    handler: Some(skiff_artifact_model::PackageCallableId::new(
                        "callable:websocket-connect",
                    )),
                    pre: None,
                    guard: None,
                    adapter_plan: GatewayAdapterPlan {
                        kind: GatewayAdapterKind::WebSocketConnect,
                        args: Vec::new(),
                    },
                },
            )]),
            ingress: vec![DeploymentIngressBinding {
                selector: selector("/connect"),
                gateway_entry_key: key,
            }],
            diagnostic_text: DeploymentDiagnosticText {
                display_name: "WebSocket admission fixture".to_string(),
                notes: BTreeMap::new(),
            },
        }
    }

    fn add_jsonrpc_method(deployment: &mut ServiceDeployment, key: &str, method: &str, path: &str) {
        let method_key = GatewayEntryKey::parse(key).unwrap();
        let method_surface = websocket_jsonrpc_surface();
        deployment.gateway_entries.insert(
            method_key.clone(),
            DeploymentGatewayEntry {
                gateway_entry_identity: gateway_entry_identity(&method_surface).unwrap(),
                protocol_surface: method_surface,
                handler: Some(skiff_artifact_model::PackageCallableId::new(format!(
                    "callable:websocket-{key}"
                ))),
                pre: None,
                guard: None,
                adapter_plan: GatewayAdapterPlan {
                    kind: GatewayAdapterKind::WebSocketJsonRpc,
                    args: vec![skiff_artifact_model::GatewayAdapterArg {
                        param: "params".to_string(),
                        source: GatewayAdapterSource::WebSocketJsonRpcParams,
                    }],
                },
            },
        );
        deployment.ingress.push(DeploymentIngressBinding {
            selector: skiff_artifact_model::IngressSelector {
                protocol: IngressProtocol::WebSocket,
                method: Some(method.to_string()),
                path: path.to_string(),
            },
            gateway_entry_key: method_key,
        });
    }

    #[test]
    fn websocket_admission_accepts_zero_or_one_exact_entry() {
        let owner = owner();
        let exact = deployment();
        let admitted = deployment_websocket_entry(&exact, &owner)
            .expect("canonical deployment facts")
            .expect("canonical sole entry");
        assert_eq!(admitted.entry_key.as_str(), WEBSOCKET_GATEWAY_ENTRY_KEY);
        assert_eq!(admitted.binding.selector, selector("/connect"));
        assert_eq!(
            admitted.websocket_entry_id,
            websocket_entry_id(&owner.service_id, admitted.entry_key).unwrap()
        );

        let mut zero = exact;
        zero.gateway_entries.clear();
        zero.ingress.clear();
        assert!(deployment_websocket_entry(&zero, &owner)
            .expect("zero WebSocket entries are legal")
            .is_none());
    }

    #[test]
    fn websocket_jsonrpc_target_admission_accepts_handlerless_physical_with_method_sibling() {
        let owner = owner();
        let mut deployment = deployment();
        deployment
            .gateway_entries
            .get_mut(&GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap())
            .unwrap()
            .handler = None;

        add_jsonrpc_method(&mut deployment, "status", "status.get", "/connect");

        deployment_websocket_entry(&deployment, &owner)
            .expect("handlerless physical entry and exact method sibling must be admitted")
            .expect("physical WebSocket entry remains the attach route");
    }

    #[test]
    fn websocket_jsonrpc_target_admission_rejects_duplicate_or_orphan_methods() {
        let owner = owner();
        let mut duplicate = deployment();
        add_jsonrpc_method(&mut duplicate, "status", "status.get", "/connect");
        add_jsonrpc_method(&mut duplicate, "status-copy", "status.get", "/connect");
        assert!(deployment_websocket_entry(&duplicate, &owner).is_err());

        let mut orphan = deployment();
        add_jsonrpc_method(&mut orphan, "status", "status.get", "/connect");
        orphan
            .gateway_entries
            .remove(&GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap());
        orphan
            .ingress
            .retain(|binding| binding.selector.method.is_some());
        assert!(deployment_websocket_entry(&orphan, &owner).is_err());
    }

    #[test]
    fn websocket_jsonrpc_target_admission_rejects_owner_path_profile_and_identity_mismatch() {
        let owner = owner();
        let mut wrong_owner = owner.clone();
        wrong_owner.service_id = "service:other".to_string();
        assert!(deployment_websocket_entry(&deployment(), &wrong_owner).is_err());

        let mut mismatched = deployment();
        add_jsonrpc_method(&mut mismatched, "status", "status.get", "/other");
        assert!(deployment_websocket_entry(&mismatched, &owner).is_err());

        let mut wrong_profile = deployment();
        add_jsonrpc_method(&mut wrong_profile, "status", "status.get", "/connect");
        let physical = wrong_profile
            .gateway_entries
            .get_mut(&GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap())
            .unwrap();
        let GatewayProtocolSurface::WebSocketConnect(surface) =
            &mut physical.protocol_surface.protocol
        else {
            unreachable!()
        };
        surface.rpc_profiles.clear();
        assert!(deployment_websocket_entry(&wrong_profile, &owner).is_err());

        let mut wrong_method_identity = deployment();
        add_jsonrpc_method(
            &mut wrong_method_identity,
            "status",
            "status.get",
            "/connect",
        );
        let physical_identity = wrong_method_identity
            .gateway_entries
            .get(&GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap())
            .unwrap()
            .gateway_entry_identity
            .clone();
        wrong_method_identity
            .gateway_entries
            .get_mut(&GatewayEntryKey::parse("status").unwrap())
            .unwrap()
            .gateway_entry_identity = physical_identity;
        assert!(deployment_websocket_entry(&wrong_method_identity, &owner).is_err());
    }

    #[test]
    fn websocket_admission_rejects_multiple_entries_and_selectors() {
        let owner = owner();
        let mut multiple_entries = deployment();
        let extra_key = GatewayEntryKey::parse("websocket:extra").unwrap();
        let extra_entry = multiple_entries
            .gateway_entries
            .values()
            .next()
            .unwrap()
            .clone();
        multiple_entries
            .gateway_entries
            .insert(extra_key, extra_entry);
        assert!(deployment_websocket_entry(&multiple_entries, &owner).is_err());

        let mut multiple_selectors = deployment();
        multiple_selectors.ingress.push(DeploymentIngressBinding {
            selector: selector("/second"),
            gateway_entry_key: GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap(),
        });
        assert!(deployment_websocket_entry(&multiple_selectors, &owner).is_err());
    }

    #[test]
    fn websocket_admission_rejects_dangling_or_noncanonical_selector_key_join() {
        let owner = owner();
        let mut dangling_entry = deployment();
        dangling_entry.ingress.clear();
        assert!(deployment_websocket_entry(&dangling_entry, &owner).is_err());

        let mut dangling_selector = deployment();
        dangling_selector.gateway_entries.clear();
        assert!(deployment_websocket_entry(&dangling_selector, &owner).is_err());

        let mut wrong_key = deployment();
        wrong_key.ingress[0].gateway_entry_key = GatewayEntryKey::parse("websocket:wrong").unwrap();
        assert!(deployment_websocket_entry(&wrong_key, &owner).is_err());

        let mut wrong_selector = deployment();
        wrong_selector.ingress[0].selector.method = Some("GET".to_string());
        assert!(deployment_websocket_entry(&wrong_selector, &owner).is_err());
    }

    #[test]
    fn websocket_admission_rejects_gateway_identity_and_surface_mismatch() {
        let owner = owner();
        let mut wrong_identity = deployment();
        wrong_identity
            .gateway_entries
            .values_mut()
            .next()
            .unwrap()
            .gateway_entry_identity = skiff_artifact_model::GatewayEntryIdentity::parse(format!(
            "skiff-gateway-entry-v2:sha256:{}",
            "0".repeat(64)
        ))
        .unwrap();
        assert!(deployment_websocket_entry(&wrong_identity, &owner).is_err());

        let mut wrong_surface = deployment();
        let entry = wrong_surface.gateway_entries.values_mut().next().unwrap();
        let GatewayProtocolSurface::WebSocketConnect(surface) =
            &mut entry.protocol_surface.protocol
        else {
            unreachable!("fixture surface is WebSocket connect");
        };
        surface.external_sources.clear();
        assert!(deployment_websocket_entry(&wrong_surface, &owner).is_err());
    }
}
