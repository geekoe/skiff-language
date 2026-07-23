use serde_json::Value;
use skiff_artifact_model::{ActorMetadataIr, DbMetadataIr, OperationRouteBinding};
use skiff_runtime_linked_program::{
    GatewayConfig, ServiceDependencyConstraint, ServiceMeta, ServiceTimeoutConfig,
};

#[derive(Debug, Clone)]
pub struct RuntimeActivation {
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
