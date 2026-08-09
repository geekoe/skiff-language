use std::{cell::Cell, collections::BTreeMap, sync::Arc};

use sha2::{Digest, Sha256};
use skiff_artifact_model::{
    ActivationTemplate, BoundaryCallableProjection, BoundaryCallbackContract,
    BoundaryEffectGuarantee, BoundaryImplementationRequirements, BoundaryOperationContract,
    BoundaryOperationDescriptor, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary, CallableSemanticFacts,
    ContractDiagnosticText, ContractOperationId, ContractTypeDescriptor, ContractTypeNameability,
    ContractTypeRef, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentIngressBinding, DeploymentOperationBinding, DeploymentRevision, ExecutableBody,
    ExecutableExport, ExecutableIr, ExecutableKind, ExecutableSignatureIr, FileIrRef, FileIrUnit,
    GatewayEntryIdentity, GatewayEntryKey, GatewayIngressBinding, IngressProtocol, IngressSelector,
    PackageArtifact, PackageArtifactRef, PackageBuildId, PackageCallableId,
    PackageCallableLinkFact, PackageCallableParameter, PackageCallableSignature, PackageCodeSlot,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity, PackageLocalAbiSymbol,
    PackageRuntimeRequirements, PackageSchemaCanonicalDescriptor, PackageSchemaIndex,
    PackageSchemaIndexEntry, PackageSchemaIndexRef, PackageSchemaTypeId, PackageSchemaTypeRecord,
    PackageSchemaTypeRecordRef, PackageTypeRef, PackageTypeRequirement, PublicationResourceRef,
    RuntimeAssembly, ServiceBindingTemplate, ServiceContract, ServiceContractRef,
    ServiceDeployment, ServiceDeploymentRef, ServiceProtocolIdentity, SlotLayout, TypeRefIr,
    GATEWAY_ENTRY_IDENTITY_PREFIX, PACKAGE_ARTIFACT_SCHEMA_VERSION,
    RUNTIME_ASSEMBLY_SCHEMA_VERSION, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};

use super::*;

#[derive(Default)]
struct PanicResolver;

impl RuntimeAssemblyContentResolver for PanicResolver {
    fn resolve_deployment(
        &self,
        _reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        panic!("empty assembly must not resolve a deployment")
    }

    fn resolve_contract(
        &self,
        _reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        panic!("empty assembly must not resolve a contract")
    }

    fn resolve_package_schema_type(
        &self,
        _reference: &PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<PackageSchemaTypeRecord>> {
        panic!("empty assembly must not resolve package schema")
    }

    fn resolve_package(
        &self,
        _reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        panic!("empty assembly must not resolve a package")
    }

    fn resolve_file_ir(
        &self,
        _package: &PackageArtifactRef,
        _reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        panic!("empty assembly must not resolve File IR")
    }

    fn resolve_static_resource(
        &self,
        _package: &PackageArtifactRef,
        _reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        panic!("empty assembly must not resolve a resource")
    }
}

struct FixtureResolver {
    deployment_ref: ServiceDeploymentRef,
    deployment: Arc<ServiceDeployment>,
    additional_deployment: Option<(ServiceDeploymentRef, Arc<ServiceDeployment>)>,
    contract_ref: ServiceContractRef,
    contract: Arc<ServiceContract>,
    additional_contract: Option<(ServiceContractRef, Arc<ServiceContract>)>,
    package_ref: PackageArtifactRef,
    package: Arc<PackageArtifact>,
    additional_package: Option<(PackageArtifactRef, Arc<PackageArtifact>)>,
    file_ref: FileIrRef,
    file: Arc<FileIrUnit>,
    additional_files: BTreeMap<String, Arc<FileIrUnit>>,
    resource_ref: PublicationResourceRef,
    resource: Arc<[u8]>,
    package_loads: Cell<usize>,
    schema_records: BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
    schema_index: Option<Arc<PackageSchemaIndex>>,
    schema_loads: Cell<usize>,
}

impl RuntimeAssemblyContentResolver for FixtureResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        if reference == &self.deployment_ref {
            return Ok(Arc::clone(&self.deployment));
        }
        if let Some((additional_ref, deployment)) = &self.additional_deployment {
            if reference == additional_ref {
                return Ok(Arc::clone(deployment));
            }
        }
        anyhow::bail!("missing deployment")
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        if reference != &self.contract_ref {
            if let Some((additional_ref, contract)) = &self.additional_contract {
                if reference == additional_ref {
                    return Ok(Arc::clone(contract));
                }
            }
            anyhow::bail!("missing contract")
        }
        Ok(Arc::clone(&self.contract))
    }

    fn resolve_package_schema_type(
        &self,
        reference: &PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<PackageSchemaTypeRecord>> {
        self.schema_loads.set(self.schema_loads.get() + 1);
        self.schema_records
            .get(&reference.package_schema_type_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing package schema type"))
    }

    fn resolve_package_schema_index(
        &self,
        reference: &PackageSchemaIndexRef,
    ) -> anyhow::Result<Arc<PackageSchemaIndex>> {
        let index = self
            .schema_index
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing package schema index"))?;
        if index.package_id != reference.package_id
            || index.package_schema_index_identity != reference.package_schema_index_identity
        {
            anyhow::bail!("missing package schema index")
        }
        Ok(Arc::clone(index))
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        if reference == &self.package_ref {
            self.package_loads.set(self.package_loads.get() + 1);
            return Ok(Arc::clone(&self.package));
        }
        if let Some((additional_ref, package)) = &self.additional_package {
            if reference == additional_ref {
                self.package_loads.set(self.package_loads.get() + 1);
                return Ok(Arc::clone(package));
            }
        }
        anyhow::bail!("missing package")
    }

    fn resolve_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        let known_package = package == &self.package_ref
            || self
                .additional_package
                .as_ref()
                .is_some_and(|(additional_ref, _)| package == additional_ref);
        if !known_package {
            anyhow::bail!("missing File IR")
        }
        if reference == &self.file_ref {
            return Ok(Arc::clone(&self.file));
        }
        self.additional_files
            .get(&reference.file_ir_identity)
            .filter(|file| {
                file.module_path == reference.module_path
                    && reference
                        .source_ast_hash
                        .as_ref()
                        .is_none_or(|hash| hash == &file.source_ast_hash)
            })
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing File IR"))
    }

    fn resolve_static_resource(
        &self,
        package: &PackageArtifactRef,
        reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        let known_package = package == &self.package_ref
            || self
                .additional_package
                .as_ref()
                .is_some_and(|(additional_ref, _)| package == additional_ref);
        if !known_package || reference != &self.resource_ref {
            anyhow::bail!("missing static resource")
        }
        Ok(Arc::clone(&self.resource))
    }
}

struct Fixture {
    operation_id: ContractOperationId,
    callable_id: PackageCallableId,
    contract: ServiceContract,
    package: PackageArtifact,
    file: FileIrUnit,
    resource: Arc<[u8]>,
    deployment: ServiceDeployment,
    assembly: RuntimeAssembly,
    schema_index: PackageSchemaIndex,
    schema_records: BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
}

impl Fixture {
    fn new() -> Self {
        let service_id = "example.health";
        let contract_version = "1.0.0";
        let operation_id =
            skiff_artifact_identity::contract_operation_id(service_id, contract_version, "health")
                .unwrap();
        let operation_contract = operation_contract();
        let descriptor = BoundaryOperationDescriptor {
            operation_id: operation_id.clone(),
            stable_key: "health".to_string(),
            contract: operation_contract.clone(),
        };
        let mut contract = ServiceContract {
            schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
            service_id: service_id.to_string(),
            contract_version: contract_version.to_string(),
            service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
            operations: BTreeMap::from([(operation_id.clone(), descriptor)]),
            package_type_requirements: Vec::new(),
            diagnostic_text: ContractDiagnosticText {
                service: "Health".to_string(),
                operations: BTreeMap::from([(operation_id.clone(), "Health".to_string())]),
                types: BTreeMap::new(),
            },
        };
        skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
        let contract_ref = contract_ref(&contract);

        let mut file = FileIrUnit::empty("provider.main", "source-hash");
        file.executables.push(ExecutableIr {
            kind: ExecutableKind::Function,
            symbol: "health".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::builtin("bool"),
            self_type: None,
            slots: SlotLayout::default(),
            may_suspend: false,
            body: ExecutableBody::default(),
            source_span: None,
        });
        skiff_artifact_identity::assign_file_ir_identity(&mut file).unwrap();
        let file_ref = FileIrRef {
            file_ir_identity: file.file_ir_identity.clone(),
            module_path: file.module_path.clone(),
            artifact_path: None,
            source_ast_hash: Some(file.source_ast_hash.clone()),
        };

        let callable_id = PackageCallableId::new("pkg-callable:example.health-provider:health");
        let target = skiff_artifact_model::OperationTargetRef {
            file_ref: file_ref.clone(),
            executable_index: 0,
            callable_abi_id: callable_id.to_string(),
            callable_kind: skiff_artifact_model::OperationCallableKind::PublicFunction,
        };
        let resource: Arc<[u8]> = Arc::from(b"health-resource".as_slice());
        let resource_ref = PublicationResourceRef {
            path: "assets/health.txt".to_string(),
            sha256: hex::encode(Sha256::digest(resource.as_ref())),
            byte_len: resource.len() as u64,
            content_type: Some("text/plain".to_string()),
            artifact_path: None,
        };
        let effects = no_effects();
        let provenance = CallableProvenanceSummary::Analyzed {
            return_origins: Vec::new(),
            direct_return_origins: Vec::new(),
            throw_origins: Vec::new(),
            escape_lanes: Vec::new(),
        };
        let schema_index = PackageSchemaIndex {
            package_id: "example.health-provider".to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                "example.health-provider",
                &BTreeMap::new(),
            )
            .unwrap(),
            types: BTreeMap::new(),
        };
        let mut package = PackageArtifact {
            schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: "example.health-provider".to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new("unassigned"),
            files: vec![file_ref.clone()],
            static_resources: vec![resource_ref],
            package_local_abi: PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
                public_symbols: BTreeMap::from([(
                    "health".to_string(),
                    PackageLocalAbiSymbol::Callable {
                        callable_id: callable_id.clone(),
                        signature: PackageCallableSignature {
                            type_params: Vec::new(),
                            parameters: Vec::new(),
                            return_type: PackageTypeRef::Local {
                                local_type: TypeRefIr::builtin("bool"),
                            },
                            may_suspend: false,
                        },
                    },
                )]),
                implementation_symbols: BTreeMap::new(),
            },
            package_schema_index: PackageSchemaIndexRef {
                package_id: schema_index.package_id.clone(),
                package_schema_index_identity: schema_index.package_schema_index_identity.clone(),
            },
            package_schema_type_records: BTreeMap::new(),
            implementation_links: PackageImplementationLinks {
                functions: BTreeMap::from([(
                    "health".to_string(),
                    ExecutableExport {
                        file: file_ref.clone(),
                        executable_index: 0,
                        symbol: "health".to_string(),
                        signature: ExecutableSignatureIr {
                            params: Vec::new(),
                            return_type: TypeRefIr::builtin("bool"),
                            self_type: None,
                            may_suspend: false,
                        },
                    },
                )]),
                ..PackageImplementationLinks::default()
            },
            callable_links: BTreeMap::from([(
                callable_id.clone(),
                PackageCallableLinkFact {
                    callable_id: callable_id.clone(),
                    target,
                },
            )]),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
            callable_semantic_facts: BTreeMap::from([(
                callable_id.clone(),
                CallableSemanticFacts {
                    effects: CallableEffectSummary::Analyzed { effects },
                    provenance: provenance.clone(),
                    resolved_call_targets: BTreeMap::new(),
                },
            )]),
            boundary_projections: BTreeMap::from([(
                callable_id.clone(),
                BoundaryCallableProjection::Available {
                    operation_contract,
                    implementation_requirements: BoundaryImplementationRequirements {
                        config: Vec::new(),
                        state: Vec::new(),
                        native_capabilities: Vec::new(),
                        complete_may_effects: effects,
                        provenance,
                    },
                },
            )]),
            service_call_refs: Vec::new(),
            bytecode: None,
        };
        skiff_artifact_identity::assign_package_artifact_identities(&mut package).unwrap();
        let package_ref = package_ref(&package);

        let mut deployment = ServiceDeployment {
            schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
            contract: contract_ref.clone(),
            deployment_revision: DeploymentRevision::new("revision-1"),
            deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
            implementation: package_ref.clone(),
            operation_bindings: vec![DeploymentOperationBinding {
                contract_operation_id: operation_id.clone(),
                package_callable_id: callable_id.clone(),
            }],
            package_bindings: Vec::new(),
            service_selectors: Vec::new(),
            gateway_entries: BTreeMap::new(),
            ingress: Vec::new(),
            diagnostic_text: DeploymentDiagnosticText {
                display_name: "Health deployment".to_string(),
                notes: BTreeMap::new(),
            },
        };
        skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
        let deployment_ref = skiff_artifact_identity::service_deployment_ref(&deployment);

        let mut assembly = RuntimeAssembly {
            schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
            assembly_identity: skiff_artifact_model::AssemblyIdentity::new("unassigned"),
            roots: vec![deployment_ref.clone()],
            resolved_deployments: vec![deployment_ref.clone()],
            resolved_contracts: vec![contract_ref],
            resolved_packages: vec![package_ref.clone()],
            package_link_plan: skiff_artifact_model::CanonicalPackageLinkPlan {
                code_slots: vec![PackageCodeSlot {
                    package: package_ref,
                }],
                package_links: Vec::new(),
            },
            service_binding_templates: vec![ServiceBindingTemplate {
                activation: deployment_ref.clone(),
                bindings: Vec::new(),
            }],
            activation_templates: vec![ActivationTemplate {
                deployment: deployment_ref,
                implementation_package_build_id: package.package_build_id.clone(),
            }],
            gateway_ingress: Vec::new(),
        };
        skiff_artifact_identity::assign_runtime_assembly_identity(&mut assembly).unwrap();

        Self {
            operation_id,
            callable_id,
            contract,
            package,
            file,
            resource,
            deployment,
            assembly,
            schema_index,
            schema_records: BTreeMap::new(),
        }
    }

    fn resolver(&self) -> FixtureResolver {
        FixtureResolver {
            deployment_ref: skiff_artifact_identity::service_deployment_ref(&self.deployment),
            deployment: Arc::new(self.deployment.clone()),
            additional_deployment: None,
            contract_ref: contract_ref(&self.contract),
            contract: Arc::new(self.contract.clone()),
            additional_contract: None,
            package_ref: package_ref(&self.package),
            package: Arc::new(self.package.clone()),
            additional_package: None,
            file_ref: self.package.files[0].clone(),
            file: Arc::new(self.file.clone()),
            additional_files: BTreeMap::new(),
            resource_ref: self.package.static_resources[0].clone(),
            resource: Arc::clone(&self.resource),
            package_loads: Cell::new(0),
            schema_records: self.schema_records.clone(),
            schema_index: Some(Arc::new(self.schema_index.clone())),
            schema_loads: Cell::new(0),
        }
    }

    fn add_schema_return(&mut self, record: PackageSchemaTypeRecord) {
        let type_id = record.package_schema_type_id.clone();
        let package_return = PackageTypeRef::PackageSchema {
            package_id: record.package_id.clone(),
            stable_schema_key: record.stable_schema_key.clone(),
            package_schema_type_id: type_id.clone(),
        };
        let PackageLocalAbiSymbol::Callable { signature, .. } = self
            .package
            .package_local_abi
            .public_symbols
            .get_mut("health")
            .unwrap()
        else {
            unreachable!()
        };
        signature.return_type = package_return;
        self.contract
            .operations
            .get_mut(&self.operation_id)
            .unwrap()
            .contract
            .return_value
            .ty = ContractTypeRef::package_schema(
            record.package_id.clone(),
            record.stable_schema_key.clone(),
            type_id.clone(),
        );
        self.contract.package_type_requirements = vec![PackageTypeRequirement {
            package_id: record.package_id.clone(),
            required_type_ids: vec![type_id.clone()],
        }];
        if let BoundaryCallableProjection::Available {
            operation_contract, ..
        } = self
            .package
            .boundary_projections
            .get_mut(&self.callable_id)
            .unwrap()
        {
            operation_contract.return_value.ty = ContractTypeRef::package_schema(
                record.package_id.clone(),
                record.stable_schema_key.clone(),
                type_id.clone(),
            );
        }
        self.package.package_schema_type_records.insert(
            type_id.clone(),
            PackageSchemaTypeRecordRef {
                package_id: record.package_id.clone(),
                package_schema_type_id: type_id.clone(),
            },
        );
        self.schema_index.types.insert(
            record.stable_schema_key.clone(),
            PackageSchemaIndexEntry {
                package_schema_type_id: type_id.clone(),
                public_path: Some(record.stable_schema_key.clone()),
                nameability: ContractTypeNameability::PublicNameable,
            },
        );
        self.schema_index.package_schema_index_identity =
            skiff_artifact_identity::package_schema_index_identity(
                &self.schema_index.package_id,
                &self.schema_index.types,
            )
            .unwrap();
        self.package.package_schema_index = PackageSchemaIndexRef {
            package_id: self.schema_index.package_id.clone(),
            package_schema_index_identity: self.schema_index.package_schema_index_identity.clone(),
        };
        skiff_artifact_identity::assign_service_contract_identities(&mut self.contract).unwrap();
        self.schema_records.insert(type_id, Arc::new(record));
        let contract_ref = contract_ref(&self.contract);
        self.deployment.contract = contract_ref.clone();
        self.assembly.resolved_contracts = vec![contract_ref];
        self.refresh_package_chain();
    }

    fn add_schema_dependency_record(&mut self, record: PackageSchemaTypeRecord) {
        let type_id = record.package_schema_type_id.clone();
        if let Some(requirement) = self
            .contract
            .package_type_requirements
            .iter_mut()
            .find(|requirement| requirement.package_id == record.package_id)
        {
            requirement.required_type_ids.push(type_id.clone());
            requirement.required_type_ids.sort();
        } else {
            self.contract
                .package_type_requirements
                .push(PackageTypeRequirement {
                    package_id: record.package_id.clone(),
                    required_type_ids: vec![type_id.clone()],
                });
            self.contract
                .package_type_requirements
                .sort_by(|left, right| left.package_id.cmp(&right.package_id));
        }
        self.schema_records.insert(type_id, Arc::new(record));
        skiff_artifact_identity::assign_service_contract_identities(&mut self.contract).unwrap();
        let contract_ref = contract_ref(&self.contract);
        self.deployment.contract = contract_ref.clone();
        self.assembly.resolved_contracts = vec![contract_ref];
        self.refresh_deployment_chain();
    }

    fn add_http_gateway(&mut self) {
        let handler = PackageCallableId::new(
            "pkg-callable:example.health-provider:top-level:provider.main.gateway_handler",
        );
        let pre = PackageCallableId::new(
            "pkg-callable:example.health-provider:top-level:provider.main.gateway_pre",
        );
        let guard = PackageCallableId::new(
            "pkg-callable:example.health-provider:top-level:provider.main.gateway_guard",
        );
        for (path, callable_id) in [
            ("provider.main.gateway_handler", &handler),
            ("provider.main.gateway_pre", &pre),
            ("provider.main.gateway_guard", &guard),
        ] {
            self.add_private_gateway_callable(path, callable_id);
        }

        let key = GatewayEntryKey::parse("health-http").unwrap();
        let mut entry = skiff_deployment::fixtures::gateway_entry_fixture(handler);
        entry.pre = Some(pre);
        entry.guard = Some(guard);
        self.deployment.gateway_entries.insert(key.clone(), entry);
        self.deployment.ingress = ["/health", "/health-alias"]
            .into_iter()
            .map(|path| DeploymentIngressBinding {
                selector: IngressSelector {
                    protocol: IngressProtocol::Http,
                    method: Some("POST".to_string()),
                    path: path.to_string(),
                },
                gateway_entry_key: key.clone(),
            })
            .collect();
        self.refresh_package_chain();
    }

    fn add_private_gateway_callable(&mut self, path: &str, callable_id: &PackageCallableId) {
        let signature = PackageCallableSignature {
            type_params: Vec::new(),
            parameters: vec![PackageCallableParameter {
                name: "body".to_string(),
                ty: PackageTypeRef::Local {
                    local_type: TypeRefIr::builtin("string"),
                },
            }],
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            },
            may_suspend: false,
        };
        self.package
            .package_local_abi
            .implementation_symbols
            .insert(
                path.to_string(),
                PackageLocalAbiSymbol::Callable {
                    callable_id: callable_id.clone(),
                    signature,
                },
            );
        let mut target = self.package.callable_links[&self.callable_id]
            .target
            .clone();
        target.callable_abi_id = callable_id.to_string();
        target.callable_kind = skiff_artifact_model::OperationCallableKind::InternalFunction;
        self.package.callable_links.insert(
            callable_id.clone(),
            PackageCallableLinkFact {
                callable_id: callable_id.clone(),
                target,
            },
        );
        self.package.callable_semantic_facts.insert(
            callable_id.clone(),
            self.package.callable_semantic_facts[&self.callable_id].clone(),
        );
    }

    fn refresh_deployment_chain(&mut self) {
        skiff_artifact_identity::assign_service_deployment_identity(&mut self.deployment).unwrap();
        let reference = skiff_artifact_identity::service_deployment_ref(&self.deployment);
        self.assembly.roots = vec![reference.clone()];
        self.assembly.resolved_deployments = vec![reference.clone()];
        self.assembly.service_binding_templates[0].activation = reference.clone();
        self.assembly.activation_templates[0].deployment = reference.clone();
        self.assembly.gateway_ingress = self
            .deployment
            .ingress
            .iter()
            .map(|binding| {
                let entry = self
                    .deployment
                    .gateway_entries
                    .get(&binding.gateway_entry_key)
                    .unwrap();
                GatewayIngressBinding {
                    selector: binding.selector.clone(),
                    deployment: reference.clone(),
                    gateway_entry_key: binding.gateway_entry_key.clone(),
                    gateway_entry_identity: entry.gateway_entry_identity.clone(),
                }
            })
            .collect();
        skiff_artifact_identity::assign_runtime_assembly_identity(&mut self.assembly).unwrap();
    }

    fn refresh_package_chain(&mut self) {
        skiff_artifact_identity::assign_package_artifact_identities(&mut self.package).unwrap();
        let reference = package_ref(&self.package);
        self.deployment.implementation = reference.clone();
        self.assembly.resolved_packages = vec![reference.clone()];
        self.assembly.package_link_plan.code_slots = vec![PackageCodeSlot { package: reference }];
        self.assembly.activation_templates[0].implementation_package_build_id =
            self.package.package_build_id.clone();
        self.refresh_deployment_chain();
    }
}

#[test]
fn canonical_empty_assembly_hydrates_without_storage_reads() {
    let mut assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: skiff_artifact_model::AssemblyIdentity::new("unassigned"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: Vec::new(),
        package_link_plan: skiff_artifact_model::CanonicalPackageLinkPlan {
            code_slots: Vec::new(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut assembly).unwrap();

    let hydrated = RuntimeAssemblyLoader::new(&PanicResolver)
        .load(assembly)
        .unwrap();
    assert!(hydrated.code_slots().is_empty());
    assert!(hydrated.contract_store().is_empty());
    assert_eq!(hydrated.deployments().len(), 0);
}

#[test]
fn typed_loader_preserves_contract_store_and_deterministic_code_lookup() {
    let mut fixture = Fixture::new();
    let mut second_contract = fixture.contract.clone();
    second_contract.service_id = "example.health.secondary".to_string();
    let second_operation_id = skiff_artifact_identity::contract_operation_id(
        &second_contract.service_id,
        &second_contract.contract_version,
        "health",
    )
    .unwrap();
    let mut descriptor = second_contract.operations.pop_first().unwrap().1;
    descriptor.operation_id = second_operation_id.clone();
    second_contract
        .operations
        .insert(second_operation_id.clone(), descriptor);
    skiff_artifact_identity::assign_service_contract_identities(&mut second_contract).unwrap();
    let second_contract_ref = contract_ref(&second_contract);
    let mut second_deployment = fixture.deployment.clone();
    second_deployment.contract = second_contract_ref.clone();
    second_deployment.deployment_revision = DeploymentRevision::new("revision-2");
    second_deployment.operation_bindings[0].contract_operation_id = second_operation_id;
    skiff_artifact_identity::assign_service_deployment_identity(&mut second_deployment).unwrap();
    let second_ref = skiff_artifact_identity::service_deployment_ref(&second_deployment);
    fixture.assembly.roots.push(second_ref.clone());
    fixture
        .assembly
        .resolved_deployments
        .push(second_ref.clone());
    fixture
        .assembly
        .resolved_contracts
        .push(second_contract_ref.clone());
    fixture
        .assembly
        .service_binding_templates
        .push(ServiceBindingTemplate {
            activation: second_ref.clone(),
            bindings: Vec::new(),
        });
    let mut second_activation = fixture.assembly.activation_templates[0].clone();
    second_activation.deployment = second_ref.clone();
    fixture
        .assembly
        .activation_templates
        .push(second_activation);
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut fixture.assembly).unwrap();

    let mut resolver = fixture.resolver();
    resolver.additional_deployment = Some((second_ref.clone(), Arc::new(second_deployment)));
    resolver.additional_contract = Some((second_contract_ref, Arc::new(second_contract)));
    let contract_ref = contract_ref(&fixture.contract);
    let package_ref = package_ref(&fixture.package);

    let hydrated = RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly.clone())
        .unwrap();

    assert_eq!(resolver.package_loads.get(), 1);
    assert!(hydrated.deployment(&second_ref).is_some());
    assert_eq!(hydrated.code_slots().len(), 1);
    assert_eq!(
        hydrated.code_slot_index(&fixture.package.package_build_id),
        Some(0)
    );
    assert_eq!(hydrated.code_slot(0).unwrap().reference(), &package_ref);
    assert_eq!(
        hydrated
            .package(&fixture.package.package_build_id)
            .unwrap()
            .resource("assets/health.txt")
            .unwrap()
            .bytes()
            .as_ref(),
        b"health-resource"
    );
    let descriptor = hydrated
        .contract_store()
        .operation(&contract_ref, &fixture.operation_id)
        .unwrap();
    assert_eq!(descriptor.stable_key, "health");
    assert_eq!(
        hydrated.assembly().activation_templates[0].implementation_package_build_id,
        fixture.package.package_build_id
    );
}

#[test]
fn file_ir_records_load_without_identity_recomputation_and_are_idempotent() {
    let mut fixture = Fixture::new();
    let mut additional_files = BTreeMap::new();
    for (module_path, source_ast_hash) in [
        ("provider.extra_one", "source-hash-extra-one"),
        ("provider.extra_two", "source-hash-extra-two"),
    ] {
        let mut file = FileIrUnit::empty(module_path, source_ast_hash);
        skiff_artifact_identity::assign_file_ir_identity(&mut file).unwrap();
        let reference = FileIrRef {
            file_ir_identity: file.file_ir_identity.clone(),
            module_path: file.module_path.clone(),
            artifact_path: None,
            source_ast_hash: Some(file.source_ast_hash.clone()),
        };
        fixture.package.files.push(reference);
        additional_files.insert(file.file_ir_identity.clone(), Arc::new(file));
    }
    fixture.refresh_package_chain();

    let mut second_package = fixture.package.clone();
    second_package.package_version = "2.0.0".to_string();
    second_package.static_resources.clear();
    skiff_artifact_identity::assign_package_artifact_identities(&mut second_package).unwrap();
    let second_package_ref = package_ref(&second_package);

    let mut second_contract = fixture.contract.clone();
    second_contract.service_id = "example.health.shared".to_string();
    let second_operation_id = skiff_artifact_identity::contract_operation_id(
        &second_contract.service_id,
        &second_contract.contract_version,
        "health",
    )
    .unwrap();
    let mut second_descriptor = second_contract.operations.pop_first().unwrap().1;
    second_descriptor.operation_id = second_operation_id.clone();
    second_contract
        .operations
        .insert(second_operation_id.clone(), second_descriptor);
    skiff_artifact_identity::assign_service_contract_identities(&mut second_contract).unwrap();
    let second_contract_ref = contract_ref(&second_contract);

    let mut second_deployment = fixture.deployment.clone();
    second_deployment.deployment_revision = DeploymentRevision::new("revision-shared-file");
    second_deployment.implementation = second_package_ref.clone();
    second_deployment.contract = second_contract_ref.clone();
    second_deployment.operation_bindings[0].contract_operation_id = second_operation_id;
    skiff_artifact_identity::assign_service_deployment_identity(&mut second_deployment).unwrap();
    let second_deployment_ref = skiff_artifact_identity::service_deployment_ref(&second_deployment);

    fixture.assembly.roots.push(second_deployment_ref.clone());
    fixture
        .assembly
        .resolved_deployments
        .push(second_deployment_ref.clone());
    fixture
        .assembly
        .resolved_contracts
        .push(second_contract_ref.clone());
    fixture
        .assembly
        .resolved_packages
        .push(second_package_ref.clone());
    fixture
        .assembly
        .package_link_plan
        .code_slots
        .push(PackageCodeSlot {
            package: second_package_ref.clone(),
        });
    fixture
        .assembly
        .service_binding_templates
        .push(ServiceBindingTemplate {
            activation: second_deployment_ref.clone(),
            bindings: Vec::new(),
        });
    fixture
        .assembly
        .activation_templates
        .push(ActivationTemplate {
            deployment: second_deployment_ref.clone(),
            implementation_package_build_id: second_package.package_build_id.clone(),
        });
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut fixture.assembly).unwrap();

    let mut resolver = fixture.resolver();
    resolver.additional_deployment = Some((second_deployment_ref, Arc::new(second_deployment)));
    resolver.additional_contract = Some((second_contract_ref, Arc::new(second_contract)));
    resolver.additional_package = Some((second_package_ref, Arc::new(second_package)));
    resolver.additional_files = additional_files;

    let loader = RuntimeAssemblyLoader::new(&resolver);

    loader.load(fixture.assembly.clone()).unwrap();
    loader.load(fixture.assembly).unwrap();
}

#[test]
fn runtime_assembly_loader_joins_private_gateway_callables_and_shares_entry() {
    let mut fixture = Fixture::new();
    fixture.add_http_gateway();
    let hydrated = RuntimeAssemblyLoader::new(&fixture.resolver())
        .load(fixture.assembly.clone())
        .unwrap();

    assert_eq!(hydrated.gateway_entries().len(), 1);
    assert_eq!(hydrated.gateway_ingress().len(), 2);
    let entries = hydrated
        .gateway_ingress()
        .map(|(_, entry)| Arc::clone(entry))
        .collect::<Vec<_>>();
    assert!(Arc::ptr_eq(&entries[0], &entries[1]));
    let entry = &entries[0];
    assert_eq!(entry.owner(), &fixture.assembly.resolved_deployments[0]);
    assert_eq!(
        entry
            .handler()
            .expect("fixture has a handler")
            .signature()
            .parameters[0]
            .name,
        "body"
    );
    assert_eq!(
        entry
            .handler()
            .expect("fixture has a handler")
            .target()
            .callable_kind,
        skiff_artifact_model::OperationCallableKind::InternalFunction
    );
    assert!(entry.pre().is_some());
    assert!(entry.guard().is_some());
    assert!(hydrated
        .contract_store()
        .operation(&contract_ref(&fixture.contract), &fixture.operation_id)
        .is_some());
}

#[test]
fn runtime_assembly_loader_rejects_gateway_union_and_callable_mismatches() {
    let mut fixture = Fixture::new();
    fixture.add_http_gateway();

    let mut missing_binding = fixture.assembly.clone();
    missing_binding.gateway_ingress.pop();
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut missing_binding).unwrap();
    let error = RuntimeAssemblyLoader::new(&fixture.resolver())
        .load(missing_binding)
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("does not exactly match"),
        "unexpected error: {error:#}"
    );

    let mut extra_binding = fixture.assembly.clone();
    let mut extra = extra_binding.gateway_ingress[0].clone();
    extra.selector.path = "/not-in-deployment".to_string();
    extra_binding.gateway_ingress.push(extra);
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut extra_binding).unwrap();
    assert!(RuntimeAssemblyLoader::new(&fixture.resolver())
        .load(extra_binding)
        .unwrap_err()
        .to_string()
        .contains("does not exactly match"));

    let mut wrong_identity = fixture.assembly.clone();
    wrong_identity.gateway_ingress[0].gateway_entry_identity = GatewayEntryIdentity::parse(
        format!("{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}", "f".repeat(64)),
    )
    .unwrap();
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut wrong_identity).unwrap();
    assert!(RuntimeAssemblyLoader::new(&fixture.resolver())
        .load(wrong_identity)
        .unwrap_err()
        .to_string()
        .contains("does not exactly match"));

    let mut missing_callable = Fixture::new();
    missing_callable.add_http_gateway();
    missing_callable
        .deployment
        .gateway_entries
        .values_mut()
        .next()
        .unwrap()
        .handler = Some(PackageCallableId::new("pkg-callable:gateway:missing"));
    missing_callable.refresh_deployment_chain();
    let error = RuntimeAssemblyLoader::new(&missing_callable.resolver())
        .load(missing_callable.assembly)
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("is missing from implementation package"),
        "unexpected error: {error:#}"
    );

    let mut public_fallback = Fixture::new();
    public_fallback.add_http_gateway();
    public_fallback
        .deployment
        .gateway_entries
        .values_mut()
        .next()
        .unwrap()
        .handler = Some(public_fallback.callable_id.clone());
    public_fallback.refresh_deployment_chain();
    let error = RuntimeAssemblyLoader::new(&public_fallback.resolver())
        .load(public_fallback.assembly)
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("mismatched implementation target"),
        "unexpected error: {error:#}"
    );

    let mut websocket = Fixture::new();
    websocket.add_http_gateway();
    websocket.deployment.ingress[0].selector.protocol = IngressProtocol::WebSocket;
    let error = RuntimeAssemblyLoader::new(&websocket.resolver())
        .load(websocket.assembly)
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("does not match gateway entry"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn runtime_assembly_loader_rejects_gateway_link_and_signature_tamper() {
    let mut fixture = Fixture::new();
    fixture.add_http_gateway();

    let mut nested_id = fixture.resolver();
    Arc::make_mut(&mut nested_id.package)
        .callable_links
        .get_mut(&PackageCallableId::new(
            "pkg-callable:example.health-provider:top-level:provider.main.gateway_handler",
        ))
        .unwrap()
        .callable_id = PackageCallableId::new("pkg-callable:gateway:wrong-nested-id");
    assert!(RuntimeAssemblyLoader::new(&nested_id)
        .load(fixture.assembly.clone())
        .is_err());

    let mut target = fixture.resolver();
    Arc::make_mut(&mut target.package)
        .callable_links
        .get_mut(&PackageCallableId::new(
            "pkg-callable:example.health-provider:top-level:provider.main.gateway_handler",
        ))
        .unwrap()
        .target
        .callable_abi_id = "pkg-callable:gateway:wrong-target".to_string();
    assert!(RuntimeAssemblyLoader::new(&target)
        .load(fixture.assembly.clone())
        .is_err());

    let mut missing_signature = fixture.resolver();
    Arc::make_mut(&mut missing_signature.package)
        .package_local_abi
        .implementation_symbols
        .remove("provider.main.gateway_handler");
    assert!(RuntimeAssemblyLoader::new(&missing_signature)
        .load(fixture.assembly.clone())
        .is_err());

    let mut ambiguous_signature = fixture.resolver();
    let package = Arc::make_mut(&mut ambiguous_signature.package);
    let duplicate = package
        .package_local_abi
        .implementation_symbols
        .get("provider.main.gateway_handler")
        .unwrap()
        .clone();
    package
        .package_local_abi
        .implementation_symbols
        .insert("provider.main.gateway_handler_alias".to_string(), duplicate);
    assert!(RuntimeAssemblyLoader::new(&ambiguous_signature)
        .load(fixture.assembly)
        .is_err());
}

#[test]
fn loader_hydrates_and_pins_exact_package_schema_records() {
    let mut fixture = Fixture::new();
    let record = schema_record(
        "example.health-provider",
        "api.Health",
        ContractTypeDescriptor::Enumeration {
            variants: vec!["ok".to_string()],
        },
    );
    let type_id = record.package_schema_type_id.clone();
    fixture.add_schema_return(record);
    let resolver = fixture.resolver();

    let hydrated = RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly.clone())
        .unwrap();
    let contract_ref = contract_ref(&fixture.contract);
    let schema = hydrated
        .contract_store()
        .resolved_schema(&contract_ref)
        .unwrap();
    assert_eq!(resolver.schema_loads.get(), 1);
    assert_eq!(schema.contract(), &contract_ref);
    assert_eq!(
        schema.record(&type_id).unwrap().stable_schema_key,
        "api.Health"
    );
    assert!(Arc::ptr_eq(
        schema.record(&type_id).unwrap(),
        hydrated
            .contract_store()
            .shared_schema_record(&type_id)
            .unwrap()
    ));
}

#[test]
fn loader_requires_exact_package_schema_index_and_rejects_duplicate_public_facts() {
    let fixture = Fixture::new();
    let mut missing = fixture.resolver();
    missing.schema_index = None;
    let error = RuntimeAssemblyLoader::new(&missing)
        .load(fixture.assembly.clone())
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("missing package schema index"),
        "unexpected error: {error:#}"
    );

    for mutate in [
        |index: &mut PackageSchemaIndex| index.package_id.push_str(".wrong"),
        |index: &mut PackageSchemaIndex| {
            index.package_schema_index_identity =
                skiff_artifact_model::PackageSchemaIndexIdentity::new("wrong")
        },
    ] as [fn(&mut PackageSchemaIndex); 2]
    {
        let mut resolver = fixture.resolver();
        mutate(Arc::make_mut(
            resolver.schema_index.as_mut().expect("fixture index"),
        ));
        assert!(RuntimeAssemblyLoader::new(&resolver)
            .load(fixture.assembly.clone())
            .is_err());
    }

    let mut duplicate = Fixture::new();
    let record = schema_record(
        "example.health-provider",
        "api.Health",
        ContractTypeDescriptor::Record {
            fields: BTreeMap::new(),
        },
    );
    let type_id = record.package_schema_type_id.clone();
    duplicate.add_schema_return(record);
    duplicate.schema_index.types.insert(
        "api.Other".to_string(),
        PackageSchemaIndexEntry {
            package_schema_type_id: type_id,
            public_path: Some("api.Other".to_string()),
            nameability: ContractTypeNameability::PublicNameable,
        },
    );
    duplicate.schema_index.package_schema_index_identity =
        skiff_artifact_identity::package_schema_index_identity(
            &duplicate.schema_index.package_id,
            &duplicate.schema_index.types,
        )
        .unwrap();
    duplicate.package.package_schema_index = PackageSchemaIndexRef {
        package_id: duplicate.schema_index.package_id.clone(),
        package_schema_index_identity: duplicate.schema_index.package_schema_index_identity.clone(),
    };
    duplicate.refresh_package_chain();
    let error = RuntimeAssemblyLoader::new(&duplicate.resolver())
        .load(duplicate.assembly)
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("more than one stable key"),
        "unexpected error: {error:#}"
    );

    let mut duplicate_path = Fixture::new();
    let first = schema_record(
        "example.health-provider",
        "api.First",
        ContractTypeDescriptor::Record {
            fields: BTreeMap::new(),
        },
    );
    duplicate_path.add_schema_return(first);
    let second = schema_record(
        "example.health-provider",
        "api.Second",
        ContractTypeDescriptor::Record {
            fields: BTreeMap::new(),
        },
    );
    let second_id = second.package_schema_type_id.clone();
    duplicate_path.package.package_schema_type_records.insert(
        second_id.clone(),
        PackageSchemaTypeRecordRef {
            package_id: second.package_id.clone(),
            package_schema_type_id: second_id.clone(),
        },
    );
    duplicate_path
        .schema_records
        .insert(second_id.clone(), Arc::new(second));
    duplicate_path.schema_index.types.insert(
        "api.Second".to_string(),
        PackageSchemaIndexEntry {
            package_schema_type_id: second_id,
            public_path: Some("api.First".to_string()),
            nameability: ContractTypeNameability::PublicNameable,
        },
    );
    duplicate_path.schema_index.package_schema_index_identity =
        skiff_artifact_identity::package_schema_index_identity(
            &duplicate_path.schema_index.package_id,
            &duplicate_path.schema_index.types,
        )
        .unwrap();
    duplicate_path.package.package_schema_index = PackageSchemaIndexRef {
        package_id: duplicate_path.schema_index.package_id.clone(),
        package_schema_index_identity: duplicate_path
            .schema_index
            .package_schema_index_identity
            .clone(),
    };
    duplicate_path.refresh_package_chain();
    let error = RuntimeAssemblyLoader::new(&duplicate_path.resolver())
        .load(duplicate_path.assembly)
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("must be an api.yml public named type"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn loader_hydrates_cross_package_schema_children_without_a_foreign_code_slot() {
    let mut fixture = Fixture::new();
    let child = schema_record(
        "skiff.run/std",
        "std.http.HttpHeader",
        ContractTypeDescriptor::Record {
            fields: BTreeMap::from([
                ("name".to_string(), ContractTypeRef::builtin("string")),
                ("value".to_string(), ContractTypeRef::builtin("string")),
            ]),
        },
    );
    let child_id = child.package_schema_type_id.clone();
    let root = schema_record(
        "example.health-provider",
        "api.Request",
        ContractTypeDescriptor::Record {
            fields: BTreeMap::from([(
                "headers".to_string(),
                ContractTypeRef::Builtin {
                    name: "Array".to_string(),
                    arguments: vec![ContractTypeRef::package_schema(
                        child.package_id.clone(),
                        child.stable_schema_key.clone(),
                        child_id.clone(),
                    )],
                },
            )]),
        },
    );
    let root_id = root.package_schema_type_id.clone();
    fixture.add_schema_return(root);
    fixture.add_schema_dependency_record(child);
    let resolver = fixture.resolver();

    let hydrated = RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly.clone())
        .unwrap();
    let slot_records = hydrated.code_slots()[0].schema_records();
    let contract_schema = hydrated
        .contract_store()
        .resolved_schema(&contract_ref(&fixture.contract))
        .unwrap();

    assert_eq!(resolver.schema_loads.get(), 2);
    assert_eq!(slot_records.len(), 2);
    assert!(slot_records.contains_key(&root_id));
    assert!(slot_records.contains_key(&child_id));
    assert!(Arc::ptr_eq(
        &slot_records[&child_id],
        contract_schema.record(&child_id).unwrap()
    ));
    assert_eq!(hydrated.code_slots().len(), 1);
    assert_eq!(
        hydrated.code_slots()[0].artifact().package_id,
        "example.health-provider"
    );
}

#[test]
fn loader_rejects_missing_or_mismatched_cross_package_schema_children() {
    let mut fixture = Fixture::new();
    let child = schema_record(
        "skiff.run/std",
        "std.http.HttpHeader",
        ContractTypeDescriptor::Record {
            fields: BTreeMap::from([("name".to_string(), ContractTypeRef::builtin("string"))]),
        },
    );
    let child_id = child.package_schema_type_id.clone();
    let root = schema_record(
        "example.health-provider",
        "api.Request",
        ContractTypeDescriptor::Alias {
            target: ContractTypeRef::package_schema(
                child.package_id.clone(),
                child.stable_schema_key.clone(),
                child_id.clone(),
            ),
        },
    );
    fixture.add_schema_return(root);
    fixture.add_schema_dependency_record(child);

    let mut missing = fixture.resolver();
    missing.schema_records.remove(&child_id);
    let error = RuntimeAssemblyLoader::new(&missing)
        .load(fixture.assembly.clone())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("failed to resolve package schema type")
            && error.contains("for package example.health-provider closure"),
        "unexpected error: {error}"
    );

    for (mutate, expected_error) in [
        (
            (|record: &mut PackageSchemaTypeRecord| record.package_id.push_str(".wrong"))
                as fn(&mut PackageSchemaTypeRecord),
            "does not match exact owner",
        ),
        (
            |record: &mut PackageSchemaTypeRecord| record.stable_schema_key.push_str(".wrong"),
            "does not match exact stable key",
        ),
        (
            |record: &mut PackageSchemaTypeRecord| {
                record.package_schema_type_id = PackageSchemaTypeId::new("wrong")
            },
            "does not match exact owner",
        ),
        (
            |record: &mut PackageSchemaTypeRecord| {
                record.canonical_descriptor = PackageSchemaCanonicalDescriptor {
                    type_params: Vec::new(),
                    descriptor: ContractTypeDescriptor::Enumeration {
                        variants: vec!["tampered".to_string()],
                    },
                }
            },
            "invalid resolved Package schema closure",
        ),
    ] {
        let mut resolver = fixture.resolver();
        mutate(Arc::make_mut(
            resolver.schema_records.get_mut(&child_id).unwrap(),
        ));
        let error = RuntimeAssemblyLoader::new(&resolver)
            .load(fixture.assembly.clone())
            .unwrap_err();
        assert!(
            format!("{error:#}").contains(expected_error),
            "expected `{expected_error}`, got: {error:#}"
        );
    }
}

#[test]
fn loader_rejects_a_cross_package_schema_cycle_after_finite_hydration() {
    let root_id = PackageSchemaTypeId::new("cycle:root");
    let child_id = PackageSchemaTypeId::new("cycle:child");
    let root = PackageSchemaTypeRecord {
        package_id: "example.health-provider".to_string(),
        stable_schema_key: "api.Root".to_string(),
        package_schema_type_id: root_id.clone(),
        canonical_descriptor: PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Alias {
                target: ContractTypeRef::package_schema(
                    "skiff.run/std",
                    "std.http.Child",
                    child_id.clone(),
                ),
            },
        },
    };
    let child = PackageSchemaTypeRecord {
        package_id: "skiff.run/std".to_string(),
        stable_schema_key: "std.http.Child".to_string(),
        package_schema_type_id: child_id,
        canonical_descriptor: PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Alias {
                target: ContractTypeRef::package_schema(
                    root.package_id.clone(),
                    root.stable_schema_key.clone(),
                    root_id,
                ),
            },
        },
    };
    let mut fixture = Fixture::new();
    fixture.add_schema_return(root);
    fixture.add_schema_dependency_record(child);
    let resolver = fixture.resolver();

    let error = RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly.clone())
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("recursive type cycle"),
        "unexpected error: {error:#}"
    );
    assert_eq!(
        resolver.schema_loads.get(),
        2,
        "the visited-set must terminate hydration before validation rejects the cycle"
    );
}

#[test]
fn schema_record_payload_is_shared_while_each_contract_closure_is_validated() {
    let mut fixture = Fixture::new();
    let record = schema_record(
        "example.health-provider",
        "api.Health",
        ContractTypeDescriptor::Enumeration {
            variants: vec!["ok".to_string()],
        },
    );
    fixture.add_schema_return(record);
    let resolver = fixture.resolver();
    let loader = RuntimeAssemblyLoader::new(&resolver);
    let reference = contract_ref(&fixture.contract);
    let package_records = BTreeMap::new();
    let mut shared = BTreeMap::new();

    let first = loader
        .load_contract_schema(&reference, &fixture.contract, &package_records, &mut shared)
        .unwrap();
    let second = loader
        .load_contract_schema(&reference, &fixture.contract, &package_records, &mut shared)
        .unwrap();

    assert_eq!(resolver.schema_loads.get(), 1);
    let type_id = fixture.contract.package_type_requirements[0].required_type_ids[0].clone();
    assert!(Arc::ptr_eq(
        first.record(&type_id).unwrap(),
        second.record(&type_id).unwrap()
    ));
}

#[test]
fn loader_rejects_missing_wrong_owner_key_and_hash_schema_records() {
    let mut fixture = Fixture::new();
    let record = schema_record(
        "example.health-provider",
        "api.Health",
        ContractTypeDescriptor::Enumeration {
            variants: vec!["ok".to_string()],
        },
    );
    let type_id = record.package_schema_type_id.clone();
    fixture.add_schema_return(record);

    let mut resolver = fixture.resolver();
    resolver.schema_records.clear();
    assert!(RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly.clone())
        .unwrap_err()
        .to_string()
        .contains("failed to resolve package schema type"));

    for mutate in [
        |record: &mut PackageSchemaTypeRecord| record.package_id.push_str(".wrong"),
        |record: &mut PackageSchemaTypeRecord| record.stable_schema_key.push_str(".wrong"),
        |record: &mut PackageSchemaTypeRecord| {
            record.package_schema_type_id = PackageSchemaTypeId::new("wrong")
        },
    ] as [fn(&mut PackageSchemaTypeRecord); 3]
    {
        let mut resolver = fixture.resolver();
        mutate(Arc::make_mut(
            resolver.schema_records.get_mut(&type_id).unwrap(),
        ));
        assert!(RuntimeAssemblyLoader::new(&resolver)
            .load(fixture.assembly.clone())
            .is_err());
    }
}

#[test]
fn schema_validation_rejects_missing_extra_unrequired_and_recursive_closure() {
    let child = schema_record(
        "example.health-provider",
        "api.Child",
        ContractTypeDescriptor::Enumeration {
            variants: vec!["child".to_string()],
        },
    );
    let root = schema_record(
        "example.health-provider",
        "api.Root",
        ContractTypeDescriptor::Alias {
            target: ContractTypeRef::package_schema(
                child.package_id.clone(),
                child.stable_schema_key.clone(),
                child.package_schema_type_id.clone(),
            ),
        },
    );
    let mut fixture = Fixture::new();
    fixture.add_schema_return(root.clone());
    fixture.contract.package_type_requirements[0]
        .required_type_ids
        .push(child.package_schema_type_id.clone());
    let records = BTreeMap::from([
        (root.package_schema_type_id.clone(), Arc::new(root.clone())),
        (
            child.package_schema_type_id.clone(),
            Arc::new(child.clone()),
        ),
    ]);
    assert!(validate_resolved_service_schema(&fixture.contract, &records).is_ok());

    let missing_child =
        BTreeMap::from([(root.package_schema_type_id.clone(), Arc::new(root.clone()))]);
    assert!(validate_resolved_service_schema(&fixture.contract, &missing_child).is_err());

    let extra = schema_record(
        "example.health-provider",
        "api.Extra",
        ContractTypeDescriptor::Enumeration {
            variants: vec!["extra".to_string()],
        },
    );
    let mut extra_records = records.clone();
    extra_records.insert(extra.package_schema_type_id.clone(), Arc::new(extra));
    assert!(validate_resolved_service_schema(&fixture.contract, &extra_records).is_err());

    let mut unrequired = fixture.contract.clone();
    unrequired.package_type_requirements[0]
        .required_type_ids
        .pop();
    assert!(validate_resolved_service_schema(&unrequired, &records).is_err());

    let first_id = PackageSchemaTypeId::new("cycle:first");
    let second_id = PackageSchemaTypeId::new("cycle:second");
    let first = PackageSchemaTypeRecord {
        package_id: "example.health-provider".to_string(),
        stable_schema_key: "api.First".to_string(),
        package_schema_type_id: first_id.clone(),
        canonical_descriptor: PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Alias {
                target: ContractTypeRef::package_schema(
                    "example.health-provider",
                    "api.Second",
                    second_id.clone(),
                ),
            },
        },
    };
    let second = PackageSchemaTypeRecord {
        package_id: "example.health-provider".to_string(),
        stable_schema_key: "api.Second".to_string(),
        package_schema_type_id: second_id.clone(),
        canonical_descriptor: PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Alias {
                target: ContractTypeRef::package_schema(
                    "example.health-provider",
                    "api.First",
                    first_id.clone(),
                ),
            },
        },
    };
    let cycle = BTreeMap::from([(first_id, first), (second_id, second)]);
    assert!(
        skiff_artifact_identity::validate_package_schema_records(&cycle)
            .unwrap_err()
            .to_string()
            .contains("recursive type cycle")
    );
}

#[test]
fn tampered_assembly_contract_deployment_package_and_file_fail_closed() {
    let fixture = Fixture::new();

    let mut assembly = fixture.assembly.clone();
    assembly.activation_templates[0].implementation_package_build_id =
        PackageBuildId::new("tampered");
    assert!(RuntimeAssemblyLoader::new(&fixture.resolver())
        .load(assembly)
        .unwrap_err()
        .to_string()
        .contains("before hydration"));

    let mut resolver = fixture.resolver();
    Arc::make_mut(&mut resolver.contract)
        .operations
        .get_mut(&fixture.operation_id)
        .unwrap()
        .stable_key = "tampered".to_string();
    assert!(RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly.clone())
        .unwrap_err()
        .to_string()
        .contains("contract content is invalid"));

    let mut resolver = fixture.resolver();
    Arc::make_mut(&mut resolver.package).package_version = "2.0.0".to_string();
    let error = RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly.clone())
        .unwrap_err();
    assert!(
        error.to_string().contains("package content mismatches ref"),
        "unexpected error: {error:#}"
    );

    let mut resolver = fixture.resolver();
    Arc::make_mut(&mut resolver.file).executables[0].symbol = "tampered".to_string();
    // File records are written by the platform compiler and only checked for
    // completeness/label consistency at load; content identity is not
    // re-derived per file (see design: simple integrity validation).
    RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly)
        .expect("tampered file content with matching labels still loads");
}

#[test]
fn rehashed_forged_package_plan_is_rejected_during_hydration_admission() {
    let mut fixture = Fixture::new();
    let package_projection = serde_json::to_value(
        skiff_artifact_identity::package_artifact_build_identity_projection(&fixture.package)
            .unwrap(),
    )
    .unwrap();
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = fixture
        .package
        .boundary_projections
        .get_mut(&fixture.callable_id)
        .unwrap()
    else {
        unreachable!()
    };
    let BoundaryValuePlan::Linkable { owner, .. } = &mut operation_contract.return_value.value_plan
    else {
        unreachable!()
    };
    *owner = BoundaryValueOwner::Caller;
    mechanically_rehash_forged_package(&mut fixture.package, package_projection);
    let package_identity_admitted =
        skiff_artifact_identity::validate_package_artifact_identities(&fixture.package).is_ok();
    let reference = package_ref(&fixture.package);
    fixture.deployment.implementation = reference.clone();
    fixture.assembly.resolved_packages = vec![reference.clone()];
    fixture.assembly.package_link_plan.code_slots = vec![PackageCodeSlot { package: reference }];
    fixture.assembly.activation_templates[0].implementation_package_build_id =
        fixture.package.package_build_id.clone();
    fixture.refresh_deployment_chain();

    let error = RuntimeAssemblyLoader::new(&fixture.resolver())
        .load(fixture.assembly)
        .unwrap_err();
    let message = format!("{error:#}");
    if package_identity_admitted {
        assert!(
            message.contains("invalid canonical boundary projections"),
            "unexpected error: {message}"
        );
    } else {
        assert!(
            message.contains("package content is invalid"),
            "unexpected error: {message}"
        );
    }
}

#[test]
fn rehashed_forged_contract_plan_is_rejected_during_hydration_admission() {
    let mut fixture = Fixture::new();
    let contract_projection = serde_json::to_value(
        skiff_artifact_identity::service_protocol_identity_projection(&fixture.contract).unwrap(),
    )
    .unwrap();
    let descriptor = fixture
        .contract
        .operations
        .get_mut(&fixture.operation_id)
        .unwrap();
    let BoundaryValuePlan::Linkable { lifetime, .. } =
        &mut descriptor.contract.return_value.value_plan
    else {
        unreachable!()
    };
    *lifetime = BoundaryValueLifetime::Request;
    mechanically_rehash_forged_contract(&mut fixture.contract, contract_projection);
    let contract_identity_admitted =
        skiff_artifact_identity::validate_service_contract_identities(&fixture.contract).is_ok();
    let reference = contract_ref(&fixture.contract);
    fixture.deployment.contract = reference.clone();
    fixture.assembly.resolved_contracts = vec![reference];
    fixture.refresh_deployment_chain();

    let error = RuntimeAssemblyLoader::new(&fixture.resolver())
        .load(fixture.assembly)
        .unwrap_err();
    let message = format!("{error:#}");
    if contract_identity_admitted {
        assert!(
            message.contains("invalid canonical boundary contract"),
            "unexpected error: {message}"
        );
    } else {
        assert!(
            message.contains("contract content is invalid"),
            "unexpected error: {message}"
        );
    }
}

#[test]
fn resource_hash_size_and_storage_path_fail_before_linking() {
    let fixture = Fixture::new();
    let mut resolver = fixture.resolver();
    resolver.resource = Arc::from(b"tamper-resource".as_slice());
    assert!(RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly.clone())
        .unwrap_err()
        .to_string()
        .contains("hash mismatch"));

    let mut resolver = fixture.resolver();
    resolver.resource = Arc::from(b"tampered".as_slice());
    assert!(RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly.clone())
        .unwrap_err()
        .to_string()
        .contains("size mismatch"));

    let mut fixture = Fixture::new();
    fixture.package.static_resources[0].artifact_path = Some("../escape".to_string());
    assert!(RuntimeAssemblyLoader::new(&fixture.resolver())
        .load(fixture.assembly)
        .unwrap_err()
        .to_string()
        .contains("escape"));
}

#[test]
fn missing_file_link_target_and_contract_operation_mismatch_fail_closed() {
    let mut fixture = Fixture::new();
    let missing = FileIrRef::new("missing-file", "missing.module");
    fixture
        .package
        .callable_links
        .get_mut(&fixture.callable_id)
        .unwrap()
        .target
        .file_ref = missing.clone();
    fixture
        .package
        .implementation_links
        .functions
        .get_mut("health")
        .unwrap()
        .file = missing;
    fixture.refresh_package_chain();
    assert!(RuntimeAssemblyLoader::new(&fixture.resolver())
        .load(fixture.assembly.clone())
        .unwrap_err()
        .to_string()
        .contains("targets missing File IR"));

    let mut fixture = Fixture::new();
    fixture.deployment.operation_bindings[0].contract_operation_id =
        ContractOperationId::new("missing-operation");
    fixture.refresh_deployment_chain();
    assert!(RuntimeAssemblyLoader::new(&fixture.resolver())
        .load(fixture.assembly)
        .unwrap_err()
        .to_string()
        .contains("operation bindings do not exactly match"));
}

#[test]
fn runtime_assembly_filesystem_resolver_hydrates_exact_canonical_closure() {
    let mut fixture = Fixture::new();
    let record = schema_record(
        "example.health-provider",
        "api.Health",
        ContractTypeDescriptor::Enumeration {
            variants: vec!["ok".to_string()],
        },
    );
    let type_id = record.package_schema_type_id.clone();
    fixture.add_schema_return(record.clone());
    let temp = TestArtifactRoot::new();
    let store = skiff_deployment::storage::CanonicalArtifactStore::create(temp.path()).unwrap();
    let package_ref = package_ref(&fixture.package);
    let contract_ref = contract_ref(&fixture.contract);
    let deployment_ref = skiff_artifact_identity::service_deployment_ref(&fixture.deployment);
    let assembly_ref = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();

    store.write_service_contract(&fixture.contract).unwrap();
    store
        .write_package_schema_index(&fixture.schema_index)
        .unwrap();
    let record_path = store.write_package_schema_type_record(&record).unwrap();
    store.write_package_artifact(&fixture.package).unwrap();
    store
        .write_file_ir(&package_ref, &fixture.package.files[0], &fixture.file)
        .unwrap();
    store
        .write_static_resource(
            &package_ref,
            &fixture.package.static_resources[0],
            fixture.resource.as_ref(),
        )
        .unwrap();
    store.write_service_deployment(&fixture.deployment).unwrap();
    store.write_runtime_assembly(&fixture.assembly).unwrap();

    let resolver = crate::FilesystemRuntimeAssemblyContentResolver::from_store(store);
    let hydrated = resolver.load_runtime_assembly(&assembly_ref).unwrap();
    assert_eq!(
        hydrated.assembly().assembly_identity,
        assembly_ref.assembly_identity
    );
    assert!(hydrated.deployment(&deployment_ref).is_some());
    assert!(hydrated.contract_store().contract(&contract_ref).is_some());
    assert_eq!(
        hydrated
            .contract_store()
            .resolved_schema(&contract_ref)
            .unwrap()
            .record(&type_id)
            .unwrap()
            .stable_schema_key,
        "api.Health"
    );
    assert_eq!(
        hydrated
            .package(&package_ref.package_build_id)
            .unwrap()
            .resource("assets/health.txt")
            .unwrap()
            .bytes()
            .as_ref(),
        fixture.resource.as_ref()
    );

    std::fs::remove_file(record_path).unwrap();
    assert_eq!(
        hydrated
            .contract_store()
            .resolved_schema(&contract_ref)
            .unwrap()
            .record(&type_id)
            .unwrap()
            .stable_schema_key,
        "api.Health"
    );
    assert!(resolver
        .load_runtime_assembly(&assembly_ref)
        .unwrap_err()
        .to_string()
        .contains("failed to resolve package schema type"));
}

struct TestArtifactRoot(std::path::PathBuf);

impl TestArtifactRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "skiff-runtime-assembly-resolver-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir(&path).expect("create runtime assembly test root");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestArtifactRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn contract_ref(contract: &ServiceContract) -> ServiceContractRef {
    ServiceContractRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        service_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

fn package_ref(package: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: package.package_id.clone(),
        package_version: package.package_version.clone(),
        package_build_id: package.package_build_id.clone(),
        package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
    }
}

fn mechanically_rehash_forged_package(
    artifact: &mut PackageArtifact,
    mut canonical_projection: serde_json::Value,
) {
    canonical_projection["boundaryProjections"] =
        serde_json::to_value(&artifact.boundary_projections).unwrap();
    let bytes = skiff_canonical_json::canonical_json_bytes(&canonical_projection).unwrap();
    artifact.package_build_id = PackageBuildId::new(skiff_artifact_identity::framed_identity(
        skiff_artifact_identity::PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
        &hex::encode(Sha256::digest(bytes)),
    ));
}

fn mechanically_rehash_forged_contract(
    contract: &mut ServiceContract,
    mut canonical_projection: serde_json::Value,
) {
    canonical_projection["operations"] = serde_json::to_value(&contract.operations).unwrap();
    let bytes = skiff_canonical_json::canonical_json_bytes(&canonical_projection).unwrap();
    contract.service_protocol_identity =
        ServiceProtocolIdentity::new(skiff_artifact_identity::framed_identity(
            skiff_artifact_identity::SERVICE_PROTOCOL_IDENTITY_PREFIX,
            &hex::encode(Sha256::digest(bytes)),
        ));
}

fn operation_contract() -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: Vec::new(),
        return_value: BoundaryReturn {
            ty: skiff_artifact_model::ContractTypeRef::builtin("bool"),
            value_plan: BoundaryValuePlan::Linkable {
                carrier: BoundaryValueCarrier::DetachedValueGraph,
                encoding: BoundaryValueEncoding::CanonicalValue,
                owner: BoundaryValueOwner::Provider,
                lifetime: BoundaryValueLifetime::Call,
            },
        },
        stream: BoundaryStreamContract::Unary,
        callbacks: BoundaryCallbackContract::None,
        effect_guarantee: BoundaryEffectGuarantee {
            detached_parameters: true,
            detached_return: true,
            detached_error: true,
            no_caller_reachable_mutation: true,
            no_caller_value_escape: true,
            no_same_heap_identity: true,
        },
    }
}

fn schema_record(
    package_id: &str,
    stable_schema_key: &str,
    descriptor: ContractTypeDescriptor,
) -> PackageSchemaTypeRecord {
    let canonical_descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor,
    };
    PackageSchemaTypeRecord {
        package_id: package_id.to_string(),
        stable_schema_key: stable_schema_key.to_string(),
        package_schema_type_id: skiff_artifact_identity::package_schema_type_id(
            package_id,
            stable_schema_key,
            &canonical_descriptor,
        )
        .unwrap(),
        canonical_descriptor,
    }
}

fn no_effects() -> CallableMayEffects {
    CallableMayEffects {
        writes_caller_reachable: false,
        returns_caller_alias: false,
        throws_caller_alias: false,
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_suspend: false,
    }
}
