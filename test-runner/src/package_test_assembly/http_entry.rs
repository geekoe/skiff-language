use std::collections::BTreeMap;

use skiff_artifact_model::{
    DeploymentGatewayEntry, DeploymentIngressBinding, ServiceAuthoringKind, ServiceContract,
    ServiceManifestAuthoring,
};
use skiff_compiler::{
    generate_service_deployment, GeneratedServiceDeploymentInput, ServiceApiProjection,
    ServiceApiVisibility,
};

use crate::{
    canonical_fixture::CanonicalFixtureError, canonical_package::CanonicalPackageProject,
    test_overlay::PublishedPackageTestOverlay,
};

use super::profile::TEST_INGRESS_CONFIG_PATH;

pub(super) fn project(
    project: &CanonicalPackageProject,
    overlay: &PublishedPackageTestOverlay,
    contract: &ServiceContract,
    runner_ingress: Option<&str>,
) -> Result<
    (
        BTreeMap<skiff_artifact_model::GatewayEntryKey, DeploymentGatewayEntry>,
        Vec<DeploymentIngressBinding>,
    ),
    CanonicalFixtureError,
> {
    let Some(test_service) = &project.test_service_profile else {
        return Ok((BTreeMap::new(), Vec::new()));
    };
    let Some(http) = test_service
        .http
        .as_ref()
        .filter(|document| !document.entries.is_empty())
    else {
        return Ok((BTreeMap::new(), Vec::new()));
    };
    let mut profile = test_service.authoring.clone();
    let requires_ingress = overlay
        .overlay
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
    let service = ServiceManifestAuthoring {
        id: contract.service_id.clone(),
        kind: ServiceAuthoringKind::Test,
        service_calls: Vec::new(),
    };
    let service_api = ServiceApiProjection {
        service_calls: Vec::new(),
        contract: contract.clone(),
        visibility: ServiceApiVisibility {
            functions: Vec::new(),
        },
        available: BTreeMap::new(),
        unavailable: BTreeMap::new(),
    };
    let generated = generate_service_deployment(GeneratedServiceDeploymentInput {
        service: &service,
        http: Some(http),
        websocket: None,
        profile_name: &test_service.profile_name,
        profile: &profile,
        service_api: &service_api,
        implementation: &overlay.overlay.artifact,
        package_closure: &overlay.dependency_packages,
        package_schema_records: &overlay.overlay.resolved_package_schema_type_records,
    })
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    Ok((generated.gateway_entries, generated.ingress))
}
