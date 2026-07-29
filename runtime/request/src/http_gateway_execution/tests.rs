use skiff_artifact_model::{
    AssemblyIdentity, DeploymentArtifactIdentity, DeploymentRevision, GatewayAdapterKind,
    GatewayDispatchMode, GatewayEntryIdentity, ServiceDeploymentRef,
};

use super::{
    validate_request_facts, HttpGatewayRequestValidationFacts, RequestError, RequestResult,
};
use crate::{BinaryHttpRequestMetadata, RuntimeGatewayIngressPin, RuntimeHttpGatewayRequest};

const ASSEMBLY_GENERATION: u64 = 17;
const GATEWAY_ENTRY_KEY: &str = "http:users.create";

struct ValidationFixture {
    assembly_identity: AssemblyIdentity,
    gateway_entry_identity: GatewayEntryIdentity,
    deployment: ServiceDeploymentRef,
    request_local_generation: u64,
}

impl ValidationFixture {
    fn new(request_local_generation: u64) -> Self {
        Self {
            assembly_identity: assembly_identity('a'),
            gateway_entry_identity: gateway_entry_identity('b'),
            deployment: deployment("service.http", 'e'),
            request_local_generation,
        }
    }

    fn target_facts(&self) -> HttpGatewayRequestValidationFacts<'_> {
        HttpGatewayRequestValidationFacts {
            gateway_entry_key: GATEWAY_ENTRY_KEY,
            assembly_identity: &self.assembly_identity,
            assembly_generation: ASSEMBLY_GENERATION,
            deployment: &self.deployment,
            gateway_entry_identity: &self.gateway_entry_identity,
            dispatch_mode: GatewayDispatchMode::Unary,
            surface_adapter_kind: GatewayAdapterKind::TypedJson,
            plan_adapter_kind: GatewayAdapterKind::TypedJson,
        }
    }

    fn request(&self) -> RuntimeHttpGatewayRequest {
        RuntimeHttpGatewayRequest {
            request_id: format!("request-{}", self.request_local_generation),
            dispatch_mode: GatewayDispatchMode::Unary,
            pin: RuntimeGatewayIngressPin {
                assembly_identity: self.assembly_identity.clone(),
                assembly_generation: ASSEMBLY_GENERATION,
                deployment: self.deployment.clone(),
                gateway_entry_identity: self.gateway_entry_identity.clone(),
            },
            ingress_method: "POST".to_string(),
            ingress_path: "/users".to_string(),
            http_request: BinaryHttpRequestMetadata {
                method: "POST".to_string(),
                url: "https://api.example.test/users".to_string(),
                path: "/users".to_string(),
                query: Vec::new(),
                headers: Vec::new(),
            },
            body: Vec::new(),
            test_effects_enabled: false,
        }
    }
}

#[test]
fn runtime_http_gateway_same_pinned_assembly_accepts_consecutive_request_generations() {
    let first = ValidationFixture::new(801);
    let second = ValidationFixture::new(802);
    assert_ne!(
        first.request_local_generation,
        second.request_local_generation
    );
    assert_ne!(first.request_local_generation, ASSEMBLY_GENERATION);
    assert_ne!(second.request_local_generation, ASSEMBLY_GENERATION);

    for fixture in [&first, &second] {
        validate_request_facts(fixture.target_facts(), &fixture.request())
            .expect("request-local generation must not replace the pinned assembly generation");
    }
}

#[test]
fn runtime_http_gateway_wrong_assembly_generation_fails_closed() {
    let fixture = ValidationFixture::new(803);
    let mut request = fixture.request();
    request.pin.assembly_generation += 1;

    assert_protocol_error(
        validate_request_facts(fixture.target_facts(), &request),
        "HTTP gateway request does not match the pinned assembly activation",
    );
}

#[test]
fn runtime_http_gateway_wrong_assembly_or_gateway_identity_fails_closed() {
    let fixture = ValidationFixture::new(804);

    let mut wrong_assembly = fixture.request();
    wrong_assembly.pin.assembly_identity = assembly_identity('c');
    assert_protocol_error(
        validate_request_facts(fixture.target_facts(), &wrong_assembly),
        "HTTP gateway request does not match the pinned assembly activation",
    );

    let mut wrong_deployment = fixture.request();
    wrong_deployment.pin.deployment = deployment("service.other", 'f');
    assert_protocol_error(
        validate_request_facts(fixture.target_facts(), &wrong_deployment),
        "HTTP gateway request does not match the pinned assembly activation",
    );

    let mut wrong_gateway = fixture.request();
    wrong_gateway.pin.gateway_entry_identity = gateway_entry_identity('d');
    assert_protocol_error(
        validate_request_facts(fixture.target_facts(), &wrong_gateway),
        "HTTP gateway request identity does not match the exact linked entry",
    );
}

fn deployment(service_id: &str, fill: char) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: service_id.to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new("revision-1"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
            "skiff-deployment-artifact-v4:sha256:{}",
            fill.to_string().repeat(64)
        )),
    }
}

#[test]
fn runtime_http_gateway_disagreeing_request_metadata_fails_closed() {
    let fixture = ValidationFixture::new(805);

    let mut wrong_method = fixture.request();
    wrong_method.http_request.method = "PUT".to_string();
    assert_protocol_error(
        validate_request_facts(fixture.target_facts(), &wrong_method),
        "HTTP gateway routing metadata and binary HTTP context disagree",
    );

    let mut wrong_path = fixture.request();
    wrong_path.http_request.path = "/other".to_string();
    assert_protocol_error(
        validate_request_facts(fixture.target_facts(), &wrong_path),
        "HTTP gateway routing metadata and binary HTTP context disagree",
    );
}

fn assert_protocol_error(result: RequestResult<()>, expected_message: &str) {
    match result {
        Err(RequestError::Protocol { target, message }) => {
            assert_eq!(target, GATEWAY_ENTRY_KEY);
            assert_eq!(message, expected_message);
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
}

fn assembly_identity(digest: char) -> AssemblyIdentity {
    AssemblyIdentity::new(format!(
        "skiff-runtime-assembly-v1:sha256:{}",
        digest.to_string().repeat(64)
    ))
}

fn gateway_entry_identity(digest: char) -> GatewayEntryIdentity {
    GatewayEntryIdentity::parse(format!(
        "skiff-gateway-entry-v2:sha256:{}",
        digest.to_string().repeat(64)
    ))
    .expect("fixture gateway entry identity")
}
