use std::sync::Arc;

use skiff_artifact_identity::{
    runtime_program_dynamic_build_id, runtime_program_service_unit_identity_bytes,
};

use crate::program::{package_unit::PackageUnit, service_unit::ServiceUnit};
use crate::resolver::{ProgramError, ProgramResult};

pub(super) fn dynamic_build_id(
    service: &ServiceUnit,
    packages: &[Arc<PackageUnit>],
) -> ProgramResult<String> {
    let service_unit_identity_bytes = runtime_program_service_unit_identity_bytes(service)
        .map_err(|error| ProgramError::RuntimeProgramBuildIdentityFailed {
            message: error.to_string(),
        })?;
    Ok(runtime_program_dynamic_build_id(
        &service_unit_identity_bytes,
        packages
            .iter()
            .map(|package| package.build_identity.as_str()),
    ))
}
