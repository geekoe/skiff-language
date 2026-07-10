use std::collections::BTreeSet;

use serde_json::Value;
use skiff_runtime_activation::RuntimeActivation;
use skiff_runtime_linked_program::{
    LinkedProgramImage, LinkedProgramImageResolverExt, RuntimeProgramIdentity,
};
use skiff_runtime_transport::protocol::{
    RuntimeCapabilitiesFrameHeaderMetadata, RuntimeDispatchModeCapability, RuntimeRegisterEnvelope,
};

use crate::error::{Result, RuntimeError};
use skiff_runtime_linked_type_plan::linked_type_ref_is_http_response_stream;

pub(super) fn runtime_register_envelope_from_program_layers(
    runtime_id: String,
    identity: &RuntimeProgramIdentity,
    image: &LinkedProgramImage,
    activation: &RuntimeActivation,
    revision_id: String,
    contract_identity: String,
    implementation_identity: String,
    artifact_identity: String,
    activation_identity: Option<String>,
) -> Result<RuntimeRegisterEnvelope> {
    runtime_register_envelope_from_projection(
        runtime_id,
        activation.service.id.as_str(),
        identity.dynamic_build_id.clone(),
        activation.version.as_str(),
        image,
        serde_json::to_value(&activation.gateway)
            .expect("runtime program gateway should serialize"),
        revision_id,
        contract_identity,
        implementation_identity,
        artifact_identity,
        activation_identity,
    )
}

fn runtime_register_envelope_from_projection(
    runtime_id: String,
    service_id: &str,
    build_id: String,
    service_version: &str,
    image: &LinkedProgramImage,
    gateway: Value,
    revision_id: String,
    contract_identity: String,
    implementation_identity: String,
    artifact_identity: String,
    activation_identity: Option<String>,
) -> Result<RuntimeRegisterEnvelope> {
    if service_id.is_empty() {
        return Err(RuntimeError::invalid_artifact(
            "runtime program service id is required".to_string(),
        ));
    }
    if !build_id.starts_with("skiff-service-build-v1:sha256:") {
        return Err(RuntimeError::invalid_artifact(format!(
            "runtime program buildId {build_id} is invalid"
        )));
    }
    if !is_bare_sha256(&revision_id) {
        return Err(RuntimeError::invalid_artifact(format!(
            "runtime program revisionId must be 64 lowercase hex, got {revision_id}"
        )));
    }

    let mut targets = image.routes.keys().cloned().collect::<Vec<_>>();
    if targets.is_empty() {
        return Err(RuntimeError::invalid_artifact(
            "runtime program must expose at least one operation target".to_string(),
        ));
    }
    targets.sort();

    let service_protocol_identity = contract_identity.clone();
    let protocol_version = protocol_version_from_identity(&service_protocol_identity)?;
    if service_version.is_empty() {
        return Err(RuntimeError::invalid_artifact(
            "runtime program service version is required".to_string(),
        ));
    }
    Ok(RuntimeRegisterEnvelope {
        envelope_type: "runtime.register",
        runtime_id,
        service_id: service_id.to_string(),
        version: service_version.to_string(),
        build_id,
        revision_id: revision_id.clone(),
        activation_identity,
        service_protocol_identity,
        contract_identity,
        targets,
        protocol_version,
        runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        code_revision_id: revision_id,
        implementation_identity,
        artifact_identity,
        capabilities: RuntimeCapabilitiesFrameHeaderMetadata {
            dispatch_modes: runtime_image_dispatch_modes(image),
            package_test_dispatch: true,
            request_cancel: true,
            runtime_program: true,
        },
        gateway_entry_identities: gateway_entry_identities_from_value(&gateway),
    })
}

fn runtime_image_dispatch_modes(image: &LinkedProgramImage) -> Vec<RuntimeDispatchModeCapability> {
    let mut modes = BTreeSet::from(["unary"]);
    for addr in image.routes.values() {
        let Ok(resolved) = image.resolve_executable(addr) else {
            continue;
        };
        if linked_type_ref_is_http_response_stream(
            resolved.executable.return_type.as_ref(),
            image,
            addr,
        ) {
            modes.insert("serverStream");
        }
    }
    modes
        .into_iter()
        .map(|mode| match mode {
            "serverStream" => RuntimeDispatchModeCapability::ServerStream,
            _ => RuntimeDispatchModeCapability::Unary,
        })
        .collect()
}

fn is_bare_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn protocol_version_from_identity(value: &str) -> Result<String> {
    let Some((version, hash)) = value.rsplit_once(":sha256:") else {
        return Err(RuntimeError::invalid_artifact(format!(
            "protocolIdentity {value} must be skiff-protocol-v1:sha256:<64 lowercase hex>"
        )));
    };
    if version != "skiff-protocol-v1"
        || hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(RuntimeError::invalid_artifact(format!(
            "protocolIdentity {value} must be skiff-protocol-v1:sha256:<64 lowercase hex>"
        )));
    }
    Ok(version.to_string())
}

fn gateway_entry_identities_from_value(value: &Value) -> Vec<String> {
    fn collect(value: &Value, identities: &mut Vec<String>) {
        match value {
            Value::Object(object) => {
                if let Some(identity) = object.get("gatewayEntryIdentity").and_then(Value::as_str) {
                    identities.push(identity.to_string());
                }
                for value in object.values() {
                    collect(value, identities);
                }
            }
            Value::Array(items) => {
                for value in items {
                    collect(value, identities);
                }
            }
            _ => {}
        }
    }

    let mut identities = Vec::new();
    collect(value, &mut identities);
    identities.sort();
    identities.dedup();
    identities
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;
    use skiff_runtime_linked_program::{
        ExecutableAddr, GatewayConfig, LinkOverlay, RuntimeTypeContext, ServiceMeta,
    };

    const SERVICE_PROTOCOL_A: &str =
        "skiff-protocol-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SERVICE_PROTOCOL_B: &str =
        "skiff-protocol-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SERVICE_REVISION: &str =
        "1111111111111111111111111111111111111111111111111111111111111111";
    const SERVICE_BUILD_ID: &str =
        "skiff-service-build-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    const IMPLEMENTATION_IDENTITY: &str =
        "skiff-implementation-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const ARTIFACT_IDENTITY: &str =
        "skiff-service-assembly-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    #[test]
    fn runtime_register_envelope_serializes_router_formal_fields() {
        let (identity, image, activation) = runtime_program_layers_fixture();

        let value = serde_json::to_value(
            runtime_register_envelope_from_program_layers(
                "runtime-1".to_string(),
                &identity,
                &image,
                &activation,
                SERVICE_REVISION.to_string(),
                SERVICE_PROTOCOL_A.to_string(),
                IMPLEMENTATION_IDENTITY.to_string(),
                ARTIFACT_IDENTITY.to_string(),
                Some("skiff-runtime-activation-v1:opaque:activation-fixture".to_string()),
            )
            .expect("register envelope should serialize"),
        )
        .expect("register envelope should serialize");

        assert_eq!(value["type"], "runtime.register");
        assert_eq!(value["runtimeId"], "runtime-1");
        assert_eq!(value["serviceId"], "example.com/service-a");
        assert_eq!(value["buildId"], SERVICE_BUILD_ID);
        assert_eq!(value["revisionId"], SERVICE_REVISION);
        assert_eq!(
            value["activationIdentity"],
            "skiff-runtime-activation-v1:opaque:activation-fixture"
        );
        assert_eq!(value["serviceProtocolIdentity"], SERVICE_PROTOCOL_A);
        assert_eq!(value["contractIdentity"], SERVICE_PROTOCOL_A);
        assert_eq!(
            value["targets"],
            json!(["service.test.Api.alpha", "service.test.Api.beta"])
        );
        assert_eq!(value["protocolVersion"], "skiff-protocol-v1");
        assert_eq!(value["runtimeVersion"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["codeRevisionId"], SERVICE_REVISION);
        assert_eq!(value["implementationIdentity"], IMPLEMENTATION_IDENTITY);
        assert_eq!(value["artifactIdentity"], ARTIFACT_IDENTITY);
        assert_eq!(
            value["capabilities"],
            json!({
                "dispatchModes": ["unary"],
                "packageTestDispatch": true,
                "requestCancel": true,
                "runtimeProgram": true
            })
        );
        assert_eq!(
            value["gatewayEntryIdentities"],
            json!([
                "skiff-gateway-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111",
                "skiff-gateway-v1:sha256:2222222222222222222222222222222222222222222222222222222222222222",
                "skiff-gateway-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333"
            ])
        );
    }

    #[test]
    fn runtime_register_envelope_uses_runtime_program_targets_and_contract_protocol() {
        let (mut identity, image, activation) = runtime_program_layers_fixture();
        identity.dynamic_build_id = SERVICE_BUILD_ID.to_string();

        let value = serde_json::to_value(
            runtime_register_envelope_from_program_layers(
                "runtime-1".to_string(),
                &identity,
                &image,
                &activation,
                SERVICE_REVISION.to_string(),
                SERVICE_PROTOCOL_B.to_string(),
                IMPLEMENTATION_IDENTITY.to_string(),
                ARTIFACT_IDENTITY.to_string(),
                None,
            )
            .expect("register envelope should serialize"),
        )
        .expect("register envelope should serialize");

        assert_eq!(
            value["targets"],
            json!(["service.test.Api.alpha", "service.test.Api.beta"])
        );
        assert_eq!(value["serviceProtocolIdentity"], SERVICE_PROTOCOL_B);
        assert_eq!(value["protocolVersion"], "skiff-protocol-v1");
    }

    #[test]
    fn runtime_register_envelope_requires_runtime_program_service_id() {
        let (identity, image, mut activation) = runtime_program_layers_fixture();
        activation.service.id.clear();

        let error = runtime_register_envelope_from_program_layers(
            "runtime-1".to_string(),
            &identity,
            &image,
            &activation,
            SERVICE_REVISION.to_string(),
            SERVICE_PROTOCOL_A.to_string(),
            IMPLEMENTATION_IDENTITY.to_string(),
            ARTIFACT_IDENTITY.to_string(),
            None,
        )
        .expect_err("missing service id should error");

        assert!(error
            .to_string()
            .contains("runtime program service id is required"));
    }

    #[test]
    fn runtime_register_envelope_requires_runtime_program_targets() {
        let (identity, mut image, activation) = runtime_program_layers_fixture();
        image.routes.clear();

        let error = runtime_register_envelope_from_program_layers(
            "runtime-1".to_string(),
            &identity,
            &image,
            &activation,
            SERVICE_REVISION.to_string(),
            SERVICE_PROTOCOL_A.to_string(),
            IMPLEMENTATION_IDENTITY.to_string(),
            ARTIFACT_IDENTITY.to_string(),
            None,
        )
        .expect_err("missing route targets should error");

        assert!(error
            .to_string()
            .contains("runtime program must expose at least one operation target"));
    }

    #[test]
    fn runtime_register_envelope_requires_service_build_id_prefix() {
        let (mut identity, image, activation) = runtime_program_layers_fixture();
        identity.dynamic_build_id = "legacy-build".to_string();

        let error = runtime_register_envelope_from_program_layers(
            "runtime-1".to_string(),
            &identity,
            &image,
            &activation,
            SERVICE_REVISION.to_string(),
            SERVICE_PROTOCOL_A.to_string(),
            IMPLEMENTATION_IDENTITY.to_string(),
            ARTIFACT_IDENTITY.to_string(),
            None,
        )
        .expect_err("legacy buildId should be rejected");

        assert!(error
            .to_string()
            .contains("buildId legacy-build is invalid"));
    }

    #[test]
    fn runtime_register_envelope_requires_bare_sha256_revision_id() {
        let (identity, image, activation) = runtime_program_layers_fixture();

        let error = runtime_register_envelope_from_program_layers(
            "runtime-1".to_string(),
            &identity,
            &image,
            &activation,
            SERVICE_BUILD_ID.to_string(),
            SERVICE_PROTOCOL_A.to_string(),
            IMPLEMENTATION_IDENTITY.to_string(),
            ARTIFACT_IDENTITY.to_string(),
            None,
        )
        .expect_err("buildId-shaped revision id should be rejected");

        assert!(error.to_string().contains("revisionId"));
        assert!(error.to_string().contains("64 lowercase hex"));
    }

    #[test]
    fn runtime_register_envelope_requires_protocol_identity_version() {
        let (identity, image, activation) = runtime_program_layers_fixture();

        let error = runtime_register_envelope_from_program_layers(
            "runtime-1".to_string(),
            &identity,
            &image,
            &activation,
            SERVICE_REVISION.to_string(),
            "skiff-protocol-v2:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            IMPLEMENTATION_IDENTITY.to_string(),
            ARTIFACT_IDENTITY.to_string(),
            None,
        )
        .expect_err("wrong protocol version should be rejected");

        assert!(error.to_string().contains("protocolIdentity"));
        assert!(error.to_string().contains("skiff-protocol-v1"));
    }

    fn runtime_program_layers_fixture() -> (
        RuntimeProgramIdentity,
        LinkedProgramImage,
        RuntimeActivation,
    ) {
        let alpha = ExecutableAddr::service(0, 0);
        let beta = ExecutableAddr::service(0, 1);
        (
            RuntimeProgramIdentity::new(SERVICE_BUILD_ID, "linked-image:test"),
            LinkedProgramImage {
                service_files: Vec::new(),
                packages: Vec::new(),
                package_files: Vec::new(),
                service_resources: Default::default(),
                package_resources: Vec::new(),
                routes: HashMap::from([
                    ("service.test.Api.alpha".to_string(), alpha.clone()),
                    ("service.test.Api.beta".to_string(), beta.clone()),
                ]),
                spawn_routes: HashMap::new(),
                operations: HashMap::from([
                    ("Api.alpha".to_string(), alpha),
                    ("Api.beta".to_string(), beta),
                ]),
                operation_receivers: HashMap::new(),
                link_overlay: LinkOverlay::default(),
                types: RuntimeTypeContext::default(),
            },
            RuntimeActivation {
                service: ServiceMeta {
                    id: "example.com/service-a".to_string(),
                    display_name: Some("Service A".to_string()),
                    metadata: Default::default(),
                },
                version: "v1".to_string(),
                package_configs: Vec::new(),
                service_dependencies: Vec::new(),
                timeout: Default::default(),
                operation_route_bindings: Vec::new(),
                db: Vec::new(),
                actors: Vec::new(),
                gateway: serde_json::from_value::<GatewayConfig>(json!({
                    "metadata": {
                        "websocket": {
                            "gatewayEntryIdentity": "skiff-gateway-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111",
                            "duplicate": {
                                "gatewayEntryIdentity": "skiff-gateway-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111"
                            },
                            "connect": {
                                "gatewayEntryIdentity": "skiff-gateway-v1:sha256:2222222222222222222222222222222222222222222222222222222222222222"
                            },
                            "receive": {
                                "gatewayEntryIdentity": "skiff-gateway-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333"
                            }
                        }
                    }
                }))
                .expect("gateway config fixture should deserialize"),
            },
        )
    }
}
