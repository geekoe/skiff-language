mod support;

use std::sync::Arc;

use skiff_artifact_model::{
    DeploymentArtifactIdentity, GatewayEntryIdentity, GatewayEntryKey, PackageRefIr,
    ServiceCallRefIndex,
};
use skiff_runtime_package_test::{PackageTestEntrypoint, PackageTestRuntimeBuilder};

use support::CanonicalFixture;

fn entrypoint(fixture: &CanonicalFixture) -> PackageTestEntrypoint {
    PackageTestEntrypoint {
        id: "case".to_string(),
        deployment: fixture.root.clone(),
        gateway_entry_key: fixture.gateway_entry_key.clone(),
        gateway_entry_identity: fixture.gateway_entry_identity.clone(),
    }
}

fn template_error(
    fixture: &CanonicalFixture,
    entrypoints: impl IntoIterator<Item = PackageTestEntrypoint>,
) -> String {
    let resolver = fixture.resolver();
    let error = PackageTestRuntimeBuilder::new(&resolver)
        .load_template(fixture.assembly.clone(), entrypoints)
        .expect_err("invalid package-test entrypoints must fail closed");
    format!("{error:#}")
}

#[test]
fn package_only_artifact_loads_through_the_typed_assembly_path() {
    let fixture = CanonicalFixture::package_only();
    let resolver = fixture.resolver();
    let template = PackageTestRuntimeBuilder::new(&resolver)
        .load_template(fixture.assembly.clone(), [entrypoint(&fixture)])
        .expect("package-only canonical assembly");
    let loaded = template.load("case").expect("canonical entrypoint");

    assert_eq!(loaded.entrypoint().deployment, fixture.root);
    assert_eq!(
        loaded.entrypoint().gateway_entry_key,
        fixture.gateway_entry_key
    );
    assert_eq!(
        loaded.entrypoint().gateway_entry_identity,
        fixture.gateway_entry_identity
    );
    assert_eq!(loaded.handler_target().unwrap().executable_index, 0);
    assert_eq!(
        loaded.handler_target().unwrap(),
        template
            .candidate()
            .gateway_entry(&fixture.root, &fixture.gateway_entry_key)
            .expect("exact linked gateway entry")
            .handler()
            .target()
    );
}

#[test]
fn canonical_package_direct_target_is_linked_into_the_shared_execution_image() {
    let fixture = CanonicalFixture::package_dependency();
    let resolver = fixture.resolver();
    let template = PackageTestRuntimeBuilder::new(&resolver)
        .load_template(fixture.assembly.clone(), [entrypoint(&fixture)])
        .expect("package dependency canonical assembly");
    let caller = fixture.direct_caller.as_ref().unwrap();
    let dependency = fixture.direct_dependency.as_ref().unwrap();
    let callable = fixture.direct_callable.as_ref().unwrap();
    let direct = template
        .candidate()
        .shared_image()
        .resolve_package_direct_call(
            &caller.package_build_id,
            &PackageRefIr::Dependency {
                dependency_ref: "helper".to_string(),
            },
            callable,
        )
        .expect("package-direct target must resolve in the shared execution image");

    assert_eq!(
        direct.dependency_package_build_id(),
        &dependency.package_build_id
    );
    assert!(template
        .candidate()
        .execution_image()
        .executable_at(direct.executable_addr())
        .is_ok());
}

#[test]
fn provider_consumer_service_call_stays_activation_relative_and_ingress_is_canonical() {
    let fixture = CanonicalFixture::provider_consumer();
    let resolver = fixture.resolver();
    let template = PackageTestRuntimeBuilder::new(&resolver)
        .load_template(fixture.assembly.clone(), [entrypoint(&fixture)])
        .expect("provider/consumer canonical assembly");
    let caller_build = &fixture.packages[0].package_build_id;
    let caller_file = &fixture.files[0].1.file_ir_identity;
    let service_call = template
        .candidate()
        .shared_image()
        .resolve_activation_relative_service_call(
            caller_build,
            caller_file,
            ServiceCallRefIndex::new(0),
        )
        .expect("canonical service call instruction");
    let binding = template
        .candidate()
        .resolve_activation_relative_service_call(&fixture.root, &service_call)
        .expect("activation-relative provider binding");

    assert_eq!(
        binding.provider(),
        fixture.service_provider.as_ref().unwrap()
    );
    assert!(template
        .candidate()
        .operation_descriptor(&fixture.root_contract, &fixture.root_operation)
        .is_some());
    assert!(template
        .candidate()
        .activation(&fixture.root)
        .and_then(|activation| activation.operation(&fixture.root_operation))
        .is_some());

    let ingress = template
        .candidate()
        .ingress(&skiff_artifact_model::ServiceIngressKey {
            deployment: fixture.root.clone(),
            selector: fixture.ingress.clone(),
        })
        .expect("linked ingress selector");
    let owned = template
        .candidate()
        .gateway_entry(&fixture.root, &fixture.gateway_entry_key)
        .expect("test-owned linked gateway entry");
    assert!(Arc::ptr_eq(ingress, owned));
    let selected = template
        .ingress_entrypoint(&skiff_artifact_model::ServiceIngressKey {
            deployment: fixture.root.clone(),
            selector: fixture.ingress.clone(),
        })
        .expect("Host ingress");
    assert_eq!(selected.entrypoint().deployment, fixture.root);
    assert_eq!(
        selected.entrypoint().gateway_entry_key,
        fixture.gateway_entry_key
    );
    assert_eq!(
        selected.entrypoint().gateway_entry_identity,
        fixture.gateway_entry_identity
    );
}

#[test]
fn entrypoint_validation_rejects_non_exact_gateway_facts() {
    let fixture = CanonicalFixture::package_only();
    let valid = entrypoint(&fixture);

    let mut empty_id = valid.clone();
    empty_id.id = " \t".to_string();
    assert!(template_error(&fixture, [empty_id]).contains("id must not be empty"));

    assert!(template_error(&fixture, [valid.clone(), valid.clone()])
        .contains("duplicate package-test entrypoint id"));
    assert!(
        template_error(&fixture, Vec::<PackageTestEntrypoint>::new())
            .contains("requires at least one entrypoint")
    );

    let mut missing_deployment = valid.clone();
    missing_deployment.deployment.service_id = "missing.test/service".to_string();
    assert!(template_error(&fixture, [missing_deployment])
        .contains("deployment is not in RuntimeAssembly"));

    let mut wrong_deployment = valid.clone();
    wrong_deployment.deployment.deployment_artifact_identity =
        DeploymentArtifactIdentity::new("wrong-deployment-artifact");
    assert!(template_error(&fixture, [wrong_deployment])
        .contains("deployment is not in RuntimeAssembly"));

    let mut wrong_key = valid.clone();
    wrong_key.gateway_entry_key =
        GatewayEntryKey::parse("wrong-gateway-entry").expect("valid wrong gateway key");
    assert!(template_error(&fixture, [wrong_key]).contains("gateway entry is missing"));

    let mut wrong_identity = valid;
    wrong_identity.gateway_entry_identity =
        GatewayEntryIdentity::parse(format!("skiff-gateway-entry-v2:sha256:{}", "0".repeat(64)))
            .expect("valid mismatched gateway identity");
    assert_ne!(
        wrong_identity.gateway_entry_identity,
        fixture.gateway_entry_identity
    );
    assert!(template_error(&fixture, [wrong_identity])
        .contains("gateway entry identity does not match"));
}

#[test]
fn server_stream_gateway_entry_is_not_a_package_test_case_entrypoint() {
    let fixture = CanonicalFixture::raw_http_server_stream();
    let error = template_error(&fixture, [entrypoint(&fixture)]);
    assert!(
        error.contains("must reference an HTTP unary gateway entry"),
        "unexpected server-stream entrypoint error: {error}"
    );
}

#[test]
fn ingress_selector_does_not_match_a_test_entrypoint_owned_by_another_deployment() {
    let fixture = CanonicalFixture::provider_consumer();
    let provider = fixture
        .service_provider
        .clone()
        .expect("provider deployment");
    let provider_entry = PackageTestEntrypoint {
        id: "provider-case".to_string(),
        deployment: provider.clone(),
        gateway_entry_key: fixture.gateway_entry_key.clone(),
        gateway_entry_identity: fixture.gateway_entry_identity.clone(),
    };
    let resolver = fixture.resolver();
    let template = PackageTestRuntimeBuilder::new(&resolver)
        .load_template(fixture.assembly.clone(), [provider_entry])
        .expect("provider owns the same exact key and protocol identity");
    assert!(template
        .candidate()
        .gateway_entry(&provider, &fixture.gateway_entry_key)
        .is_some());

    let error = template
        .ingress_entrypoint(&skiff_artifact_model::ServiceIngressKey {
            deployment: fixture.root.clone(),
            selector: fixture.ingress.clone(),
        })
        .expect_err("root selector must not match a provider-owned test entrypoint");
    assert!(error.to_string().contains("has no test-owned entrypoint"));
}

#[test]
fn missing_and_tampered_package_artifacts_fail_closed() {
    let fixture = CanonicalFixture::package_only();
    let mut missing = fixture.resolver();
    missing.packages.clear();
    let error = PackageTestRuntimeBuilder::new(&missing)
        .load_template(fixture.assembly.clone(), [entrypoint(&fixture)])
        .unwrap_err();
    assert!(format!("{error:#}").contains("missing package"));

    let mut tampered = fixture.resolver();
    let package = tampered.packages.values_mut().next().unwrap();
    Arc::make_mut(package).package_version = "tampered".to_string();
    let error = PackageTestRuntimeBuilder::new(&tampered)
        .load_template(fixture.assembly.clone(), [entrypoint(&fixture)])
        .unwrap_err();
    let error = format!("{error:#}");
    assert!(
        error.contains("package content mismatches ref") && error.contains("tampered"),
        "unexpected tampered package error: {error}"
    );
}

#[test]
fn ambiguous_provider_and_service_call_without_deployment_are_rejected_by_assembly_resolution() {
    let fixture = CanonicalFixture::provider_consumer();
    let mut duplicate = fixture.deployments[1].clone();
    duplicate.deployment_revision = skiff_artifact_model::DeploymentRevision::new("provider-r2");
    skiff_artifact_identity::assign_service_deployment_identity(&mut duplicate).unwrap();
    let mut deployments = fixture.deployments.clone();
    deployments.push(duplicate);
    let error = skiff_deployment::assembly::resolve_runtime_assembly(
        std::slice::from_ref(&fixture.root),
        &deployments,
        &fixture.contracts,
        &fixture.packages,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("multiple deployment providers"),
        "unexpected resolution error: {error}"
    );

    let error = skiff_deployment::assembly::resolve_runtime_assembly(
        std::slice::from_ref(&fixture.root),
        std::slice::from_ref(&fixture.deployments[0]),
        &fixture.contracts,
        &fixture.packages,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("provider"),
        "unexpected resolution error: {error}"
    );
}
