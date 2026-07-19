use skiff_artifact_model::{RuntimeAssembly, ServiceDeployment, ServiceDeploymentInput};

use crate::Result;

pub fn validate_deployment_input(input: &ServiceDeploymentInput) -> Result<()> {
    skiff_artifact_identity::validate_service_deployment_input(input)?;
    Ok(())
}

pub fn validate_deployment(deployment: &ServiceDeployment) -> Result<()> {
    skiff_artifact_identity::validate_service_deployment_identity(deployment)?;
    Ok(())
}

pub fn validate_assembly(assembly: &RuntimeAssembly) -> Result<()> {
    skiff_artifact_identity::validate_runtime_assembly_identity(assembly)?;
    Ok(())
}
