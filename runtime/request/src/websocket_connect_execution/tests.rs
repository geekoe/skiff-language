use super::*;
use skiff_artifact_model::{
    AssemblyIdentity, DeploymentArtifactIdentity, DeploymentRevision, GatewayEntryIdentity,
    GatewayEntryKey, IngressSelector, ServiceDeploymentRef, WebSocketEntryId,
    WEBSOCKET_GATEWAY_ENTRY_KEY,
};
struct Fixture {
    key: GatewayEntryKey,
    selector: IngressSelector,
    assembly: AssemblyIdentity,
    gateway_identity: GatewayEntryIdentity,
    deployment: ServiceDeploymentRef,
    websocket_entry_id: WebSocketEntryId,
    request: RuntimeWebSocketConnectIngress,
}

impl Fixture {
    fn new() -> Self {
        let assembly = AssemblyIdentity::new("assembly:websocket");
        let gateway_identity = GatewayEntryIdentity::parse(format!(
            "skiff-gateway-entry-v2:sha256:{}",
            "1".repeat(64)
        ))
        .unwrap();
        let websocket_entry_id = WebSocketEntryId::parse(format!(
            "skiff-websocket-entry-v1:sha256:{}",
            "2".repeat(64)
        ))
        .unwrap();
        let deployment = ServiceDeploymentRef {
            service_id: "service.websocket".to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: DeploymentRevision::new("revision-1"),
            deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
                "skiff-deployment-artifact-v4:sha256:{}",
                "3".repeat(64)
            )),
        };
        let selector = IngressSelector {
            protocol: IngressProtocol::WebSocket,
            method: None,
            path: "/connect".to_string(),
        };
        let request = RuntimeWebSocketConnectIngress {
            request_id: "request-1".to_string(),
            pin: crate::RuntimeGatewayIngressPin {
                assembly_identity: assembly.clone(),
                assembly_generation: 7,
                deployment: deployment.clone(),
                gateway_entry_identity: gateway_identity.clone(),
            },
            ingress_path: selector.path.clone(),
            connection_id: "connection-1".to_string(),
            url: "wss://websocket.test/connect".to_string(),
            query: Vec::new(),
            headers: Vec::new(),
            cookies: Vec::new(),
            version: None,
            websocket_entry_id: websocket_entry_id.clone(),
            connect_gateway_entry_identity: gateway_identity.clone(),
            test_effects_enabled: false,
        };
        Self {
            key: GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap(),
            selector,
            assembly,
            gateway_identity,
            deployment,
            websocket_entry_id,
            request,
        }
    }

    fn facts(&self) -> RuntimeWebSocketConnectRequestTargetFacts<'_> {
        RuntimeWebSocketConnectRequestTargetFacts {
            gateway_entry_key: &self.key,
            selector: &self.selector,
            assembly_identity: &self.assembly,
            assembly_generation: 7,
            deployment: &self.deployment,
            gateway_entry_identity: &self.gateway_identity,
            websocket_entry_id: &self.websocket_entry_id,
        }
    }
}

#[test]
fn websocket_connect_request_projection_matches_exact_activation_entry() {
    let fixture = Fixture::new();
    validate_request_facts(fixture.facts(), &fixture.request)
        .expect("exact request facts should validate");
}

#[test]
fn websocket_connect_request_rejects_projected_activation_and_generation_mismatches() {
    let fixture = Fixture::new();
    let mut mutations = Vec::new();

    let mut wrong_routing_identity = fixture.request.clone();
    wrong_routing_identity.pin.gateway_entry_identity =
        GatewayEntryIdentity::parse(format!("skiff-gateway-entry-v2:sha256:{}", "3".repeat(64)))
            .unwrap();
    mutations.push(wrong_routing_identity);

    let mut wrong_connect_identity = fixture.request.clone();
    wrong_connect_identity.connect_gateway_entry_identity =
        GatewayEntryIdentity::parse(format!("skiff-gateway-entry-v2:sha256:{}", "4".repeat(64)))
            .unwrap();
    mutations.push(wrong_connect_identity);

    let mut wrong_entry_id = fixture.request.clone();
    wrong_entry_id.websocket_entry_id = WebSocketEntryId::parse(format!(
        "skiff-websocket-entry-v1:sha256:{}",
        "5".repeat(64)
    ))
    .unwrap();
    mutations.push(wrong_entry_id);

    let mut wrong_assembly = fixture.request.clone();
    wrong_assembly.pin.assembly_identity = AssemblyIdentity::new("assembly:other");
    mutations.push(wrong_assembly);

    let mut stale_generation = fixture.request.clone();
    stale_generation.pin.assembly_generation = 6;
    mutations.push(stale_generation);

    let mut wrong_deployment = fixture.request.clone();
    wrong_deployment.pin.deployment.service_id = "service.other".to_string();
    mutations.push(wrong_deployment);

    for mutation in mutations {
        assert!(validate_request_facts(fixture.facts(), &mutation).is_err());
    }
}
