use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use anyhow::Context;
use serde_json::json;
use skiff_artifact_identity::{gateway_entry_identity, websocket_entry_id};
use skiff_artifact_model::{
    AssemblyActivationServiceDb, ContractOperationId, DbMetadataIndexIr, DbMetadataIr,
    GatewayAdapterKind, GatewayProtocolSurface, IngressProtocol, OperationTargetRef,
    PackageBuildId, ServiceContract, ServiceContractRef, ServiceDeploymentRef, StateBindingKind,
    WEBSOCKET_GATEWAY_ENTRY_KEY,
};
use skiff_runtime_activation::{ActivationContext, ActivationId};
use skiff_runtime_capability_context::{
    DbCapabilitySource, DbProviderBuildInput, DbProviderConfig, DbProviderSource,
};
use skiff_runtime_eval::{AdmittedPackageSchemaRecords, RuntimeAssemblyEvalResolver};
use skiff_runtime_linker::AssemblyLinkedCandidate;

/// Immutable activation owners and canonical target facts published with one assembly generation.
#[derive(Debug)]
pub(crate) struct ActiveAssemblyContextSet {
    activations: BTreeMap<ActivationId, Arc<ActivationContext>>,
    activations_by_deployment: BTreeMap<ServiceDeploymentRef, Arc<ActivationContext>>,
    contracts: BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    schema_records: BTreeMap<ServiceContractRef, AdmittedPackageSchemaRecords>,
    operation_targets: BTreeMap<(ActivationId, ContractOperationId), OperationTargetRef>,
    db_sources: BTreeMap<ActivationId, DbCapabilitySource>,
}

impl ActiveAssemblyContextSet {
    pub(crate) fn from_candidate(
        candidate: &AssemblyLinkedCandidate,
        generation: u64,
        runtime_replica_id: &str,
        db_provider: &DbProviderSource,
        service_db: Option<&AssemblyActivationServiceDb>,
    ) -> anyhow::Result<Self> {
        if runtime_replica_id.trim().is_empty() {
            anyhow::bail!("runtime replica id must be non-empty for activation construction");
        }
        let mut activations = BTreeMap::new();
        let mut activations_by_deployment = BTreeMap::new();
        let mut operation_targets = BTreeMap::new();
        let mut db_sources = BTreeMap::new();
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
            let websocket_entry = admitted_websocket_entry(candidate, deployment)?
                .map(AdmittedWebSocketEntry::into_activation_parts);
            let activation = ActivationContext::from_assembly_templates_with_websocket_entry(
                candidate.assembly().assembly_identity.clone(),
                generation,
                runtime_replica_id,
                linked.source(),
                binding_template,
                websocket_entry,
            )
            .with_context(|| {
                format!(
                    "failed to construct activation context for deployment {:?}",
                    deployment
                )
            })?;
            let database_bindings = linked
                .source()
                .state_bindings
                .iter()
                .filter(|binding| binding.kind == StateBindingKind::Database)
                .collect::<Vec<_>>();
            let database_namespaces = database_bindings
                .iter()
                .map(|binding| binding.namespace.as_str())
                .collect::<BTreeSet<_>>();
            if database_namespaces.len() > 1 {
                anyhow::bail!(
                    "activation {:?} has database state bindings for multiple namespaces",
                    deployment
                );
            }
            let runtime_program_db =
                activation_db_metadata(candidate, linked.implementation_package_build_id())?;
            let db_source = match (database_bindings.first(), runtime_program_db.is_empty()) {
                (None, true) => DbCapabilitySource::unavailable(),
                (None, false) => anyhow::bail!(
                    "activation {:?} has DB metadata without a database state binding",
                    deployment
                ),
                (Some(_), true) => anyhow::bail!(
                    "activation {:?} has a database state binding without DB metadata",
                    deployment
                ),
                (Some(binding), false) => {
                    let provider = service_db.ok_or_else(|| {
                        anyhow::anyhow!(
                            "activation {:?} requires Router-supplied serviceDb",
                            deployment
                        )
                    })?;
                    db_provider
                        .build(DbProviderBuildInput {
                            service_id: deployment.service_id.clone(),
                            state_namespace: binding.namespace.clone(),
                            config: DbProviderConfig::opaque(json!({
                                "mongoUrl": provider.mongo_url,
                            })),
                            runtime_program_db,
                        })
                        .map_err(|error| anyhow::anyhow!(error.to_string()))?
                }
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
            db_sources,
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

    #[cfg(test)]
    pub(crate) fn admitted_schema_records(
        &self,
        contract: &ServiceContractRef,
    ) -> Option<AdmittedPackageSchemaRecords> {
        self.schema_records.get(contract).cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdmittedWebSocketEntry {
    pub(crate) selector: skiff_artifact_model::IngressSelector,
    pub(crate) gateway_entry_key: skiff_artifact_model::GatewayEntryKey,
    pub(crate) gateway_entry_identity: skiff_artifact_model::GatewayEntryIdentity,
    pub(crate) websocket_entry_id: skiff_artifact_model::WebSocketEntryId,
}

impl AdmittedWebSocketEntry {
    fn into_activation_parts(
        self,
    ) -> (
        skiff_artifact_model::IngressSelector,
        skiff_artifact_model::GatewayEntryKey,
        skiff_artifact_model::GatewayEntryIdentity,
        skiff_artifact_model::WebSocketEntryId,
    ) {
        (
            self.selector,
            self.gateway_entry_key,
            self.gateway_entry_identity,
            self.websocket_entry_id,
        )
    }
}

pub(crate) fn admitted_websocket_entry(
    candidate: &AssemblyLinkedCandidate,
    owner: &ServiceDeploymentRef,
) -> anyhow::Result<Option<AdmittedWebSocketEntry>> {
    let activation = candidate.activation(owner).ok_or_else(|| {
        anyhow::anyhow!("WebSocket admission owner {owner:?} has no linked activation")
    })?;
    let deployment = activation.deployment();

    let websocket_entries = deployment
        .gateway_entries
        .iter()
        .filter(|(_, entry)| {
            matches!(
                entry.protocol_surface.protocol,
                GatewayProtocolSurface::WebSocketConnect(_)
            )
        })
        .collect::<Vec<_>>();
    if websocket_entries.len() > 1 {
        anyhow::bail!("activation {owner:?} declares more than one WebSocket gateway entry");
    }

    let websocket_bindings = deployment
        .ingress
        .iter()
        .filter(|binding| binding.selector.protocol == IngressProtocol::WebSocket)
        .collect::<Vec<_>>();
    if websocket_bindings.len() > 1 {
        anyhow::bail!("activation {owner:?} declares more than one WebSocket ingress selector");
    }

    let (entry_key, deployment_entry, binding) =
        match (websocket_entries.first(), websocket_bindings.first()) {
            (None, None) => return Ok(None),
            (Some(_), None) => anyhow::bail!(
                "activation {owner:?} WebSocket gateway entry has no ingress selector"
            ),
            (None, Some(binding)) => anyhow::bail!(
                "activation {owner:?} WebSocket selector {:?} has no WebSocket gateway entry",
                binding.selector
            ),
            (Some((entry_key, entry)), Some(binding)) => (*entry_key, *entry, *binding),
        };

    if entry_key.as_str() != WEBSOCKET_GATEWAY_ENTRY_KEY || binding.gateway_entry_key != *entry_key
    {
        anyhow::bail!(
            "activation {owner:?} WebSocket selector does not join the compiler-owned {WEBSOCKET_GATEWAY_ENTRY_KEY:?} entry"
        );
    }
    if binding.selector.method.is_some()
        || binding.selector.host.trim().is_empty()
        || binding.selector.path.trim().is_empty()
    {
        anyhow::bail!(
            "activation {owner:?} WebSocket selector {:?} is not canonical",
            binding.selector
        );
    }
    if deployment_entry.adapter_plan.kind != GatewayAdapterKind::WebSocketConnect {
        anyhow::bail!("activation {owner:?} WebSocket entry has a non-connect adapter plan");
    }
    if deployment_entry.handler.is_none() && !deployment_entry.adapter_plan.args.is_empty() {
        anyhow::bail!("activation {owner:?} handler-free WebSocket entry has adapter arguments");
    }

    let expected_gateway_identity = gateway_entry_identity(&deployment_entry.protocol_surface)
        .context("failed to compute canonical WebSocket gateway identity")?;
    if deployment_entry.gateway_entry_identity != expected_gateway_identity {
        anyhow::bail!(
            "activation {owner:?} WebSocket gateway identity does not match its protocol surface"
        );
    }

    let linked_entry = candidate
        .gateway_entry(owner, entry_key)
        .ok_or_else(|| anyhow::anyhow!("activation {owner:?} WebSocket entry is not linked"))?;
    let selected_entry = candidate
        .ingress(&binding.selector)
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

    let websocket_entry_id = websocket_entry_id(&owner.service_id, entry_key)
        .context("failed to compute canonical WebSocket entry id")?;
    Ok(Some(AdmittedWebSocketEntry {
        selector: binding.selector.clone(),
        gateway_entry_key: entry_key.clone(),
        gateway_entry_identity: deployment_entry.gateway_entry_identity.clone(),
        websocket_entry_id,
    }))
}

fn activation_db_metadata(
    candidate: &AssemblyLinkedCandidate,
    root: &PackageBuildId,
) -> anyhow::Result<Vec<DbMetadataIr>> {
    let image = candidate.execution_image().shared_packages();
    let mut pending = vec![(root.clone(), true, None)];
    let mut visited = BTreeSet::new();
    let mut active_collection_owners = BTreeMap::new();
    let mut projected_collection_builds = BTreeMap::new();
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
        let (owner, collection_names) = match edge {
            None => (
                format!("service package {build_id}"),
                source_collections
                    .iter()
                    .map(|name| (name.clone(), name.clone()))
                    .collect::<BTreeMap<_, _>>(),
            ),
            Some((owner, mapping)) => {
                let names = skiff_artifact_model::resolve_dependency_collection_names(
                    &source_collections,
                    &mapping,
                )
                .map_err(|message| {
                    anyhow::anyhow!(
                        "activation DB metadata {owner} has invalid collection mapping: {message}"
                    )
                })?;
                (owner, names)
            }
        };
        if !source_collections.is_empty() {
            if let Some(first_owner) =
                projected_collection_builds.insert(build_id.clone(), owner.clone())
            {
                anyhow::bail!(
                    "activation DB metadata package {build_id} has multiple active collection projections from {first_owner} and {owner}"
                );
            }
        }
        for target in collection_names.values() {
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
            for declaration in file.declarations.db.values() {
                metadata.push(DbMetadataIr {
                    module_path: file.module_path.clone(),
                    source_role: if is_root { "service" } else { "package" }.to_string(),
                    package_id: (!is_root).then(|| code.artifact().package_id.clone()),
                    package_version: (!is_root).then(|| code.artifact().package_version.clone()),
                    file_ir_identity: Some(file.file_ir_identity.clone()),
                    kind: declaration.kind,
                    ty: declaration.type_ref.clone(),
                    type_name: declaration.type_name.clone(),
                    collection_name: collection_names
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
        (
            &left.source_role,
            &left.package_id,
            &left.package_version,
            &left.module_path,
            &left.type_name,
        )
            .cmp(&(
                &right.source_role,
                &right.package_id,
                &right.package_version,
                &right.module_path,
                &right.type_name,
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
