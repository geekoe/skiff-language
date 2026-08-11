mod common;

use common::{package_project::compile_service_package_project, TestDir};
use serde_json::json;
use skiff_artifact_model::{
    HttpGatewayDocumentAuthoring, ServiceManifestAuthoring, WebSocketGatewayDocumentAuthoring,
};
use skiff_compiler::{
    generate_service_deployment, generate_service_deployment_with_validated_packages,
    GeneratedServiceDeploymentError, GeneratedServiceDeploymentInput,
    GeneratedServicePackageAdmissions, ServiceApiProjection,
};
use skiff_compiler_core::id::PublicationId;

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
        service: &manifest(),
        http: None,
        websocket: None,
        service_api: api,
        implementation: artifact,
        package_closure: closure,
        package_schema_records,
    })
    .unwrap()
}

fn generate_with_manifest(
    artifact: &skiff_artifact_model::PackageArtifact,
    api: &ServiceApiProjection,
    package_schema_records: &std::collections::BTreeMap<
        skiff_artifact_model::PackageSchemaTypeId,
        skiff_artifact_model::PackageSchemaTypeRecord,
    >,
    service: &ServiceManifestAuthoring,
) -> skiff_artifact_model::ServiceDeployment {
    generate_service_deployment(GeneratedServiceDeploymentInput {
        service,
        http: None,
        websocket: None,
        service_api: api,
        implementation: artifact,
        package_closure: &[],
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
    root.write(
        "service.yml",
        "id: example.com/registry\nserviceCalls:\n  - read\n",
    );
    root.write("api.yml", "read: main.read\n");
    root.write(
        "main.skiff",
        format!(
            "function read() -> string {{ return {response} }}\nfunction configured() -> string {{ return config.require<string>(\"registry.token\") }}\n"
        ),
    );
    compile_service_package_project(root.path()).unwrap()
}

fn manifest() -> ServiceManifestAuthoring {
    manifest_with_calls(&["read"])
}

fn manifest_with_calls(service_calls: &[&str]) -> ServiceManifestAuthoring {
    ServiceManifestAuthoring {
        id: "example.com/registry".to_string(),
        kind: skiff_artifact_model::ServiceAuthoringKind::Service,
        service_calls: service_calls
            .iter()
            .map(|path| (*path).to_string())
            .collect(),
    }
}

fn compile_selection_fixture(
    name: &str,
    service_calls: &[&str],
) -> (
    common::package_project::PublishedPackageProject,
    ServiceApiProjection,
) {
    let root = TestDir::new("skiff-compiler", name);
    root.write(
        "package.yml",
        "id: example.com/registry-package\nversion: 7.4.0\n",
    );
    root.write(
        "service.yml",
        if service_calls.is_empty() {
            "id: example.com/registry\nserviceCalls: []\n".to_string()
        } else {
            format!(
                "id: example.com/registry\nserviceCalls:\n{}",
                service_calls
                    .iter()
                    .map(|path| format!("  - {path}\n"))
                    .collect::<String>()
            )
        },
    );
    root.write("api.yml", "read: main.read\nwrite: main.write\n");
    root.write(
        "main.skiff",
        "function read() -> string { return \"read\" }\nfunction write() -> string { return \"write\" }\nfunction configured() -> string { return config.require<string>(\"registry.token\") }\n",
    );
    compile_service_package_project(root.path()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_exact_operations_without_runtime_config_values() {
        let (project, service_api) = compile_fixture("generated-positive", "\"ok\"");
        let service = manifest();
        let deployment = generate_service_deployment(GeneratedServiceDeploymentInput {
            service: &service,
            http: None,
            websocket: None,
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
        assert_eq!(
            deployment.operation_bindings[0].package_callable_id,
            service_api.available["read"]
        );
        let operation_wire = serde_json::to_value(&deployment.operation_bindings[0]).unwrap();
        assert_eq!(
            operation_wire["packageCallableId"],
            json!(service_api.available["read"].as_str())
        );
        assert!(operation_wire.get("packagePublicPath").is_none());
        assert!(deployment.gateway_entries.is_empty());
        assert!(deployment.ingress.is_empty());

        let explicit_empty_http = HttpGatewayDocumentAuthoring::default();
        let explicit_empty = generate_service_deployment(GeneratedServiceDeploymentInput {
            service: &service,
            http: Some(&explicit_empty_http),
            websocket: None,
            service_api: &service_api,
            implementation: &project.package.artifact,
            package_closure: &[],
            package_schema_records: &project.package.resolved_package_schema_type_records,
        })
        .unwrap();
        assert!(explicit_empty.gateway_entries.is_empty());
        assert!(explicit_empty.ingress.is_empty());
    }

    #[test]
    fn validated_package_admission_cannot_be_reused_for_different_input() {
        let (project, service_api) =
            compile_fixture("generated-validated-package-mismatch", "\"ok\"");
        let admissions =
            GeneratedServicePackageAdmissions::admit(&project.package.artifact, &[]).unwrap();
        let mut different = project.package.artifact.clone();
        different.package_id = "example.com/different-package".to_string();
        let service = manifest();

        let error = generate_service_deployment_with_validated_packages(
            GeneratedServiceDeploymentInput {
                service: &service,
                http: None,
                websocket: None,
                service_api: &service_api,
                implementation: &different,
                package_closure: &[],
                package_schema_records: &project.package.resolved_package_schema_type_records,
            },
            &admissions,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            GeneratedServiceDeploymentError::InvalidInput(message)
                if message.contains("validated implementation PackageArtifact")
        ));
    }

    #[test]
    fn dependency_declared_collection_remains_provider_owned_logical_input() {
        let root = TestDir::new("skiff-compiler", "declared-collection-identity");
        root.write(
            "package.yml",
            r#"
id: example.com/mapping-service-package
version: 1.0.0
packages:
  - id: example.com/mapping-store
    version: 1.0.0
    alias: store
  - id: example.com/mapping-audit
    version: 1.0.0
    alias: audit
"#,
        );
        root.write(
            "service.yml",
            "id: example.com/mapping-service\nserviceCalls:\n  - read\n",
        );
        root.write("api.yml", "read: main.read\n");
        root.write(
            "main.skiff",
            "function read() -> string { return \"ok\" }\n",
        );

        let dependency_path = std::path::PathBuf::from(".skiff-packages")
            .join(
                PublicationId::parse("example.com/mapping-store")
                    .unwrap()
                    .artifact_path(),
            )
            .join("1.0.0");
        root.write(
            dependency_path.join("package.yml"),
            "id: example.com/mapping-store\nversion: 1.0.0\n",
        );
        root.write(dependency_path.join("api.yml"), "{}\n");
        root.write(
            dependency_path.join("store.skiff"),
            r#"
type PackageSecret { id: string, value: string }
db object PackageSecret {
  name "package_secret"
  primary key(id)
}
"#,
        );
        let audit_dependency_path = std::path::PathBuf::from(".skiff-packages")
            .join(
                PublicationId::parse("example.com/mapping-audit")
                    .unwrap()
                    .artifact_path(),
            )
            .join("1.0.0");
        root.write(
            audit_dependency_path.join("package.yml"),
            "id: example.com/mapping-audit\nversion: 1.0.0\n",
        );
        root.write(audit_dependency_path.join("api.yml"), "{}\n");
        root.write(
            audit_dependency_path.join("audit.skiff"),
            r#"
type PackageSecret { id: string, value: string }
db object PackageSecret {
  name "package_secret"
  primary key(id)
}
"#,
        );

        let (project, service_api) = compile_service_package_project(root.path()).unwrap();
        for package_id in ["example.com/mapping-store", "example.com/mapping-audit"] {
            let dependency = project
                .dependency(package_id, "1.0.0")
                .expect("fresh dependency artifact");
            assert!(dependency
                .file_ir_units
                .iter()
                .flat_map(|file| file.unit.declarations.db.values())
                .any(
                    |declaration| declaration.collection_name.as_deref() == Some("package_secret")
                ));
        }

        let service = ServiceManifestAuthoring {
            id: "example.com/mapping-service".to_string(),
            kind: skiff_artifact_model::ServiceAuthoringKind::Service,
            service_calls: vec!["read".to_string()],
        };
        let closure = project
            .dependency_packages
            .iter()
            .map(|package| package.artifact.clone())
            .collect::<Vec<_>>();
        let deployment = generate_service_deployment(GeneratedServiceDeploymentInput {
            service: &service,
            http: None,
            websocket: None,
            service_api: &service_api,
            implementation: &project.package.artifact,
            package_closure: &closure,
            package_schema_records: &project.package.resolved_package_schema_type_records,
        })
        .unwrap();
        assert_eq!(deployment.package_bindings.len(), 2);
        assert!(deployment
            .package_bindings
            .iter()
            .all(|binding| binding.package.package_id != "example.com/mapping-service-package"));
    }

    #[test]
    fn automatic_service_api_mapping_fails_closed() {
        let (project, mut service_api) = compile_fixture("generated-negative", "\"ok\"");

        service_api.available.clear();
        let error = generate_service_deployment(GeneratedServiceDeploymentInput {
            service: &manifest(),
            http: None,
            websocket: None,
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
        skiff_artifact_identity::assign_service_contract_identities(&mut duplicate.contract)
            .unwrap();
        let error = generate_service_deployment(GeneratedServiceDeploymentInput {
            service: &manifest(),
            http: None,
            websocket: None,
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
    fn generated_deployment_identity_failure_uses_compiler_owned_error_shape() {
        let (project, mut service_api) =
            compile_fixture("generated-identity-error-facade", "\"ok\"");
        service_api.contract.service_protocol_identity =
            skiff_artifact_model::ServiceProtocolIdentity::new("sha256-deadbeef");

        let error = generate_service_deployment(GeneratedServiceDeploymentInput {
            service: &manifest(),
            http: None,
            websocket: None,
            service_api: &service_api,
            implementation: &project.package.artifact,
            package_closure: &[],
            package_schema_records: &project.package.resolved_package_schema_type_records,
        })
        .unwrap_err();

        let GeneratedServiceDeploymentError::Identity { message } = error else {
            panic!("expected compiler-owned identity error facade");
        };
        assert!(message.contains("identity"), "{message}");
    }

    #[test]
    fn generated_deployment_projection_failure_uses_compiler_owned_error_shape() {
        let (project, service_api) = compile_fixture("generated-projection-error-facade", "\"ok\"");
        let repeated = vec![
            project.package.artifact.clone(),
            project.package.artifact.clone(),
        ];

        let error = generate_service_deployment(GeneratedServiceDeploymentInput {
            service: &manifest(),
            http: None,
            websocket: None,
            service_api: &service_api,
            implementation: &project.package.artifact,
            package_closure: &repeated,
            package_schema_records: &project.package.resolved_package_schema_type_records,
        })
        .unwrap_err();

        let GeneratedServiceDeploymentError::Projection { message } = error else {
            panic!("expected compiler-owned projection error facade");
        };
        assert!(message.contains("repeats build"), "{message}");
    }

    #[test]
    fn generated_service_deployment_projects_named_http_without_contract_operation() {
        let root = TestDir::new("skiff-compiler", "generated-http-gateway");
        root.write(
            "package.yml",
            "id: example.com/http-package\nversion: 7.4.0\n",
        );
        root.write("service.yml", "id: example.com/http\n");
        root.write("api.yml", "health: main.health\n");
        root.write(
            "main.skiff",
            r#"import std

function raw(request: std.http.HttpRequest) -> std.http.HttpResponse {
  return std.http.noContent()
}

function health() -> string {
  return "ok"
}
"#,
        );
        let (project, service_api) = compile_service_package_project(root.path()).unwrap();
        let service_source = "id: example.com/http\n";
        let service = serde_yaml::from_str::<ServiceManifestAuthoring>(service_source).unwrap();
        let http = serde_yaml::from_str::<HttpGatewayDocumentAuthoring>(
            r#"
raw:
  method: GET
  path: /artifacts
  kind: rawHttp
  handler: main.raw
  adapterArgs:
    - param: request
      source: { kind: http.request }
"#,
        )
        .unwrap();
        let closure = project
            .dependency_packages
            .iter()
            .map(|package| package.artifact.clone())
            .collect::<Vec<_>>();
        let deployment = generate_service_deployment(GeneratedServiceDeploymentInput {
            service: &service,
            http: Some(&http),
            websocket: None,
            service_api: &service_api,
            implementation: &project.package.artifact,
            package_closure: &closure,
            package_schema_records: &project.package.resolved_package_schema_type_records,
        })
        .unwrap();
        assert!(deployment.operation_bindings.is_empty());
        assert_eq!(deployment.gateway_entries.len(), 1);
        assert_eq!(deployment.ingress.len(), 1);
    }

    #[test]
    fn generated_service_deployment_refuses_legacy_websocket_operation_ingress() {
        let error = serde_yaml::from_str::<WebSocketGatewayDocumentAuthoring>(
            r#"
routes:
  - path: /events
    operation: read
"#,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("unknown field `routes`"),
            "{error}"
        );
    }

    #[test]
    fn service_identity_mismatch_fails_closed() {
        let (project, service_api) = compile_fixture("generated-unbound", "\"ok\"");
        let mut wrong_service = manifest();
        wrong_service.id = "example.com/other".to_string();
        let error = generate_service_deployment(GeneratedServiceDeploymentInput {
            service: &wrong_service,
            http: None,
            websocket: None,
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
    fn manifest_selection_canonicalizes_revision_and_changes_only_service_outputs() {
        let (left_project, left_api) =
            compile_selection_fixture("generated-selection-left", &["read", "write"]);
        let (right_project, right_api) =
            compile_selection_fixture("generated-selection-right", &["write", "read"]);
        assert_eq!(
            left_project.package.artifact,
            right_project.package.artifact
        );
        assert_eq!(left_api.contract, right_api.contract);

        let left_manifest = manifest_with_calls(&["read", "write"]);
        let right_manifest = manifest_with_calls(&["write", "read"]);
        let left = generate_with_manifest(
            &left_project.package.artifact,
            &left_api,
            &left_project.package.resolved_package_schema_type_records,
            &left_manifest,
        );
        let right = generate_with_manifest(
            &right_project.package.artifact,
            &right_api,
            &right_project.package.resolved_package_schema_type_records,
            &right_manifest,
        );
        assert_eq!(left.deployment_revision, right.deployment_revision);
        assert_eq!(
            left.deployment_artifact_identity,
            right.deployment_artifact_identity
        );
        assert_eq!(left.operation_bindings, right.operation_bindings);

        let (subset_project, subset_api) =
            compile_selection_fixture("generated-selection-subset", &["read"]);
        assert_eq!(
            left_project.package.artifact,
            subset_project.package.artifact
        );
        assert_eq!(
            left_project.package.artifact.package_build_id,
            subset_project.package.artifact.package_build_id
        );
        assert_eq!(
            left_project
                .package
                .artifact
                .package_local_abi
                .local_abi_identity,
            subset_project
                .package
                .artifact
                .package_local_abi
                .local_abi_identity
        );
        assert_ne!(
            left_api.contract.service_protocol_identity,
            subset_api.contract.service_protocol_identity
        );
        let subset_manifest = manifest_with_calls(&["read"]);
        let subset = generate_with_manifest(
            &subset_project.package.artifact,
            &subset_api,
            &subset_project.package.resolved_package_schema_type_records,
            &subset_manifest,
        );
        assert_ne!(left.operation_bindings, subset.operation_bindings);
        assert_ne!(left.deployment_revision, subset.deployment_revision);
        assert_ne!(
            left.deployment_artifact_identity,
            subset.deployment_artifact_identity
        );

        let (zero_project, zero_api) = compile_selection_fixture("generated-selection-zero", &[]);
        let missing =
            serde_yaml::from_str::<ServiceManifestAuthoring>("id: example.com/registry\n").unwrap();
        let explicit_empty = serde_yaml::from_str::<ServiceManifestAuthoring>(
            "id: example.com/registry\nserviceCalls: []\n",
        )
        .unwrap();
        let missing_deployment = generate_with_manifest(
            &zero_project.package.artifact,
            &zero_api,
            &zero_project.package.resolved_package_schema_type_records,
            &missing,
        );
        let empty_deployment = generate_with_manifest(
            &zero_project.package.artifact,
            &zero_api,
            &zero_project.package.resolved_package_schema_type_records,
            &explicit_empty,
        );
        assert!(missing_deployment.operation_bindings.is_empty());
        assert_eq!(
            missing_deployment.deployment_revision,
            empty_deployment.deployment_revision
        );
        assert_eq!(
            missing_deployment.deployment_artifact_identity,
            empty_deployment.deployment_artifact_identity
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
}
