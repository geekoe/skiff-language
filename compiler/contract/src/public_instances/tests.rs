use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{ContractOperationId, InterfaceInstantiationRef, PackageCallableId};

use super::*;

#[test]
fn exact_projection_canonicalizes_interfaces_and_preserves_declaration_slots() {
    let facts = ServicePublicInstanceOperationFacts::try_from_interfaces([
        interface_row(
            "worker",
            "interface:zeta",
            &[
                ("abi:zeta:first", "worker.zeta"),
                ("abi:zeta:second", "worker.alpha"),
            ],
        ),
        interface_row(
            "worker",
            "interface:alpha",
            &[("abi:alpha:only", "worker.beta")],
        ),
        interface_row(
            "unselected",
            "interface:ignored",
            &[("abi:ignored", "unselected.run")],
        ),
    ])
    .unwrap();
    let selection = selection(BTreeMap::from([(
        "worker".to_string(),
        BTreeSet::from([
            "worker.alpha".to_string(),
            "worker.beta".to_string(),
            "worker.zeta".to_string(),
        ]),
    )]));

    let projected = project_public_instances(&selection, &facts).unwrap();
    let operation_ids = ["worker.alpha", "worker.beta", "worker.zeta", "direct"]
        .into_iter()
        .map(|key| {
            (
                key.to_string(),
                ContractOperationId::new(format!("op:{key}")),
            )
        })
        .collect();
    let bound = bind_contract_operation_ids(projected, &operation_ids).unwrap();

    assert_eq!(
        bound.keys().map(String::as_str).collect::<Vec<_>>(),
        ["worker"]
    );
    let interfaces = &bound["worker"].interfaces;
    assert_eq!(interfaces.len(), 2);
    assert_eq!(interfaces[0].interface.interface_abi_id, "interface:alpha");
    assert_eq!(interfaces[1].interface.interface_abi_id, "interface:zeta");
    assert_eq!(
        interfaces[1]
            .methods
            .iter()
            .map(|method| method.method_abi_id.as_str())
            .collect::<Vec<_>>(),
        ["abi:zeta:first", "abi:zeta:second"]
    );
    assert_eq!(
        interfaces[1]
            .methods
            .iter()
            .map(|method| method.contract_operation_id.as_str())
            .collect::<Vec<_>>(),
        ["op:worker.zeta", "op:worker.alpha"]
    );
}

#[test]
fn marker_interface_keeps_an_empty_declaration_slot_vector() {
    let facts = ServicePublicInstanceOperationFacts::try_from_interfaces([interface_row(
        "marker",
        "interface:marker",
        &[],
    )])
    .unwrap();
    let selection = selection(BTreeMap::from([("marker".to_string(), BTreeSet::new())]));

    let projected = project_public_instances(&selection, &facts).unwrap();
    let bound = bind_contract_operation_ids(projected, &BTreeMap::new()).unwrap();

    assert_eq!(bound["marker"].interfaces.len(), 1);
    assert!(bound["marker"].interfaces[0].methods.is_empty());
}

#[test]
fn projection_rejects_missing_and_extra_operation_coverage() {
    let only_run = ServicePublicInstanceOperationFacts::try_from_interfaces([interface_row(
        "worker",
        "interface:worker",
        &[("abi:run", "worker.run")],
    )])
    .unwrap();
    let expects_run_and_stop = selection(BTreeMap::from([(
        "worker".to_string(),
        BTreeSet::from(["worker.run".to_string(), "worker.stop".to_string()]),
    )]));
    assert!(matches!(
        project_public_instances(&expects_run_and_stop, &only_run),
        Err(ContractDefinitionError::MissingPublicInstanceOperations {
            public_instance,
            operation_stable_keys,
        }) if public_instance == "worker" && operation_stable_keys == ["worker.stop"]
    ));

    let expects_nothing = selection(BTreeMap::from([("worker".to_string(), BTreeSet::new())]));
    assert!(matches!(
        project_public_instances(&expects_nothing, &only_run),
        Err(ContractDefinitionError::UnexpectedPublicInstanceOperation {
            public_instance,
            operation_stable_key,
        }) if public_instance == "worker" && operation_stable_key == "worker.run"
    ));
}

#[test]
fn checked_input_rejects_duplicate_interfaces_and_operation_keys() {
    let duplicate_interface = ServicePublicInstanceOperationFacts::try_from_interfaces([
        interface_row("worker", "interface:worker", &[("abi:run", "worker.run")]),
        interface_row("worker", "interface:worker", &[("abi:stop", "worker.stop")]),
    ]);
    assert!(matches!(
        duplicate_interface,
        Err(ContractDefinitionError::DuplicatePublicInstanceInterface { .. })
    ));

    let duplicate_operation = ServicePublicInstanceOperationFacts::try_from_interfaces([
        interface_row("first", "interface:first", &[("abi:first", "shared.run")]),
        interface_row(
            "second",
            "interface:second",
            &[("abi:second", "shared.run")],
        ),
    ]);
    assert!(matches!(
        duplicate_operation,
        Err(ContractDefinitionError::DuplicatePublicInstanceOperation {
            operation_stable_key,
        }) if operation_stable_key == "shared.run"
    ));

    let open_interface = ServicePublicInstanceInterfaceOperations::try_new(
        "worker",
        InterfaceInstantiationRef {
            interface_abi_id: "interface:generic".to_string(),
            canonical_type_args: vec![skiff_artifact_model::TypeRefIr::TypeParam {
                name: "T".to_string(),
            }],
        },
        Vec::new(),
    );
    assert!(matches!(
        open_interface,
        Err(ContractDefinitionError::OpenPublicInstanceInterface {
            public_instance,
            ..
        }) if public_instance == "worker"
    ));
}

#[test]
fn binding_rejects_an_operation_absent_from_the_compiled_contract() {
    let facts = ServicePublicInstanceOperationFacts::try_from_interfaces([interface_row(
        "worker",
        "interface:worker",
        &[("abi:run", "worker.run")],
    )])
    .unwrap();
    let selection = selection(BTreeMap::from([(
        "worker".to_string(),
        BTreeSet::from(["worker.run".to_string()]),
    )]));
    let projected = project_public_instances(&selection, &facts).unwrap();

    assert!(matches!(
        bind_contract_operation_ids(projected, &BTreeMap::new()),
        Err(ContractDefinitionError::UnknownPublicInstanceOperation {
            public_instance,
            operation_stable_key,
        }) if public_instance == "worker" && operation_stable_key == "worker.run"
    ));
}

fn selection(public_instances: BTreeMap<String, BTreeSet<String>>) -> ServiceCallSelection {
    let roots = public_instances.keys().cloned().collect();
    let operations = public_instances
        .values()
        .flatten()
        .map(|key| {
            (
                key.clone(),
                PackageCallableId::new(format!("callable:{key}")),
            )
        })
        .collect();
    ServiceCallSelection {
        roots,
        operations,
        public_instances,
    }
}

fn interface_row(
    public_root: &str,
    interface_abi_id: &str,
    slots: &[(&str, &str)],
) -> ServicePublicInstanceInterfaceOperations {
    ServicePublicInstanceInterfaceOperations::try_new(
        public_root,
        InterfaceInstantiationRef {
            interface_abi_id: interface_abi_id.to_string(),
            canonical_type_args: Vec::new(),
        },
        slots
            .iter()
            .map(|(method_abi_id, operation_stable_key)| {
                ServicePublicInstanceOperationSlot::try_new(*method_abi_id, *operation_stable_key)
                    .unwrap()
            })
            .collect(),
    )
    .unwrap()
}
