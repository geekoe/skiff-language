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

use super::super::*;

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
    consumer_deployment_ref: ServiceDeploymentRef,
    consumer_package_ref: PackageArtifactRef,
    consumer_file_ir_identity: String,
}

impl FullChainFixture {
    fn new() -> Self {
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
        let consumer_file =
            implementation_file("consumer.main", "check", Some(provider_call.clone()));
        let consumer_file_ref = file_ref(&consumer_file);
        let consumer_file_ir_identity = consumer_file_ref.file_ir_identity.clone();
        let consumer_package = implementation_package(
            "example.phase-three-consumer",
            "check",
            consumer_callable_id.clone(),
            &consumer_file,
            operation_contract,
            Some((provider_requirement, provider_call)),
        );
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
                config_literals: Vec::new(),
                secret_refs: Vec::new(),
                state_bindings: Vec::new(),
                resource_bindings: Vec::new(),
                runtime_capability_bindings: Vec::new(),
                policy: policy(),
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
                config_literals: Vec::new(),
                secret_refs: Vec::new(),
                state_bindings: Vec::new(),
                resource_bindings: Vec::new(),
                runtime_capability_bindings: Vec::new(),
                policy: policy(),
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
                    provider_package_ref,
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
            consumer_deployment_ref,
            consumer_package_ref,
            consumer_file_ir_identity,
        }
    }
}

struct CollectionMappingFixture {
    assembly: RuntimeAssembly,
    resolver: CountingResolver,
}

impl CollectionMappingFixture {
    fn new(mapping: BTreeMap<String, String>, root_collection: Option<&str>) -> Self {
        Self::build(mapping, root_collection, false, None)
    }

    fn with_stateful_diamond(
        direct_mapping: BTreeMap<String, String>,
        transitive_mapping: BTreeMap<String, String>,
    ) -> Self {
        Self::build(direct_mapping, None, false, Some(transitive_mapping))
    }

    fn with_dependency_target_collision() -> Self {
        Self::build(
            BTreeMap::from([(
                "package_secret".to_string(),
                "mapped_package_secret".to_string(),
            )]),
            None,
            true,
            None,
        )
    }

    fn build(
        mapping: BTreeMap<String, String>,
        root_collection: Option<&str>,
        include_colliding_dependency: bool,
        diamond_mapping: Option<BTreeMap<String, String>>,
    ) -> Self {
        let base = FullChainFixture::new();
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
        let mut consumer_file = base
            .resolver
            .files
            .iter()
            .find(|(reference, _, _)| reference == &base.consumer_package_ref)
            .map(|(_, _, file)| file.as_ref().clone())
            .expect("consumer file");
        if let Some(collection_name) = root_collection {
            insert_db_collection(&mut consumer_file, "ServiceSecret", collection_name);
            replace_package_file(&mut consumer_package, &consumer_file);
            consumer_package
                .runtime_requirements
                .state
                .push(PackageStateRequirement {
                    key: "database".to_string(),
                    kind: StateBindingKind::Database,
                });
        }

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
        dependency_package
            .runtime_requirements
            .state
            .push(PackageStateRequirement {
                key: "database".to_string(),
                kind: StateBindingKind::Database,
            });
        skiff_artifact_identity::assign_package_artifact_identities(&mut dependency_package)
            .unwrap();
        let dependency_ref = package_ref(&dependency_package);

        let diamond_subject = diamond_mapping.as_ref().map(|mapping| {
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
                collection_name_mapping: mapping.clone(),
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
            package
                .runtime_requirements
                .state
                .push(PackageStateRequirement {
                    key: "database".to_string(),
                    kind: StateBindingKind::Database,
                });
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
                collection_name_mapping: mapping.clone(),
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
                    collection_name_mapping: BTreeMap::new(),
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
                    collection_name_mapping: BTreeMap::from([(
                        "cache_secret".to_string(),
                        "mapped_package_secret".to_string(),
                    )]),
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
            collection_name_mapping: mapping,
        }];
        if let Some((_, package)) = &diamond_subject {
            package_bindings.extend([
                PackageBinding {
                    key: PackageRequirementKey {
                        caller_package_build_id: consumer_package_ref.package_build_id.clone(),
                        package_requirement_alias: "subject".to_string(),
                    },
                    package: package_ref(package),
                    collection_name_mapping: BTreeMap::new(),
                },
                PackageBinding {
                    key: PackageRequirementKey {
                        caller_package_build_id: package.package_build_id.clone(),
                        package_requirement_alias: "store".to_string(),
                    },
                    package: dependency_ref.clone(),
                    collection_name_mapping: diamond_mapping
                        .clone()
                        .expect("diamond subject has a mapping"),
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
                collection_name_mapping: BTreeMap::from([(
                    "cache_secret".to_string(),
                    "mapped_package_secret".to_string(),
                )]),
            });
        }
        consumer_deployment.package_bindings = package_bindings;
        consumer_deployment.state_bindings = vec![StateBinding {
            requirement_key: "database".to_string(),
            kind: StateBindingKind::Database,
            namespace: "collection-mapping-fixture".to_string(),
        }];
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
    file.declarations.db.insert(
        type_name.to_string(),
        DbDeclarationIr {
            type_ref: TypeRefIr::Record {
                fields: fields.clone(),
            },
            type_name: type_name.to_string(),
            collection_name: collection_name.to_string(),
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

fn replace_package_file(package: &mut PackageArtifact, file: &FileIrUnit) {
    let old_ref = package.files[0].clone();
    let new_ref = file_ref(file);
    package.files[0] = new_ref.clone();
    for export in package.implementation_links.functions.values_mut() {
        if export.file == old_ref {
            export.file = new_ref.clone();
        }
    }
    for fact in package.callable_links.values_mut() {
        if fact.target.file_ref == old_ref {
            fact.target.file_ref = new_ref.clone();
        }
    }
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

#[tokio::test]
async fn committed_recovery_nonempty_generation_survives_restart_with_exact_registration() {
    let fixture = FullChainFixture::new();
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();

    let first = AssemblyAdmissionController::new(
        "runtime-a",
        skiff_runtime_capability_context::DbProviderSource::unavailable(),
    );
    let first_active = first
        .recover_committed("prod", 7, &reference, &fixture.resolver, None)
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
        .recover_committed("prod", 7, &reference, &fixture.resolver, None)
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
async fn collection_mapping_reaches_db_provider_exactly_and_survives_reload() {
    let fixture = CollectionMappingFixture::new(
        BTreeMap::from([(
            "package_secret".to_string(),
            "mapped_package_secret".to_string(),
        )]),
        None,
    );
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
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
            &fixture.resolver,
            Some(&service_db),
        )
        .await
        .expect("mapped collection fixture must admit");
    controller
        .recover_committed(
            "fixture",
            7,
            &reference,
            &fixture.resolver,
            Some(&service_db),
        )
        .await
        .expect("reload must rebuild the exact mapped metadata");

    let inputs = provider.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0].runtime_program_db, inputs[1].runtime_program_db);
    let mut collections = inputs[0]
        .runtime_program_db
        .iter()
        .map(|metadata| {
            assert_eq!(metadata.source_role, "package");
            assert_eq!(
                metadata.package_id.as_deref(),
                Some("example.mapping-store")
            );
            metadata.collection_name.clone()
        })
        .collect::<Vec<_>>();
    collections.sort();
    assert_eq!(
        collections,
        vec![
            "mapped_package_secret".to_string(),
            "package_audit".to_string(),
        ]
    );
}

#[tokio::test]
async fn identical_stateful_diamond_has_one_effective_projection_in_any_edge_order() {
    for (label, mapping, expected_collections) in [
        (
            "empty",
            BTreeMap::new(),
            vec!["package_audit", "package_secret"],
        ),
        (
            "mapped",
            BTreeMap::from([(
                "package_secret".to_string(),
                "mapped_package_secret".to_string(),
            )]),
            vec!["mapped_package_secret", "package_audit"],
        ),
    ] {
        for reverse_links in [false, true] {
            let mut fixture =
                CollectionMappingFixture::with_stateful_diamond(mapping.clone(), mapping.clone());
            if reverse_links {
                fixture.assembly.package_link_plan.package_links.reverse();
                skiff_artifact_identity::assign_runtime_assembly_identity(&mut fixture.assembly)
                    .unwrap();
                fixture.resolver.assembly = Arc::new(fixture.assembly.clone());
            }
            let reference =
                skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
            let provider = CapturingDbProvider::default();
            let controller = AssemblyAdmissionController::new(
                format!("runtime-diamond-{label}-{reverse_links}"),
                skiff_runtime_capability_context::DbProviderSource::new(provider.clone()),
            );

            controller
                .recover_committed(
                    "fixture",
                    7,
                    &reference,
                    &fixture.resolver,
                    Some(&mapping_service_db()),
                )
                .await
                .expect("identical stateful diamond must admit");

            let inputs = provider.inputs.lock().unwrap();
            assert_eq!(inputs.len(), 1);
            assert_eq!(inputs[0].state_namespace, "collection-mapping-fixture");
            let mut collections = inputs[0]
                .runtime_program_db
                .iter()
                .filter(|metadata| metadata.package_id.as_deref() == Some("example.mapping-store"))
                .map(|metadata| metadata.collection_name.as_str())
                .collect::<Vec<_>>();
            collections.sort_unstable();
            let mut expected = expected_collections.clone();
            expected.sort_unstable();
            assert_eq!(collections, expected);
        }
    }
}

#[tokio::test]
async fn same_build_stateful_diamond_with_different_projection_fails_closed() {
    let fixture = CollectionMappingFixture::with_stateful_diamond(
        BTreeMap::new(),
        BTreeMap::from([(
            "package_secret".to_string(),
            "mapped_package_secret".to_string(),
        )]),
    );
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let provider = CapturingDbProvider::default();
    let controller = AssemblyAdmissionController::new(
        "runtime-diamond-drift",
        skiff_runtime_capability_context::DbProviderSource::new(provider.clone()),
    );

    let error = controller
        .recover_committed(
            "fixture",
            7,
            &reference,
            &fixture.resolver,
            Some(&mapping_service_db()),
        )
        .await
        .expect_err("same build with different resolved projection must fail closed");
    let error = format!("{error:#}");
    assert!(
        error.contains("different active collection projections"),
        "{error}"
    );
    assert!(provider.inputs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn collection_mapping_unknown_source_and_partial_collision_fail_closed() {
    for (label, mapping, expected) in [
        (
            "unknown",
            BTreeMap::from([(
                "missing_collection".to_string(),
                "mapped_collection".to_string(),
            )]),
            "is not declared",
        ),
        (
            "partial collision",
            BTreeMap::from([("package_secret".to_string(), "package_audit".to_string())]),
            "both resolve",
        ),
    ] {
        let fixture = CollectionMappingFixture::new(mapping, None);
        let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
        let provider = CapturingDbProvider::default();
        let controller = AssemblyAdmissionController::new(
            format!("runtime-{label}"),
            skiff_runtime_capability_context::DbProviderSource::new(provider.clone()),
        );
        let error = controller
            .recover_committed(
                "fixture",
                1,
                &reference,
                &fixture.resolver,
                Some(&mapping_service_db()),
            )
            .await
            .expect_err("invalid collection mapping must fail closed");
        let error = format!("{error:#}");
        assert!(error.contains(expected), "{label}: {error}");
        assert!(provider.inputs.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn mapped_dependency_collision_with_service_collection_fails_closed() {
    let fixture = CollectionMappingFixture::new(
        BTreeMap::from([(
            "package_secret".to_string(),
            "mapped_package_secret".to_string(),
        )]),
        Some("mapped_package_secret"),
    );
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let controller = AssemblyAdmissionController::new(
        "runtime-service-collision",
        skiff_runtime_capability_context::DbProviderSource::new(CapturingDbProvider::default()),
    );

    let error = controller
        .recover_committed(
            "fixture",
            1,
            &reference,
            &fixture.resolver,
            Some(&mapping_service_db()),
        )
        .await
        .expect_err("service/dependency collection collision must fail closed");
    let error = format!("{error:#}");
    assert!(error.contains("collides between"), "{error}");
}

#[tokio::test]
async fn mapped_targets_from_distinct_dependencies_cannot_collide() {
    let fixture = CollectionMappingFixture::with_dependency_target_collision();
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let provider = CapturingDbProvider::default();
    let controller = AssemblyAdmissionController::new(
        "runtime-dependency-collision",
        skiff_runtime_capability_context::DbProviderSource::new(provider.clone()),
    );

    let error = controller
        .recover_committed(
            "fixture",
            1,
            &reference,
            &fixture.resolver,
            Some(&mapping_service_db()),
        )
        .await
        .expect_err("two dependency collection targets must not collide");
    let error = format!("{error:#}");
    assert!(error.contains("collides between"), "{error}");
    assert!(
        error.contains("store") && error.contains("cache"),
        "{error}"
    );
    assert!(provider.inputs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn assembly_collection_mapping_drift_fails_before_host_activation() {
    let mut fixture = CollectionMappingFixture::new(
        BTreeMap::from([(
            "package_secret".to_string(),
            "mapped_package_secret".to_string(),
        )]),
        None,
    );
    fixture.assembly.package_link_plan.package_links[0]
        .collection_name_mapping
        .insert(
            "package_secret".to_string(),
            "drifted_package_secret".to_string(),
        );
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut fixture.assembly).unwrap();
    fixture.resolver.assembly = Arc::new(fixture.assembly.clone());
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let provider = CapturingDbProvider::default();
    let controller = AssemblyAdmissionController::new(
        "runtime-drift",
        skiff_runtime_capability_context::DbProviderSource::new(provider.clone()),
    );

    let error = controller
        .recover_committed(
            "fixture",
            1,
            &reference,
            &fixture.resolver,
            Some(&mapping_service_db()),
        )
        .await
        .expect_err("deployment/assembly mapping drift must fail closed");
    let error = format!("{error:#}");
    assert!(
        error.contains("collection mapping") || error.contains("canonical link plan"),
        "{error}"
    );
    assert!(provider.inputs.lock().unwrap().is_empty());
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
        runtime_requirements: PackageRuntimeRequirements {
            config: Vec::new(),
            state: Vec::new(),
            resources: Vec::new(),
            runtime_capabilities: Vec::new(),
        },
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
                    runtime_capabilities: Vec::new(),
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

fn policy() -> DeploymentPolicy {
    DeploymentPolicy {
        timeout_ms: Some(1_000),
        resources: ResourcePolicy {
            cpu_millis: 100,
            memory_bytes: 1_048_576,
        },
        activation: ActivationPolicy {
            max_concurrency: 4,
            idle_timeout_ms: None,
        },
        principal: "service:phase-three".to_string(),
    }
}
