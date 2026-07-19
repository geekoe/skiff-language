use std::sync::Arc;

use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, FileIrRef, FileIrUnit, PackageArtifact,
    PackageArtifactRef, PublicationResourceRef, RuntimeAssembly, ServiceContract,
    ServiceContractRef, ServiceDeployment, ServiceDeploymentRef, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_runtime_loader::{RuntimeAssemblyContentResolver, RuntimeAssemblyLoader};

use super::*;

mod fixtures;

use fixtures::CycleFixture;

struct NoContent;

impl RuntimeAssemblyContentResolver for NoContent {
    fn resolve_deployment(
        &self,
        _reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        panic!("empty assembly must not resolve deployments")
    }

    fn resolve_contract(
        &self,
        _reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        panic!("empty assembly must not resolve contracts")
    }

    fn resolve_package(
        &self,
        _reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        panic!("empty assembly must not resolve packages")
    }

    fn resolve_file_ir(
        &self,
        _package: &PackageArtifactRef,
        _reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        panic!("empty assembly must not resolve File IR")
    }

    fn resolve_static_resource(
        &self,
        _package: &PackageArtifactRef,
        _reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        panic!("empty assembly must not resolve resources")
    }
}

#[test]
fn empty_assembly_links_and_all_candidate_lookups_fail_closed() {
    let mut assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("unassigned"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: Vec::new(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: Vec::new(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        global_ingress: Vec::new(),
    };
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut assembly).unwrap();
    let hydrated = RuntimeAssemblyLoader::new(&NoContent)
        .load(assembly)
        .unwrap();

    let candidate = link_runtime_assembly(hydrated).unwrap();

    assert!(candidate.is_empty());
    assert_eq!(candidate.activations().len(), 0);
    assert_eq!(candidate.ingress_bindings().len(), 0);
    assert!(candidate
        .activation(&ServiceDeploymentRef {
            service_id: "missing".to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: skiff_artifact_model::DeploymentRevision::new("missing"),
            deployment_artifact_identity: skiff_artifact_model::DeploymentArtifactIdentity::new(
                "missing"
            ),
        })
        .is_none());
}

#[test]
fn candidate_keeps_code_shared_and_service_bindings_activation_relative() {
    let fixture = CycleFixture::new();
    let hydrated = RuntimeAssemblyLoader::new(&fixture.resolver)
        .load(fixture.assembly.clone())
        .unwrap();

    let candidate = link_runtime_assembly(hydrated).unwrap();

    assert_eq!(candidate.shared_image().code_slots().len(), 2);
    let activation_a = candidate.activation(&fixture.activation_a).unwrap();
    let activation_b = candidate.activation(&fixture.activation_b).unwrap();
    assert_eq!(
        activation_a.implementation_code_slot(),
        activation_b.implementation_code_slot()
    );
    assert_eq!(
        activation_a.implementation_package_build_id(),
        &fixture.shared_build
    );
    assert_ne!(
        activation_a.source().config_literals,
        activation_b.source().config_literals
    );
    assert_ne!(
        activation_a.source().state_bindings,
        activation_b.source().state_bindings
    );
    assert_ne!(
        activation_a.source().resource_bindings,
        activation_b.source().resource_bindings
    );

    let direct_call = candidate
        .shared_image()
        .resolve_package_direct_call_by_alias(
            &fixture.shared_build,
            "helper",
            &fixture.helper_callable,
        )
        .unwrap();
    assert_eq!(
        direct_call.dependency_package_build_id(),
        &fixture.helper_build
    );
    assert_eq!(direct_call.package_callable_id(), &fixture.helper_callable);
    assert_eq!(
        candidate
            .shared_image()
            .code_by_build(&fixture.helper_build)
            .unwrap()
            .static_resources()
            .get("assets/helper.txt")
            .unwrap()
            .bytes
            .as_ref(),
        b"shared helper resource"
    );

    let service_call = candidate
        .shared_image()
        .resolve_activation_relative_service_call(
            &fixture.shared_build,
            &fixture.shared_file_identity,
            skiff_artifact_model::ServiceCallRefIndex::new(0),
        )
        .unwrap();
    let binding_a = candidate
        .resolve_activation_relative_service_call(&fixture.activation_a, &service_call)
        .unwrap();
    let binding_b = candidate
        .resolve_activation_relative_service_call(&fixture.activation_b, &service_call)
        .unwrap();
    assert_eq!(binding_a.provider(), &fixture.activation_b);
    assert_eq!(binding_b.provider(), &fixture.activation_a);
    assert_ne!(binding_a.provider(), binding_b.provider());

    let provider_operation = candidate
        .activation(binding_a.provider())
        .unwrap()
        .operation(service_call.operation_id())
        .unwrap();
    assert_eq!(
        provider_operation.package_callable_id(),
        &fixture.service_callable
    );
    assert_eq!(
        provider_operation.target().callable_abi_id,
        fixture.service_callable.as_str()
    );
}

#[test]
fn candidate_retains_canonical_contract_descriptor_and_typed_ingress() {
    let fixture = CycleFixture::new();
    let hydrated = RuntimeAssemblyLoader::new(&fixture.resolver)
        .load(fixture.assembly.clone())
        .unwrap();
    let candidate = link_runtime_assembly(hydrated).unwrap();

    let through_candidate = candidate
        .operation_descriptor(&fixture.contract_ref, &fixture.operation_id)
        .unwrap();
    let through_store = candidate
        .contract_store()
        .operation_descriptor(&fixture.contract_ref, &fixture.operation_id)
        .unwrap();
    assert!(std::ptr::eq(through_candidate, through_store));
    assert_eq!(through_candidate.operation_id, fixture.operation_id);

    let ingress = candidate.ingress(&fixture.ingress_selector).unwrap();
    assert_eq!(ingress.deployment, fixture.activation_a);
    assert_eq!(ingress.contract, fixture.contract_ref);
    assert_eq!(ingress.contract_operation_id, fixture.operation_id);
}

#[test]
fn tampered_activation_template_fails_before_a_partial_candidate_exists() {
    let mut fixture = CycleFixture::new();
    fixture.assembly.activation_templates[0].config_literals[0].value =
        skiff_artifact_model::MetadataValue::String("tampered".to_string());
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut fixture.assembly).unwrap();

    let error = RuntimeAssemblyLoader::new(&fixture.resolver)
        .load(fixture.assembly)
        .unwrap_err();

    assert!(
        error.to_string().contains("activation template"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn canonical_calls_cannot_fall_back_to_the_service_specific_converter() {
    let fixture = CycleFixture::new();
    let shared_file = fixture
        .resolver
        .file(&fixture.shared_build, &fixture.shared_file_identity);

    let error = crate::linked_file_unit_from_artifact(shared_file)
        .expect_err("canonical calls require the assembly linker");

    assert!(error.to_string().contains("RuntimeAssembly"));
}

#[test]
fn missing_provider_callable_is_rejected_before_linking_a_candidate() {
    let mut fixture = CycleFixture::new();
    fixture.tamper_deployment_callable();

    let error = RuntimeAssemblyLoader::new(&fixture.resolver)
        .load(fixture.assembly)
        .unwrap_err();

    assert!(
        error.to_string().contains("missing callable"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn link_plan_abi_protocol_and_ingress_tamper_fail_closed() {
    let fixture = CycleFixture::new();

    let mut wrong_abi = fixture.assembly.clone();
    wrong_abi.package_link_plan.package_links[0]
        .package
        .package_local_abi_identity =
        skiff_artifact_model::PackageLocalAbiIdentity::new("tampered-abi");
    assert!(skiff_artifact_identity::assign_runtime_assembly_identity(&mut wrong_abi).is_err());

    let mut wrong_protocol = fixture.assembly.clone();
    wrong_protocol.service_binding_templates[0].bindings[0]
        .contract
        .service_protocol_identity =
        skiff_artifact_model::ServiceProtocolIdentity::new("tampered-protocol");
    assert!(
        skiff_artifact_identity::assign_runtime_assembly_identity(&mut wrong_protocol).is_err()
    );

    let mut ingress_collision = fixture.assembly;
    ingress_collision
        .global_ingress
        .push(ingress_collision.global_ingress[0].clone());
    assert!(
        skiff_artifact_identity::assign_runtime_assembly_identity(&mut ingress_collision).is_err()
    );
}
