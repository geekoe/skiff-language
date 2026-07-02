//! LP2 temporary adapter for promoted linked image identity/types.
//!
//! Owner: skiff-runtime-linked-program DTO contract.
//! Deletion/narrowing point: after runtime callers import
//! `skiff_runtime_linked_program::{LinkedProgramImage, RuntimeProgramIdentity}` directly.

pub use skiff_runtime_linked_program::{LinkedProgramImage, RuntimeProgramIdentity};

#[cfg(any(test, feature = "test-support"))]
use super::TestRuntimeProgram;
#[cfg(any(test, feature = "test-support"))]
use crate::activation::RuntimeActivation;
#[cfg(any(test, feature = "test-support"))]
use crate::config_view::RuntimeConfigView;
#[cfg(any(test, feature = "test-support"))]
use std::sync::Arc;

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone)]
pub struct RuntimeProgramLayers {
    pub identity: RuntimeProgramIdentity,
    pub image: Arc<LinkedProgramImage>,
    pub activation: Arc<RuntimeActivation>,
}

#[cfg(any(test, feature = "test-support"))]
impl RuntimeProgramLayers {
    pub fn new(
        identity: RuntimeProgramIdentity,
        image: Arc<LinkedProgramImage>,
        activation: Arc<RuntimeActivation>,
    ) -> Self {
        Self {
            identity,
            image,
            activation,
        }
    }

    pub fn from_owned(
        identity: RuntimeProgramIdentity,
        image: LinkedProgramImage,
        activation: RuntimeActivation,
    ) -> Self {
        Self::new(identity, Arc::new(image), Arc::new(activation))
    }

    pub(crate) fn to_test_runtime_program(&self) -> TestRuntimeProgram {
        TestRuntimeProgram {
            service: self.activation.service.clone(),
            version: self.activation.version.clone(),
            build_id: self.identity.dynamic_build_id.clone(),
            service_files: self.image.service_files.clone(),
            packages: self.image.packages.clone(),
            package_files: self.image.package_files.clone(),
            package_configs: self
                .activation
                .package_configs
                .iter()
                .cloned()
                .map(RuntimeConfigView::from_value)
                .collect(),
            service_dependencies: self.activation.service_dependencies.clone(),
            timeout: self.activation.timeout.clone(),
            operation_route_bindings: self.activation.operation_route_bindings.clone(),
            routes: self.image.routes.clone(),
            spawn_routes: self.image.spawn_routes.clone(),
            operations: self.image.operations.clone(),
            operation_receivers: self.image.operation_receivers.clone(),
            db: self.activation.db.clone(),
            actors: self.activation.actors.clone(),
            link_overlay: self.image.link_overlay.clone(),
            gateway: self.activation.gateway.clone(),
            types: self.image.types.clone(),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
#[allow(dead_code)]
impl TestRuntimeProgram {
    pub fn runtime_program_identity(&self) -> RuntimeProgramIdentity {
        RuntimeProgramIdentity::from_dynamic_build_id(self.build_id.clone())
    }

    pub fn linked_image(&self) -> LinkedProgramImage {
        LinkedProgramImage {
            service_files: self.service_files.clone(),
            packages: self.packages.clone(),
            package_files: self.package_files.clone(),
            routes: self.routes.clone(),
            spawn_routes: self.spawn_routes.clone(),
            operations: self.operations.clone(),
            operation_receivers: self.operation_receivers.clone(),
            link_overlay: self.link_overlay.clone(),
            types: self.types.clone(),
        }
    }

    pub fn activation_view(&self) -> RuntimeActivation {
        RuntimeActivation {
            service: self.service.clone(),
            version: self.version.clone(),
            package_configs: self
                .package_configs
                .iter()
                .map(|config| config.resolved_config_value().clone())
                .collect(),
            service_dependencies: self.service_dependencies.clone(),
            timeout: self.timeout.clone(),
            operation_route_bindings: self.operation_route_bindings.clone(),
            db: self.db.clone(),
            actors: self.actors.clone(),
            gateway: self.gateway.clone(),
        }
    }
}
