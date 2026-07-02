use serde_json::Value;
use skiff_artifact_model::{ActorMetadataIr, DbMetadataIr, OperationRouteBinding};
use skiff_runtime_linked_program::{
    GatewayConfig, LinkedProgramImage, ServiceDependencyConstraint, ServiceMeta,
    ServiceTimeoutConfig,
};
use skiff_runtime_linker::{LinkedImageActivationFacts, ProgramResult};

use crate::requirements::ActivationRequirements;

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

#[derive(Debug, Clone)]
struct RuntimeActivationInput {
    requirements: ActivationRequirements,
    service: ServiceMeta,
    version: String,
    package_configs: Vec<Value>,
    service_dependencies: Vec<ServiceDependencyConstraint>,
    timeout: ServiceTimeoutConfig,
    operation_route_bindings: Vec<OperationRouteBinding>,
    db: Vec<DbMetadataIr>,
    actors: Vec<ActorMetadataIr>,
    gateway: GatewayConfig,
}

impl RuntimeActivationInput {
    fn from_facts(requirements: ActivationRequirements, facts: LinkedImageActivationFacts) -> Self {
        Self {
            requirements,
            service: facts.service,
            version: facts.version,
            package_configs: facts.package_configs,
            service_dependencies: facts.service_dependencies,
            timeout: facts.timeout,
            operation_route_bindings: facts.operation_route_bindings,
            db: facts.db,
            actors: facts.actors,
            gateway: facts.gateway,
        }
    }
}

pub fn build_runtime_activation_for_image(
    image: &LinkedProgramImage,
    facts: LinkedImageActivationFacts,
) -> ProgramResult<RuntimeActivation> {
    let requirements = ActivationRequirements::from_linked_image(image);
    build_runtime_activation(RuntimeActivationInput::from_facts(requirements, facts))
}

fn build_runtime_activation(input: RuntimeActivationInput) -> ProgramResult<RuntimeActivation> {
    input
        .requirements
        .validate_runtime_facts(input.package_configs.len(), &input.operation_route_bindings)?;

    Ok(RuntimeActivation {
        service: input.service,
        version: input.version,
        package_configs: input.package_configs,
        service_dependencies: input.service_dependencies,
        timeout: input.timeout,
        operation_route_bindings: input.operation_route_bindings,
        db: input.db,
        actors: input.actors,
        gateway: input.gateway,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use skiff_runtime_linked_program::OperationIngressKind;
    use skiff_runtime_linker::ProgramError;

    use super::*;

    #[test]
    fn build_runtime_activation_copies_runtime_facts() {
        let activation = build_runtime_activation(RuntimeActivationInput {
            requirements: ActivationRequirements::for_test(1, ["operation:ping"]),
            service: ServiceMeta {
                id: "skiff.run/service".to_string(),
                display_name: Some("Service".to_string()),
                metadata: Default::default(),
            },
            version: "0.1.0".to_string(),
            package_configs: vec![json!({ "feature": true })],
            service_dependencies: Vec::new(),
            timeout: ServiceTimeoutConfig {
                default_ms: Some(2_500),
                methods: Default::default(),
            },
            operation_route_bindings: vec![OperationRouteBinding {
                ingress_kind: OperationIngressKind::HttpGateway,
                selector: "GET /ping".to_string(),
                operation_abi_id: "operation:ping".to_string(),
            }],
            db: Vec::new(),
            actors: Vec::new(),
            gateway: GatewayConfig::default(),
        })
        .expect("activation build should succeed");

        assert_eq!(activation.service.id, "skiff.run/service");
        assert_eq!(activation.service.display_name.as_deref(), Some("Service"));
        assert_eq!(activation.version, "0.1.0");
        assert_eq!(activation.package_configs, vec![json!({ "feature": true })]);
        assert_eq!(activation.service_dependencies, Vec::new());
        assert_eq!(activation.timeout.default_ms, Some(2_500));
        assert_eq!(activation.operation_route_bindings.len(), 1);
        assert_eq!(activation.operation_route_bindings[0].selector, "GET /ping");
        assert!(activation.db.is_empty());
        assert!(activation.actors.is_empty());
        assert_eq!(activation.gateway, GatewayConfig::default());
    }

    #[test]
    fn build_runtime_activation_rejects_too_many_package_configs() {
        let error = build_runtime_activation(RuntimeActivationInput {
            requirements: ActivationRequirements::for_test(1, ["operation:ping"]),
            service: ServiceMeta {
                id: "skiff.run/service".to_string(),
                display_name: None,
                metadata: Default::default(),
            },
            version: "0.1.0".to_string(),
            package_configs: vec![json!({ "a": true }), json!({ "b": true })],
            service_dependencies: Vec::new(),
            timeout: ServiceTimeoutConfig::default(),
            operation_route_bindings: Vec::new(),
            db: Vec::new(),
            actors: Vec::new(),
            gateway: GatewayConfig::default(),
        })
        .expect_err("package configs beyond linked package slots should fail closed");

        assert_eq!(
            error,
            ProgramError::ActivationPackageConfigsExceedLinkedPackageSlots {
                package_config_count: 2,
                linked_package_count: 1,
            }
        );
    }

    #[test]
    fn build_runtime_activation_rejects_route_binding_for_missing_operation() {
        let error = build_runtime_activation(RuntimeActivationInput {
            requirements: ActivationRequirements::for_test(0, ["operation:ping"]),
            service: ServiceMeta {
                id: "skiff.run/service".to_string(),
                display_name: None,
                metadata: Default::default(),
            },
            version: "0.1.0".to_string(),
            package_configs: Vec::new(),
            service_dependencies: Vec::new(),
            timeout: ServiceTimeoutConfig::default(),
            operation_route_bindings: vec![OperationRouteBinding {
                ingress_kind: OperationIngressKind::HttpGateway,
                selector: "GET /missing".to_string(),
                operation_abi_id: "operation:missing".to_string(),
            }],
            db: Vec::new(),
            actors: Vec::new(),
            gateway: GatewayConfig::default(),
        })
        .expect_err("route binding for a missing linked operation should fail closed");

        assert_eq!(
            error,
            ProgramError::ActivationRouteBindingUnknownOperation {
                selector: "GET /missing".to_string(),
                operation_abi_id: "operation:missing".to_string(),
            }
        );
    }
}
