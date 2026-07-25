use std::collections::BTreeSet;

use skiff_artifact_model::{
    ContractOperationId, ContractRequirement, PackageCallableId, PackageLocalAbiIdentity,
    ServiceProtocolIdentity,
};
use skiff_compiler_source::{
    ExpressionKey, ExpressionOwnerKey, ResolvedCallTarget, ResolvedCallTargetFacts,
};

use super::*;
use crate::ServiceCallLoweringError;

#[test]
fn one_contract_call_uses_the_typed_requirement_and_operation_identity() {
    let expression = key("api", "run", 0);
    let requirement = requirement("echo", "protocol:echo");
    let operation_id = ContractOperationId::new("operation:echo");
    let targets = target_facts([(
        expression.clone(),
        contract_target(requirement.clone(), operation_id.clone()),
    )]);

    let lowered = lower_service_calls(&targets).unwrap();

    assert_eq!(
        lowered.service_requirements(),
        &[skiff_artifact_model::ServiceRequirement {
            contract_requirement: requirement.clone(),
            service_binding_slot: 0,
            used_operations: BTreeSet::from([operation_id.clone()]),
        }]
    );
    assert_eq!(lowered.call_sites().len(), 1);
    assert_eq!(lowered.call_sites()[0].expression(), &expression);
    assert_eq!(
        lowered.call_sites()[0].call_ref().service_requirement_slot,
        0
    );
    assert_eq!(
        lowered.call_sites()[0].call_ref().contract_operation_id,
        operation_id
    );
    assert_eq!(
        lowered.call_sites()[0]
            .call_ref()
            .expected_protocol_identity,
        requirement.expected_protocol_identity
    );
}

#[test]
fn slots_and_used_operations_are_canonical_across_multiple_and_duplicate_calls() {
    let alpha_echo = ContractOperationId::new("operation:alpha-echo");
    let alpha_status = ContractOperationId::new("operation:alpha-status");
    let zeta_send = ContractOperationId::new("operation:zeta-send");
    let alpha = requirement("alpha", "protocol:alpha");
    let zeta = requirement("zeta", "protocol:zeta");
    let targets = target_facts([
        (
            key("z_module", "run", 4),
            contract_target(zeta.clone(), zeta_send.clone()),
        ),
        (
            key("a_module", "run", 2),
            contract_target(alpha.clone(), alpha_status.clone()),
        ),
        (
            key("a_module", "run", 1),
            contract_target(alpha.clone(), alpha_echo.clone()),
        ),
        (
            key("a_module", "run", 3),
            contract_target(alpha.clone(), alpha_echo.clone()),
        ),
    ]);

    let lowered = lower_service_calls(&targets).unwrap();
    assert_eq!(lowered.service_requirements().len(), 2);
    let alpha_requirement = &lowered.service_requirements()[0];
    assert_eq!(alpha_requirement.contract_requirement, alpha);
    assert_eq!(alpha_requirement.service_binding_slot, 0);
    assert_eq!(
        alpha_requirement.used_operations,
        BTreeSet::from([alpha_echo.clone(), alpha_status.clone()])
    );
    let zeta_requirement = &lowered.service_requirements()[1];
    assert_eq!(zeta_requirement.contract_requirement, zeta);
    assert_eq!(zeta_requirement.service_binding_slot, 1);
    assert_eq!(
        zeta_requirement.used_operations,
        BTreeSet::from([zeta_send.clone()])
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

    let reordered_targets = target_facts([
        (
            key("a_module", "run", 3),
            contract_target(alpha.clone(), alpha_echo.clone()),
        ),
        (
            key("a_module", "run", 1),
            contract_target(alpha.clone(), alpha_echo),
        ),
        (key("z_module", "run", 4), contract_target(zeta, zeta_send)),
        (
            key("a_module", "run", 2),
            contract_target(alpha, alpha_status),
        ),
    ]);
    assert_eq!(
        lower_service_calls(&reordered_targets).unwrap(),
        lowered,
        "fact insertion order must not perturb slots or owner-local refs"
    );
}

#[test]
fn unused_contract_declaration_produces_no_runtime_requirement() {
    let lowered = lower_service_calls(&ResolvedCallTargetFacts::empty()).unwrap();
    assert!(lowered.service_requirements().is_empty());
    assert!(lowered.call_sites().is_empty());
}

#[test]
fn inconsistent_requirement_identity_for_one_alias_fails_closed() {
    let targets = target_facts([
        (
            key("api", "run", 0),
            contract_target(
                requirement("echo", "protocol:echo-v1"),
                ContractOperationId::new("operation:echo"),
            ),
        ),
        (
            key("api", "run", 1),
            contract_target(
                requirement("echo", "protocol:echo-v2"),
                ContractOperationId::new("operation:status"),
            ),
        ),
    ]);

    assert!(matches!(
        lower_service_calls(&targets),
        Err(ServiceCallLoweringError::ContractRequirementMismatch { alias })
            if alias == "echo"
    ));
}

#[test]
fn direct_package_call_target_is_not_rewritten_as_a_service_call() {
    let expression = key("api", "run", 0);
    let package_target = ResolvedCallTarget::DependencyPackageFunction {
        package_requirement_alias: "utils".to_string(),
        compiler_owned: false,
        package_callable_id: PackageCallableId::new("callable:format"),
        expected_local_abi: PackageLocalAbiIdentity::new("local-abi:utils"),
        exact_signature: None,
    };
    let targets = target_facts([(expression.clone(), package_target.clone())]);

    let lowered = lower_service_calls(&targets).unwrap();
    assert!(lowered.service_requirements().is_empty());
    assert!(lowered.call_sites().is_empty());
    assert_eq!(targets.target(&expression), Some(&package_target));
}

#[test]
fn lowered_runtime_refs_contain_no_provider_or_deployment_target() {
    let targets = target_facts([(
        key("api", "run", 0),
        contract_target(
            requirement("echo", "protocol:echo"),
            ContractOperationId::new("operation:echo"),
        ),
    )]);
    let lowered = lower_service_calls(&targets).unwrap();
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

fn requirement(alias: &str, protocol: &str) -> ContractRequirement {
    ContractRequirement {
        alias: alias.to_string(),
        service_id: format!("example.{alias}"),
        contract_version: "1.0.0".to_string(),
        expected_protocol_identity: ServiceProtocolIdentity::new(protocol),
    }
}

fn contract_target(
    contract_requirement: ContractRequirement,
    contract_operation_id: ContractOperationId,
) -> ResolvedCallTarget {
    ResolvedCallTarget::ContractOperation {
        contract_requirement,
        contract_operation_id,
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
