use std::fs;

use serde_json::{json, Value};
use skiff_artifact_identity::{
    assign_runtime_assembly_identity, runtime_assembly_ref, service_contract_ref,
    RuntimeAssemblyRecordPath, ServiceContractRecordPath,
};
use skiff_artifact_model::{RuntimeAssembly, RuntimeAssemblyRef, ServiceContract};
use skiff_deployment::{
    fixtures::runtime_assembly_fixture,
    storage::{CanonicalArtifactStore, RuntimeAssemblyPointer, ServiceContractPointer},
};

use super::{fixtures, invoke, RouterSnapshot, TestRoot};

#[test]
fn ecosystem_store_snapshot_uses_exact_refs_after_pointers_drift() {
    let root = TestRoot::new("snapshot-pointer-drift");
    let store = CanonicalArtifactStore::create(root.path()).unwrap();
    let (first_contract, first_assembly, first_reference) = write_snapshot_fixture(&store, "echo");
    let (second_contract, second_assembly, second_reference) =
        write_snapshot_fixture(&store, "echo-v2");

    let first_contract_pointer =
        ServiceContractPointer::new(service_contract_ref(&first_contract).unwrap()).unwrap();
    let second_contract_pointer =
        ServiceContractPointer::new(service_contract_ref(&second_contract).unwrap()).unwrap();
    store
        .compare_and_swap_service_contract_pointer(None, &first_contract_pointer)
        .unwrap();
    store
        .compare_and_swap_service_contract_pointer(
            Some(&first_contract_pointer),
            &second_contract_pointer,
        )
        .unwrap();

    let first_assembly_pointer =
        RuntimeAssemblyPointer::new("active", first_reference.clone()).unwrap();
    let second_assembly_pointer = RuntimeAssemblyPointer::new("active", second_reference).unwrap();
    store
        .compare_and_swap_runtime_assembly_pointer(None, &first_assembly_pointer)
        .unwrap();
    store
        .compare_and_swap_runtime_assembly_pointer(
            Some(&first_assembly_pointer),
            &second_assembly_pointer,
        )
        .unwrap();

    let snapshot = invoke(
        root.path(),
        json!({
            "operation": "readRouterSnapshot",
            "assembly": first_reference
        }),
    )
    .expect("exact snapshot");
    let snapshot: RouterSnapshot = serde_json::from_value(snapshot).unwrap();
    assert_eq!(snapshot.assembly, first_assembly);
    assert_eq!(snapshot.service_contracts, vec![first_contract]);
    assert_ne!(snapshot.assembly, second_assembly);
}

#[test]
fn ecosystem_store_snapshot_rejects_tampered_assembly_and_contract() {
    let assembly_root = TestRoot::new("snapshot-assembly-tamper");
    let assembly_store = CanonicalArtifactStore::create(assembly_root.path()).unwrap();
    let (_, _, assembly_reference) = write_snapshot_fixture(&assembly_store, "echo");
    let assembly_path = assembly_root.path().join(
        RuntimeAssemblyRecordPath::new(&assembly_reference)
            .unwrap()
            .as_relative_path()
            .as_path(),
    );
    mutate_json(&assembly_path, |value| {
        value["globalIngress"][0]["selector"]["path"] = json!("/tampered");
    });
    assert!(snapshot(assembly_root.path(), &assembly_reference).is_err());

    let contract_root = TestRoot::new("snapshot-contract-tamper");
    let contract_store = CanonicalArtifactStore::create(contract_root.path()).unwrap();
    let (contract, _, contract_assembly_reference) =
        write_snapshot_fixture(&contract_store, "echo");
    let contract_path = contract_root.path().join(
        ServiceContractRecordPath::new(&service_contract_ref(&contract).unwrap())
            .unwrap()
            .as_relative_path()
            .as_path(),
    );
    mutate_json(&contract_path, |value| {
        let descriptor = value["operations"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        descriptor["stableKey"] = json!("tampered");
    });
    assert!(snapshot(contract_root.path(), &contract_assembly_reference).is_err());
}

fn write_snapshot_fixture(
    store: &CanonicalArtifactStore,
    stable_key: &str,
) -> (ServiceContract, RuntimeAssembly, RuntimeAssemblyRef) {
    let contract = fixtures::contract(stable_key);
    let contract_reference = service_contract_ref(&contract).unwrap();
    let operation = contract.operations.keys().next().unwrap().clone();
    let mut assembly = runtime_assembly_fixture().unwrap();
    assembly.resolved_contracts = vec![contract_reference.clone()];
    for ingress in &mut assembly.global_ingress {
        ingress.contract = contract_reference.clone();
        ingress.contract_operation_id = operation.clone();
    }
    assign_runtime_assembly_identity(&mut assembly).unwrap();
    let reference = runtime_assembly_ref(&assembly).unwrap();
    store.write_service_contract(&contract).unwrap();
    store.write_runtime_assembly(&assembly).unwrap();
    (contract, assembly, reference)
}

fn snapshot(root: &std::path::Path, reference: &RuntimeAssemblyRef) -> Result<Value, String> {
    invoke(
        root,
        json!({
            "operation": "readRouterSnapshot",
            "assembly": reference
        }),
    )
}

fn mutate_json(path: &std::path::Path, mutate: impl FnOnce(&mut Value)) {
    let mut value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    mutate(&mut value);
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}
