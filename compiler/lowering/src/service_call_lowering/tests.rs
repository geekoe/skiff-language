use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryErrorContract, BoundaryOperationContract, BoundaryOperationDescriptor, BoundaryReturn,
    BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, ContractOperationId, ContractRequirement,
    PackageCallableId, PackageLocalAbiIdentity, ServiceProtocolIdentity,
};
use skiff_compiler_source::{
    ExpressionKey, ExpressionOwnerKey, ResolvedCallTarget, ResolvedCallTargetFacts,
};

use super::*;
use crate::{
    ContractDependencyOperationIndex, ContractDependencyOperationIndexEntry,
    ServiceCallLoweringError,
};

#[test]
fn actual_calls_get_alias_stable_slots_used_operations_and_call_site_refs() {
    let alpha_echo = operation("operation:alpha-echo", "echo");
    let alpha_status = operation("operation:alpha-status", "status");
    let zeta_send = operation("operation:zeta-send", "send");
    let unused_ping = operation("operation:unused-ping", "ping");
    let operation_index = index([
        entry("zeta", "protocol:zeta", [zeta_send.clone()]),
        entry("unused", "protocol:unused", [unused_ping]),
        entry(
            "alpha",
            "protocol:alpha",
            [alpha_echo.clone(), alpha_status.clone()],
        ),
    ]);
    let targets = target_facts([
        (
            key("z_module", "run", 4),
            contract_target("zeta", &zeta_send, "protocol:zeta"),
        ),
        (
            key("a_module", "run", 2),
            contract_target("alpha", &alpha_status, "protocol:alpha"),
        ),
        (
            key("a_module", "run", 1),
            contract_target("alpha", &alpha_echo, "protocol:alpha"),
        ),
        (
            key("a_module", "run", 3),
            contract_target("alpha", &alpha_echo, "protocol:alpha"),
        ),
    ]);

    let lowered = lower_service_calls(&targets, &operation_index).unwrap();
    assert_eq!(lowered.service_requirements().len(), 2);
    let alpha = &lowered.service_requirements()[0];
    assert_eq!(alpha.contract_requirement.alias, "alpha");
    assert_eq!(alpha.service_binding_slot, 0);
    assert_eq!(
        alpha.used_operations,
        BTreeSet::from([
            alpha_echo.operation_id.clone(),
            alpha_status.operation_id.clone()
        ])
    );
    let zeta = &lowered.service_requirements()[1];
    assert_eq!(zeta.contract_requirement.alias, "zeta");
    assert_eq!(zeta.service_binding_slot, 1);
    assert_eq!(
        zeta.used_operations,
        BTreeSet::from([zeta_send.operation_id.clone()])
    );

    let call_sites = lowered.call_sites();
    assert_eq!(call_sites.len(), 4, "duplicate calls remain distinct sites");
    assert_eq!(call_sites[0].expression(), &key("a_module", "run", 1));
    assert_eq!(call_sites[0].call_ref().service_requirement_slot, 0);
    assert_eq!(call_sites[1].expression(), &key("a_module", "run", 2));
    assert_eq!(call_sites[2].expression(), &key("a_module", "run", 3));
    assert_eq!(call_sites[3].expression(), &key("z_module", "run", 4));
    assert_eq!(call_sites[3].call_ref().service_requirement_slot, 1);
    assert_eq!(
        lowered
            .service_call_ref_index(&key("a_module", "run", 1))
            .unwrap(),
        lowered
            .service_call_ref_index(&key("a_module", "run", 3))
            .unwrap(),
        "same tuple in one file must intern to one owner-local index"
    );
    assert_eq!(lowered.file_service_call_refs("a_module").len(), 2);
    assert_eq!(lowered.file_service_call_refs("z_module").len(), 1);
    assert_eq!(
        lowered
            .service_call_ref_index(&key("a_module", "run", 1))
            .unwrap()
            .index(),
        0,
        "File IR refs use canonical tuple order"
    );
    assert_eq!(
        lowered
            .service_call_ref_index(&key("a_module", "run", 2))
            .unwrap()
            .index(),
        1
    );
    assert_eq!(
        lowered
            .service_call_ref_index(&key("z_module", "run", 4))
            .unwrap()
            .index(),
        0,
        "the index owner is each File IR unit, not the package"
    );
    assert_eq!(
        lowered.service_call_ref_closure(),
        lowered.service_call_refs().cloned().collect(),
        "typed package closure must equal the exact union of per-file tables"
    );

    let reordered_index = index([
        entry(
            "alpha",
            "protocol:alpha",
            [alpha_status.clone(), alpha_echo.clone()],
        ),
        entry("zeta", "protocol:zeta", [zeta_send]),
        entry(
            "unused",
            "protocol:unused",
            [operation("operation:unused-ping", "ping")],
        ),
    ]);
    assert_eq!(
        lower_service_calls(&targets, &reordered_index).unwrap(),
        lowered,
        "declaration and operation insertion order must not perturb slots"
    );
}

#[test]
fn unused_contract_declaration_produces_no_runtime_requirement() {
    let index = index([entry(
        "unused",
        "protocol:unused",
        [operation("operation:ping", "ping")],
    )]);
    let targets = ResolvedCallTargetFacts::empty();
    let lowered = lower_service_calls(&targets, &index).unwrap();
    assert!(lowered.service_requirements().is_empty());
    assert!(lowered.call_sites().is_empty());
}

#[test]
fn protocol_unknown_alias_and_unknown_operation_fail_closed() {
    let echo = operation("operation:echo", "echo");
    let index = index([entry("echo", "protocol:echo", [echo.clone()])]);

    let protocol_mismatch = target_facts([(
        key("api", "run", 0),
        contract_target("echo", &echo, "protocol:wrong"),
    )]);
    assert!(matches!(
        lower_service_calls(&protocol_mismatch, &index),
        Err(ServiceCallLoweringError::ProtocolIdentityMismatch { .. })
    ));

    let unknown_alias = target_facts([(
        key("api", "run", 0),
        contract_target("missing", &echo, "protocol:echo"),
    )]);
    assert!(matches!(
        lower_service_calls(&unknown_alias, &index),
        Err(ServiceCallLoweringError::UnknownContractAlias { .. })
    ));

    let missing = operation("operation:missing", "missing");
    let unknown_operation = target_facts([(
        key("api", "run", 0),
        contract_target("echo", &missing, "protocol:echo"),
    )]);
    assert!(matches!(
        lower_service_calls(&unknown_operation, &index),
        Err(ServiceCallLoweringError::UnknownContractOperation { .. })
    ));
}

#[test]
fn direct_package_call_target_is_not_rewritten_as_a_service_call() {
    let expression = key("api", "run", 0);
    let package_target = ResolvedCallTarget::DependencyPackageFunction {
        package_requirement_alias: "utils".to_string(),
        package_callable_id: PackageCallableId::new("callable:format"),
        expected_local_abi: PackageLocalAbiIdentity::new("local-abi:utils"),
    };
    let targets = target_facts([(expression.clone(), package_target.clone())]);

    let lowered =
        lower_service_calls(&targets, &ContractDependencyOperationIndex::default()).unwrap();
    assert!(lowered.service_requirements().is_empty());
    assert!(lowered.call_sites().is_empty());
    assert_eq!(targets.target(&expression), Some(&package_target));
}

#[test]
fn operation_index_rejects_duplicate_alias_and_mismatched_nested_identity() {
    let operation = operation("operation:echo", "echo");
    let duplicate = ContractDependencyOperationIndex::build([
        entry("echo", "protocol:echo", [operation.clone()]),
        entry("echo", "protocol:echo", [operation.clone()]),
    ]);
    assert!(matches!(
        duplicate,
        Err(ServiceCallLoweringError::DuplicateContractAlias { .. })
    ));

    let mut operations = BTreeMap::new();
    operations.insert(ContractOperationId::new("operation:map-key"), operation);
    let mismatch =
        ContractDependencyOperationIndex::build([ContractDependencyOperationIndexEntry::new(
            requirement("echo", "protocol:echo"),
            operations,
        )]);
    assert!(matches!(
        mismatch,
        Err(ServiceCallLoweringError::OperationIdentityMismatch { .. })
    ));
}

#[test]
fn lowered_runtime_refs_contain_only_contract_identity_and_slot_facts() {
    let echo = operation("operation:echo", "echo");
    let index = index([entry("echo", "protocol:echo", [echo.clone()])]);
    let targets = target_facts([(
        key("api", "run", 0),
        contract_target("echo", &echo, "protocol:echo"),
    )]);
    let lowered = lower_service_calls(&targets, &index).unwrap();
    let wire = serde_json::to_string(&(
        lowered.service_requirements(),
        lowered.service_call_refs().collect::<Vec<_>>(),
    ))
    .unwrap();
    for forbidden in [
        "providerPackageId",
        "providerBuildId",
        "serviceUnit",
        "publicationAbi",
        "deploymentRevision",
        "route",
        "executableTarget",
    ] {
        assert!(
            !wire.contains(forbidden),
            "unexpected provider field {forbidden}"
        );
    }
}

fn index<const N: usize>(
    entries: [ContractDependencyOperationIndexEntry; N],
) -> ContractDependencyOperationIndex {
    ContractDependencyOperationIndex::build(entries).unwrap()
}

fn entry<const N: usize>(
    alias: &str,
    protocol: &str,
    operations: [BoundaryOperationDescriptor; N],
) -> ContractDependencyOperationIndexEntry {
    ContractDependencyOperationIndexEntry::new(
        requirement(alias, protocol),
        operations
            .into_iter()
            .map(|operation| (operation.operation_id.clone(), operation))
            .collect(),
    )
}

fn requirement(alias: &str, protocol: &str) -> ContractRequirement {
    ContractRequirement {
        alias: alias.to_string(),
        service_id: format!("example.{alias}"),
        contract_version: "1.0.0".to_string(),
        expected_protocol_identity: ServiceProtocolIdentity::new(protocol),
    }
}

fn operation(operation_id: &str, stable_key: &str) -> BoundaryOperationDescriptor {
    BoundaryOperationDescriptor {
        operation_id: ContractOperationId::new(operation_id),
        stable_key: stable_key.to_string(),
        contract: BoundaryOperationContract {
            parameters: Vec::new(),
            return_value: BoundaryReturn {
                ty: skiff_artifact_model::ContractTypeRef::builtin("unit"),
                value_plan: linkable(),
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
    }
}

fn linkable() -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner: BoundaryValueOwner::Provider,
        lifetime: BoundaryValueLifetime::Call,
    }
}

fn contract_target(
    alias: &str,
    operation: &BoundaryOperationDescriptor,
    protocol: &str,
) -> ResolvedCallTarget {
    ResolvedCallTarget::ContractOperation {
        contract_requirement: requirement(alias, protocol),
        contract_operation_id: operation.operation_id.clone(),
    }
}

fn target_facts<const N: usize>(
    targets: [(ExpressionKey, ResolvedCallTarget); N],
) -> ResolvedCallTargetFacts {
    ResolvedCallTargetFacts::from_targets(targets.into_iter().collect())
}

fn key(module: &str, function: &str, preorder_index: u32) -> ExpressionKey {
    ExpressionKey::new(
        module,
        ExpressionOwnerKey::Function(function.to_string()),
        preorder_index,
    )
}
