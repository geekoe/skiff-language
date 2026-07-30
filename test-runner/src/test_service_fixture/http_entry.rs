use skiff_artifact_identity::ValidatedPackageArtifact;
use skiff_artifact_model::{ServiceContract, ServiceDeployment};
use skiff_compiler::{
    generate_service_deployment_with_validated_packages, GeneratedServiceDeploymentInput,
};

use crate::{canonical_fixture::CanonicalFixtureError, canonical_package::CanonicalPackageProject};

use super::profile::TEST_INGRESS_CONFIG_PATH;

pub(super) fn project(
    project: &CanonicalPackageProject,
    contract: &ServiceContract,
    runner_ingress: Option<&str>,
    implementation: &ValidatedPackageArtifact,
    package_closure: &[ValidatedPackageArtifact],
) -> Result<ServiceDeployment, CanonicalFixtureError> {
    let Some(test_service) = &project.test_service_profile else {
        return Err(CanonicalFixtureError::InvalidInput(
            "external gateway projection requires a kind:test service profile".to_string(),
        ));
    };
    let mut service_api = project.service_api.clone().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(
            "compiled test service omitted its ordinary service API projection".to_string(),
        )
    })?;
    service_api.contract = contract.clone();
    let mut service = test_service.service_root.service.clone();
    service.id = contract.service_id.clone();
    let mut profile = test_service.authoring.clone();
    let requires_ingress = project
        .package
        .artifact
        .runtime_requirements
        .config
        .iter()
        .any(|requirement| requirement.path == TEST_INGRESS_CONFIG_PATH);
    if requires_ingress {
        let ingress_url = runner_ingress.ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(format!(
                "test service {} requires the runner-owned {TEST_INGRESS_CONFIG_PATH} binding",
                test_service.service_id
            ))
        })?;
        if profile.config.is_null() {
            profile.config = serde_json::json!({});
        }
        let config = profile.config.as_object_mut().ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(
                "test service config.skiff-test.yml field config must be a path-keyed object"
                    .to_string(),
            )
        })?;
        config.insert(
            TEST_INGRESS_CONFIG_PATH.to_string(),
            serde_json::Value::String(ingress_url.to_string()),
        );
    }
    let generated = generate_service_deployment_with_validated_packages(
        GeneratedServiceDeploymentInput {
            service: &service,
            http: test_service.service_root.http.as_ref(),
            websocket: test_service.service_root.websocket.as_ref(),
            profile_name: &test_service.profile_name,
            profile: &profile,
            service_api: &service_api,
            implementation: &project.package.artifact,
            package_closure: &project.dependency_packages,
            package_schema_records: &project.package.resolved_package_schema_type_records,
        },
        implementation,
        package_closure,
    )
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    Ok(generated)
}
