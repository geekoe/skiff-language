use skiff_artifact_model::{OperationTargetRef, PackageBuildId};
use skiff_runtime_linked_program::{
    AssemblyExecutable, AssemblyExecutionImage, ExecutableAddr, LinkedFileUnit,
    RuntimeExecutionPackage, RuntimeTypeContext, TypeAddr,
};

/// Read-only executable/type-plan handoff for one package code owner in an assembly image.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeAssemblyTypePlanTarget<'a> {
    image: &'a AssemblyExecutionImage,
    code: &'a RuntimeExecutionPackage,
}

impl<'a> RuntimeAssemblyTypePlanTarget<'a> {
    pub fn from_execution_image(
        image: &'a AssemblyExecutionImage,
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

    pub fn image(&self) -> &'a AssemblyExecutionImage {
        self.image
    }

    pub fn code(&self) -> &'a RuntimeExecutionPackage {
        self.code
    }

    pub fn files(&self) -> &'a [std::sync::Arc<LinkedFileUnit>] {
        self.code.files()
    }

    pub fn types(&self) -> &'a RuntimeTypeContext {
        self.image.types()
    }

    pub fn entry_executable(
        &self,
        target: &OperationTargetRef,
    ) -> Result<AssemblyExecutable<'a>, RuntimeAssemblyTypePlanSeamError> {
        self.image
            .entry_executable(self.code.package_build_id(), target)
            .map_err(
                |error| RuntimeAssemblyTypePlanSeamError::InvalidExecutionTarget {
                    package_build_id: self.code.package_build_id().clone(),
                    detail: error.to_string(),
                },
            )
    }

    pub fn executable_at(
        &self,
        addr: &ExecutableAddr,
    ) -> Result<AssemblyExecutable<'a>, RuntimeAssemblyTypePlanSeamError> {
        self.image.executable_at(addr).map_err(|error| {
            RuntimeAssemblyTypePlanSeamError::InvalidExecutionTarget {
                package_build_id: self.code.package_build_id().clone(),
                detail: error.to_string(),
            }
        })
    }

    pub fn type_addr(
        &self,
        file_ir_identity: &str,
        type_index: usize,
    ) -> Result<TypeAddr, RuntimeAssemblyTypePlanSeamError> {
        self.image
            .type_addr(self.code.package_build_id(), file_ir_identity, type_index)
            .map_err(
                |error| RuntimeAssemblyTypePlanSeamError::InvalidTypeTarget {
                    package_build_id: self.code.package_build_id().clone(),
                    detail: error.to_string(),
                },
            )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeAssemblyTypePlanSeamError {
    #[error("assembly execution image has no package code for {package_build_id}")]
    MissingPackageCode { package_build_id: PackageBuildId },
    #[error("assembly package {package_build_id} has an invalid executable target: {detail}")]
    InvalidExecutionTarget {
        package_build_id: PackageBuildId,
        detail: String,
    },
    #[error("assembly package {package_build_id} has an invalid type target: {detail}")]
    InvalidTypeTarget {
        package_build_id: PackageBuildId,
        detail: String,
    },
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use skiff_artifact_model::{
        AssemblyIdentity, CanonicalPackageLinkPlan, PackageBuildId, RuntimeAssembly,
        RUNTIME_ASSEMBLY_SCHEMA_VERSION,
    };
    use skiff_runtime_linked_program::{
        AssemblyExecutionImage, RuntimeTypeContext, ServiceErrorTypeIndex, SharedPackageLinkedImage,
    };

    use super::*;

    #[test]
    fn assembly_execution_type_plan_view_fails_closed_for_unknown_package() {
        let assembly = RuntimeAssembly {
            schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
            assembly_identity: AssemblyIdentity::new("assembly:empty"),
            roots: Vec::new(),
            resolved_deployments: Vec::new(),
            resolved_contracts: Vec::new(),
            resolved_packages: Vec::new(),
            package_link_plan: CanonicalPackageLinkPlan {
                code_slots: Vec::new(),
                package_links: Vec::new(),
            },
            service_binding_templates: Vec::new(),
            activation_templates: Vec::new(),
            gateway_ingress: Vec::new(),
        };
        let shared = Arc::new(
            SharedPackageLinkedImage::from_runtime_assembly(&assembly, Vec::new()).unwrap(),
        );
        let image = AssemblyExecutionImage::try_new(
            shared,
            Vec::new(),
            RuntimeTypeContext::default(),
            Arc::new(ServiceErrorTypeIndex::default()),
        )
        .unwrap();

        assert!(matches!(
            RuntimeAssemblyTypePlanTarget::from_execution_image(
                &image,
                &PackageBuildId::new("missing")
            ),
            Err(RuntimeAssemblyTypePlanSeamError::MissingPackageCode {
                package_build_id
            }) if package_build_id == PackageBuildId::new("missing")
        ));
    }
}
