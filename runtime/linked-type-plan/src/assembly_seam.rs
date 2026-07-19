use skiff_artifact_model::PackageBuildId;
use skiff_runtime_linked_program::{SharedPackageCode, SharedPackageLinkedImage};

/// Exact immutable package-code handoff for the future assembly execution plane.
///
/// Phase 03 intentionally stops before creating a [`crate::ProgramTypeView`]: that view belongs
/// to the activation-owned linked execution program materialized in Phase 04.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeAssemblyTypePlanTarget<'a> {
    image: &'a SharedPackageLinkedImage,
    code: &'a SharedPackageCode,
}

impl<'a> RuntimeAssemblyTypePlanTarget<'a> {
    pub fn from_shared_image(
        image: &'a SharedPackageLinkedImage,
        package_build_id: &PackageBuildId,
    ) -> Result<Self, RuntimeAssemblyTypePlanSeamError> {
        let code = image.code_by_build(package_build_id).ok_or_else(|| {
            RuntimeAssemblyTypePlanSeamError::MissingPackageCode {
                package_build_id: package_build_id.clone(),
            }
        })?;
        Ok(Self {
            image,
            code: code.as_ref(),
        })
    }

    pub fn image(&self) -> &'a SharedPackageLinkedImage {
        self.image
    }

    pub fn code(&self) -> &'a SharedPackageCode {
        self.code
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeAssemblyTypePlanSeamError {
    #[error("shared runtime assembly image has no package code for {package_build_id}")]
    MissingPackageCode { package_build_id: PackageBuildId },
}
