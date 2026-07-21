mod support;

use std::sync::Arc;

use skiff_artifact_model::{PackageRefIr, ServiceCallRefIndex};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeValue},
};
use skiff_runtime_package_test::{PackageTestEntrypoint, PackageTestRuntimeBuilder};

use support::CanonicalFixture;

fn entrypoint(fixture: &CanonicalFixture) -> PackageTestEntrypoint {
    PackageTestEntrypoint {
        id: "case".to_string(),
        deployment: fixture.root.clone(),
        contract: fixture.root_contract.clone(),
        operation: fixture.root_operation.clone(),
    }
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
    assert_eq!(loaded.operation_target().unwrap().executable_index, 0);
}

#[test]
fn canonical_package_direct_mutation_keeps_the_callers_heap_identity() {
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

    // Package-direct dispatch does not materialize an InProcessBoundary value graph. Exercise
    // the real runtime mutation primitive against the exact caller handle after resolving the
    // callee in the shared assembly image: the caller must observe the callee-side write.
    let mut request_heap = RequestHeap::default();
    let caller_handle = request_heap
        .alloc_array(vec![RuntimeValue::String("caller".to_string())])
        .expect("caller array");
    let callee_argument = RuntimeValue::Heap(caller_handle);
    skiff_runtime_eval::program_mutation::assign_program_index_target(
        &mut request_heap,
        &callee_argument,
        &RuntimeValue::Number(0.0),
        RuntimeValue::String("package-callee".to_string()),
    )
    .expect("same-heap callee mutation");
    let HeapNode::Array(caller_items) = request_heap.get(caller_handle).unwrap() else {
        panic!("caller handle must remain an array")
    };
    assert_eq!(
        caller_items,
        &[RuntimeValue::String("package-callee".to_string())]
    );
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
    assert_eq!(
        template
            .ingress_entrypoint(&fixture.ingress)
            .expect("Host ingress")
            .entrypoint()
            .operation,
        fixture.root_operation
    );
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
    assert!(format!("{error:#}").contains("package content is invalid"));
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
