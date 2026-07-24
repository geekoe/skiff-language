mod common;

use common::{package_project::compile_package_project, TestDir};
use serde_json::json;
use skiff_artifact_model::{
    BoundaryUnavailableReason, ServiceConfigProfileAuthoring, ServiceManifestAuthoring,
};
use skiff_compiler::{
    generate_service_deployment, GeneratedServiceDeploymentInput, ServiceApiProjection,
};
use skiff_compiler_contract::project_service_api;

#[test]
fn generates_exact_operations_ingress_and_profile_bindings() {
    let (project, service_api) = compile_fixture("generated-positive", "\"ok\"");
    let service = manifest("read");
    let profile = profile();
    let deployment = generate_service_deployment(GeneratedServiceDeploymentInput {
        service: &service,
        profile_name: "prod",
        profile: &profile,
        service_api: &service_api,
        implementation: &project.package.artifact,
        package_closure: &[],
        package_schema_records: &project.package.resolved_package_schema_type_records,
    })
    .unwrap();

    assert_eq!(deployment.contract.service_id, "example.com/registry");
    assert_eq!(deployment.contract.contract_version, "7.4.0");
    assert_eq!(
        deployment.implementation,
        skiff_artifact_identity::package_artifact_ref(&project.package.artifact).unwrap()
    );
    assert_eq!(deployment.operation_bindings.len(), 1);
    assert_eq!(deployment.ingress.len(), 1);
    assert_eq!(
        deployment.ingress[0].contract_operation_id,
        deployment.operation_bindings[0].contract_operation_id
    );
    assert_eq!(deployment.config_literals[0].path, "registry.token");
    assert_eq!(deployment.policy.principal, "service:registry");
}

#[test]
fn ingress_and_mapping_fail_closed() {
    let (project, mut service_api) = compile_fixture("generated-negative", "\"ok\"");
    let profile = profile();
    service_api.unavailable.insert(
        "notPublic".to_string(),
        vec![BoundaryUnavailableReason::UnknownEffect],
    );

    let error = generate_service_deployment(GeneratedServiceDeploymentInput {
        service: &manifest("notPublic"),
        profile_name: "prod",
        profile: &profile,
        service_api: &service_api,
        implementation: &project.package.artifact,
        package_closure: &[],
        package_schema_records: &project.package.resolved_package_schema_type_records,
    })
    .unwrap_err();
    assert!(error.to_string().contains("boundary unavailable"));

    service_api.available.clear();
    let error = generate_service_deployment(GeneratedServiceDeploymentInput {
        service: &manifest("read"),
        profile_name: "prod",
        profile: &profile,
        service_api: &service_api,
        implementation: &project.package.artifact,
        package_closure: &[],
        package_schema_records: &project.package.resolved_package_schema_type_records,
    })
    .unwrap_err();
    assert!(error.to_string().contains("missing"));

    let (duplicate_project, mut duplicate) = compile_fixture("generated-duplicate", "\"ok\"");
    let callable = duplicate.available["read"].clone();
    let original = duplicate
        .contract
        .operations
        .values()
        .next()
        .unwrap()
        .clone();
    let duplicate_id = skiff_artifact_identity::contract_operation_id(
        "example.com/registry",
        "7.4.0",
        "readAlias",
    )
    .unwrap();
    duplicate.contract.operations.insert(
        duplicate_id.clone(),
        skiff_artifact_model::BoundaryOperationDescriptor {
            operation_id: duplicate_id,
            stable_key: "readAlias".to_string(),
            contract: original.contract,
        },
    );
    duplicate
        .available
        .insert("readAlias".to_string(), callable);
    skiff_artifact_identity::assign_service_contract_identities(&mut duplicate.contract).unwrap();
    let error = generate_service_deployment(GeneratedServiceDeploymentInput {
        service: &manifest("read"),
        profile_name: "prod",
        profile: &profile,
        service_api: &duplicate,
        implementation: &duplicate_project.package.artifact,
        package_closure: &[],
        package_schema_records: &duplicate_project
            .package
            .resolved_package_schema_type_records,
    })
    .unwrap_err();
    assert!(error.to_string().contains("duplicate source callable"));
}

#[test]
fn unbound_requirement_and_identity_mismatch_fail_closed() {
    let (project, service_api) = compile_fixture("generated-unbound", "\"ok\"");
    let mut missing = profile();
    missing.config = json!({});
    let error = generate_service_deployment(GeneratedServiceDeploymentInput {
        service: &manifest("read"),
        profile_name: "prod",
        profile: &missing,
        service_api: &service_api,
        implementation: &project.package.artifact,
        package_closure: &[],
        package_schema_records: &project.package.resolved_package_schema_type_records,
    })
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("missing config binding registry.token"));

    let mut wrong_service = manifest("read");
    wrong_service.id = "example.com/other".to_string();
    let error = generate_service_deployment(GeneratedServiceDeploymentInput {
        service: &wrong_service,
        profile_name: "prod",
        profile: &profile(),
        service_api: &service_api,
        implementation: &project.package.artifact,
        package_closure: &[],
        package_schema_records: &project.package.resolved_package_schema_type_records,
    })
    .unwrap_err();
    assert!(error.to_string().contains("does not match"));
}

#[test]
fn compatible_rebuild_changes_package_identity_not_service_api_identity() {
    let (left, left_api) = compile_fixture("generated-rebuild-left", "\"left\"");
    let (right, right_api) = compile_fixture("generated-rebuild-right", "\"right\"");
    assert_ne!(
        left.package.artifact.package_build_id,
        right.package.artifact.package_build_id
    );
    assert_eq!(
        left_api.contract.service_protocol_identity,
        right_api.contract.service_protocol_identity
    );

    let left_deployment = generate(
        &left.package.artifact,
        &[],
        &left_api,
        &left.package.resolved_package_schema_type_records,
    );
    let right_deployment = generate(
        &right.package.artifact,
        &[],
        &right_api,
        &right.package.resolved_package_schema_type_records,
    );
    assert_ne!(
        left_deployment.implementation.package_build_id,
        right_deployment.implementation.package_build_id
    );
    assert_eq!(
        left_deployment.contract.service_protocol_identity,
        right_deployment.contract.service_protocol_identity
    );
}

#[test]
fn generated_service_package_and_deployment_identities_ignore_human_version_relabeling() {
    let (base, base_api) = compile_fixture("generated-version-base", "\"ok\"");
    let base_deployment = generate(
        &base.package.artifact,
        &[],
        &base_api,
        &base.package.resolved_package_schema_type_records,
    );

    let mut relabeled_artifact = base.package.artifact.clone();
    relabeled_artifact.package_version = "99.0.0".to_string();
    let mut relabeled_api = base_api.clone();
    relabeled_api.contract.contract_version = "99.0.0".to_string();
    skiff_artifact_identity::assign_service_contract_identities(&mut relabeled_api.contract)
        .unwrap();
    let relabeled_deployment = generate(
        &relabeled_artifact,
        &[],
        &relabeled_api,
        &base.package.resolved_package_schema_type_records,
    );

    assert_eq!(
        base.package.artifact.package_build_id,
        relabeled_artifact.package_build_id
    );
    assert_eq!(
        base.package.artifact.package_local_abi.local_abi_identity,
        relabeled_artifact.package_local_abi.local_abi_identity
    );
    assert_eq!(
        base_deployment.deployment_artifact_identity,
        relabeled_deployment.deployment_artifact_identity
    );
    assert_ne!(
        base_deployment.contract.contract_version,
        relabeled_deployment.contract.contract_version
    );
}

fn generate(
    artifact: &skiff_artifact_model::PackageArtifact,
    closure: &[skiff_artifact_model::PackageArtifact],
    api: &ServiceApiProjection,
    package_schema_records: &std::collections::BTreeMap<
        skiff_artifact_model::PackageSchemaTypeId,
        skiff_artifact_model::PackageSchemaTypeRecord,
    >,
) -> skiff_artifact_model::ServiceDeployment {
    generate_service_deployment(GeneratedServiceDeploymentInput {
        service: &manifest("read"),
        profile_name: "prod",
        profile: &profile(),
        service_api: api,
        implementation: artifact,
        package_closure: closure,
        package_schema_records,
    })
    .unwrap()
}

fn compile_fixture(
    name: &str,
    response: &str,
) -> (
    common::package_project::PublishedPackageProject,
    ServiceApiProjection,
) {
    let root = TestDir::new("skiff-compiler", name);
    root.write(
        "package.yml",
        "id: example.com/registry-package\nversion: 7.4.0\n",
    );
    root.write("api.yml", "read: main.read\n");
    root.write(
        "main.skiff",
        &format!(
            "function read() -> string {{ return {response} }}\nfunction configured() -> string {{ return config.require<string>(\"registry.token\") }}\n"
        ),
    );
    let project = compile_package_project(root.path()).unwrap();
    let api = project_service_api(
        "example.com/registry",
        &project.package.artifact,
        &project.package.package_schema_type_records,
    )
    .unwrap();
    (project, api)
}

fn manifest(operation: &str) -> ServiceManifestAuthoring {
    ServiceManifestAuthoring {
        id: "example.com/registry".to_string(),
        http: Some(json!({
            "routes": [{
                "method": "GET",
                "path": "/artifacts",
                "operation": operation
            }]
        })),
        websocket: None,
        timeout: None,
    }
}

fn profile() -> ServiceConfigProfileAuthoring {
    ServiceConfigProfileAuthoring {
        config: json!({"registry.token": "token"}),
        secrets: json!({}),
        state: json!({}),
        resources: json!({}),
        timeout: json!(1000),
        quota: json!({"cpuMillis": 100, "memoryBytes": 1048576}),
        principal: json!("service:registry"),
        lifecycle: json!({"maxConcurrency": 4}),
    }
}
