use std::{collections::HashMap, sync::Arc};

use skiff_artifact_model::{ActorMetadataIr, DbMetadataIr};
use skiff_runtime_activation::RuntimeActivation;
use skiff_runtime_linked_program::{
    ConstAddr, ExecutableAddr, GatewayConfig, LinkOverlay, LinkedFileUnit, LinkedProgramImage,
    OperationRouteBinding, PackageUnit, RuntimeProgramIdentity, RuntimeTypeContext,
    ServiceDependencyConstraint, ServiceMeta, ServiceTimeoutConfig,
};

use crate::EvalRuntimeProgramSource;

#[derive(Debug, Clone)]
pub struct RuntimeProgram {
    pub service: ServiceMeta,
    pub version: String,
    pub build_id: String,
    pub service_files: Vec<Arc<LinkedFileUnit>>,
    pub packages: Vec<Arc<PackageUnit>>,
    pub package_files: Vec<Vec<Arc<LinkedFileUnit>>>,
    pub service_dependencies: Vec<ServiceDependencyConstraint>,
    pub timeout: ServiceTimeoutConfig,
    pub operation_route_bindings: Vec<OperationRouteBinding>,
    pub routes: HashMap<String, ExecutableAddr>,
    pub spawn_routes: HashMap<String, ExecutableAddr>,
    pub operations: HashMap<String, ExecutableAddr>,
    pub operation_receivers: HashMap<String, ConstAddr>,
    pub db: Vec<DbMetadataIr>,
    pub actors: Vec<ActorMetadataIr>,
    pub link_overlay: LinkOverlay,
    pub gateway: GatewayConfig,
    pub types: RuntimeTypeContext,
}

impl RuntimeProgram {
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
            package_configs: Vec::new(),
            service_dependencies: self.service_dependencies.clone(),
            timeout: self.timeout.clone(),
            operation_route_bindings: self.operation_route_bindings.clone(),
            db: self.db.clone(),
            actors: self.actors.clone(),
            gateway: self.gateway.clone(),
        }
    }
}

impl EvalRuntimeProgramSource for RuntimeProgram {
    fn service_id(&self) -> &str {
        &self.service.id
    }

    fn service_files(&self) -> &[Arc<LinkedFileUnit>] {
        &self.service_files
    }

    fn packages(&self) -> &[Arc<PackageUnit>] {
        &self.packages
    }

    fn package_files(&self) -> &[Vec<Arc<LinkedFileUnit>>] {
        &self.package_files
    }

    fn spawn_routes(&self) -> &HashMap<String, ExecutableAddr> {
        &self.spawn_routes
    }

    fn link_overlay(&self) -> &LinkOverlay {
        &self.link_overlay
    }

    fn types(&self) -> &RuntimeTypeContext {
        &self.types
    }
}
