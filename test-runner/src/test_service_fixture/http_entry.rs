use skiff_artifact_model::{ServiceContract, ServiceDeployment};
use skiff_compiler::{
    generate_service_deployment_with_validated_packages, GeneratedServiceDeploymentInput,
    GeneratedServicePackageAdmissions,
};

use crate::{canonical_fixture::CanonicalFixtureError, canonical_package::CanonicalPackageProject};

pub(super) fn project(
    project: &CanonicalPackageProject,
    contract: &ServiceContract,
    admissions: &GeneratedServicePackageAdmissions,
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
    let generated = generate_service_deployment_with_validated_packages(
        GeneratedServiceDeploymentInput {
            service: &service,
            http: test_service.service_root.http.as_ref(),
            websocket: test_service.service_root.websocket.as_ref(),
            service_api: &service_api,
            implementation: &project.package.artifact,
            package_closure: &project.dependency_packages,
            package_schema_records: &project.package.resolved_package_schema_type_records,
        },
        admissions,
    )
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    Ok(generated)
}
