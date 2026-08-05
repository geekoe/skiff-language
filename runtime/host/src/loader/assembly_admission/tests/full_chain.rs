use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use skiff_artifact_model::*;
use skiff_deployment::{
    assembly::resolve_runtime_assembly, projection::project_service_deployment,
};

use crate::loader::config_snapshot::snapshot_for_assembly as config_snapshot_for_assembly;

use super::super::*;

mod db_index_provisioning;

struct CountingResolver {
    assembly: Arc<RuntimeAssembly>,
    deployments: Vec<(ServiceDeploymentRef, Arc<ServiceDeployment>)>,
    contracts: Vec<(ServiceContractRef, Arc<ServiceContract>)>,
    packages: Vec<(PackageArtifactRef, Arc<PackageArtifact>)>,
    files: Vec<(PackageArtifactRef, FileIrRef, Arc<FileIrUnit>)>,
    reads: AtomicUsize,
}

impl RuntimeAssemblyRecordResolver for CountingResolver {
    fn resolve_runtime_assembly(
        &self,
        _reference: &RuntimeAssemblyRef,
    ) -> anyhow::Result<Arc<RuntimeAssembly>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::clone(&self.assembly))
    }
}

impl RuntimeAssemblyContentResolver for CountingResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.deployments
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, deployment)| Arc::clone(deployment))
            .ok_or_else(|| anyhow::anyhow!("missing deployment"))
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.contracts
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, contract)| Arc::clone(contract))
            .ok_or_else(|| anyhow::anyhow!("missing contract"))
    }

    fn resolve_package_schema_index(
        &self,
        reference: &PackageSchemaIndexRef,
    ) -> anyhow::Result<Arc<PackageSchemaIndex>> {
        let package = self
            .packages
            .iter()
            .find(|(_, package)| package.package_schema_index == *reference)
            .map(|(_, package)| package)
            .ok_or_else(|| anyhow::anyhow!("missing package schema index"))?;
        if !package.package_schema_type_records.is_empty() {
            anyhow::bail!("full-chain fixture only supports an empty package schema index");
        }
        let types = BTreeMap::new();
        let identity =
            skiff_artifact_identity::package_schema_index_identity(&reference.package_id, &types)?;
        if identity != reference.package_schema_index_identity {
            anyhow::bail!("package schema index identity mismatch");
        }
        Ok(Arc::new(PackageSchemaIndex {
            package_id: reference.package_id.clone(),
            package_schema_index_identity: identity,
            types,
        }))
    }

    fn resolve_package_schema_type(
        &self,
        reference: &skiff_artifact_model::PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::PackageSchemaTypeRecord>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("missing package schema record {reference:?}")
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.packages
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, package)| Arc::clone(package))
            .ok_or_else(|| anyhow::anyhow!("missing package"))
    }

    fn resolve_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.files
            .iter()
            .find(|(candidate_package, candidate_file, _)| {
                candidate_package == package && candidate_file == reference
            })
            .map(|(_, _, file)| Arc::clone(file))
            .ok_or_else(|| anyhow::anyhow!("missing File IR"))
    }

    fn resolve_static_resource(
        &self,
        _package: &PackageArtifactRef,
        _reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("fixture has no static resources")
    }
}

struct FullChainFixture {
    assembly: RuntimeAssembly,
    resolver: CountingResolver,
    provider_contract_ref: ServiceContractRef,
    provider_contract: Arc<ServiceContract>,
    provider_operation_id: ContractOperationId,
    provider_callable_id: PackageCallableId,
    provider_deployment_ref: ServiceDeploymentRef,
    provider_package_ref: PackageArtifactRef,
    consumer_deployment_ref: ServiceDeploymentRef,
    consumer_package_ref: PackageArtifactRef,
    consumer_file_ir_identity: String,
}

impl FullChainFixture {
    fn new() -> Self {
        Self::with_consumer_collection(None)
    }

    fn with_consumer_collection(root_collection: Option<&str>) -> Self {
        Self::build(root_collection, None)
    }

    fn with_consumer_config(requirement: PackageConfigRequirement) -> Self {
        Self::build(None, Some(requirement))
    }

    fn build(
        root_collection: Option<&str>,
        consumer_config: Option<PackageConfigRequirement>,
    ) -> Self {
        let operation_contract = operation_contract();
        let (provider_contract, provider_operation_id) = service_contract(
            "example.phase-three.provider",
            "health",
            "Phase three provider",
            operation_contract.clone(),
        );
        let provider_contract_ref = contract_ref(&provider_contract);
        let (consumer_contract, consumer_operation_id) = service_contract(
            "example.phase-three.consumer",
            "check",
            "Phase three consumer",
            operation_contract.clone(),
        );
        let consumer_contract_ref = contract_ref(&consumer_contract);

        let provider_callable_id =
            PackageCallableId::new("pkg-callable:example.phase-three-provider:health");
        let provider_file = implementation_file("provider.main", "health", None);
        let provider_file_ref = file_ref(&provider_file);
        let provider_package = implementation_package(
            "example.phase-three-provider",
            "health",
            provider_callable_id.clone(),
            &provider_file,
            operation_contract.clone(),
            None,
        );
        let provider_package_ref = package_ref(&provider_package);

        let service_requirement_slot = 7;
        let provider_call = ServiceCallRef {
            service_requirement_slot,
            contract_operation_id: provider_operation_id.clone(),
            expected_protocol_identity: provider_contract_ref.service_protocol_identity.clone(),
        };
        let provider_requirement = ContractRequirement {
            alias: "provider".to_string(),
            service_id: provider_contract_ref.service_id.clone(),
            contract_version: provider_contract_ref.contract_version.clone(),
            expected_protocol_identity: provider_contract_ref.service_protocol_identity.clone(),
        };
        let consumer_callable_id =
            PackageCallableId::new("pkg-callable:example.phase-three-consumer:check");
        let mut consumer_file =
            implementation_file("consumer.main", "check", Some(provider_call.clone()));
        if let Some(collection_name) = root_collection {
            insert_db_collection(&mut consumer_file, "ServiceSecret", collection_name);
        }
        let consumer_file_ref = file_ref(&consumer_file);
        let consumer_file_ir_identity = consumer_file_ref.file_ir_identity.clone();
        let mut consumer_package = implementation_package(
            "example.phase-three-consumer",
            "check",
            consumer_callable_id.clone(),
            &consumer_file,
            operation_contract,
            Some((provider_requirement, provider_call)),
        );
        if let Some(requirement) = consumer_config {
            let boundary_requirement = match &requirement.access {
                PackageConfigAccess::Presence => None,
                PackageConfigAccess::Optional { value_type } => Some(BoundaryConfigRequirement {
                    path: requirement.path.clone(),
                    value_type: value_type.clone(),
                    required: false,
                }),
                PackageConfigAccess::Required { value_type } => Some(BoundaryConfigRequirement {
                    path: requirement.path.clone(),
                    value_type: value_type.clone(),
                    required: true,
                }),
            };
            consumer_package.runtime_requirements.config = vec![requirement];
            let BoundaryCallableProjection::Available {
                implementation_requirements,
                ..
            } = consumer_package
                .boundary_projections
                .get_mut(&consumer_callable_id)
                .expect("consumer callable projection")
            else {
                unreachable!("consumer fixture projection must be available")
            };
            implementation_requirements.config = boundary_requirement.into_iter().collect();
            skiff_artifact_identity::assign_package_artifact_identities(&mut consumer_package)
                .unwrap();
        }
        let consumer_package_ref = package_ref(&consumer_package);

        let provider_deployment = project_service_deployment(
            ServiceDeploymentInput {
                schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
                contract: provider_contract_ref.clone(),
                deployment_revision: DeploymentRevision::new("provider-revision-1"),
                implementation: provider_package_ref.clone(),
                operation_bindings: vec![ServiceDeploymentOperationInput {
                    contract_operation_id: provider_operation_id.clone(),
                    package_callable_id: provider_callable_id.clone(),
                }],
                package_bindings: Vec::new(),
                service_selectors: Vec::new(),
                gateway_entries: BTreeMap::new(),
                ingress: Vec::new(),
                diagnostic_text: DeploymentDiagnosticText {
                    display_name: "Phase three provider deployment".to_string(),
                    notes: BTreeMap::new(),
                },
            },
            &provider_contract,
            std::slice::from_ref(&provider_package),
            &BTreeMap::new(),
        )
        .unwrap();
        let provider_deployment_ref =
            skiff_artifact_identity::service_deployment_ref(&provider_deployment);
        let consumer_deployment = project_service_deployment(
            ServiceDeploymentInput {
                schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
                contract: consumer_contract_ref.clone(),
                deployment_revision: DeploymentRevision::new("consumer-revision-1"),
                implementation: consumer_package_ref.clone(),
                operation_bindings: vec![ServiceDeploymentOperationInput {
                    contract_operation_id: consumer_operation_id.clone(),
                    package_callable_id: consumer_callable_id,
                }],
                package_bindings: Vec::new(),
                service_selectors: vec![ServiceSelectorBinding {
                    key: ServiceRequirementKey {
                        caller_package_build_id: consumer_package_ref.package_build_id.clone(),
                        service_requirement_slot,
                    },
                    contract: provider_contract_ref.clone(),
                }],
                gateway_entries: BTreeMap::new(),
                ingress: Vec::new(),
                diagnostic_text: DeploymentDiagnosticText {
                    display_name: "Phase three consumer deployment".to_string(),
                    notes: BTreeMap::new(),
                },
            },
            &consumer_contract,
            std::slice::from_ref(&consumer_package),
            &BTreeMap::new(),
        )
        .unwrap();
        let consumer_deployment_ref =
            skiff_artifact_identity::service_deployment_ref(&consumer_deployment);
        let assembly = resolve_runtime_assembly(
            std::slice::from_ref(&consumer_deployment_ref),
            &[consumer_deployment.clone(), provider_deployment.clone()],
            &[consumer_contract.clone(), provider_contract.clone()],
            &[consumer_package.clone(), provider_package.clone()],
        )
        .unwrap();
        let provider_contract = Arc::new(provider_contract);
        let resolver = CountingResolver {
            assembly: Arc::new(assembly.clone()),
            deployments: vec![
                (
                    consumer_deployment_ref.clone(),
                    Arc::new(consumer_deployment),
                ),
                (
                    provider_deployment_ref.clone(),
                    Arc::new(provider_deployment),
                ),
            ],
            contracts: vec![
                (consumer_contract_ref, Arc::new(consumer_contract)),
                (
                    provider_contract_ref.clone(),
                    Arc::clone(&provider_contract),
                ),
            ],
            packages: vec![
                (consumer_package_ref.clone(), Arc::new(consumer_package)),
                (provider_package_ref.clone(), Arc::new(provider_package)),
            ],
            files: vec![
                (
                    consumer_package_ref.clone(),
                    consumer_file_ref,
                    Arc::new(consumer_file),
                ),
                (
                    provider_package_ref.clone(),
                    provider_file_ref,
                    Arc::new(provider_file),
                ),
            ],
            reads: AtomicUsize::new(0),
        };
        Self {
            assembly,
            resolver,
            provider_contract_ref,
            provider_contract,
            provider_operation_id,
            provider_callable_id,
            provider_deployment_ref,
            provider_package_ref,
            consumer_deployment_ref,
            consumer_package_ref,
            consumer_file_ir_identity,
        }
    }
}

struct CollectionIdentityFixture {
    assembly: RuntimeAssembly,
    resolver: CountingResolver,
}

impl CollectionIdentityFixture {
    fn new(mapping: BTreeMap<String, String>, root_collection: Option<&str>) -> Self {
        Self::build_collection_identity(mapping, root_collection, false, None)
    }

    fn with_stateful_diamond(
        direct_mapping: BTreeMap<String, String>,
        transitive_mapping: BTreeMap<String, String>,
    ) -> Self {
        Self::build_collection_identity(direct_mapping, None, false, Some(transitive_mapping))
    }

    fn with_dependency_target_collision() -> Self {
        Self::build_collection_identity(
            BTreeMap::from([(
                "package_secret".to_string(),
                "mapped_package_secret".to_string(),
            )]),
            None,
            true,
            None,
        )
    }

    fn build_collection_identity(
        _mapping: BTreeMap<String, String>,
        root_collection: Option<&str>,
        include_colliding_dependency: bool,
        diamond_mapping: Option<BTreeMap<String, String>>,
    ) -> Self {
        let base = FullChainFixture::with_consumer_collection(root_collection);
        let consumer_deployment = base
            .resolver
            .deployments
            .iter()
            .find(|(reference, _)| reference == &base.consumer_deployment_ref)
            .map(|(_, deployment)| deployment.as_ref().clone())
            .expect("consumer deployment");
        let consumer_contract_ref = consumer_deployment.contract.clone();
        let consumer_contract = base
            .resolver
            .contracts
            .iter()
            .find(|(reference, _)| reference == &consumer_contract_ref)
            .map(|(_, contract)| contract.as_ref().clone())
            .expect("consumer contract");
        let provider_deployment = base
            .resolver
            .deployments
            .iter()
            .find(|(reference, _)| reference == &base.provider_deployment_ref)
            .map(|(_, deployment)| deployment.as_ref().clone())
            .expect("provider deployment");
        let provider_package = base
            .resolver
            .packages
            .iter()
            .find(|(reference, _)| reference.package_id == "example.phase-three-provider")
            .map(|(_, package)| package.as_ref().clone())
            .expect("provider package");
        let provider_file = base
            .resolver
            .files
            .iter()
            .find(|(reference, _, _)| reference.package_id == "example.phase-three-provider")
            .map(|(_, _, file)| file.as_ref().clone())
            .expect("provider file");

        let mut consumer_package = base
            .resolver
            .packages
            .iter()
            .find(|(reference, _)| reference == &base.consumer_package_ref)
            .map(|(_, package)| package.as_ref().clone())
            .expect("consumer package");
        let consumer_file = base
            .resolver
            .files
            .iter()
            .find(|(reference, _, _)| reference == &base.consumer_package_ref)
            .map(|(_, _, file)| file.as_ref().clone())
            .expect("consumer file");
        let mut dependency_file = implementation_file("mapping.store", "noop", None);
        insert_db_collection(&mut dependency_file, "PackageSecret", "package_secret");
        insert_db_collection(&mut dependency_file, "PackageAudit", "package_audit");
        let dependency_callable = PackageCallableId::new("pkg-callable:example.mapping-store:noop");
        let mut dependency_package = implementation_package(
            "example.mapping-store",
            "noop",
            dependency_callable,
            &dependency_file,
            operation_contract(),
            None,
        );
        skiff_artifact_identity::assign_package_artifact_identities(&mut dependency_package)
            .unwrap();
        let dependency_ref = package_ref(&dependency_package);

        let diamond_subject = diamond_mapping.as_ref().map(|_| {
            let file = implementation_file("mapping.subject", "noop", None);
            let callable = PackageCallableId::new("pkg-callable:example.mapping-subject:noop");
            let mut package = implementation_package(
                "example.mapping-subject",
                "noop",
                callable,
                &file,
                operation_contract(),
                None,
            );
            package.package_requirements.push(PackageRequirement {
                alias: "store".to_string(),
                package_id: dependency_package.package_id.clone(),
                exact_version: dependency_package.package_version.clone(),
                expected_local_abi: dependency_package
                    .package_local_abi
                    .local_abi_identity
                    .clone(),
                expected_package_build: Some(dependency_package.package_build_id.clone()),
            });
            skiff_artifact_identity::assign_package_artifact_identities(&mut package).unwrap();
            (file, package)
        });

        let colliding_dependency = include_colliding_dependency.then(|| {
            let mut file = implementation_file("mapping.cache", "noop", None);
            insert_db_collection(&mut file, "CacheSecret", "cache_secret");
            let callable = PackageCallableId::new("pkg-callable:example.mapping-cache:noop");
            let mut package = implementation_package(
                "example.mapping-cache",
                "noop",
                callable,
                &file,
                operation_contract(),
                None,
            );
            skiff_artifact_identity::assign_package_artifact_identities(&mut package).unwrap();
            (file, package)
        });

        consumer_package
            .package_requirements
            .push(PackageRequirement {
                alias: "store".to_string(),
                package_id: dependency_package.package_id.clone(),
                exact_version: dependency_package.package_version.clone(),
                expected_local_abi: dependency_package
                    .package_local_abi
                    .local_abi_identity
                    .clone(),
                expected_package_build: None,
            });
        if let Some((_, package)) = &diamond_subject {
            consumer_package
                .package_requirements
                .push(PackageRequirement {
                    alias: "subject".to_string(),
                    package_id: package.package_id.clone(),
                    exact_version: package.package_version.clone(),
                    expected_local_abi: package.package_local_abi.local_abi_identity.clone(),
                    expected_package_build: Some(package.package_build_id.clone()),
                });
        }
        if let Some((_, package)) = &colliding_dependency {
            consumer_package
                .package_requirements
                .push(PackageRequirement {
                    alias: "cache".to_string(),
                    package_id: package.package_id.clone(),
                    exact_version: package.package_version.clone(),
                    expected_local_abi: package.package_local_abi.local_abi_identity.clone(),
                    expected_package_build: None,
                });
        }
        skiff_artifact_identity::assign_package_artifact_identities(&mut consumer_package).unwrap();
        let consumer_package_ref = package_ref(&consumer_package);

        let mut consumer_deployment = consumer_deployment;
        consumer_deployment.implementation = consumer_package_ref.clone();
        for selector in &mut consumer_deployment.service_selectors {
            selector.key.caller_package_build_id = consumer_package_ref.package_build_id.clone();
        }
        let mut package_bindings = vec![PackageBinding {
            key: PackageRequirementKey {
                caller_package_build_id: consumer_package_ref.package_build_id.clone(),
                package_requirement_alias: "store".to_string(),
            },
            package: dependency_ref.clone(),
        }];
        if let Some((_, package)) = &diamond_subject {
            package_bindings.extend([
                PackageBinding {
                    key: PackageRequirementKey {
                        caller_package_build_id: consumer_package_ref.package_build_id.clone(),
                        package_requirement_alias: "subject".to_string(),
                    },
                    package: package_ref(package),
                },
                PackageBinding {
                    key: PackageRequirementKey {
                        caller_package_build_id: package.package_build_id.clone(),
                        package_requirement_alias: "store".to_string(),
                    },
                    package: dependency_ref.clone(),
                },
            ]);
        }
        if let Some((_, package)) = &colliding_dependency {
            package_bindings.push(PackageBinding {
                key: PackageRequirementKey {
                    caller_package_build_id: consumer_package_ref.package_build_id.clone(),
                    package_requirement_alias: "cache".to_string(),
                },
                package: package_ref(package),
            });
        }
        consumer_deployment.package_bindings = package_bindings;
        skiff_artifact_identity::assign_service_deployment_identity(&mut consumer_deployment)
            .unwrap();
        let consumer_deployment_ref =
            skiff_artifact_identity::service_deployment_ref(&consumer_deployment);
        let provider_deployment_ref =
            skiff_artifact_identity::service_deployment_ref(&provider_deployment);
        let provider_contract = base.provider_contract.as_ref().clone();
        let provider_contract_ref = contract_ref(&provider_contract);
        let provider_package_ref = package_ref(&provider_package);
        let consumer_file_ref = file_ref(&consumer_file);
        let provider_file_ref = file_ref(&provider_file);
        let dependency_file_ref = file_ref(&dependency_file);

        let mut packages = vec![
            consumer_package.clone(),
            provider_package.clone(),
            dependency_package.clone(),
        ];
        if let Some((_, package)) = &colliding_dependency {
            packages.push(package.clone());
        }
        if let Some((_, package)) = &diamond_subject {
            packages.push(package.clone());
        }
        let assembly = resolve_runtime_assembly(
            std::slice::from_ref(&consumer_deployment_ref),
            &[consumer_deployment.clone(), provider_deployment.clone()],
            &[consumer_contract.clone(), provider_contract.clone()],
            &packages,
        )
        .unwrap();
        let mut resolver_packages = vec![
            (consumer_package_ref.clone(), Arc::new(consumer_package)),
            (provider_package_ref.clone(), Arc::new(provider_package)),
            (dependency_ref.clone(), Arc::new(dependency_package)),
        ];
        let mut resolver_files = vec![
            (
                consumer_package_ref,
                consumer_file_ref,
                Arc::new(consumer_file),
            ),
            (
                provider_package_ref,
                provider_file_ref,
                Arc::new(provider_file),
            ),
            (
                dependency_ref,
                dependency_file_ref,
                Arc::new(dependency_file),
            ),
        ];
        if let Some((file, package)) = colliding_dependency {
            let reference = package_ref(&package);
            resolver_packages.push((reference.clone(), Arc::new(package)));
            resolver_files.push((reference, file_ref(&file), Arc::new(file)));
        }
        if let Some((file, package)) = diamond_subject {
            let reference = package_ref(&package);
            resolver_packages.push((reference.clone(), Arc::new(package)));
            resolver_files.push((reference, file_ref(&file), Arc::new(file)));
        }
        let resolver = CountingResolver {
            assembly: Arc::new(assembly.clone()),
            deployments: vec![
                (consumer_deployment_ref, Arc::new(consumer_deployment)),
                (provider_deployment_ref, Arc::new(provider_deployment)),
            ],
            contracts: vec![
                (consumer_contract_ref, Arc::new(consumer_contract)),
                (provider_contract_ref, Arc::new(provider_contract)),
            ],
            packages: resolver_packages,
            files: resolver_files,
            reads: AtomicUsize::new(0),
        };
        Self { assembly, resolver }
    }
}

fn insert_db_collection(file: &mut FileIrUnit, type_name: &str, collection_name: &str) {
    let fields = BTreeMap::from([
        ("id".to_string(), TypeRefIr::builtin("string")),
        ("value".to_string(), TypeRefIr::builtin("string")),
    ]);
    let type_index = file.type_table.len() as u32;
    file.type_table.push(TypeDeclIr {
        name: type_name.to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: fields.clone(),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.declarations.types.insert(
        type_name.to_string(),
        TypeDeclarationIr {
            type_index,
            symbol: format!("{}.{}", file.module_path, type_name),
            source_span: None,
        },
    );
    file.declarations.db.insert(
        type_name.to_string(),
        DbDeclarationIr {
            type_ref: TypeRefIr::LocalType { type_index },
            type_name: type_name.to_string(),
            collection_name: Some(collection_name.to_string()),
            implements: None,
            kind: DbObjectKindIr::Object,
            key: DbObjectKeyIr {
                name: "id".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
            fields: fields
                .into_iter()
                .map(|(name, ty)| DbObjectFieldIr {
                    name,
                    ty,
                    storage: DbFieldStorageIr::Identity,
                })
                .collect(),
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
    skiff_artifact_identity::assign_file_ir_identity(file).unwrap();
}

fn insert_db_contract(file: &mut FileIrUnit, type_name: &str) {
    let fields = BTreeMap::from([
        ("id".to_string(), TypeRefIr::builtin("string")),
        ("status".to_string(), TypeRefIr::builtin("string")),
    ]);
    let type_index = file.type_table.len() as u32;
    file.type_table.push(TypeDeclIr {
        name: type_name.to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: fields.clone(),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.declarations.types.insert(
        type_name.to_string(),
        TypeDeclarationIr {
            type_index,
            symbol: format!("{}.{}", file.module_path, type_name),
            source_span: None,
        },
    );
    file.declarations.db.insert(
        type_name.to_string(),
        DbDeclarationIr {
            type_ref: TypeRefIr::LocalType { type_index },
            type_name: type_name.to_string(),
            collection_name: None,
            implements: None,
            kind: DbObjectKindIr::Contract,
            key: DbObjectKeyIr {
                name: "id".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
            fields: fields
                .into_iter()
                .map(|(name, ty)| DbObjectFieldIr {
                    name,
                    ty,
                    storage: DbFieldStorageIr::Identity,
                })
                .collect(),
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
    skiff_artifact_identity::assign_file_ir_identity(file).unwrap();
}

#[derive(Clone, Default)]
struct CapturingDbProvider {
    inputs: Arc<Mutex<Vec<skiff_runtime_capability_context::DbProviderBuildInput>>>,
}

impl skiff_runtime_capability_context::DbProviderFactory for CapturingDbProvider {
    fn build(
        &self,
        input: skiff_runtime_capability_context::DbProviderBuildInput,
    ) -> skiff_runtime_capability_context::DbCapabilityResult<
        skiff_runtime_capability_context::DbCapabilitySource,
    > {
        self.inputs.lock().unwrap().push(input);
        Ok(skiff_runtime_capability_context::DbCapabilitySource::unavailable())
    }
}

fn mapping_service_db() -> AssemblyActivationServiceDb {
    AssemblyActivationServiceDb {
        mongo_url: "mongodb://fixture.invalid".to_string(),
    }
}

fn fixture_config_snapshot(
    fixture: &FullChainFixture,
    values: BTreeMap<(ServiceDeploymentRef, PackageBuildId), BTreeMap<String, serde_json::Value>>,
) -> (
    RuntimeConfigSnapshotRef,
    crate::loader::config_snapshot::TestSnapshotResolver,
) {
    let reference = skiff_runtime_config_snapshot::new_runtime_config_snapshot_ref();
    let snapshot = fixture_config_snapshot_record(fixture, reference.clone(), values);
    (
        reference,
        crate::loader::config_snapshot::TestSnapshotResolver::new(snapshot),
    )
}

fn fixture_config_snapshot_record(
    fixture: &FullChainFixture,
    reference: RuntimeConfigSnapshotRef,
    values: BTreeMap<(ServiceDeploymentRef, PackageBuildId), BTreeMap<String, serde_json::Value>>,
) -> skiff_runtime_config_snapshot::RuntimeConfigSnapshot {
    let deployments = fixture
        .assembly
        .resolved_deployments
        .iter()
        .map(|deployment_ref| {
            let deployment = fixture
                .resolver
                .resolve_deployment(deployment_ref)
                .expect("fixture deployment should resolve");
            let mut packages = BTreeSet::from([deployment.implementation.package_build_id.clone()]);
            packages.extend(
                deployment
                    .package_bindings
                    .iter()
                    .map(|binding| binding.package.package_build_id.clone()),
            );
            skiff_runtime_config_snapshot::RuntimeConfigDeployment::new(
                deployment_ref.clone(),
                packages
                    .into_iter()
                    .map(|package_build_id| {
                        let config = values
                            .get(&(deployment_ref.clone(), package_build_id.clone()))
                            .cloned()
                            .unwrap_or_default();
                        skiff_runtime_config_snapshot::RuntimeConfigPackage::new(
                            package_build_id,
                            config,
                        )
                        .expect("fixture Package config should be valid")
                    })
                    .collect(),
            )
            .expect("fixture deployment config should be valid")
        })
        .collect();
    let snapshot = skiff_runtime_config_snapshot::RuntimeConfigSnapshot::new(
        "fixture",
        reference.clone(),
        deployments,
    )
    .expect("fixture config snapshot should be valid");
    snapshot
}

#[tokio::test]
async fn committed_recovery_nonempty_generation_survives_restart_with_exact_registration() {
    let fixture = FullChainFixture::new();
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let (config_snapshot, config_resolver) =
        config_snapshot_for_assembly("prod", &fixture.assembly, &fixture.resolver);

    let first = AssemblyAdmissionController::new(
        "runtime-a",
        skiff_runtime_capability_context::DbProviderSource::unavailable(),
    );
    let first_active = first
        .recover_committed(
            "prod",
            7,
            &reference,
            &config_snapshot,
            &fixture.resolver,
            &config_resolver,
            None,
        )
        .await
        .expect("non-empty committed generation must recover");
    let first_reads = fixture.resolver.reads.load(Ordering::SeqCst);
    assert_eq!(first_active.generation(), 7);
    assert!(!first_active.is_empty());
    assert!(first_reads > 1);
    assert!(matches!(
        first.registration().unwrap(),
        Some(AssemblyActivationControl::Register {
            generation: 7,
            assembly,
            replica_id,
            ..
        }) if assembly == reference && replica_id == "runtime-a"
    ));

    let restarted = AssemblyAdmissionController::new(
        "runtime-a",
        skiff_runtime_capability_context::DbProviderSource::unavailable(),
    );
    let restarted_active = restarted
        .recover_committed(
            "prod",
            7,
            &reference,
            &config_snapshot,
            &fixture.resolver,
            &config_resolver,
            None,
        )
        .await
        .expect("restart must rebuild the same non-empty committed generation");
    assert_eq!(restarted_active.generation(), 7);
    assert_eq!(restarted_active.identity(), &reference.assembly_identity);
    assert!(fixture.resolver.reads.load(Ordering::SeqCst) > first_reads);
    assert!(matches!(
        restarted.registration().unwrap(),
        Some(AssemblyActivationControl::Register {
            generation: 7,
            assembly,
            replica_id,
            ..
        }) if assembly == reference && replica_id == "runtime-a"
    ));
}

#[tokio::test]
async fn config_snapshot_requires_exact_deployment_and_package_partitions_before_activation() {
    let fixture = FullChainFixture::new();
    let assembly_ref = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();

    let missing_deployment_ref = skiff_runtime_config_snapshot::new_runtime_config_snapshot_ref();
    let exact =
        fixture_config_snapshot_record(&fixture, missing_deployment_ref.clone(), BTreeMap::new());
    let missing_deployment = skiff_runtime_config_snapshot::RuntimeConfigSnapshot::new(
        "fixture",
        missing_deployment_ref.clone(),
        exact
            .deployments()
            .iter()
            .filter(|partition| partition.deployment() != &fixture.consumer_deployment_ref)
            .cloned()
            .collect(),
    )
    .unwrap();

    let missing_package_ref = skiff_runtime_config_snapshot::new_runtime_config_snapshot_ref();
    let exact =
        fixture_config_snapshot_record(&fixture, missing_package_ref.clone(), BTreeMap::new());
    let missing_package = skiff_runtime_config_snapshot::RuntimeConfigSnapshot::new(
        "fixture",
        missing_package_ref.clone(),
        exact
            .deployments()
            .iter()
            .map(|partition| {
                if partition.deployment() == &fixture.consumer_deployment_ref {
                    skiff_runtime_config_snapshot::RuntimeConfigDeployment::new(
                        partition.deployment().clone(),
                        Vec::new(),
                    )
                    .unwrap()
                } else {
                    partition.clone()
                }
            })
            .collect(),
    )
    .unwrap();

    let extra_package_ref = skiff_runtime_config_snapshot::new_runtime_config_snapshot_ref();
    let exact =
        fixture_config_snapshot_record(&fixture, extra_package_ref.clone(), BTreeMap::new());
    let extra_package = skiff_runtime_config_snapshot::RuntimeConfigSnapshot::new(
        "fixture",
        extra_package_ref.clone(),
        exact
            .deployments()
            .iter()
            .map(|partition| {
                if partition.deployment() == &fixture.consumer_deployment_ref {
                    let mut packages = partition.packages().to_vec();
                    packages.push(
                        skiff_runtime_config_snapshot::RuntimeConfigPackage::new(
                            PackageBuildId::new("build:not-in-assembly"),
                            BTreeMap::new(),
                        )
                        .unwrap(),
                    );
                    packages.sort_by(|left, right| {
                        left.package_build_id().cmp(right.package_build_id())
                    });
                    skiff_runtime_config_snapshot::RuntimeConfigDeployment::new(
                        partition.deployment().clone(),
                        packages,
                    )
                    .unwrap()
                } else {
                    partition.clone()
                }
            })
            .collect(),
    )
    .unwrap();

    for (reference, snapshot) in [
        (missing_deployment_ref, missing_deployment),
        (missing_package_ref, missing_package),
        (extra_package_ref, extra_package),
    ] {
        let controller = AssemblyAdmissionController::default();
        let error = controller
            .recover_committed(
                "fixture",
                1,
                &assembly_ref,
                &reference,
                &fixture.resolver,
                &crate::loader::config_snapshot::TestSnapshotResolver::new(snapshot),
                None,
            )
            .await
            .expect_err("non-exact config snapshot must fail before activation");
        assert!(
            format!("{error:#}").contains("do not exactly match"),
            "unexpected error: {error:#}"
        );
        assert!(controller.active().unwrap().is_none());
    }
}

#[tokio::test]
async fn config_snapshot_validates_required_types_and_isolates_deployment_views() {
    let fixture = FullChainFixture::with_consumer_config(PackageConfigRequirement {
        path: "api.key".to_string(),
        access: PackageConfigAccess::Required {
            value_type: "string".to_string(),
        },
    });
    let assembly_ref = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();

    for value in [None, Some(serde_json::json!({"api": {"key": 42}}))] {
        let values = value
            .map(|value| {
                let serde_json::Value::Object(config) = value else {
                    unreachable!()
                };
                BTreeMap::from([(
                    (
                        fixture.consumer_deployment_ref.clone(),
                        fixture.consumer_package_ref.package_build_id.clone(),
                    ),
                    config.into_iter().collect(),
                )])
            })
            .unwrap_or_default();
        let (reference, resolver) = fixture_config_snapshot(&fixture, values);
        let controller = AssemblyAdmissionController::default();
        let error = controller
            .recover_committed(
                "fixture",
                1,
                &assembly_ref,
                &reference,
                &fixture.resolver,
                &resolver,
                None,
            )
            .await
            .expect_err("missing or mistyped required config must fail admission");
        let message = format!("{error:#}");
        assert!(
            message.contains("required value is missing") || message.contains("must be a string"),
            "unexpected error: {message}"
        );
        assert!(controller.active().unwrap().is_none());
    }

    let serde_json::Value::Object(config) = serde_json::json!({"api": {"key": "top-secret"}})
    else {
        unreachable!()
    };
    let values = BTreeMap::from([(
        (
            fixture.consumer_deployment_ref.clone(),
            fixture.consumer_package_ref.package_build_id.clone(),
        ),
        config.into_iter().collect(),
    )]);
    let (reference, resolver) = fixture_config_snapshot(&fixture, values);
    let controller = AssemblyAdmissionController::default();
    let active = controller
        .recover_committed(
            "fixture",
            1,
            &assembly_ref,
            &reference,
            &fixture.resolver,
            &resolver,
            None,
        )
        .await
        .expect("exact typed config snapshot should activate");
    let consumer = active
        .contexts()
        .config_views(&fixture.consumer_deployment_ref)
        .unwrap();
    let provider = active
        .contexts()
        .config_views(&fixture.provider_deployment_ref)
        .unwrap();
    assert_eq!(
        consumer.service().resolved_config_value()["api"]["key"],
        "top-secret"
    );
    assert_eq!(
        provider.service().resolved_config_value(),
        &serde_json::json!({})
    );
    assert!(!format!("{consumer:?}").contains("top-secret"));
}

#[tokio::test]
async fn provider_owner_rebinding_selects_provider_config_instead_of_caller_config() {
    let fixture = FullChainFixture::with_consumer_config(PackageConfigRequirement {
        path: "api.key".to_string(),
        access: PackageConfigAccess::Required {
            value_type: "string".to_string(),
        },
    });
    let values = BTreeMap::from([
        (
            (
                fixture.consumer_deployment_ref.clone(),
                fixture.consumer_package_ref.package_build_id.clone(),
            ),
            BTreeMap::from([("api".to_string(), serde_json::json!({"key": "caller-key"}))]),
        ),
        (
            (
                fixture.provider_deployment_ref.clone(),
                fixture.provider_package_ref.package_build_id.clone(),
            ),
            BTreeMap::from([(
                "api".to_string(),
                serde_json::json!({"key": "provider-key"}),
            )]),
        ),
    ]);
    let (reference, resolver) = fixture_config_snapshot(&fixture, values);
    let assembly_ref = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let controller = AssemblyAdmissionController::default();
    let active = controller
        .recover_committed(
            "fixture",
            1,
            &assembly_ref,
            &reference,
            &fixture.resolver,
            &resolver,
            None,
        )
        .await
        .expect("two-owner config snapshot should activate");
    let caller = active
        .contexts()
        .activation_for_deployment(&fixture.consumer_deployment_ref)
        .expect("caller activation");
    let caller_request = RequestActivationContext::begin(caller).expect("request generation");
    let eval = skiff_runtime_eval::RuntimeAssemblyEvalTarget::new(
        Arc::clone(active.candidate().execution_image()),
        caller_request,
        Arc::clone(active.contexts()) as Arc<dyn RuntimeAssemblyEvalResolver>,
    )
    .expect("caller eval target");
    let linked_call = active
        .candidate()
        .shared_image()
        .resolve_activation_relative_service_call(
            &fixture.consumer_package_ref.package_build_id,
            &fixture.consumer_file_ir_identity,
            ServiceCallRefIndex::new(0),
        )
        .expect("linked service call");
    let provider = eval
        .resolve_service_call(&linked_call)
        .expect("provider target");
    let provider_eval = eval
        .with_request_activation(provider.provider_request().clone())
        .expect("provider eval target");
    crate::eval_capability_adapter::provider_execution_facts_for_test(
        active.contexts(),
        active.candidate().execution_image(),
        &provider_eval,
        skiff_runtime_eval::program_execution::ActivationExecutionOperation::ServiceCall {
            operation_id: fixture.provider_operation_id.clone(),
        },
    )
    .expect("provider owner facts");

    let caller_config = active
        .contexts()
        .config_views(&fixture.consumer_deployment_ref)
        .expect("caller config");
    let provider_config = active
        .contexts()
        .config_views(&fixture.provider_deployment_ref)
        .expect("provider config");
    assert_eq!(
        caller_config.service().resolved_config_value()["api"]["key"],
        "caller-key"
    );
    assert_eq!(
        provider_config.service().resolved_config_value()["api"]["key"],
        "provider-key"
    );
}

#[tokio::test]
async fn projected_nonempty_assembly_admits_and_active_lookup_is_io_free() {
    let fixture = FullChainFixture::new();
    let controller = AssemblyAdmissionController::default();

    let active = controller
        .admit(fixture.assembly.clone(), &fixture.resolver)
        .await
        .expect("projected non-empty assembly should admit");

    assert_eq!(active.identity(), &fixture.assembly.assembly_identity);
    assert_eq!(active.candidate().shared_image().code_slots().len(), 2);
    assert_eq!(active.candidate().activations().len(), 2);
    let stored_contract = active
        .contract_store()
        .contract(&fixture.provider_contract_ref)
        .unwrap();
    assert!(Arc::ptr_eq(stored_contract, &fixture.provider_contract));
    assert_eq!(
        stored_contract.service_protocol_identity,
        fixture.provider_contract_ref.service_protocol_identity
    );
    let expected_descriptor = fixture
        .provider_contract
        .operations
        .get(&fixture.provider_operation_id)
        .unwrap();
    let linked_call = active
        .candidate()
        .shared_image()
        .resolve_activation_relative_service_call(
            &fixture.consumer_package_ref.package_build_id,
            &fixture.consumer_file_ir_identity,
            ServiceCallRefIndex::new(0),
        )
        .unwrap();
    assert_eq!(
        linked_call.caller_package_build_id(),
        &fixture.consumer_package_ref.package_build_id
    );
    assert_eq!(linked_call.service_requirement_slot(), 7);
    assert_eq!(linked_call.operation_id(), &fixture.provider_operation_id);
    assert_eq!(
        linked_call.expected_protocol_identity(),
        &fixture.provider_contract_ref.service_protocol_identity
    );
    let binding = active
        .candidate()
        .resolve_activation_relative_service_call(&fixture.consumer_deployment_ref, &linked_call)
        .unwrap();
    assert_eq!(
        &binding.key().caller_package_build_id,
        &fixture.consumer_package_ref.package_build_id
    );
    assert_eq!(binding.key().service_requirement_slot, 7);
    assert_eq!(binding.contract(), &fixture.provider_contract_ref);
    assert_eq!(binding.provider(), &fixture.provider_deployment_ref);
    let provider_operation = active
        .activation(binding.provider())
        .unwrap()
        .operation(linked_call.operation_id())
        .unwrap();
    assert_eq!(
        provider_operation.package_callable_id(),
        &fixture.provider_callable_id
    );
    let active_descriptor = active
        .operation_descriptor(binding.contract(), linked_call.operation_id())
        .unwrap();
    assert!(std::ptr::eq(active_descriptor, expected_descriptor));
    assert!(std::ptr::eq(
        active_descriptor,
        active
            .contract_store()
            .operation_descriptor(binding.contract(), linked_call.operation_id())
            .unwrap()
    ));
    assert_eq!(
        active_descriptor.contract.return_value.value_plan,
        expected_descriptor.contract.return_value.value_plan
    );
    assert!(matches!(
        &active_descriptor.contract.return_value.value_plan,
        BoundaryValuePlan::Linkable {
            owner: BoundaryValueOwner::Provider,
            ..
        }
    ));

    let reads_after_admit = fixture.resolver.reads.load(Ordering::SeqCst);
    assert!(reads_after_admit > 0);
    let binding_wire =
        serde_json::to_string(&active.candidate().assembly().service_binding_templates).unwrap();
    assert!(!binding_wire.contains("stableKey"));
    assert!(!binding_wire.contains("valuePlan"));
    assert_eq!(
        fixture.resolver.reads.load(Ordering::SeqCst),
        reads_after_admit,
        "active service binding, contract, provider, and route lookup must not trigger artifact I/O"
    );

    let mut tampered = fixture.assembly.clone();
    tampered.assembly_identity = AssemblyIdentity::new("tampered-candidate");
    assert!(controller.admit(tampered, &fixture.resolver).await.is_err());
    assert!(Arc::ptr_eq(&active, &controller.active().unwrap().unwrap()));
    assert_eq!(
        fixture.resolver.reads.load(Ordering::SeqCst),
        reads_after_admit,
        "failed reload must fail before content I/O and preserve active"
    );
}

#[tokio::test]
async fn logical_collection_identity_reaches_db_provider_exactly_and_survives_reload() {
    let fixture = CollectionIdentityFixture::new(BTreeMap::new(), None);
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let (config_snapshot, config_resolver) =
        config_snapshot_for_assembly("fixture", &fixture.assembly, &fixture.resolver);
    let provider = CapturingDbProvider::default();
    let controller = AssemblyAdmissionController::new(
        "runtime-mapping",
        skiff_runtime_capability_context::DbProviderSource::new(provider.clone()),
    );
    let service_db = mapping_service_db();

    controller
        .recover_committed(
            "fixture",
            7,
            &reference,
            &config_snapshot,
            &fixture.resolver,
            &config_resolver,
            Some(&service_db),
        )
        .await
        .expect("Package-owned collection fixture must admit");
    controller
        .recover_committed(
            "fixture",
            7,
            &reference,
            &config_snapshot,
            &fixture.resolver,
            &config_resolver,
            Some(&service_db),
        )
        .await
        .expect("reload must rebuild the exact Package-owned metadata");

    let inputs = provider.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0].environment, inputs[1].environment);
    assert_eq!(inputs[0].service_id, inputs[1].service_id);
    assert_eq!(inputs[0].runtime_program_db, inputs[1].runtime_program_db);
    let mut collections = inputs[0]
        .runtime_program_db
        .iter()
        .map(|metadata| {
            assert_eq!(metadata.metadata.source_role, "package");
            assert_eq!(
                metadata.metadata.package_id.as_deref(),
                Some("example.mapping-store")
            );
            assert_eq!(
                metadata.target.target_id.package_artifact_ref.package_id,
                "example.mapping-store"
            );
            metadata
                .metadata
                .collection_name
                .clone()
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    collections.sort();
    assert_eq!(
        collections,
        vec!["package_audit".to_string(), "package_secret".to_string()]
    );
}

#[tokio::test]
async fn db_contract_declarations_do_not_become_provider_collections() {
    let base = FullChainFixture::with_consumer_collection(Some("service_secret"));
    let mut consumer_package = base
        .resolver
        .packages
        .iter()
        .find(|(reference, _)| reference.package_id == "example.phase-three-consumer")
        .map(|(_, package)| package.as_ref().clone())
        .expect("consumer package");
    let mut consumer_file = base
        .resolver
        .files
        .iter()
        .find(|(reference, _, _)| reference.package_id == "example.phase-three-consumer")
        .map(|(_, _, file)| file.as_ref().clone())
        .expect("consumer file");
    insert_db_contract(&mut consumer_file, "AgentThread");
    skiff_artifact_identity::assign_file_ir_identity(&mut consumer_file).unwrap();
    skiff_artifact_identity::assign_package_artifact_identities(&mut consumer_package).unwrap();
    let consumer_package_ref = package_ref(&consumer_package);
    let consumer_file_ref = file_ref(&consumer_file);

    let mut consumer_deployment = base
        .resolver
        .deployments
        .iter()
        .find(|(reference, _)| reference == &base.consumer_deployment_ref)
        .map(|(_, deployment)| deployment.as_ref().clone())
        .expect("consumer deployment");
    consumer_deployment.implementation = consumer_package_ref.clone();
    for selector in &mut consumer_deployment.service_selectors {
        selector.key.caller_package_build_id = consumer_package_ref.package_build_id.clone();
    }
    consumer_deployment.package_bindings = vec![PackageBinding {
        key: PackageRequirementKey {
            caller_package_build_id: consumer_package_ref.package_build_id.clone(),
            package_requirement_alias: "provider".to_string(),
        },
        package: base.provider_package_ref.clone(),
    }];
    skiff_artifact_identity::assign_service_deployment_identity(&mut consumer_deployment).unwrap();
    let consumer_deployment_ref =
        skiff_artifact_identity::service_deployment_ref(&consumer_deployment);

    let provider_deployment = base
        .resolver
        .deployments
        .iter()
        .find(|(reference, _)| reference == &base.provider_deployment_ref)
        .map(|(_, deployment)| deployment.as_ref().clone())
        .expect("provider deployment");
    let provider_contract = base
        .resolver
        .contracts
        .iter()
        .find(|(reference, _)| reference == &base.provider_contract_ref)
        .map(|(_, contract)| contract.as_ref().clone())
        .expect("provider contract");
    let consumer_contract = base
        .resolver
        .contracts
        .iter()
        .find(|(_, contract)| contract.service_id == "example.phase-three.consumer")
        .map(|(_, contract)| contract.as_ref().clone())
        .expect("consumer contract");
    let provider_deployment_ref =
        skiff_artifact_identity::service_deployment_ref(&provider_deployment);

    let mut packages = base
        .resolver
        .packages
        .iter()
        .map(|(_, package)| package.as_ref().clone())
        .collect::<Vec<_>>();
    packages.retain(|package| package.package_id != "example.phase-three-consumer");
    packages.push(consumer_package.clone());
    let mut files = base
        .resolver
        .files
        .iter()
        .map(|(_, _, file)| file.as_ref().clone())
        .collect::<Vec<_>>();
    files.retain(|file| file.module_path != consumer_file.module_path);
    files.push(consumer_file.clone());
    let assembly = resolve_runtime_assembly(
        std::slice::from_ref(&consumer_deployment_ref),
        &[consumer_deployment.clone(), provider_deployment.clone()],
        &[consumer_contract.clone(), provider_contract.clone()],
        &packages,
    )
    .unwrap();
    let resolver = CountingResolver {
        assembly: Arc::new(assembly.clone()),
        deployments: vec![
            (consumer_deployment_ref, Arc::new(consumer_deployment)),
            (provider_deployment_ref, Arc::new(provider_deployment)),
        ],
        contracts: vec![
            (
                skiff_artifact_identity::service_contract_ref(&consumer_contract).unwrap(),
                Arc::new(consumer_contract),
            ),
            (
                skiff_artifact_identity::service_contract_ref(&provider_contract).unwrap(),
                Arc::new(provider_contract),
            ),
        ],
        packages: vec![
            (consumer_package_ref, Arc::new(consumer_package)),
            (
                base.provider_package_ref.clone(),
                base.resolver
                    .packages
                    .iter()
                    .find(|(reference, _)| reference.package_id == "example.phase-three-provider")
                    .map(|(_, package)| Arc::clone(package))
                    .expect("provider package"),
            ),
        ],
        files: vec![
            (
                consumer_file_ref,
                file_ref(&consumer_file),
                Arc::new(consumer_file),
            ),
            (
                base.provider_package_ref,
                base.resolver
                    .files
                    .iter()
                    .find(|(reference, _, _)| {
                        reference.package_id == "example.phase-three-provider"
                    })
                    .map(|(_, file_ref, _)| file_ref.clone())
                    .expect("provider file ref"),
                base.resolver
                    .files
                    .iter()
                    .find(|(reference, _, _)| {
                        reference.package_id == "example.phase-three-provider"
                    })
                    .map(|(_, _, file)| Arc::clone(file))
                    .expect("provider file"),
            ),
        ],
        reads: AtomicUsize::new(0),
    };

    let reference = skiff_artifact_identity::runtime_assembly_ref(&assembly).unwrap();
    let (config_snapshot, config_resolver) =
        config_snapshot_for_assembly("fixture", &assembly, &resolver);
    let provider = CapturingDbProvider::default();
    let controller = AssemblyAdmissionController::new(
        "runtime-contract-exclusion",
        skiff_runtime_capability_context::DbProviderSource::new(provider.clone()),
    );
    controller
        .recover_committed(
            "fixture",
            7,
            &reference,
            &config_snapshot,
            &resolver,
            &config_resolver,
            Some(&mapping_service_db()),
        )
        .await
        .expect("assembly with a db contract declaration must admit");

    let inputs = provider.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 1);
    let mut collections = inputs[0]
        .runtime_program_db
        .iter()
        .map(|metadata| {
            (
                metadata.metadata.type_name.clone(),
                metadata
                    .metadata
                    .collection_name
                    .as_deref()
                    .unwrap_or("")
                    .to_string(),
                metadata.metadata.kind,
            )
        })
        .collect::<Vec<_>>();
    collections.sort();
    assert_eq!(
        collections,
        vec![(
            "ServiceSecret".to_string(),
            "service_secret".to_string(),
            DbObjectKindIr::Object
        )]
    );
    assert!(
        inputs[0]
            .runtime_program_db
            .iter()
            .all(|entry| entry.metadata.kind != DbObjectKindIr::Contract),
        "db contract declarations must not reach the DB provider"
    );
}

#[tokio::test]
async fn reachable_db_metadata_requires_router_supplied_service_db() {
    let fixture = CollectionIdentityFixture::new(BTreeMap::new(), None);
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let (config_snapshot, config_resolver) =
        config_snapshot_for_assembly("fixture", &fixture.assembly, &fixture.resolver);
    let controller = AssemblyAdmissionController::new(
        "runtime-missing-service-db",
        skiff_runtime_capability_context::DbProviderSource::new(CapturingDbProvider::default()),
    );

    let error = controller
        .recover_committed(
            "fixture",
            7,
            &reference,
            &config_snapshot,
            &fixture.resolver,
            &config_resolver,
            None,
        )
        .await
        .expect_err("DB metadata without Router serviceDb must fail closed");

    assert!(
        format!("{error:#}").contains("Router-supplied serviceDb"),
        "unexpected error: {error:#}"
    );
    assert!(controller.active().unwrap().is_none());
}

#[tokio::test]
async fn identical_stateful_diamond_has_one_metadata_owner_in_any_edge_order() {
    for reverse_links in [false, true] {
        let mut fixture =
            CollectionIdentityFixture::with_stateful_diamond(BTreeMap::new(), BTreeMap::new());
        if reverse_links {
            fixture.assembly.package_link_plan.package_links.reverse();
            skiff_artifact_identity::assign_runtime_assembly_identity(&mut fixture.assembly)
                .unwrap();
            fixture.resolver.assembly = Arc::new(fixture.assembly.clone());
        }
        let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
        let (config_snapshot, config_resolver) =
            config_snapshot_for_assembly("fixture", &fixture.assembly, &fixture.resolver);
        let provider = CapturingDbProvider::default();
        let controller = AssemblyAdmissionController::new(
            format!("runtime-diamond-{reverse_links}"),
            skiff_runtime_capability_context::DbProviderSource::new(provider.clone()),
        );

        controller
            .recover_committed(
                "fixture",
                7,
                &reference,
                &config_snapshot,
                &fixture.resolver,
                &config_resolver,
                Some(&mapping_service_db()),
            )
            .await
            .expect("identical stateful diamond must admit");

        let inputs = provider.inputs.lock().unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].environment, "fixture");
        assert_eq!(inputs[0].service_id, fixture.assembly.roots[0].service_id);
        let mut collections = inputs[0]
            .runtime_program_db
            .iter()
            .filter(|metadata| {
                metadata.metadata.package_id.as_deref() == Some("example.mapping-store")
            })
            .map(|metadata| metadata.metadata.collection_name.as_deref().unwrap_or(""))
            .collect::<Vec<_>>();
        collections.sort_unstable();
        assert_eq!(collections, vec!["package_audit", "package_secret"]);
    }
}

#[tokio::test]
async fn equal_bare_collection_names_from_service_and_package_remain_distinct_targets() {
    let fixture = CollectionIdentityFixture::new(BTreeMap::new(), Some("package_secret"));
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let (config_snapshot, config_resolver) =
        config_snapshot_for_assembly("fixture", &fixture.assembly, &fixture.resolver);
    let provider = CapturingDbProvider::default();
    let controller = AssemblyAdmissionController::new(
        "runtime-service-collision",
        skiff_runtime_capability_context::DbProviderSource::new(provider.clone()),
    );

    controller
        .recover_committed(
            "fixture",
            1,
            &reference,
            &config_snapshot,
            &fixture.resolver,
            &config_resolver,
            Some(&mapping_service_db()),
        )
        .await
        .expect("Package identity, not the bare name, owns physical collection storage");

    let inputs = provider.inputs.lock().unwrap();
    let same_bare_name = inputs[0]
        .runtime_program_db
        .iter()
        .filter(|entry| entry.metadata.collection_name.as_deref() == Some("package_secret"))
        .map(|entry| {
            entry
                .target
                .target_id
                .package_artifact_ref
                .package_id
                .as_str()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        same_bare_name,
        BTreeSet::from(["example.phase-three-consumer", "example.mapping-store"])
    );
}

fn service_contract(
    service_id: &str,
    stable_key: &str,
    display_name: &str,
    operation_contract: BoundaryOperationContract,
) -> (ServiceContract, ContractOperationId) {
    let contract_version = "1.0.0";
    let operation_id =
        skiff_artifact_identity::contract_operation_id(service_id, contract_version, stable_key)
            .unwrap();
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: contract_version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(
            operation_id.clone(),
            BoundaryOperationDescriptor {
                operation_id: operation_id.clone(),
                stable_key: stable_key.to_string(),
                contract: operation_contract,
            },
        )]),
        package_type_requirements: Vec::new(),
        diagnostic_text: ContractDiagnosticText {
            service: display_name.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
    (contract, operation_id)
}

fn implementation_file(
    module_path: &str,
    symbol: &str,
    service_call: Option<ServiceCallRef>,
) -> FileIrUnit {
    let mut file = FileIrUnit::empty(module_path, format!("source:{module_path}"));
    file.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("bool"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody::default(),
        source_span: None,
    });
    if let Some(service_call) = service_call {
        file.external_refs.service_call_refs.push(service_call);
        file.executables[0].body.expressions.push(ExprIr::Call {
            call: CallIr {
                target: CallTargetIr::ServiceCall {
                    service_call_ref_index: ServiceCallRefIndex::new(0),
                },
                site: InstructionSourceSite::Synthetic {
                    reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                },
                args: Vec::new(),
                type_args: BTreeMap::new(),
                metadata: BTreeMap::new(),
            },
        });
    }
    skiff_artifact_identity::assign_file_ir_identity(&mut file).unwrap();
    file
}

fn implementation_package(
    package_id: &str,
    public_path: &str,
    callable_id: PackageCallableId,
    file: &FileIrUnit,
    operation_contract: BoundaryOperationContract,
    service_dependency: Option<(ContractRequirement, ServiceCallRef)>,
) -> PackageArtifact {
    let file_ref = file_ref(file);
    let entry = file
        .executables
        .first()
        .expect("fixture implementation must expose its entry executable");
    let effects = no_effects();
    let provenance = CallableProvenanceSummary::Analyzed {
        return_origins: Vec::new(),
        direct_return_origins: Vec::new(),
        throw_origins: Vec::new(),
        escape_lanes: Vec::new(),
    };
    let mut contract_requirements = Vec::new();
    let mut service_requirements = Vec::new();
    let mut service_call_refs = Vec::new();
    if let Some((contract_requirement, service_call)) = service_dependency {
        contract_requirements.push(contract_requirement.clone());
        service_requirements.push(ServiceRequirement {
            contract_requirement,
            service_binding_slot: service_call.service_requirement_slot,
            used_operations: BTreeSet::from([service_call.contract_operation_id.clone()]),
        });
        service_call_refs.push(service_call);
    }
    let mut package = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: vec![file_ref.clone()],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::from([(
                public_path.to_string(),
                PackageLocalAbiSymbol::Callable {
                    callable_id: callable_id.clone(),
                    signature: PackageCallableSignature {
                        type_params: entry.type_params.clone(),
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
            package_id: package_id.to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                package_id,
                &BTreeMap::new(),
            )
            .expect("empty Package schema index is canonical"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks {
            functions: BTreeMap::from([(
                public_path.to_string(),
                ExecutableExport {
                    file: file_ref.clone(),
                    executable_index: 0,
                    symbol: entry.symbol.clone(),
                    signature: ExecutableSignatureIr {
                        params: entry.params.clone(),
                        return_type: entry.return_type.clone(),
                        self_type: entry.self_type.clone(),
                        may_suspend: entry.may_suspend,
                    },
                },
            )]),
            ..PackageImplementationLinks::default()
        },
        callable_links: BTreeMap::from([(
            callable_id.clone(),
            PackageCallableLinkFact {
                callable_id: callable_id.clone(),
                target: OperationTargetRef {
                    file_ref,
                    executable_index: 0,
                    callable_abi_id: callable_id.to_string(),
                    callable_kind: OperationCallableKind::PublicFunction,
                },
            },
        )]),
        package_requirements: Vec::new(),
        contract_requirements,
        service_requirements,
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::from([(
            callable_id.clone(),
            CallableSemanticFacts {
                effects: CallableEffectSummary::Analyzed {
                    effects: effects.clone(),
                },
                provenance: provenance.clone(),
                resolved_call_targets: BTreeMap::new(),
            },
        )]),
        boundary_projections: BTreeMap::from([(
            callable_id,
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
        service_call_refs,
    };
    skiff_artifact_identity::assign_package_artifact_identities(&mut package).unwrap();
    package
}

fn file_ref(file: &FileIrUnit) -> FileIrRef {
    FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
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

fn operation_contract() -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: Vec::new(),
        return_value: BoundaryReturn {
            ty: ContractTypeRef::builtin("bool"),
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

struct ContractImplementationFixture {
    assembly: RuntimeAssembly,
    resolver: CountingResolver,
    consumer_deployment_refs: Vec<ServiceDeploymentRef>,
    consumer_package_refs: Vec<PackageArtifactRef>,
    provider_package_ref: PackageArtifactRef,
}

fn db_index(name: &str, unique: bool, fields: &[(&str, i32)]) -> DbIndexIr {
    DbIndexIr {
        name: name.to_string(),
        unique,
        fields: fields
            .iter()
            .map(|(field, direction)| DbIndexFieldIr {
                field: FieldPathIr {
                    text: field.to_string(),
                    segments: vec![field.to_string()],
                },
                direction: match direction {
                    1 => DbIndexDirectionIr::Asc,
                    -1 => DbIndexDirectionIr::Desc,
                    _ => unreachable!("fixture index direction"),
                },
            })
            .collect(),
    }
}

fn db_object_declaration(
    type_index: u32,
    name: &str,
    collection_name: &str,
    implements: Option<TypeRefIr>,
    indexes: Vec<DbIndexIr>,
) -> DbDeclarationIr {
    DbDeclarationIr {
        type_ref: TypeRefIr::LocalType { type_index },
        type_name: name.to_string(),
        collection_name: Some(collection_name.to_string()),
        implements,
        kind: DbObjectKindIr::Object,
        key: DbObjectKeyIr {
            name: "id".to_string(),
            ty: TypeRefIr::builtin("string"),
        },
        fields: vec![
            DbObjectFieldIr {
                name: "status".to_string(),
                ty: TypeRefIr::builtin("string"),
                storage: DbFieldStorageIr::Identity,
            },
            DbObjectFieldIr {
                name: "updatedAt".to_string(),
                ty: TypeRefIr::builtin("string"),
                storage: DbFieldStorageIr::Identity,
            },
        ],
        retention: None,
        leases: Vec::new(),
        indexes,
        source_span: None,
    }
}

fn db_contract_declaration(
    type_index: u32,
    name: &str,
    indexes: Vec<DbIndexIr>,
) -> DbDeclarationIr {
    DbDeclarationIr {
        type_ref: TypeRefIr::LocalType { type_index },
        type_name: name.to_string(),
        collection_name: None,
        implements: None,
        kind: DbObjectKindIr::Contract,
        key: DbObjectKeyIr {
            name: "id".to_string(),
            ty: TypeRefIr::builtin("string"),
        },
        fields: vec![
            DbObjectFieldIr {
                name: "status".to_string(),
                ty: TypeRefIr::builtin("string"),
                storage: DbFieldStorageIr::Identity,
            },
            DbObjectFieldIr {
                name: "updatedAt".to_string(),
                ty: TypeRefIr::builtin("string"),
                storage: DbFieldStorageIr::Identity,
            },
        ],
        retention: None,
        leases: Vec::new(),
        indexes,
        source_span: None,
    }
}

fn contract_implements_ref(symbol_path: &str) -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::Dependency {
                dependency_ref: "provider".to_string(),
            },
            symbol_path: symbol_path.to_string(),
            abi_expectation: None,
        },
    }
}

/// Builds one service deployment with the given consumer package and binds it
/// to the provider package under the `provider` alias.
fn contract_consumer_deployment(
    base: &FullChainFixture,
    consumer_package: &PackageArtifact,
    revision: &str,
    service_contract: &ServiceContract,
    contract_ref: &ServiceContractRef,
) -> ServiceDeployment {
    let mut deployment = base
        .resolver
        .deployments
        .iter()
        .find(|(reference, _)| reference == &base.consumer_deployment_ref)
        .map(|(_, deployment)| deployment.as_ref().clone())
        .expect("consumer deployment");
    deployment.deployment_revision = DeploymentRevision::new(revision);
    deployment.implementation = package_ref(consumer_package);
    for selector in &mut deployment.service_selectors {
        selector.key.caller_package_build_id = consumer_package.package_build_id.clone();
    }
    deployment.package_bindings = vec![PackageBinding {
        key: PackageRequirementKey {
            caller_package_build_id: consumer_package.package_build_id.clone(),
            package_requirement_alias: "provider".to_string(),
        },
        package: base.provider_package_ref.clone(),
    }];
    deployment.contract = contract_ref.clone();
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    deployment
}

/// Builds one consumer service package containing the given DB declarations
/// plus the service call to the provider operation.
fn contract_consumer_package(
    base: &FullChainFixture,
    package_id_suffix: &str,
    declarations: Vec<(String, DbDeclarationIr)>,
) -> (PackageArtifact, FileIrUnit) {
    let provider_call = ServiceCallRef {
        service_requirement_slot: 7,
        contract_operation_id: base.provider_operation_id.clone(),
        expected_protocol_identity: base
            .provider_contract
            .service_protocol_identity
            .clone(),
    };
    let mut file = implementation_file("consumer.main", "check", Some(provider_call));
    for (type_name, declaration) in declarations {
        let type_index = local_type_index(&declaration);
        file.type_table.push(TypeDeclIr {
            name: type_name.clone(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([
                    ("id".to_string(), TypeRefIr::builtin("string")),
                    ("status".to_string(), TypeRefIr::builtin("string")),
                    ("updatedAt".to_string(), TypeRefIr::builtin("string")),
                ]),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        });
        file.declarations.types.insert(
            type_name.clone(),
            TypeDeclarationIr {
                type_index,
                symbol: format!("{}.{}", file.module_path, type_name),
                source_span: None,
            },
        );
        file.declarations.db.insert(type_name, declaration);
    }
    skiff_artifact_identity::assign_file_ir_identity(&mut file).unwrap();
    let callable_id = PackageCallableId::new(format!(
        "pkg-callable:example.phase-three-consumer:{package_id_suffix}:check"
    ));
    let package = implementation_package(
        "example.phase-three-consumer",
        "check",
        callable_id,
        &file,
        operation_contract(),        Some((
            ContractRequirement {
                alias: "provider".to_string(),
                service_id: base.provider_contract_ref.service_id.clone(),
                contract_version: base.provider_contract_ref.contract_version.clone(),
                expected_protocol_identity: base
                    .provider_contract_ref
                    .service_protocol_identity
                    .clone(),
            },
            provider_call,
        )),
    );
    (package, file)
}

/// Builds one provider package whose file contains the given DB declarations
/// (contracts) exported under the given implementation symbol paths.
fn contract_provider_package(
    base: &FullChainFixture,
    declarations: Vec<(String, DbDeclarationIr)>,
) -> (PackageArtifact, FileIrUnit) {
    let mut file = implementation_file("provider.main", "health", None);
    let mut implementation_symbols = BTreeMap::new();
    let mut type_exports = BTreeMap::new();
    for (type_name, declaration) in declarations {
        let type_index = local_type_index(&declaration);
        file.type_table.push(TypeDeclIr {
            name: type_name.clone(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([
                    ("id".to_string(), TypeRefIr::builtin("string")),
                    ("status".to_string(), TypeRefIr::builtin("string")),
                    ("updatedAt".to_string(), TypeRefIr::builtin("string")),
                ]),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        });
        file.declarations.types.insert(
            type_name.clone(),
            TypeDeclarationIr {
                type_index,
                symbol: format!("model.{type_name}"),
                source_span: None,
            },
        );
        file.declarations.db.insert(type_name, declaration);
        let symbol_path = format!("model.{type_name}");
        let descriptor = file.type_table[type_index as usize].descriptor.clone();
        implementation_symbols.insert(
            symbol_path.clone(),
            PackageLocalAbiSymbol::Type {
                local_type_id: format!("type:example.phase-three-provider:{symbol_path}"),
                descriptor: descriptor.clone(),
                is_alias: false,
                is_interface: false,
                type_params: Vec::new(),
                interface_methods: Vec::new(),
                actor: None,
            },
        );
        type_exports.insert(
            symbol_path,
            TypeExport {
                file: file_ref(&file),
                type_index,
                symbol: format!("model.{type_name}"),
                is_interface: false,
                descriptor: Some(descriptor),
                type_params: Vec::new(),
                interface_methods: Vec::new(),
                actor: None,
            },
        );
    }
    skiff_artifact_identity::assign_file_ir_identity(&mut file).unwrap();
    let file_ref = file_ref(&file);
    let callable_id = PackageCallableId::new("pkg-callable:example.phase-three-provider:health");
    let mut package = implementation_package(
        "example.phase-three-provider",
        "health",
        callable_id,
        &file,
        operation_contract(),
        None,
    );
    package.files = vec![file_ref.clone()];
    package.package_local_abi.implementation_symbols = implementation_symbols;
    package.package_local_abi.public_symbols = BTreeMap::new();
    package.implementation_links.types = type_exports;
    package.implementation_links.functions = BTreeMap::from([(
        "health".to_string(),
        ExecutableExport {
            file: file_ref,
            executable_index: 0,
            symbol: "health".to_string(),
            signature: ExecutableSignatureIr {
                params: Vec::new(),
                return_type: TypeRefIr::builtin("bool"),
                self_type: None,
                may_suspend: false,
            },
        },
    )]);
    skiff_artifact_identity::assign_package_artifact_identities(&mut package).unwrap();
    (package, file)
}

fn local_type_index(declaration: &DbDeclarationIr) -> u32 {
    match &declaration.type_ref {
        TypeRefIr::LocalType { type_index } => *type_index,
        _ => panic!("fixture db declaration must name its local type"),
    }
}

fn build_contract_implementation_fixture(
    consumer_versions: Vec<Vec<(String, DbDeclarationIr)>>,
    provider_declarations: Vec<(String, DbDeclarationIr)>,
) -> ContractImplementationFixture {
    let base = FullChainFixture::new();
    let consumer_contract = base
        .resolver
        .contracts
        .iter()
        .find(|(_, contract)| contract.service_id == "example.phase-three.consumer")
        .map(|(_, contract)| contract.as_ref().clone())
        .expect("consumer contract");
    let consumer_contract_ref = skiff_artifact_identity::service_contract_ref(&consumer_contract)
        .unwrap();
    let provider_contract = base
        .resolver
        .contracts
        .iter()
        .find(|(reference, _)| reference == &base.provider_contract_ref)
        .map(|(_, contract)| contract.as_ref().clone())
        .expect("provider contract");
    let provider_contract_ref = base.provider_contract_ref.clone();

    let mut consumer_packages = Vec::new();
    let mut consumer_files = Vec::new();
    let mut consumer_deployments = Vec::new();
    let mut consumer_deployment_refs = Vec::new();
    let mut consumer_package_refs = Vec::new();
    for (version, declarations) in consumer_versions.into_iter().enumerate() {
        let (consumer_package, consumer_file) =
            contract_consumer_package(&base, &version.to_string(), declarations);
        let consumer_package_ref = package_ref(&consumer_package);
        let consumer_file_ref = file_ref(&consumer_file);
        let mut deployment = contract_consumer_deployment(
            &base,
            &consumer_package,
            &format!("consumer-revision-{}", version + 1),
            &consumer_contract,
            &consumer_contract_ref,
        );
        let deployment_ref = skiff_artifact_identity::service_deployment_ref(&deployment);
        consumer_deployments.push(deployment.clone());
        consumer_deployment_refs.push(deployment_ref.clone());
        consumer_package_refs.push(consumer_package_ref.clone());
        consumer_packages.push(Arc::new(consumer_package));
        consumer_files.push((consumer_package_ref, consumer_file_ref, Arc::new(consumer_file)));
    }

    let (provider_package, provider_file) = contract_provider_package(&base, provider_declarations);
    let provider_package_ref = package_ref(&provider_package);
    let provider_file_ref = file_ref(&provider_file);

    let mut deployments = consumer_deployments.clone();
    let mut provider_deployment = base
        .resolver
        .deployments
        .iter()
        .find(|(reference, _)| reference == &base.provider_deployment_ref)
        .map(|(_, deployment)| deployment.as_ref().clone())
        .expect("provider deployment");
    provider_deployment.implementation = provider_package_ref.clone();
    skiff_artifact_identity::assign_service_deployment_identity(&mut provider_deployment).unwrap();
    deployments.push(provider_deployment);
    let deployment_refs = consumer_deployment_refs
        .iter()
        .cloned()
        .chain(std::iter::once(
            skiff_artifact_identity::service_deployment_ref(
                deployments.last().expect("provider deployment"),
            ),
        ))
        .collect::<Vec<_>>();
    let assembly = resolve_runtime_assembly(
        &deployment_refs,
        &deployments,
        &[consumer_contract.clone(), provider_contract.clone()],
        &consumer_packages
            .iter()
            .map(|package| package.as_ref().clone())
            .chain(std::iter::once(provider_package.clone()))
            .collect::<Vec<_>>(),
    )
    .expect("contract implementation assembly should resolve");
    let resolver = CountingResolver {
        assembly: Arc::new(assembly.clone()),
        deployments: deployments
            .iter()
            .cloned()
            .map(|deployment| {
                let reference =
                    skiff_artifact_identity::service_deployment_ref(&deployment);
                (reference, Arc::new(deployment))
            })
            .collect(),
        contracts: vec![
            (consumer_contract_ref, Arc::new(consumer_contract)),
            (provider_contract_ref, Arc::new(provider_contract)),
        ],
        packages: consumer_packages
            .iter()
            .map(|package| {
                (
                    package_ref(package),
                    Arc::clone(package),
                )
            })
            .chain(std::iter::once((
                provider_package_ref.clone(),
                Arc::new(provider_package),
            )))
            .collect(),
        files: consumer_files
            .iter()
            .map(|(package_ref, file_ref, file)| (package_ref.clone(), file_ref.clone(), Arc::clone(file)))
            .chain(std::iter::once((
                provider_package_ref.clone(),
                provider_file_ref,
                Arc::new(provider_file),
            )))
            .collect(),
        reads: AtomicUsize::new(0),
    };
    ContractImplementationFixture {
        assembly,
        resolver,
        consumer_deployment_refs,
        consumer_package_refs,
        provider_package_ref,
    }
}

async fn admit_contract_implementation_fixture(
    fixture: &ContractImplementationFixture,
    runtime_id: &str,
) -> anyhow::Result<()> {
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let (config_snapshot, config_resolver) =
        config_snapshot_for_assembly("fixture", &fixture.assembly, &fixture.resolver);
    let provider = CapturingDbProvider::default();
    let controller = AssemblyAdmissionController::new(
        runtime_id,
        skiff_runtime_capability_context::DbProviderSource::new(provider.clone()),
    );
    controller
        .recover_committed(
            "fixture",
            9,
            &reference,
            &config_snapshot,
            &fixture.resolver,
            &config_resolver,
            Some(&mapping_service_db()),
        )
        .await
}

async fn contract_implementation_admission_error(
    fixture: &ContractImplementationFixture,
    runtime_id: &str,
) -> String {
    admit_contract_implementation_fixture(fixture, runtime_id)
        .await
        .expect_err("contract implementation coverage must fail admission")
        .to_string()
}

#[tokio::test]
async fn db_object_implements_contract_admits_with_required_index_coverage() {
    let by_status_updated = db_index("byStatusUpdated", false, &[("status", 1), ("updatedAt", -1)]);
    let fixture = build_contract_implementation_fixture(
        vec![vec![(
            "Thread".to_string(),
            db_object_declaration(
                0,
                "Thread",
                "threads",
                Some(contract_implements_ref("model.AgentThread")),
                vec![
                    by_status_updated.clone(),
                    db_index("byOwner", false, &[("ownerUserId", 1)]),
                ],
            ),
        )]],
        vec![(
            "AgentThread".to_string(),
            db_contract_declaration(0, "AgentThread", vec![by_status_updated]),
        )],
    );
    admit_contract_implementation_fixture(&fixture, "runtime-contract-implementation")
        .await
        .expect("covered contract implementation must admit");
}

#[tokio::test]
async fn db_contract_missing_required_index_fails_admission() {
    let fixture = build_contract_implementation_fixture(
        vec![vec![(
            "Thread".to_string(),
            db_object_declaration(
                0,
                "Thread",
                "threads",
                Some(contract_implements_ref("model.AgentThread")),
                vec![db_index("byOwner", false, &[("ownerUserId", 1)])],
            ),
        )]],
        vec![(
            "AgentThread".to_string(),
            db_contract_declaration(
                0,
                "AgentThread",
                vec![db_index("byStatusUpdated", false, &[("status", 1), ("updatedAt", -1)])],
            ),
        )],
    );
    let error = contract_implementation_admission_error(&fixture, "runtime-missing-index").await;
    assert!(error.contains("requires managed index"), "{error}");
    assert!(error.contains("byStatusUpdated"), "{error}");
}

#[tokio::test]
async fn db_contract_without_implementation_fails_admission() {
    let fixture = build_contract_implementation_fixture(
        vec![vec![(
            "Thread".to_string(),
            db_object_declaration(0, "Thread", "threads", None, Vec::new()),
        )]],
        vec![(
            "AgentThread".to_string(),
            db_contract_declaration(0, "AgentThread", Vec::new()),
        )],
    );
    let error = contract_implementation_admission_error(&fixture, "runtime-missing-implementation")
        .await;
    assert!(error.contains("has no implementing db object"), "{error}");
}

#[tokio::test]
async fn db_contract_with_duplicate_implementations_fails_admission() {
    let fixture = build_contract_implementation_fixture(
        vec![vec![
            (
                "Thread".to_string(),
                db_object_declaration(
                    0,
                    "Thread",
                    "threads",
                    Some(contract_implements_ref("model.AgentThread")),
                    Vec::new(),
                ),
            ),
            (
                "ThreadCopy".to_string(),
                db_object_declaration(
                    1,
                    "ThreadCopy",
                    "thread_copy",
                    Some(contract_implements_ref("model.AgentThread")),
                    Vec::new(),
                ),
            ),
        ]],
        vec![(
            "AgentThread".to_string(),
            db_contract_declaration(0, "AgentThread", Vec::new()),
        )],
    );
    let error = contract_implementation_admission_error(&fixture, "runtime-duplicate-implementation")
        .await;
    assert!(
        error.contains("implemented more than once inside package build"),
        "{error}"
    );
}

#[tokio::test]
async fn db_object_implements_non_contract_type_fails_admission() {
    let fixture = build_contract_implementation_fixture(
        vec![vec![(
            "Thread".to_string(),
            db_object_declaration(
                0,
                "Thread",
                "threads",
                Some(contract_implements_ref("model.Session")),
                Vec::new(),
            ),
        )]],
        Vec::new(),
    );
    let error = contract_implementation_admission_error(&fixture, "runtime-non-contract-target")
        .await;
    assert!(
        error.contains("not a db contract declaration"),
        "{error}"
    );
}

#[tokio::test]
async fn db_contract_multi_version_implementation_merges_index_coverage() {
    let by_status_updated = db_index("byStatusUpdated", false, &[("status", 1), ("updatedAt", -1)]);
    let fixture = build_contract_implementation_fixture(
        vec![
            vec![(
                "Thread".to_string(),
                db_object_declaration(
                    0,
                    "Thread",
                    "threads",
                    Some(contract_implements_ref("model.AgentThread")),
                    vec![
                        by_status_updated.clone(),
                        db_index("byOwner", false, &[("ownerUserId", 1)]),
                    ],
                ),
            )],
            vec![(
                "Thread".to_string(),
                db_object_declaration(
                    0,
                    "Thread",
                    "threads",
                    Some(contract_implements_ref("model.AgentThread")),
                    vec![by_status_updated.clone()],
                ),
            )],
        ],
        vec![(
            "AgentThread".to_string(),
            db_contract_declaration(0, "AgentThread", vec![by_status_updated]),
        )],
    );
    admit_contract_implementation_fixture(&fixture, "runtime-multi-version-coverage")
        .await
        .expect("multi-version implementations of one contract must merge");
}

#[tokio::test]
async fn db_contract_multi_version_conflicting_index_definition_still_rejected() {
    let by_status_updated = db_index("byStatusUpdated", false, &[("status", 1), ("updatedAt", -1)]);
    let fixture = build_contract_implementation_fixture(
        vec![
            vec![(
                "Thread".to_string(),
                db_object_declaration(
                    0,
                    "Thread",
                    "threads",
                    Some(contract_implements_ref("model.AgentThread")),
                    vec![db_index("byStatusUpdated", false, &[("status", 1)])],
                ),
            )],
            vec![(
                "Thread".to_string(),
                db_object_declaration(
                    0,
                    "Thread",
                    "threads",
                    Some(contract_implements_ref("model.AgentThread")),
                    vec![by_status_updated.clone()],
                ),
            )],
        ],
        vec![(
            "AgentThread".to_string(),
            db_contract_declaration(0, "AgentThread", vec![by_status_updated]),
        )],
    );
    let error = contract_implementation_admission_error(&fixture, "runtime-conflicting-index")
        .await;
    assert!(
        error.contains("conflicting definitions"),
        "{error}"
    );
}
