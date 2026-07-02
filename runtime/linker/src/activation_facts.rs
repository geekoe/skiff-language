use std::{collections::HashMap, sync::Arc};

use serde_json::Value;
use skiff_artifact_model::{ActorMetadataIr, DbMetadataIr, OperationRouteBinding};

use super::package_config::package_activation_configs;
use crate::program::{
    GatewayConfig, PackageSlot, PackageUnit, ProgramResult, ServiceDependencyConstraint,
    ServiceMeta, ServiceTimeoutConfig, ServiceUnit,
};

#[derive(Debug, Clone)]
pub struct LinkedImageActivationFacts {
    pub service: ServiceMeta,
    pub version: String,
    pub package_configs: Vec<Value>,
    pub service_dependencies: Vec<ServiceDependencyConstraint>,
    pub timeout: ServiceTimeoutConfig,
    pub operation_route_bindings: Vec<OperationRouteBinding>,
    pub db: Vec<DbMetadataIr>,
    pub actors: Vec<ActorMetadataIr>,
    pub gateway: GatewayConfig,
}

pub fn linker_activation_facts(
    service: &ServiceUnit,
    packages: &[Arc<PackageUnit>],
    package_slots_by_id: &HashMap<String, PackageSlot>,
) -> ProgramResult<LinkedImageActivationFacts> {
    Ok(LinkedImageActivationFacts {
        service: service.service.clone(),
        version: service.version.clone(),
        package_configs: package_activation_configs(service, packages, package_slots_by_id)?,
        service_dependencies: service.service_dependencies.clone(),
        timeout: service.timeout.clone(),
        operation_route_bindings: service.operation_route_bindings.clone(),
        db: service.db.clone(),
        actors: service.actors.clone(),
        gateway: service.gateway.clone(),
    })
}
