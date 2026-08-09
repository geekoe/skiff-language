use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::json;
use skiff_artifact_identity::{
    assign_bytecode_identity, assign_file_ir_identity, assign_package_artifact_identities,
    assign_service_contract_identities, assign_service_deployment_identity, contract_operation_id,
    package_artifact_ref, package_schema_index_identity, runtime_assembly_ref,
    service_contract_ref, service_deployment_ref, PackageArtifactRecordPath,
    PackageBytecodeRecordPath, ReleasePointerPath, BYTECODE_IDENTITY_PREFIX,
};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryOperationDescriptor, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    BytecodeArtifact, BytecodeArtifactRef, BytecodeFunctionOrigin, BytecodeImage,
    BytecodePoolEntry, BytecodePools, ContractDiagnosticText, DebugBinding, DebugTable,
    DeploymentArtifactIdentity, DeploymentRevision, FileIrRef, FileIrUnit, FrameLayout,
    FrozenConstantGraph, PackageArtifact, PackageBuildId, PackageCallableId,
    PackageExecutableCoordinate, PackageImplementationLinks, PackageLocalAbi,
    PackageLocalAbiIdentity, PackageRuntimeRequirements, PackageSchemaIndex, PackageSchemaIndexRef,
    RelocatableBytecodeFunction, ServiceContract, ServiceProtocolIdentity, StatementChargeKind,
    StatementEntry, TypeRefIr, ValueDropPlan, ValueTransferPlan, BYTECODE_ISA_VERSION,
    BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION, PACKAGE_ARTIFACT_SCHEMA_VERSION,
    SERVICE_CONTRACT_SCHEMA_VERSION,
};

use super::*;
use crate::fixtures::{empty_runtime_assembly_fixture, service_deployment_fixture};

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
        bytecode: None,
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
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
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    };
    assign_package_artifact_identities(&mut artifact).expect("package identities");
    artifact
}

fn empty_schema_index(artifact: &PackageArtifact) -> PackageSchemaIndex {
    PackageSchemaIndex {
        package_id: artifact.package_schema_index.package_id.clone(),
        package_schema_index_identity: artifact
            .package_schema_index
            .package_schema_index_identity
            .clone(),
        types: BTreeMap::new(),
    }
}

fn declared_package_ref(artifact: &PackageArtifact) -> skiff_artifact_model::PackageArtifactRef {
    skiff_artifact_model::PackageArtifactRef {
        package_id: artifact.package_id.clone(),
        package_version: artifact.package_version.clone(),
        package_build_id: artifact.package_build_id.clone(),
        package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
    }
}

fn package_copy_fixture() -> (PackageArtifact, FileIrUnit, Vec<u8>) {
    use sha2::{Digest, Sha256};

    let mut artifact = package_fixture();
    let mut file = FileIrUnit::empty("checkpoint.copy", "copy-source-hash");
    assign_file_ir_identity(&mut file).unwrap();
    let resource = b"package-copy-resource".to_vec();
    artifact.files.push(FileIrRef::new(
        file.file_ir_identity.clone(),
        file.module_path.clone(),
    ));
    artifact
        .static_resources
        .push(skiff_artifact_model::PublicationResourceRef {
            path: "assets/copy.txt".to_string(),
            sha256: hex::encode(Sha256::digest(&resource)),
            byte_len: resource.len() as u64,
            content_type: Some("text/plain".to_string()),
            artifact_path: None,
        });
    assign_package_artifact_identities(&mut artifact).unwrap();
    (artifact, file, resource)
}

fn write_package_copy_closure(
    store: &CanonicalArtifactStore,
    artifact: &PackageArtifact,
    file: &FileIrUnit,
    resource: &[u8],
) {
    let reference = package_artifact_ref(artifact).unwrap();
    store
        .write_package_schema_index(&empty_schema_index(artifact))
        .unwrap();
    store
        .write_file_ir(&reference, &artifact.files[0], file)
        .unwrap();
    store
        .write_static_resource(&reference, &artifact.static_resources[0], resource)
        .unwrap();
    store.write_package_artifact(artifact).unwrap();
}

#[test]
fn package_copy_admission_cache_is_content_identity_and_source_exact() {
    let (_source_root, source) = test_store();
    let (_target_root, target) = test_store();
    let (_other_root, other_source) = test_store();
    let (first, file, resource) = package_copy_fixture();
    let mut second = first.clone();
    second.package_version = "2.0.0".to_string();
    assign_package_artifact_identities(&mut second).unwrap();
    let first_ref = package_artifact_ref(&first).unwrap();
    let second_ref = package_artifact_ref(&second).unwrap();
    write_package_copy_closure(&source, &first, &file, &resource);
    write_package_copy_closure(&source, &second, &file, &resource);
    write_package_copy_closure(&other_source, &first, &file, &resource);

    let mut cache = PackageArtifactAdmissionCache::default();
    {
        let admitted = cache.admit(&source, &first_ref).unwrap();
        assert_eq!(admitted.reference(), &first_ref);
        target
            .write_validated_package_copy_records(admitted)
            .unwrap();
    }
    assert_eq!(
        target
            .read_file_ir(&first_ref, &first.files[0])
            .unwrap()
            .as_ref(),
        &file
    );
    assert_eq!(
        target
            .read_static_resource(&first_ref, &first.static_resources[0])
            .unwrap()
            .as_ref(),
        resource
    );
    assert_eq!(cache.admission_count(), 1);
    cache.admit(&source, &first_ref).unwrap();
    assert_eq!(
        cache.admission_count(),
        1,
        "same source root and identity must not repeat full admission"
    );

    cache.admit(&source, &second_ref).unwrap();
    assert_eq!(
        cache.admission_count(),
        2,
        "a different artifact identity requires its own admission"
    );
    cache.admit(&other_source, &first_ref).unwrap();
    assert_eq!(
        cache.admission_count(),
        3,
        "an identical record from another source root cannot reuse admission"
    );

    let package_path = PackageArtifactRecordPath::new(&first_ref).unwrap();
    let package_host_path = source
        .root()
        .join(package_path.as_relative_path().as_path());
    let original_package = fs::read(&package_host_path).unwrap();
    let mut tampered_package = original_package.clone();
    tampered_package.push(b'\n');
    fs::write(&package_host_path, tampered_package).unwrap();
    assert!(cache.admit(&source, &first_ref).is_err());
    assert_eq!(cache.admission_count(), 3);
    fs::write(&package_host_path, &original_package).unwrap();

    let schema_path =
        skiff_artifact_identity::PackageSchemaIndexRecordPath::new(&first.package_schema_index)
            .unwrap();
    let schema_host_path = source.root().join(schema_path.as_relative_path().as_path());
    let original_schema = fs::read(&schema_host_path).unwrap();
    let mut tampered_schema = original_schema.clone();
    tampered_schema.push(b'\n');
    fs::write(&schema_host_path, tampered_schema).unwrap();
    assert!(cache.admit(&source, &first_ref).is_err());
    assert_eq!(cache.admission_count(), 3);
    fs::write(&schema_host_path, original_schema).unwrap();

    let file_path =
        skiff_artifact_identity::PackageFileIrRecordPath::new(&first_ref, &first.files[0]).unwrap();
    let file_host_path = source.root().join(file_path.as_relative_path().as_path());
    let original_file = fs::read(&file_host_path).unwrap();
    let mut tampered_file = original_file.clone();
    tampered_file.push(b'\n');
    fs::write(&file_host_path, tampered_file).unwrap();
    assert!(cache.admit(&source, &first_ref).is_err());
    assert_eq!(cache.admission_count(), 3);
    fs::write(&file_host_path, original_file).unwrap();

    let resource_path = skiff_artifact_identity::PackageResourceRecordPath::new(
        &first_ref,
        &first.static_resources[0],
    )
    .unwrap();
    let resource_host_path = source
        .root()
        .join(resource_path.as_relative_path().as_path());
    let original_resource = fs::read(&resource_host_path).unwrap();
    let mut tampered_resource = original_resource.clone();
    tampered_resource.push(b'!');
    fs::write(&resource_host_path, tampered_resource).unwrap();
    assert!(cache.admit(&source, &first_ref).is_err());
    assert_eq!(cache.admission_count(), 3);
    fs::write(&resource_host_path, original_resource).unwrap();

    let target_package_path = target
        .root()
        .join(package_path.as_relative_path().as_path());
    let mut target_tamper = fs::read(&target_package_path).unwrap();
    target_tamper.push(b'\n');
    fs::write(&target_package_path, target_tamper).unwrap();
    let admitted = cache.admit(&source, &first_ref).unwrap();
    assert!(target
        .write_validated_package_copy_records(admitted)
        .is_err());
}

#[test]
fn package_copy_cache_never_admits_an_invalid_declared_identity() {
    let (_source_root, source) = test_store();
    let mut invalid = package_fixture();
    invalid.package_build_id =
        PackageBuildId::new(format!("skiff-package-build-v10:sha256:{}", "0".repeat(64)));
    let invalid_ref = declared_package_ref(&invalid);
    source
        .write_package_schema_index(&empty_schema_index(&invalid))
        .unwrap();
    let path = PackageArtifactRecordPath::new(&invalid_ref).unwrap();
    let host_path = source.root().join(path.as_relative_path().as_path());
    fs::create_dir_all(host_path.parent().unwrap()).unwrap();
    fs::write(
        &host_path,
        skiff_canonical_json::canonical_json_bytes(&invalid).unwrap(),
    )
    .unwrap();

    let mut cache = PackageArtifactAdmissionCache::default();
    assert!(cache.admit(&source, &invalid_ref).is_err());
    assert_eq!(
        cache.admission_count(),
        0,
        "failed first admission must not populate the cache"
    );
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
        },
    };
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(operation_id.clone(), descriptor)]),
        public_instances: BTreeMap::new(),
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
fn coordinate_collision_pair_has_independent_records_and_cas() {
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
fn release_pointer_round_trip_atomic_overwrite_cas_and_unset() {
    let (_temp, store) = test_store();
    let first = service_deployment_fixture().expect("deployment");
    let mut second = first.clone();
    second.deployment_revision = DeploymentRevision::new("revision-2");
    assign_service_deployment_identity(&mut second).expect("deployment identities");
    assert_ne!(
        first.deployment_artifact_identity, second.deployment_artifact_identity,
        "different revisions must produce different buildIds"
    );
    store.write_service_deployment(&first).unwrap();
    store.write_service_deployment(&second).unwrap();

    let first_ref = service_deployment_ref(&first);
    let second_ref = service_deployment_ref(&second);
    let first_pointer = ReleasePointer::new("dev", first_ref).unwrap();
    let second_pointer = ReleasePointer::new("dev", second_ref).unwrap();
    let pointer_path = ReleasePointerPath::new("dev", "example.echo", "1.0.0").unwrap();
    let host_path = store.root().join(pointer_path.as_relative_path().as_path());

    store.write_release_pointer(&first_pointer).unwrap();
    assert_eq!(
        store
            .read_release_pointer("dev", "example.echo", "1.0.0")
            .unwrap(),
        Some(first_pointer.clone())
    );
    assert!(store
        .compare_and_swap_release_pointer(None, &first_pointer)
        .is_err());
    store
        .compare_and_swap_release_pointer(Some(&first_pointer), &first_pointer)
        .unwrap();

    store.write_release_pointer(&second_pointer).unwrap();
    assert_eq!(
        store
            .read_release_pointer("dev", "example.echo", "1.0.0")
            .unwrap(),
        Some(second_pointer.clone())
    );
    assert_eq!(
        store
            .read_service_deployment(&first_pointer.deployment)
            .unwrap()
            .deployment_artifact_identity,
        first.deployment_artifact_identity,
        "the overwritten buildId record must remain readable"
    );
    assert!(store
        .compare_and_swap_release_pointer(Some(&first_pointer), &first_pointer)
        .is_err());

    let original = fs::read(&host_path).unwrap();
    let mut tampered = original.clone();
    tampered.push(b'\n');
    fs::write(&host_path, tampered).unwrap();
    assert!(store
        .read_release_pointer("dev", "example.echo", "1.0.0")
        .is_err());
    fs::write(&host_path, &original).unwrap();

    let missing_ref = skiff_artifact_model::ServiceDeploymentRef {
        service_id: "example.echo".to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new("revision-9"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
            "skiff-deployment-artifact-v4:sha256:{}",
            "f".repeat(64)
        )),
    };
    let missing_pointer = ReleasePointer::new("dev", missing_ref).unwrap();
    assert!(store.write_release_pointer(&missing_pointer).is_err());
    assert!(store
        .compare_and_swap_release_pointer(None, &missing_pointer)
        .is_err());

    let prod_pointer = ReleasePointer::new("prod", first_pointer.deployment.clone()).unwrap();
    store.write_release_pointer(&prod_pointer).unwrap();
    assert_eq!(
        store
            .read_release_pointer("prod", "example.echo", "1.0.0")
            .unwrap(),
        Some(prod_pointer.clone())
    );
    assert_eq!(
        store
            .unset_release_pointer("prod", "example.echo", "1.0.0", None)
            .unwrap(),
        Some(prod_pointer.clone()),
        "unset without expectation must remove an existing pointer"
    );
    assert_eq!(
        store
            .unset_release_pointer("prod", "example.echo", "1.0.0", None)
            .unwrap(),
        None
    );

    assert_eq!(
        store
            .read_release_pointer("dev", "example.echo", "1.0.0")
            .unwrap(),
        Some(second_pointer.clone())
    );
    assert!(store
        .unset_release_pointer("dev", "example.echo", "1.0.0", Some(&first_pointer))
        .is_err());
    assert_eq!(
        store
            .unset_release_pointer("dev", "example.echo", "1.0.0", Some(&second_pointer))
            .unwrap(),
        Some(second_pointer)
    );
    assert_eq!(
        store
            .read_release_pointer("dev", "example.echo", "1.0.0")
            .unwrap(),
        None
    );
    assert!(!host_path.exists());
    assert_eq!(
        store
            .unset_release_pointer("dev", "example.echo", "1.0.0", None)
            .unwrap(),
        None,
        "unsetting an absent pointer must be idempotent"
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

/// Hand-built structurally valid bytecode artifact (passes C1–C8). Not
/// encoder/emitter produced; `bytecode_identity` starts as a placeholder and
/// must be assigned before use.
fn bytecode_fixture() -> BytecodeArtifact {
    let mut functions = BTreeMap::new();
    functions.insert(
        "module::main".to_string(),
        RelocatableBytecodeFunction {
            function_key: "module::main".to_string(),
            origin: BytecodeFunctionOrigin::Executable {
                executable: PackageExecutableCoordinate {
                    file_ir_identity: "file-ir:module".to_string(),
                    module_path: "module".to_string(),
                    executable_index: 0,
                },
            },
            type_parameters: Vec::new(),
            self_type_ref: None,
            words: vec![0x14, 0x25],
            relocations: Vec::new(),
            call_loan_layouts: Vec::new(),
            frame_layout: FrameLayout {
                slot_count: 1,
                slot_type_refs: vec![0],
                parameter_slots: Vec::new(),
                writable_local_slots: Vec::new(),
                result_count: 0,
                result_type_refs: Vec::new(),
                result_plans: Vec::new(),
                slot_plans: vec![ValueTransferPlan::SnapshotShare {
                    drop: ValueDropPlan::Trivial,
                }],
            },
            max_operand_depth: 2,
            effect_summary_ref: PackageCallableId::new("operation:module:main"),
            exception_regions: Vec::new(),
            active_regions: Vec::new(),
            switch_tables: Vec::new(),
            statement_entries: vec![StatementEntry {
                pc: 0,
                statement_id: "s:main:entry".to_string(),
                charge_kind: StatementChargeKind::FunctionEntry,
            }],
            source_map: Vec::new(),
        },
    );
    BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: skiff_artifact_model::bytecode::opcodes::opcode_table_fingerprint(
        ),
        native_value_lifecycle_registry:
            skiff_artifact_model::native_value_lifecycle_registry_identity().clone(),
        bytecode_identity: bytecode_identity_leaf('0'),
        image: BytecodeImage {
            functions,
            pools: BytecodePools {
                types: vec![BytecodePoolEntry::TypeRef {
                    ty: TypeRefIr::builtin("string"),
                }],
                ..BytecodePools::default()
            },
            constant_roots: BTreeMap::new(),
            frozen_constant_graph: FrozenConstantGraph::default(),
            debug_table: Some(DebugTable {
                bindings: vec![DebugBinding {
                    function_key: "module::main".to_string(),
                    pc: 0,
                    name: "x".to_string(),
                    slot: 0,
                }],
            }),
        },
    }
}

fn bytecode_identity_leaf(character: char) -> String {
    format!(
        "{BYTECODE_IDENTITY_PREFIX}:{}",
        std::iter::repeat_n(character, 64).collect::<String>()
    )
}

#[test]
fn bytecode_record_write_read_and_fail_closed_paths() {
    let (_temp, store) = test_store();
    let mut package = package_fixture();
    let mut bytecode = bytecode_fixture();
    assign_bytecode_identity(&mut bytecode).unwrap();
    let reference = BytecodeArtifactRef::new(bytecode.bytecode_identity.clone());
    package.bytecode = Some(reference.clone());
    assign_package_artifact_identities(&mut package).unwrap();
    let package_ref = package_artifact_ref(&package).unwrap();

    // D19: the bytecode record is written before the referencing package
    // record (same order as file-ir records), so readers never observe a
    // package record pointing at a missing bytecode record.
    let written = store
        .write_package_bytecode(&package_ref, &bytecode)
        .unwrap();
    let canonical = PackageBytecodeRecordPath::new(&package_ref, &reference).unwrap();
    assert_eq!(
        written,
        store.root().join(canonical.as_relative_path().as_path())
    );
    store.write_package_artifact(&package).unwrap();
    assert_eq!(
        store
            .read_package_artifact(&package_ref)
            .unwrap()
            .bytecode
            .as_ref(),
        Some(&reference)
    );

    let validated = store
        .read_package_bytecode(&package_ref, &reference)
        .unwrap();
    assert_eq!(validated.artifact(), &bytecode);
    assert!(validated.exactly_matches(&bytecode));
    assert_eq!(validated.reference(), &reference);
    assert_eq!(
        PackageBytecodeRecordPath::new(&package_ref, validated.reference())
            .unwrap()
            .as_str(),
        canonical.as_str()
    );
    assert_eq!(
        validated.artifact().bytecode_identity,
        validated.reference().bytecode_identity
    );
    assert_eq!(
        validated.view().native_value_lifecycle_registry(),
        skiff_artifact_model::native_value_lifecycle_registry_identity()
    );
    assert_eq!(validated.view().functions().len(), 1);
    let stored_function = &validated.view().functions()[0];
    assert_eq!(
        stored_function.origin,
        BytecodeFunctionOrigin::Executable {
            executable: PackageExecutableCoordinate {
                file_ir_identity: "file-ir:module".to_string(),
                module_path: "module".to_string(),
                executable_index: 0,
            },
        }
    );
    assert_eq!(stored_function.self_type_ref, None);
    assert_eq!(stored_function.frame_layout.slot_type_refs, vec![0]);
    assert!(stored_function.frame_layout.writable_local_slots.is_empty());
    assert!(stored_function.frame_layout.result_type_refs.is_empty());
    assert!(stored_function.call_loan_layouts.is_empty());
    assert_eq!(
        stored_function.effect_summary_ref,
        PackageCallableId::new("operation:module:main")
    );

    // The declared artifact path must exactly equal the canonical record path
    // (validate_declared_path, mirroring FileIrRef).
    let mut declared = reference.clone();
    declared.artifact_path = Some(canonical.as_str().to_string());
    assert!(store.read_package_bytecode(&package_ref, &declared).is_ok());
    let mut wrong_path = declared.clone();
    wrong_path.artifact_path = Some(format!(
        "records/package-artifacts/example~dcom~checkpoint/1.0.0/{}/bytecode/{}.json",
        package_ref.package_build_id,
        "0".repeat(64)
    ));
    assert!(store
        .read_package_bytecode(&package_ref, &wrong_path)
        .is_err());

    // Missing record fails closed.
    let missing = BytecodeArtifactRef::new(bytecode_identity_leaf('0'));
    assert!(store.read_package_bytecode(&package_ref, &missing).is_err());

    // Identity mismatch fails closed: tampering the stored record makes the
    // read fail (raw identity check + C9 admission).
    let record_path = store.root().join(canonical.as_relative_path().as_path());
    let tampered = String::from_utf8(fs::read(&record_path).unwrap())
        .unwrap()
        .replace(&bytecode.bytecode_identity, &bytecode_identity_leaf('f'));
    fs::write(&record_path, tampered).unwrap();
    assert!(store
        .read_package_bytecode(&package_ref, &reference)
        .is_err());

    // Writes never touch an existing immutable record (content-addressed).
    let mut changed = bytecode.clone();
    let BytecodeFunctionOrigin::Executable { executable } = &mut changed
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .origin
    else {
        unreachable!()
    };
    executable.executable_index = 1;
    assign_bytecode_identity(&mut changed).unwrap();
    assert_ne!(changed.bytecode_identity, bytecode.bytecode_identity);
    let changed_ref = BytecodeArtifactRef::new(changed.bytecode_identity.clone());
    store
        .write_package_bytecode(&package_ref, &changed)
        .unwrap();
    assert_eq!(
        store
            .read_package_bytecode(&package_ref, &changed_ref)
            .unwrap()
            .artifact(),
        &changed
    );
}
