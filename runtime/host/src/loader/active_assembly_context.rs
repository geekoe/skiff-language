use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use anyhow::Context;
use skiff_artifact_identity::{gateway_entry_identity, websocket_entry_id};
use skiff_artifact_model::{
    AssemblyActivationServiceDb, CanonicalActiveCollectionProjection, ContractOperationId,
    DbMetadataIndexIr, DbMetadataIr, DeploymentGatewayEntry, DeploymentIngressBinding,
    GatewayAdapterKind, GatewayEntryIdentity, GatewayEntryKey, GatewayProtocolSurface,
    GatewayWebSocketRpcProfile, IngressProtocol, OperationTargetRef, PackageBuildId,
    ServiceContract, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef, WebSocketEntryId,
    WEBSOCKET_GATEWAY_ENTRY_KEY,
};
use skiff_runtime_activation::{ActivationContext, ActivationId};
use skiff_runtime_capability_context::{
    DbCapabilitySource, DbCapabilityTarget, DbCapabilityTargetId, DbProviderBuildInput,
    DbProviderConfig, DbProviderSource, DbProviderTargetMetadata,
};
use skiff_runtime_eval::{AdmittedPackageSchemaRecords, RuntimeAssemblyEvalResolver};
use skiff_runtime_linker::{AssemblyLinkedCandidate, LinkedGatewayEntry};

use super::config_snapshot::{materialize_snapshot_config, ActivationConfigViews};

/// Immutable activation owners and canonical target facts published with one assembly generation.
#[derive(Debug)]
pub(crate) struct ActiveAssemblyContextSet {
    activations: BTreeMap<ActivationId, Arc<ActivationContext>>,
    activations_by_deployment: BTreeMap<ServiceDeploymentRef, Arc<ActivationContext>>,
    contracts: BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    schema_records: BTreeMap<ServiceContractRef, AdmittedPackageSchemaRecords>,
    operation_targets: BTreeMap<(ActivationId, ContractOperationId), OperationTargetRef>,
    config_views: BTreeMap<ServiceDeploymentRef, Arc<ActivationConfigViews>>,
    db_sources: BTreeMap<ActivationId, DbCapabilitySource>,
    websocket_entries: BTreeMap<ServiceDeploymentRef, AdmittedWebSocketEntry>,
}

impl ActiveAssemblyContextSet {
    pub(crate) fn from_candidate(
        candidate: &AssemblyLinkedCandidate,
        generation: u64,
        runtime_replica_id: &str,
        db_provider: &DbProviderSource,
        service_db: Option<&AssemblyActivationServiceDb>,
        environment: Option<&str>,
        config_snapshot: Option<&skiff_runtime_config_snapshot::RuntimeConfigSnapshot>,
    ) -> anyhow::Result<Self> {
        if runtime_replica_id.trim().is_empty() {
            anyhow::bail!("runtime replica id must be non-empty for activation construction");
        }
        let mut activations = BTreeMap::new();
        let mut activations_by_deployment = BTreeMap::new();
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
            let runtime_program_db =
                activation_db_metadata(candidate, linked.implementation_package_build_id())?;
            let db_source = if runtime_program_db.is_empty() {
                DbCapabilitySource::unavailable()
            } else {
                let environment = environment.ok_or_else(|| {
                    anyhow::anyhow!(
                        "activation {:?} with DB metadata requires a trusted environment",
                        deployment
                    )
                })?;
                skiff_artifact_model::validate_activation_environment(environment)
                    .map_err(anyhow::Error::msg)?;
                let provider = service_db.ok_or_else(|| {
                    anyhow::anyhow!(
                        "activation {:?} requires Router-supplied serviceDb",
                        deployment
                    )
                })?;
                db_provider
                    .build(DbProviderBuildInput {
                        environment: environment.to_string(),
                        service_id: deployment.service_id.clone(),
                        config: DbProviderConfig::mongo(provider.mongo_url.as_str())
                            .map_err(|error| anyhow::anyhow!(error.to_string()))?,
                        runtime_program_db,
                    })
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?
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
    let mut pending = vec![(root.clone(), true, None)];
    let mut visited = BTreeSet::new();
    let mut active_collection_owners = BTreeMap::new();
    let mut projected_collection_builds =
        BTreeMap::<PackageBuildId, (CanonicalActiveCollectionProjection, String, bool)>::new();
    let mut metadata = Vec::new();
    while let Some((build_id, is_root, edge)) = pending.pop() {
        let code = image.code_by_build(&build_id).ok_or_else(|| {
            anyhow::anyhow!("activation DB metadata package {build_id} is not loaded")
        })?;
        let source_collections = code
            .files()
            .iter()
            .flat_map(|file| file.declarations.db.values())
            .map(|declaration| declaration.collection_name.clone())
            .collect::<BTreeSet<_>>();
        let (owner, projection) = match edge {
            None => (
                format!("service package {build_id}"),
                CanonicalActiveCollectionProjection::resolve(&source_collections, &BTreeMap::new())
                    .expect("empty root collection projection is canonical"),
            ),
            Some((owner, mapping)) => {
                let projection =
                    CanonicalActiveCollectionProjection::resolve(&source_collections, &mapping)
                        .map_err(|message| {
                            anyhow::anyhow!(
                        "activation DB metadata {owner} has invalid collection mapping: {message}"
                    )
                        })?;
                (owner, projection)
            }
        };
        if !source_collections.is_empty() {
            if let Some((first_projection, first_owner, first_is_root)) =
                projected_collection_builds.get(&build_id)
            {
                if *first_is_root || is_root {
                    anyhow::bail!(
                        "activation DB collection owner collides between {first_owner} and {owner}"
                    );
                }
                if first_projection != &projection {
                    anyhow::bail!(
                        "activation DB metadata package {build_id} has different active collection projections from {first_owner} and {owner}"
                    );
                }
                continue;
            }
            projected_collection_builds.insert(
                build_id.clone(),
                (projection.clone(), owner.clone(), is_root),
            );
        }
        for target in projection.collection_names().values() {
            if let Some(first_owner) =
                active_collection_owners.insert(target.clone(), owner.clone())
            {
                anyhow::bail!(
                    "activation DB collection target {target:?} collides between {first_owner} and {owner}"
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
                        collection_name: projection
                            .collection_names()
                            .get(&declaration.collection_name)
                            .expect("declared collection was projected")
                            .clone(),
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
                                where_expr: index.where_expr.clone(),
                            })
                            .collect(),
                    },
                });
            }
        }
        for link in &candidate.assembly().package_link_plan.package_links {
            if link.key.caller_package_build_id == build_id {
                pending.push((
                    link.package.package_build_id.clone(),
                    false,
                    Some((
                        format!(
                            "dependency {}:{} -> {}",
                            link.key.caller_package_build_id,
                            link.key.package_requirement_alias,
                            link.package.package_build_id
                        ),
                        link.collection_name_mapping.clone(),
                    )),
                ));
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
            resource_bindings: Vec::new(),
            runtime_capability_bindings: Vec::new(),
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
