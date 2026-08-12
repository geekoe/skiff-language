//! Strict, ephemeral routing authority derived from one exact deployment.
//!
//! Routing views are deliberately process-local: they have private fields, no
//! public constructor, and no serialization implementation. Callers must use
//! [`validate_strict_deployment_routing_view`] or the canonical store reader.
//!
//! ```compile_fail
//! use skiff_deployment::routing_view::DeploymentRoutingAuthority;
//!
//! let authority = DeploymentRoutingAuthority {
//!     build_id: todo!(),
//!     registry_descriptor: todo!(),
//! };
//! ```
//!
//! ```compile_fail
//! use skiff_deployment::routing_view::StrictDeploymentRoutingView;
//!
//! fn persist(view: &StrictDeploymentRoutingView) {
//!     let _ = serde_json::to_value(view).unwrap();
//! }
//! ```

use std::sync::Arc;

use skiff_artifact_model::{
    DeploymentArtifactIdentity, PackageArtifact, PlatformErrorProjectionRegistryRef,
    ServiceDeployment, ServiceDeploymentRef,
};

use crate::projection::package_closure::PackageClosure;

pub use crate::projection::RoutingViewError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentRoutingAuthority {
    build_id: DeploymentArtifactIdentity,
    registry_descriptor: PlatformErrorProjectionRegistryRef,
}

impl DeploymentRoutingAuthority {
    pub fn build_id(&self) -> &DeploymentArtifactIdentity {
        &self.build_id
    }

    pub fn registry_descriptor(&self) -> &PlatformErrorProjectionRegistryRef {
        &self.registry_descriptor
    }
}

#[derive(Debug)]
pub struct StrictDeploymentRoutingView {
    deployment_ref: ServiceDeploymentRef,
    deployment: Arc<ServiceDeployment>,
    authority: DeploymentRoutingAuthority,
}

impl StrictDeploymentRoutingView {
    pub fn reference(&self) -> &ServiceDeploymentRef {
        &self.deployment_ref
    }

    pub fn deployment(&self) -> &Arc<ServiceDeployment> {
        &self.deployment
    }

    pub fn authority(&self) -> &DeploymentRoutingAuthority {
        &self.authority
    }
}

/// Validates one exact deployment and its complete package closure, then
/// derives the immutable routing authority consumed by Router.
pub fn validate_strict_deployment_routing_view(
    deployment_ref: &ServiceDeploymentRef,
    deployment: Arc<ServiceDeployment>,
    packages: &[Arc<PackageArtifact>],
) -> Result<StrictDeploymentRoutingView, RoutingViewError> {
    skiff_artifact_identity::validate_service_deployment_ref(deployment_ref, &deployment).map_err(
        |identity_error| RoutingViewError::InvalidDeployment {
            identity_error: Box::new(identity_error),
        },
    )?;

    let closure = PackageClosure::resolve_for_routing(&deployment, packages)?;
    let implementation = closure.implementation_for_ref(&deployment.implementation);
    let registry_descriptor = implementation.platform_error_projection_registry.clone();
    validate_registry_descriptor(implementation, &registry_descriptor)?;
    for package in closure.artifacts() {
        validate_registry_descriptor(package, &package.platform_error_projection_registry)?;
        if package.platform_error_projection_registry != registry_descriptor {
            return Err(RoutingViewError::MixedRegistryDescriptors {
                implementation_build_id: implementation.package_build_id.clone(),
                implementation_fingerprint: registry_descriptor.fingerprint().to_string(),
                package_build_id: package.package_build_id.clone(),
                package_fingerprint: package
                    .platform_error_projection_registry
                    .fingerprint()
                    .to_string(),
            });
        }
    }

    Ok(StrictDeploymentRoutingView {
        deployment_ref: deployment_ref.clone(),
        authority: DeploymentRoutingAuthority {
            build_id: deployment.deployment_artifact_identity.clone(),
            registry_descriptor,
        },
        deployment,
    })
}

fn validate_registry_descriptor(
    package: &PackageArtifact,
    descriptor: &PlatformErrorProjectionRegistryRef,
) -> Result<(), RoutingViewError> {
    skiff_artifact_model::validate_platform_error_projection_registry_ref_shape(descriptor).map_err(
        |source| RoutingViewError::InvalidRegistryDescriptor {
            build_id: package.package_build_id.clone(),
            source,
        },
    )
}
