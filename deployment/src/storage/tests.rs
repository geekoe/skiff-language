use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::json;
use skiff_artifact_identity::{
    assign_file_ir_identity, assign_package_artifact_identities,
    assign_service_contract_identities, contract_operation_id, package_artifact_ref,
    package_schema_index_identity, runtime_assembly_ref, service_contract_ref,
    service_deployment_ref, EnvironmentActivationStatePath, PackageArtifactRecordPath,
};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryErrorContract, BoundaryOperationContract, BoundaryOperationDescriptor, BoundaryReturn,
    BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, ContractDiagnosticText, FileIrRef, FileIrUnit,
    PackageArtifact, PackageBuildId, PackageImplementationLinks, PackageLocalAbi,
    PackageLocalAbiIdentity, PackageRuntimeRequirements, PackageSchemaIndexRef, ServiceContract,
    ServiceProtocolIdentity, PACKAGE_ARTIFACT_SCHEMA_VERSION, SERVICE_CONTRACT_SCHEMA_VERSION,
};

use super::*;
use crate::fixtures::{
    empty_runtime_assembly_fixture, runtime_assembly_fixture, service_deployment_fixture,
};

static TEST_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let sequence = TEST_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-ecosystem-storage-test-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create temp root");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn test_store() -> (TestRoot, CanonicalArtifactStore) {
    let temp = TestRoot::new();
    let store = CanonicalArtifactStore::create(temp.path()).expect("artifact store");
    (temp, store)
}

fn package_fixture() -> PackageArtifact {
    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: "example.com/checkpoint".to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: Vec::new(),
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: "example.com/checkpoint".to_string(),
            package_schema_index_identity: package_schema_index_identity(
                "example.com/checkpoint",
                &BTreeMap::new(),
            )
            .unwrap(),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements {
            config: Vec::new(),
            state: Vec::new(),
            resources: Vec::new(),
            runtime_capabilities: Vec::new(),
        },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    };
    assign_package_artifact_identities(&mut artifact).expect("package identities");
    artifact
}

fn contract_fixture() -> ServiceContract {
    let service_id = "example.com/checkpoint";
    let version = "1.0.0";
    let operation_id = contract_operation_id(service_id, version, "health").unwrap();
    let descriptor = BoundaryOperationDescriptor {
        operation_id: operation_id.clone(),
        stable_key: "health".to_string(),
        contract: BoundaryOperationContract {
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
            errors: BoundaryErrorContract::None,
            stream: BoundaryStreamContract::Unary,
            cancellation: BoundaryCancellationContract::NotCancellable,
            callbacks: BoundaryCallbackContract::None,
            may_suspend: false,
            effect_guarantee: BoundaryEffectGuarantee {
                detached_parameters: true,
                detached_return: true,
                detached_error: true,
                no_caller_reachable_mutation: true,
                no_caller_value_escape: true,
                no_same_heap_identity: true,
            },
        },
    };
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(operation_id.clone(), descriptor)]),
        package_type_requirements: Vec::new(),
        diagnostic_text: ContractDiagnosticText {
            service: "Checkpoint".to_string(),
            operations: BTreeMap::from([(operation_id, "Health".to_string())]),
            types: BTreeMap::new(),
        },
    };
    assign_service_contract_identities(&mut contract).expect("contract identities");
    contract
}

#[test]
fn four_typed_records_round_trip_as_identical_canonical_bytes_and_pointers_cas() {
    let (_temp, store) = test_store();
    let package = package_fixture();
    let contract = contract_fixture();
    let deployment = service_deployment_fixture().expect("deployment");
    let assembly = empty_runtime_assembly_fixture().expect("assembly");

    let package_path = store.write_package_artifact(&package).unwrap();
    let contract_path = store.write_service_contract(&contract).unwrap();
    let deployment_path = store.write_service_deployment(&deployment).unwrap();
    let assembly_path = store.write_runtime_assembly(&assembly).unwrap();

    let package_ref = package_artifact_ref(&package).unwrap();
    let contract_ref = service_contract_ref(&contract).unwrap();
    let deployment_ref = service_deployment_ref(&deployment);
    let assembly_ref = runtime_assembly_ref(&assembly).unwrap();

    let round_trips = [
        (
            package_path,
            skiff_canonical_json::canonical_json_bytes(
                store.read_package_artifact(&package_ref).unwrap().as_ref(),
            )
            .unwrap(),
        ),
        (
            contract_path,
            skiff_canonical_json::canonical_json_bytes(
                store.read_service_contract(&contract_ref).unwrap().as_ref(),
            )
            .unwrap(),
        ),
        (
            deployment_path,
            skiff_canonical_json::canonical_json_bytes(
                store
                    .read_service_deployment(&deployment_ref)
                    .unwrap()
                    .as_ref(),
            )
            .unwrap(),
        ),
        (
            assembly_path,
            skiff_canonical_json::canonical_json_bytes(
                store.read_runtime_assembly(&assembly_ref).unwrap().as_ref(),
            )
            .unwrap(),
        ),
    ];
    for (path, round_trip) in round_trips {
        assert_eq!(fs::read(path).unwrap(), round_trip);
    }

    let package_pointer = PackageArtifactPointer::new(package_ref).unwrap();
    store
        .compare_and_swap_package_artifact_pointer(None, &package_pointer)
        .unwrap();
    assert_eq!(
        store
            .read_package_artifact_pointer("example.com/checkpoint", "1.0.0")
            .unwrap(),
        Some(package_pointer.clone())
    );
    assert!(store
        .compare_and_swap_package_artifact_pointer(None, &package_pointer)
        .is_err());

    let contract_pointer = ServiceContractPointer::new(contract_ref).unwrap();
    store
        .compare_and_swap_service_contract_pointer(None, &contract_pointer)
        .unwrap();
    let deployment_pointer = ServiceDeploymentPointer::new(deployment_ref).unwrap();
    store
        .compare_and_swap_service_deployment_pointer(None, &deployment_pointer)
        .unwrap();
    let assembly_pointer = RuntimeAssemblyPointer::new("stable", assembly_ref).unwrap();
    store
        .compare_and_swap_runtime_assembly_pointer(None, &assembly_pointer)
        .unwrap();
}

#[test]
fn activation_storage_coordinate_collision_pair_has_independent_records_and_cas() {
    let (_temp, store) = test_store();
    let mut slash = package_fixture();
    slash.package_id = "a.b/c/d".to_string();
    slash.package_schema_index = PackageSchemaIndexRef {
        package_id: slash.package_id.clone(),
        package_schema_index_identity: package_schema_index_identity(
            &slash.package_id,
            &BTreeMap::new(),
        )
        .unwrap(),
    };
    assign_package_artifact_identities(&mut slash).unwrap();
    let mut adjacent_dots = package_fixture();
    adjacent_dots.package_id = "a.b/c..d".to_string();
    adjacent_dots.package_schema_index = PackageSchemaIndexRef {
        package_id: adjacent_dots.package_id.clone(),
        package_schema_index_identity: package_schema_index_identity(
            &adjacent_dots.package_id,
            &BTreeMap::new(),
        )
        .unwrap(),
    };
    assign_package_artifact_identities(&mut adjacent_dots).unwrap();

    let slash_path = store.write_package_artifact(&slash).unwrap();
    let adjacent_dots_path = store.write_package_artifact(&adjacent_dots).unwrap();
    assert_ne!(slash_path, adjacent_dots_path);
    assert!(slash_path.is_file());
    assert!(adjacent_dots_path.is_file());

    let slash_ref = package_artifact_ref(&slash).unwrap();
    let adjacent_dots_ref = package_artifact_ref(&adjacent_dots).unwrap();
    assert_eq!(
        store.read_package_artifact(&slash_ref).unwrap().package_id,
        slash.package_id
    );
    assert_eq!(
        store
            .read_package_artifact(&adjacent_dots_ref)
            .unwrap()
            .package_id,
        adjacent_dots.package_id
    );
    let slash_pointer = PackageArtifactPointer::new(slash_ref.clone()).unwrap();
    let adjacent_dots_pointer = PackageArtifactPointer::new(adjacent_dots_ref.clone()).unwrap();
    store
        .compare_and_swap_package_artifact_pointer(None, &slash_pointer)
        .unwrap();
    store
        .compare_and_swap_package_artifact_pointer(None, &adjacent_dots_pointer)
        .unwrap();

    assert_eq!(
        store
            .read_package_artifact_pointer(&slash_ref.package_id, &slash_ref.package_version)
            .unwrap(),
        Some(slash_pointer)
    );
    assert_eq!(
        store
            .read_package_artifact_pointer(
                &adjacent_dots_ref.package_id,
                &adjacent_dots_ref.package_version,
            )
            .unwrap(),
        Some(adjacent_dots_pointer)
    );
}

#[test]
fn storage_rejects_tamper_unknown_duplicate_missing_and_cross_root_content() {
    let (_temp, store) = test_store();
    let package = package_fixture();
    let reference = package_artifact_ref(&package).unwrap();
    let path = store.write_package_artifact(&package).unwrap();

    let original = fs::read(&path).unwrap();
    let tampered = String::from_utf8(original.clone())
        .unwrap()
        .replace(reference.package_build_id.as_str(), "tampered");
    fs::write(&path, tampered).unwrap();
    assert!(store.read_package_artifact(&reference).is_err());

    fs::write(&path, &original).unwrap();
    let mut unknown: serde_json::Value = serde_json::from_slice(&original).unwrap();
    unknown
        .as_object_mut()
        .unwrap()
        .insert("legacy".to_string(), json!(true));
    fs::write(&path, serde_json::to_vec(&unknown).unwrap()).unwrap();
    assert!(store.read_package_artifact(&reference).is_err());

    let duplicate = String::from_utf8(original.clone()).unwrap().replacen(
        '{',
        "{\"schemaVersion\":\"duplicate\",",
        1,
    );
    fs::write(&path, duplicate).unwrap();
    assert!(store.read_package_artifact(&reference).is_err());

    fs::remove_file(&path).unwrap();
    assert!(store.read_package_artifact(&reference).is_err());

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let (_outside_temp, outside) = test_store();
        let canonical_path = PackageArtifactRecordPath::new(&reference).unwrap();
        let link = store
            .root()
            .join(canonical_path.as_relative_path().as_path());
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        let outside_file = outside.root().join("outside.json");
        fs::write(&outside_file, original).unwrap();
        symlink(outside_file, link).unwrap();
        assert!(store.read_package_artifact(&reference).is_err());
        assert!(store.write_package_artifact(&package).is_err());
    }
}

#[test]
fn file_and_resource_records_validate_exact_identity_path_and_content() {
    use sha2::{Digest, Sha256};

    let (_temp, store) = test_store();
    let mut package = package_fixture();
    let mut file = FileIrUnit::empty("checkpoint.main", "source-hash");
    assign_file_ir_identity(&mut file).unwrap();
    let resource = b"checkpoint-resource";
    let resource_hash = hex::encode(Sha256::digest(resource));
    package.files.push(FileIrRef::new(
        file.file_ir_identity.clone(),
        file.module_path.clone(),
    ));
    package
        .static_resources
        .push(skiff_artifact_model::PublicationResourceRef {
            path: "assets/checkpoint.txt".to_string(),
            sha256: resource_hash,
            byte_len: resource.len() as u64,
            content_type: Some("text/plain".to_string()),
            artifact_path: None,
        });
    assign_package_artifact_identities(&mut package).unwrap();
    let package_ref = package_artifact_ref(&package).unwrap();
    store.write_package_artifact(&package).unwrap();
    store
        .write_file_ir(&package_ref, &package.files[0], &file)
        .unwrap();
    store
        .write_static_resource(&package_ref, &package.static_resources[0], resource)
        .unwrap();
    assert_eq!(
        store
            .read_file_ir(&package_ref, &package.files[0])
            .unwrap()
            .file_ir_identity,
        file.file_ir_identity
    );
    assert_eq!(
        store
            .read_static_resource(&package_ref, &package.static_resources[0])
            .unwrap()
            .as_ref(),
        resource
    );
}

#[test]
fn activation_prepare_abort_commit_and_crash_recovery_are_fail_closed() {
    let (_temp, store) = test_store();
    let committed_assembly = empty_runtime_assembly_fixture().unwrap();
    let candidate_assembly = runtime_assembly_fixture().unwrap();
    store.write_runtime_assembly(&committed_assembly).unwrap();
    store.write_runtime_assembly(&candidate_assembly).unwrap();
    let committed_ref = runtime_assembly_ref(&committed_assembly).unwrap();
    let candidate_ref = runtime_assembly_ref(&candidate_assembly).unwrap();

    let initial = EnvironmentActivationState::initial("test", 7, committed_ref.clone());
    store.initialize_environment_activation(&initial).unwrap();
    let committed_bytes = skiff_canonical_json::canonical_json_bytes(&initial.committed).unwrap();
    let state_path = EnvironmentActivationStatePath::new("test").unwrap();
    let state_host_path = store.root().join(state_path.as_relative_path().as_path());
    fs::write(
        state_host_path.with_file_name(".activation.json.tmp-crash"),
        b"{",
    )
    .unwrap();
    assert_eq!(store.read_environment_activation("test").unwrap(), initial);

    assert!(store
        .prepare_environment_activation(
            "test",
            "rollback",
            7,
            7,
            candidate_ref.clone(),
            vec!["replica-a".to_string()],
        )
        .is_err());
    assert_eq!(store.read_environment_activation("test").unwrap(), initial);
    assert!(store
        .prepare_environment_activation(
            "test",
            "wrong-assembly-domain",
            7,
            8,
            skiff_artifact_model::RuntimeAssemblyRef {
                assembly_identity: skiff_artifact_model::AssemblyIdentity::new(
                    "skiff-service-protocol-v2:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ),
            },
            vec!["replica-a".to_string()],
        )
        .is_err());
    assert_eq!(store.read_environment_activation("test").unwrap(), initial);

    let prepared = store
        .prepare_environment_activation(
            "test",
            "activation-8",
            7,
            8,
            candidate_ref.clone(),
            vec!["replica-b".to_string(), "replica-a".to_string()],
        )
        .unwrap();
    assert_eq!(prepared.committed, initial.committed);
    assert_eq!(
        prepared.pending.as_ref().unwrap().participant_replica_ids,
        ["replica-a", "replica-b"]
    );
    assert!(store
        .prepare_environment_activation(
            "test",
            "stale",
            6,
            7,
            candidate_ref.clone(),
            vec!["replica-a".to_string()],
        )
        .is_err());
    assert!(store
        .commit_environment_activation(
            "test",
            "activation-8",
            7,
            8,
            &candidate_ref,
            &["replica-a".to_string(), "replica-b".to_string()],
            &["replica-a".to_string()],
        )
        .is_err());

    assert_eq!(
        prepared
            .recovery_action(&["replica-a".to_string()], &["replica-a".to_string()])
            .unwrap(),
        ActivationRecoveryAction::AbortPending {
            activation_id: "activation-8".to_string()
        }
    );
    assert_eq!(
        prepared
            .recovery_action(
                &["replica-a".to_string(), "replica-b".to_string()],
                &["replica-a".to_string(), "replica-b".to_string()]
            )
            .unwrap(),
        ActivationRecoveryAction::CommitPending
    );
    assert_eq!(
        prepared
            .recovery_action(
                &["replica-a".to_string(), "replica-b".to_string()],
                &["replica-a".to_string()]
            )
            .unwrap(),
        ActivationRecoveryAction::ReplayPrepare {
            replica_ids: vec!["replica-b".to_string()]
        }
    );

    let aborted = store
        .abort_environment_activation("test", "activation-8", 7)
        .unwrap();
    assert!(aborted.pending.is_none());
    assert_eq!(
        skiff_canonical_json::canonical_json_bytes(&aborted.committed).unwrap(),
        committed_bytes
    );

    store
        .prepare_environment_activation(
            "test",
            "activation-8b",
            7,
            8,
            candidate_ref.clone(),
            vec!["replica-a".to_string(), "replica-b".to_string()],
        )
        .unwrap();
    let committed = store
        .commit_environment_activation(
            "test",
            "activation-8b",
            7,
            8,
            &candidate_ref,
            &["replica-b".to_string(), "replica-a".to_string()],
            &["replica-b".to_string(), "replica-a".to_string()],
        )
        .unwrap();
    assert_eq!(committed.committed.generation, 8);
    assert!(committed.pending.is_none());
    assert_eq!(
        committed.recovery_action(&[], &[]).unwrap(),
        ActivationRecoveryAction::StableCommitted
    );
    assert_eq!(
        store
            .commit_environment_activation("test", "activation-8b", 7, 8, &candidate_ref, &[], &[],)
            .unwrap(),
        committed,
        "post-commit notification replay must be idempotent"
    );
    assert!(store
        .commit_environment_activation("test", "activation-8b", 6, 8, &candidate_ref, &[], &[],)
        .is_err());
    assert!(store
        .commit_environment_activation("test", "", 7, 8, &candidate_ref, &[], &[],)
        .is_err());
}
