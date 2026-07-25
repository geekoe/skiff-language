use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use anyhow::Context;
use serde_json::json;
use skiff_artifact_model::{
    AssemblyActivationServiceDb, ContractOperationId, DbMetadataIndexIr, DbMetadataIr,
    OperationTargetRef, PackageBuildId, ServiceContract, ServiceContractRef, ServiceDeploymentRef,
    StateBindingKind,
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
            let activation = ActivationContext::from_assembly_templates(
                candidate.assembly().assembly_identity.clone(),
                generation,
                runtime_replica_id,
                linked.source(),
                binding_template,
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

fn activation_db_metadata(
    candidate: &AssemblyLinkedCandidate,
    root: &PackageBuildId,
) -> anyhow::Result<Vec<DbMetadataIr>> {
    let image = candidate.execution_image().shared_packages();
    let mut pending = vec![(root.clone(), true)];
    let mut visited = BTreeSet::new();
    let mut metadata = Vec::new();
    while let Some((build_id, is_root)) = pending.pop() {
        if !visited.insert(build_id.clone()) {
            continue;
        }
        let code = image.code_by_build(&build_id).ok_or_else(|| {
            anyhow::anyhow!("activation DB metadata package {build_id} is not loaded")
        })?;
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
                    collection_name: declaration.collection_name.clone(),
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
                pending.push((link.package.package_build_id.clone(), false));
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
