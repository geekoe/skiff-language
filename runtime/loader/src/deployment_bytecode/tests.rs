use super::*;

use skiff_artifact_model::{
    bytecode::opcodes::opcode_table_fingerprint, BytecodeArtifact, BytecodeImage, BytecodePools,
    ContractDiagnosticText, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentRevision, FrozenConstantGraph, FrozenConstantNode, LiteralIr,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
    PackageRuntimeRequirements, PackageSchemaIndexIdentity, PackageSchemaIndexRef,
    ServiceProtocolIdentity, ServiceSelectorBinding, BYTECODE_ISA_VERSION, BYTECODE_MAGIC,
    BYTECODE_SCHEMA_VERSION, PACKAGE_ARTIFACT_SCHEMA_VERSION, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};

fn admitted_bytecode(seed: &str) -> Arc<ValidatedBytecodeArtifact> {
    let mut artifact = BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: opcode_table_fingerprint(),
        bytecode_identity: "unassigned".to_string(),
        image: BytecodeImage {
            functions: BTreeMap::new(),
            pools: BytecodePools {
                constants: Vec::new(),
                types: Vec::new(),
                shapes: Vec::new(),
                effects: Vec::new(),
                resume: Vec::new(),
                callback_capture: Vec::new(),
            },
            frozen_constant_graph: FrozenConstantGraph {
                nodes: vec![FrozenConstantNode::Literal {
                    literal: LiteralIr::String {
                        value: seed.to_string(),
                    },
                }],
            },
            debug_table: None,
        },
    };
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
}

fn package_artifact(
    package_id: &str,
    build_id: &str,
    bytecode: Option<BytecodeArtifactRef>,
) -> Arc<PackageArtifact> {
    Arc::new(PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new(build_id),
        files: Vec::new(),
        static_resources: Vec::new(),
        bytecode,
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new(format!("abi:{package_id}")),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: PackageSchemaIndexIdentity::new(format!(
                "schema:{package_id}"
            )),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks {
            types: BTreeMap::new(),
            constants: BTreeMap::new(),
            functions: BTreeMap::new(),
            impl_methods: BTreeMap::new(),
            operation_targets: BTreeMap::new(),
        },
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    })
}

fn package_reference(artifact: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: artifact.package_id.clone(),
        package_version: artifact.package_version.clone(),
        package_build_id: artifact.package_build_id.clone(),
        package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
    }
}

fn contract_reference(service_id: &str) -> ServiceContractRef {
    ServiceContractRef {
        service_id: service_id.to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new(format!("protocol:{service_id}")),
    }
}

fn contract(reference: &ServiceContractRef) -> Arc<ServiceContract> {
    Arc::new(ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: reference.service_id.clone(),
        contract_version: reference.contract_version.clone(),
        service_protocol_identity: reference.service_protocol_identity.clone(),
        operations: BTreeMap::new(),
        package_type_requirements: Vec::new(),
        diagnostic_text: ContractDiagnosticText {
            service: reference.service_id.clone(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    })
}

fn deployment(
    implementation: PackageArtifactRef,
    contract: ServiceContractRef,
    service_selectors: Vec<ServiceSelectorBinding>,
) -> Arc<ServiceDeployment> {
    Arc::new(ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract,
        deployment_revision: DeploymentRevision::new("revision:consumer"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("deployment:consumer"),
        implementation,
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors,
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "consumer".to_string(),
            notes: BTreeMap::new(),
        },
    })
}

fn deployment_reference(deployment: &ServiceDeployment) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: deployment.contract.service_id.clone(),
        contract_version: deployment.contract.contract_version.clone(),
        deployment_revision: deployment.deployment_revision.clone(),
        deployment_artifact_identity: deployment.deployment_artifact_identity.clone(),
    }
}

fn hydrated_package(
    package_id: &str,
    build_id: &str,
    bytecode: &Arc<ValidatedBytecodeArtifact>,
) -> HydratedBytecodePackage {
    let artifact = package_artifact(package_id, build_id, Some(bytecode.reference().clone()));
    HydratedBytecodePackage::checked(package_reference(&artifact), artifact, Arc::clone(bytecode))
        .unwrap()
}

#[test]
fn package_checked_constructor_admits_only_exact_token_and_exposes_opaque_getters() {
    let bytecode = admitted_bytecode("exact");
    let artifact = package_artifact(
        "example.package",
        "build:package",
        Some(bytecode.reference().clone()),
    );
    let reference = package_reference(&artifact);
    let hydrated = HydratedBytecodePackage::checked(
        reference.clone(),
        Arc::clone(&artifact),
        Arc::clone(&bytecode),
    )
    .unwrap();

    assert_eq!(hydrated.reference(), &reference);
    assert!(Arc::ptr_eq(hydrated.artifact(), &artifact));
    assert!(Arc::ptr_eq(hydrated.bytecode(), &bytecode));
}

#[test]
fn package_checked_constructor_classifies_missing_and_mismatched_bytecode() {
    let bytecode = admitted_bytecode("expected");
    let missing = package_artifact("example.missing", "build:missing", None);
    let missing_reference = package_reference(&missing);
    assert!(matches!(
        HydratedBytecodePackage::checked(
            missing_reference.clone(),
            missing,
            Arc::clone(&bytecode)
        ),
        Err(DeploymentBytecodeHydrationError::MissingBytecode { package })
            if package == missing_reference
    ));

    let other = admitted_bytecode("other");
    let mismatched = package_artifact(
        "example.mismatch",
        "build:mismatch",
        Some(bytecode.reference().clone()),
    );
    let mismatched_reference = package_reference(&mismatched);
    assert!(matches!(
        HydratedBytecodePackage::checked(mismatched_reference, mismatched, other),
        Err(DeploymentBytecodeHydrationError::ReferenceMismatch {
            expected: DeploymentBytecodeReference::PackageBytecode { .. },
            actual: DeploymentBytecodeReference::PackageBytecode { .. },
        })
    ));
}

#[test]
fn deployment_checked_constructor_canonicalizes_consumer_facts() {
    let bytecode = admitted_bytecode("shared");
    let first = hydrated_package("example.a", "build:a", &bytecode);
    let first_reference = first.reference().clone();
    let second = hydrated_package("example.b", "build:b", &bytecode);
    let own_contract = contract_reference("example.consumer");
    let dependency_contract = contract_reference("example.provider");
    let dependency_key = ServiceRequirementKey {
        caller_package_build_id: first_reference.package_build_id.clone(),
        service_requirement_slot: 4,
    };
    let deployment = deployment(
        first_reference,
        own_contract.clone(),
        vec![ServiceSelectorBinding {
            key: dependency_key.clone(),
            contract: dependency_contract.clone(),
        }],
    );
    let reference = deployment_reference(&deployment);
    let contracts = BTreeMap::from([
        (own_contract.clone(), contract(&own_contract)),
        (dependency_contract.clone(), contract(&dependency_contract)),
    ]);
    let dependency = HydratedServiceDependency::new(
        dependency_key.clone(),
        dependency_contract.clone(),
        BTreeSet::from([ContractOperationId::new("operation:provider.call")]),
    );

    let hydrated = HydratedDeploymentBytecode::checked(
        reference.clone(),
        Arc::clone(&deployment),
        contracts,
        vec![dependency],
        vec![second, first],
    )
    .unwrap();

    assert_eq!(hydrated.reference(), &reference);
    assert!(Arc::ptr_eq(hydrated.deployment(), &deployment));
    assert_eq!(hydrated.contract_store().len(), 2);
    let row = hydrated
        .service_dependencies()
        .get(&dependency_key)
        .unwrap();
    assert_eq!(row.key(), &dependency_key);
    assert_eq!(row.contract(), &dependency_contract);
    assert!(row
        .used_operations()
        .contains(&ContractOperationId::new("operation:provider.call")));
    assert_eq!(
        hydrated.packages().keys().cloned().collect::<Vec<_>>(),
        vec![
            PackageBuildId::new("build:a"),
            PackageBuildId::new("build:b")
        ]
    );
}

#[test]
fn deployment_checked_constructor_rejects_duplicate_package_and_service_slot() {
    let bytecode = admitted_bytecode("duplicates");
    let package = hydrated_package("example.package", "build:package", &bytecode);
    let package_again = hydrated_package("example.package", "build:package", &bytecode);
    let package_reference = package.reference().clone();
    let own_contract = contract_reference("example.consumer");
    let deployment_record = deployment(package_reference, own_contract.clone(), Vec::new());
    let reference = deployment_reference(&deployment_record);
    let contracts = BTreeMap::from([(own_contract.clone(), contract(&own_contract))]);
    assert!(matches!(
        HydratedDeploymentBytecode::checked(
            reference,
            deployment_record,
            contracts,
            Vec::new(),
            vec![package, package_again],
        ),
        Err(DeploymentBytecodeHydrationError::DuplicatePackage { .. })
    ));

    let package = hydrated_package("example.package", "build:package", &bytecode);
    let package_reference = package.reference().clone();
    let dependency_contract = contract_reference("example.provider");
    let key = ServiceRequirementKey {
        caller_package_build_id: package_reference.package_build_id.clone(),
        service_requirement_slot: 1,
    };
    let deployment = deployment(
        package_reference,
        own_contract.clone(),
        vec![ServiceSelectorBinding {
            key: key.clone(),
            contract: dependency_contract.clone(),
        }],
    );
    let reference = deployment_reference(&deployment);
    let contracts = BTreeMap::from([
        (own_contract.clone(), contract(&own_contract)),
        (dependency_contract.clone(), contract(&dependency_contract)),
    ]);
    let dependency = || {
        HydratedServiceDependency::new(key.clone(), dependency_contract.clone(), BTreeSet::new())
    };
    assert!(matches!(
        HydratedDeploymentBytecode::checked(
            reference,
            deployment,
            contracts,
            vec![dependency(), dependency()],
            vec![package],
        ),
        Err(DeploymentBytecodeHydrationError::DuplicateServiceSlot { key: duplicate })
            if duplicate == key
    ));
}

#[test]
fn deployment_checked_constructor_rejects_contract_mismatch() {
    let bytecode = admitted_bytecode("contract-mismatch");
    let package = hydrated_package("example.package", "build:package", &bytecode);
    let package_reference = package.reference().clone();
    let own_contract = contract_reference("example.consumer");
    let expected_contract = contract_reference("example.expected");
    let actual_contract = contract_reference("example.actual");
    let key = ServiceRequirementKey {
        caller_package_build_id: package_reference.package_build_id.clone(),
        service_requirement_slot: 7,
    };
    let deployment = deployment(
        package_reference,
        own_contract.clone(),
        vec![ServiceSelectorBinding {
            key: key.clone(),
            contract: expected_contract.clone(),
        }],
    );
    let reference = deployment_reference(&deployment);
    let contracts = BTreeMap::from([
        (own_contract.clone(), contract(&own_contract)),
        (expected_contract.clone(), contract(&expected_contract)),
        (actual_contract.clone(), contract(&actual_contract)),
    ]);
    let dependency =
        HydratedServiceDependency::new(key.clone(), actual_contract.clone(), BTreeSet::new());

    assert!(matches!(
        HydratedDeploymentBytecode::checked(
            reference,
            deployment,
            contracts,
            vec![dependency],
            vec![package],
        ),
        Err(DeploymentBytecodeHydrationError::ContractMismatch {
            key: Some(mismatch_key),
            expected: Some(expected),
            actual: Some(actual),
        }) if mismatch_key == key && expected == expected_contract && actual == actual_contract
    ));
}

mod load;
