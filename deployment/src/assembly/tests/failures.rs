use std::collections::BTreeSet;

use skiff_artifact_identity::assign_package_artifact_identities;
use skiff_artifact_model::{
    ContractOperationId, GatewayEntryIdentity, PackageBuildId, PackageConfigRequirement,
    PackageLocalAbiIdentity, GATEWAY_ENTRY_IDENTITY_PREFIX,
};

use super::fixtures::*;
use crate::assembly::{resolve_runtime_assembly, AssemblyResolutionError};

#[test]
fn service_provider_resolution_rejects_zero_multiple_and_protocol_mismatch() {
    let consumer_contract = contract("service.consumer-errors");
    let provider_contract = contract("service.provider-errors");
    let consumer_package = package(
        "package.consumer-errors",
        &[],
        &[("provider", &provider_contract, 0)],
    );
    let provider_package = package("package.provider-errors", &[], &[]);
    let consumer = deployment(
        &consumer_contract,
        &consumer_package,
        "consumer-r1",
        Vec::new(),
        vec![service_selector(&consumer_package, 0, &provider_contract)],
    );
    let provider_a = deployment(
        &provider_contract,
        &provider_package,
        "provider-r1",
        Vec::new(),
        Vec::new(),
    );
    let provider_b = deployment(
        &provider_contract,
        &provider_package,
        "provider-r2",
        Vec::new(),
        Vec::new(),
    );
    let root = [deployment_ref(&consumer)];
    let contracts = [consumer_contract.clone(), provider_contract.clone()];
    let packages = [consumer_package.clone(), provider_package.clone()];

    let missing =
        resolve_runtime_assembly(&root, &[consumer.clone()], &contracts, &packages).unwrap_err();
    assert!(matches!(
        missing,
        AssemblyResolutionError::MissingServiceProvider(_)
    ));

    let ambiguous = resolve_runtime_assembly(
        &root,
        &[consumer.clone(), provider_b, provider_a],
        &contracts,
        &packages,
    )
    .unwrap_err();
    assert!(matches!(
        ambiguous,
        AssemblyResolutionError::AmbiguousServiceProvider { .. }
    ));

    let incompatible_contract =
        contract_with_stable_key("service.provider-errors", "different-operation");
    let incompatible_provider = deployment(
        &incompatible_contract,
        &provider_package,
        "provider-incompatible",
        Vec::new(),
        Vec::new(),
    );
    let mismatch = resolve_runtime_assembly(
        &root,
        &[consumer, incompatible_provider],
        &[consumer_contract, provider_contract, incompatible_contract],
        &packages,
    )
    .unwrap_err();
    assert!(matches!(
        mismatch,
        AssemblyResolutionError::ServiceProviderProtocolMismatch { .. }
    ));
}

#[test]
fn package_edges_reject_version_abi_and_build_lookup_mismatches() {
    let root_contract = contract("service.package-errors");
    let dependency = package("package.dependency-errors", &[], &[]);
    let root_package = package("package.root-errors", &[("dependency", &dependency)], &[]);
    let valid_binding = package_binding(&root_package, "dependency", &dependency);
    let candidates = [root_package.clone(), dependency.clone()];

    let missing_package = deployment(
        &root_contract,
        &root_package,
        "missing-package",
        vec![valid_binding.clone()],
        Vec::new(),
    );
    let missing_package_root = [deployment_ref(&missing_package)];
    let error = resolve_runtime_assembly(
        &missing_package_root,
        &[missing_package],
        std::slice::from_ref(&root_contract),
        std::slice::from_ref(&root_package),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        AssemblyResolutionError::MissingPackageArtifact(_)
    ));

    let mut wrong_version_binding = valid_binding.clone();
    wrong_version_binding.package.package_version = "9.0.0".to_string();
    let wrong_version = deployment(
        &root_contract,
        &root_package,
        "wrong-version",
        vec![wrong_version_binding],
        Vec::new(),
    );
    let wrong_version_root = [deployment_ref(&wrong_version)];
    let error = resolve_runtime_assembly(
        &wrong_version_root,
        &[wrong_version],
        std::slice::from_ref(&root_contract),
        &candidates,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        AssemblyResolutionError::PackageRequirementMismatch { .. }
    ));

    let mut wrong_abi_binding = valid_binding.clone();
    wrong_abi_binding.package.package_local_abi_identity =
        PackageLocalAbiIdentity::new("wrong-abi");
    let wrong_abi = deployment(
        &root_contract,
        &root_package,
        "wrong-abi",
        vec![wrong_abi_binding],
        Vec::new(),
    );
    let wrong_abi_root = [deployment_ref(&wrong_abi)];
    let error = resolve_runtime_assembly(
        &wrong_abi_root,
        &[wrong_abi],
        std::slice::from_ref(&root_contract),
        &candidates,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        AssemblyResolutionError::PackageRequirementMismatch { .. }
    ));

    let mut wrong_build_binding = valid_binding;
    wrong_build_binding.package.package_build_id = PackageBuildId::new("missing-build");
    let wrong_build = deployment(
        &root_contract,
        &root_package,
        "wrong-build",
        vec![wrong_build_binding],
        Vec::new(),
    );
    let wrong_build_root = [deployment_ref(&wrong_build)];
    let error = resolve_runtime_assembly(
        &wrong_build_root,
        &[wrong_build],
        &[root_contract],
        &candidates,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        AssemblyResolutionError::MissingPackageArtifact(_)
    ));
}

#[test]
fn the_same_caller_edge_cannot_select_activation_relative_builds() {
    let contract_a = contract("service.package-link-a");
    let contract_b = contract("service.package-link-b");
    let dependency_a = package("package.link-target", &[], &[]);
    let mut dependency_b = dependency_a.clone();
    dependency_b
        .runtime_requirements
        .config
        .push(PackageConfigRequirement {
            path: "variant".to_string(),
            access: skiff_artifact_model::PackageConfigAccess::Optional {
                value_type: "string".to_string(),
            },
        });
    assign_package_artifact_identities(&mut dependency_b).unwrap();
    assert_eq!(
        dependency_a.package_local_abi.local_abi_identity,
        dependency_b.package_local_abi.local_abi_identity
    );
    assert_ne!(dependency_a.package_build_id, dependency_b.package_build_id);

    let shared_caller = package(
        "package.shared-caller",
        &[("dependency", &dependency_a)],
        &[],
    );
    let deployment_a = deployment(
        &contract_a,
        &shared_caller,
        "revision-a",
        vec![package_binding(&shared_caller, "dependency", &dependency_a)],
        Vec::new(),
    );
    let deployment_b = deployment(
        &contract_b,
        &shared_caller,
        "revision-b",
        vec![package_binding(&shared_caller, "dependency", &dependency_b)],
        Vec::new(),
    );

    let error = resolve_runtime_assembly(
        &[deployment_ref(&deployment_a), deployment_ref(&deployment_b)],
        &[deployment_a, deployment_b],
        &[contract_b, contract_a],
        &[shared_caller, dependency_b, dependency_a],
    )
    .unwrap_err();
    assert!(matches!(
        error,
        AssemblyResolutionError::ConflictingPackageLink { .. }
    ));
}

#[test]
fn one_activation_cannot_resolve_multiple_builds_for_one_package_id() {
    let root_contract = contract("service.package-build-ambiguity");
    let dependency_a = package("package.shared-build", &[], &[]);
    let mut dependency_b = dependency_a.clone();
    dependency_b
        .runtime_requirements
        .config
        .push(PackageConfigRequirement {
            path: "variant".to_string(),
            access: skiff_artifact_model::PackageConfigAccess::Optional {
                value_type: "string".to_string(),
            },
        });
    assign_package_artifact_identities(&mut dependency_b).unwrap();
    assert_eq!(
        dependency_a.package_local_abi.local_abi_identity,
        dependency_b.package_local_abi.local_abi_identity
    );
    assert_ne!(dependency_a.package_build_id, dependency_b.package_build_id);

    let root_package = package(
        "package.root-build-ambiguity",
        &[
            ("dependency-a", &dependency_a),
            ("dependency-b", &dependency_b),
        ],
        &[],
    );
    let root_deployment = deployment(
        &root_contract,
        &root_package,
        "root-revision",
        vec![
            package_binding(&root_package, "dependency-a", &dependency_a),
            package_binding(&root_package, "dependency-b", &dependency_b),
        ],
        Vec::new(),
    );

    let error = resolve_runtime_assembly(
        &[deployment_ref(&root_deployment)],
        &[root_deployment],
        std::slice::from_ref(&root_contract),
        &[root_package, dependency_a, dependency_b],
    )
    .unwrap_err();
    assert!(matches!(
        error,
        AssemblyResolutionError::MultiplePackageBuildsForId { .. }
    ));
}

#[test]
fn selectors_and_operations_must_describe_exact_template_edges() {
    let consumer_contract = contract("service.template-consumer");
    let provider_contract = contract("service.template-provider");
    let provider_package = package("package.template-provider", &[], &[]);
    let provider = deployment(
        &provider_contract,
        &provider_package,
        "provider-r1",
        Vec::new(),
        Vec::new(),
    );

    let no_requirement = package("package.no-requirement", &[], &[]);
    let unexpected = deployment(
        &consumer_contract,
        &no_requirement,
        "unexpected-selector",
        Vec::new(),
        vec![service_selector(&no_requirement, 0, &provider_contract)],
    );
    let error = resolve_runtime_assembly(
        &[deployment_ref(&unexpected)],
        &[unexpected, provider.clone()],
        &[consumer_contract.clone(), provider_contract.clone()],
        &[no_requirement, provider_package.clone()],
    )
    .unwrap_err();
    assert!(matches!(
        error,
        AssemblyResolutionError::UnexpectedServiceSelector { .. }
    ));

    let requiring = package(
        "package.missing-selector",
        &[],
        &[("provider", &provider_contract, 0)],
    );
    let missing = deployment(
        &consumer_contract,
        &requiring,
        "missing-selector",
        Vec::new(),
        Vec::new(),
    );
    let error = resolve_runtime_assembly(
        &[deployment_ref(&missing)],
        &[missing, provider.clone()],
        &[consumer_contract.clone(), provider_contract.clone()],
        &[requiring, provider_package.clone()],
    )
    .unwrap_err();
    assert!(matches!(
        error,
        AssemblyResolutionError::MissingServiceSelector { .. }
    ));

    let mut dangling = package(
        "package.dangling-operation",
        &[],
        &[("provider", &provider_contract, 0)],
    );
    let missing_operation = ContractOperationId::new("operation.missing");
    dangling.service_requirements[0].used_operations = BTreeSet::from([missing_operation.clone()]);
    dangling.service_call_refs[0].contract_operation_id = missing_operation;
    assign_package_artifact_identities(&mut dangling).unwrap();
    let dangling_deployment = deployment(
        &consumer_contract,
        &dangling,
        "dangling-operation",
        Vec::new(),
        vec![service_selector(&dangling, 0, &provider_contract)],
    );
    let error = resolve_runtime_assembly(
        &[deployment_ref(&dangling_deployment)],
        &[dangling_deployment, provider],
        &[consumer_contract, provider_contract],
        &[dangling, provider_package],
    )
    .unwrap_err();
    assert!(matches!(
        error,
        AssemblyResolutionError::MissingServiceOperation { .. }
    ));
}

#[test]
fn deployment_gateway_ingress_allows_same_selector_for_distinct_services() {
    let contract_a = contract("service.ingress-a");
    let contract_b = contract("service.ingress-b");
    let package_a = package("package.ingress-a", &[], &[]);
    let package_b = package("package.ingress-b", &[], &[]);
    let mut deployment_a = deployment(
        &contract_a,
        &package_a,
        "revision-a",
        Vec::new(),
        Vec::new(),
    );
    let mut deployment_b = deployment(
        &contract_b,
        &package_b,
        "revision-b",
        Vec::new(),
        Vec::new(),
    );
    add_http_ingress(&mut deployment_a, &contract_a, "/v1/models");
    add_http_ingress(&mut deployment_b, &contract_b, "/v1/models");

    let assembly = resolve_runtime_assembly(
        &[deployment_ref(&deployment_b), deployment_ref(&deployment_a)],
        &[deployment_a.clone(), deployment_b.clone()],
        &[contract_a, contract_b],
        &[package_b, package_a],
    )
    .unwrap();
    assert_eq!(assembly.gateway_ingress.len(), 2);
    assert_ne!(
        assembly.gateway_ingress[0].deployment,
        assembly.gateway_ingress[1].deployment
    );
    assert_eq!(
        assembly.gateway_ingress[0].selector,
        assembly.gateway_ingress[1].selector
    );
}

#[test]
fn deployment_gateway_ingress_rejects_missing_key_and_wrong_identity() {
    let contract = contract("service.ingress-invalid");
    let package = package("package.ingress-invalid", &[], &[]);
    let mut deployment = deployment(
        &contract,
        &package,
        "revision-invalid",
        Vec::new(),
        Vec::new(),
    );
    add_http_ingress(&mut deployment, &contract, "/call");

    let mut missing = deployment.clone();
    missing.gateway_entries.clear();
    assert!(matches!(
        resolve_runtime_assembly(
            &[deployment_ref(&missing)],
            &[missing],
            std::slice::from_ref(&contract),
            std::slice::from_ref(&package),
        )
        .unwrap_err(),
        AssemblyResolutionError::Artifact(_)
    ));

    let mut wrong_identity = deployment;
    wrong_identity
        .gateway_entries
        .values_mut()
        .next()
        .unwrap()
        .gateway_entry_identity = GatewayEntryIdentity::parse(format!(
        "{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}",
        "f".repeat(64)
    ))
    .unwrap();
    assert!(matches!(
        resolve_runtime_assembly(
            &[deployment_ref(&wrong_identity)],
            &[wrong_identity],
            &[contract],
            &[package],
        )
        .unwrap_err(),
        AssemblyResolutionError::Artifact(_)
    ));
}

#[test]
fn tampered_candidate_identity_fails_before_resolution() {
    let root_contract = contract("service.tamper");
    let root_package = package("package.tamper", &[], &[]);
    let mut root = deployment(
        &root_contract,
        &root_package,
        "revision",
        Vec::new(),
        Vec::new(),
    );
    root.deployment_revision = "tampered".into();

    let error = resolve_runtime_assembly(
        &[deployment_ref(&root)],
        &[root],
        &[root_contract],
        &[root_package],
    )
    .unwrap_err();
    assert!(matches!(error, AssemblyResolutionError::Artifact(_)));
}
