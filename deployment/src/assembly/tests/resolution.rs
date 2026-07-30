use std::collections::BTreeMap;

use skiff_artifact_identity::{
    assign_package_artifact_identities, assign_service_deployment_identity,
    validate_runtime_assembly_identity,
};
use skiff_artifact_model::{
    DeploymentIngressBinding, PackageConfigAccess, PackageConfigRequirement,
};

use super::fixtures::*;
use crate::assembly::resolve_runtime_assembly;

#[test]
fn empty_roots_produce_the_canonical_empty_assembly() {
    let assembly = resolve_runtime_assembly(&[], &[], &[], &[]).unwrap();
    assert_eq!(
        assembly,
        crate::fixtures::empty_runtime_assembly_fixture().unwrap()
    );
    validate_runtime_assembly_identity(&assembly).unwrap();
}

#[test]
fn service_cycle_closes_iteratively_with_activation_scoped_slot_zero() {
    let contract_a = contract("service.a");
    let contract_b = contract("service.b");
    let package_a = package("package.a", &[], &[("b", &contract_b, 0)]);
    let package_b = package("package.b", &[], &[("a", &contract_a, 0)]);
    let deployment_a = deployment(
        &contract_a,
        &package_a,
        "revision-a",
        Vec::new(),
        vec![service_selector(&package_a, 0, &contract_b)],
    );
    let deployment_b = deployment(
        &contract_b,
        &package_b,
        "revision-b",
        Vec::new(),
        vec![service_selector(&package_b, 0, &contract_a)],
    );

    let assembly = resolve_runtime_assembly(
        &[deployment_ref(&deployment_a)],
        &[deployment_b.clone(), deployment_a.clone()],
        &[contract_b, contract_a],
        &[package_b.clone(), package_a.clone()],
    )
    .unwrap();

    assert_eq!(assembly.resolved_deployments.len(), 2);
    assert_eq!(assembly.service_binding_templates.len(), 2);
    let keys = assembly
        .service_binding_templates
        .iter()
        .map(|template| template.bindings[0].key.clone())
        .collect::<Vec<_>>();
    assert!(keys.iter().any(|key| {
        key.caller_package_build_id == package_a.package_build_id
            && key.service_requirement_slot == 0
    }));
    assert!(keys.iter().any(|key| {
        key.caller_package_build_id == package_b.package_build_id
            && key.service_requirement_slot == 0
    }));
    validate_runtime_assembly_identity(&assembly).unwrap();
}

#[test]
fn package_diamond_links_each_build_once_and_is_input_order_independent() {
    let root_contract = contract("service.root");
    let leaf = package("package.leaf", &[], &[]);
    let left = package("package.left", &[("leaf", &leaf)], &[]);
    let right = package("package.right", &[("leaf", &leaf)], &[]);
    let root = package("package.root", &[("left", &left), ("right", &right)], &[]);
    let deployment = deployment(
        &root_contract,
        &root,
        "revision-root",
        vec![
            package_binding(&right, "leaf", &leaf),
            package_binding(&root, "right", &right),
            package_binding(&left, "leaf", &leaf),
            package_binding(&root, "left", &left),
        ],
        Vec::new(),
    );
    let root_ref = deployment_ref(&deployment);

    let first = resolve_runtime_assembly(
        std::slice::from_ref(&root_ref),
        std::slice::from_ref(&deployment),
        std::slice::from_ref(&root_contract),
        &[root.clone(), left.clone(), right.clone(), leaf.clone()],
    )
    .unwrap();
    let second = resolve_runtime_assembly(
        &[root_ref.clone(), root_ref],
        &[deployment],
        &[root_contract],
        &[leaf, right, left, root],
    )
    .unwrap();

    assert_eq!(first, second);
    assert_eq!(first.resolved_packages.len(), 4);
    assert_eq!(first.package_link_plan.code_slots.len(), 4);
    assert_eq!(first.package_link_plan.package_links.len(), 4);
}

#[test]
fn collection_mapping_is_preserved_from_requirement_through_assembly_link() {
    let root_contract = contract("service.collection-mapping");
    let dependency = package("package.collection-store", &[], &[]);
    let mut root = package("package.collection-service", &[("store", &dependency)], &[]);
    let mapping = BTreeMap::from([
        (
            "package_secret".to_string(),
            "mapped_package_secret".to_string(),
        ),
        (
            "package_audit".to_string(),
            "mapped_package_audit".to_string(),
        ),
    ]);
    root.package_requirements[0].collection_name_mapping = mapping.clone();
    assign_package_artifact_identities(&mut root).unwrap();
    let mut binding = package_binding(&root, "store", &dependency);
    binding.collection_name_mapping = mapping.clone();
    let deployment = deployment(
        &root_contract,
        &root,
        "revision-mapped",
        vec![binding],
        Vec::new(),
    );

    let assembly = resolve_runtime_assembly(
        &[deployment_ref(&deployment)],
        std::slice::from_ref(&deployment),
        std::slice::from_ref(&root_contract),
        &[root, dependency],
    )
    .unwrap();

    assert_eq!(
        assembly.package_link_plan.package_links[0].collection_name_mapping,
        mapping
    );
    validate_runtime_assembly_identity(&assembly).unwrap();
}

#[test]
fn two_activations_share_one_code_slot_but_keep_distinct_templates() {
    let contract_a = contract("service.shared-a");
    let contract_b = contract("service.shared-b");
    let shared = package("package.shared", &[], &[]);
    let deployment_a = deployment(&contract_a, &shared, "revision-a", Vec::new(), Vec::new());
    let deployment_b = deployment(&contract_b, &shared, "revision-b", Vec::new(), Vec::new());

    let assembly = resolve_runtime_assembly(
        &[deployment_ref(&deployment_b), deployment_ref(&deployment_a)],
        &[deployment_a.clone(), deployment_b.clone()],
        &[contract_b.clone(), contract_a.clone()],
        std::slice::from_ref(&shared),
    )
    .unwrap();
    let reordered = resolve_runtime_assembly(
        &[deployment_ref(&deployment_a), deployment_ref(&deployment_b)],
        &[deployment_b, deployment_a],
        &[contract_a, contract_b],
        std::slice::from_ref(&shared),
    )
    .unwrap();

    assert_eq!(assembly, reordered);
    assert_eq!(assembly.package_link_plan.code_slots.len(), 1);
    assert_eq!(assembly.activation_templates.len(), 2);
    assert_eq!(assembly.service_binding_templates.len(), 2);
    assert_ne!(
        assembly.activation_templates[0].deployment,
        assembly.activation_templates[1].deployment
    );
}

#[test]
fn gateway_ingress_projects_exact_entries_and_is_canonical_across_deployments() {
    let contract_a = contract("service.gateway-a");
    let contract_b = contract("service.gateway-b");
    let package_a = package("package.gateway-a", &[], &[]);
    let package_b = package("package.gateway-b", &[], &[]);
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
    add_http_ingress(&mut deployment_a, &contract_a, "/primary");
    let alias = DeploymentIngressBinding {
        selector: skiff_artifact_model::IngressSelector {
            protocol: skiff_artifact_model::IngressProtocol::Http,
            method: Some("POST".to_string()),
            path: "/alias".to_string(),
        },
        gateway_entry_key: deployment_a.ingress[0].gateway_entry_key.clone(),
    };
    deployment_a.ingress.push(alias);
    assign_service_deployment_identity(&mut deployment_a).unwrap();
    add_http_ingress(&mut deployment_b, &contract_b, "/call");

    let assembly = resolve_runtime_assembly(
        &[deployment_ref(&deployment_b), deployment_ref(&deployment_a)],
        &[deployment_b.clone(), deployment_a.clone()],
        &[contract_b, contract_a],
        &[package_b, package_a],
    )
    .unwrap();
    let reordered = resolve_runtime_assembly(
        &[deployment_ref(&deployment_a), deployment_ref(&deployment_b)],
        &[deployment_a.clone(), deployment_b],
        &[contract("service.gateway-a"), contract("service.gateway-b")],
        &[
            package("package.gateway-a", &[], &[]),
            package("package.gateway-b", &[], &[]),
        ],
    )
    .unwrap();

    assert_eq!(assembly, reordered);
    assert_eq!(assembly.gateway_ingress.len(), 3);
    let a_bindings = assembly
        .gateway_ingress
        .iter()
        .filter(|binding| binding.deployment == deployment_ref(&deployment_a))
        .collect::<Vec<_>>();
    assert_eq!(a_bindings.len(), 2);
    assert_eq!(
        a_bindings[0].gateway_entry_key,
        a_bindings[1].gateway_entry_key
    );
    assert_eq!(
        a_bindings[0].gateway_entry_identity,
        a_bindings[1].gateway_entry_identity
    );
    let entry = deployment_a
        .gateway_entries
        .get(&a_bindings[0].gateway_entry_key)
        .unwrap();
    assert_eq!(
        a_bindings[0].gateway_entry_identity,
        entry.gateway_entry_identity
    );
}

#[test]
fn changing_the_unique_provider_changes_the_assembly_identity() {
    let consumer_contract = contract("service.consumer");
    let provider_contract = contract("service.provider");
    let consumer_package = package(
        "package.consumer",
        &[],
        &[("provider", &provider_contract, 7)],
    );
    let provider_package = package("package.provider", &[], &[]);
    let consumer = deployment(
        &consumer_contract,
        &consumer_package,
        "consumer-r1",
        Vec::new(),
        vec![service_selector(&consumer_package, 7, &provider_contract)],
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
    let roots = [deployment_ref(&consumer)];
    let contracts = [consumer_contract, provider_contract];
    let packages = [consumer_package, provider_package];

    let first = resolve_runtime_assembly(
        &roots,
        &[consumer.clone(), provider_a],
        &contracts,
        &packages,
    )
    .unwrap();
    let second =
        resolve_runtime_assembly(&roots, &[consumer, provider_b], &contracts, &packages).unwrap();

    assert_ne!(first.assembly_identity, second.assembly_identity);
    let first_provider = &first
        .service_binding_templates
        .iter()
        .find(|template| !template.bindings.is_empty())
        .unwrap()
        .bindings[0]
        .provider;
    let second_provider = &second
        .service_binding_templates
        .iter()
        .find(|template| !template.bindings.is_empty())
        .unwrap()
        .bindings[0]
        .provider;
    assert_ne!(first_provider, second_provider);
}

#[test]
fn changing_a_resolved_build_or_activation_template_changes_identity() {
    let root_contract = contract("service.identity-root");
    let dependency_a = package("package.identity-dependency", &[], &[]);
    let mut dependency_b = dependency_a.clone();
    dependency_b
        .runtime_requirements
        .config
        .push(PackageConfigRequirement {
            path: "variant".to_string(),
            access: PackageConfigAccess::Optional {
                value_type: "string".to_string(),
            },
        });
    assign_package_artifact_identities(&mut dependency_b).unwrap();
    assert_eq!(
        dependency_a.package_local_abi.local_abi_identity,
        dependency_b.package_local_abi.local_abi_identity
    );

    let root_package = package(
        "package.identity-root",
        &[("dependency", &dependency_a)],
        &[],
    );
    let deployment_a = deployment(
        &root_contract,
        &root_package,
        "revision",
        vec![package_binding(&root_package, "dependency", &dependency_a)],
        Vec::new(),
    );
    let deployment_b = deployment(
        &root_contract,
        &root_package,
        "revision",
        vec![package_binding(&root_package, "dependency", &dependency_b)],
        Vec::new(),
    );
    let build_a = resolve_runtime_assembly(
        &[deployment_ref(&deployment_a)],
        &[deployment_a],
        std::slice::from_ref(&root_contract),
        &[root_package.clone(), dependency_a.clone()],
    )
    .unwrap();
    let build_b = resolve_runtime_assembly(
        &[deployment_ref(&deployment_b)],
        &[deployment_b],
        std::slice::from_ref(&root_contract),
        &[root_package.clone(), dependency_b],
    )
    .unwrap();
    assert_ne!(build_a.assembly_identity, build_b.assembly_identity);
}
