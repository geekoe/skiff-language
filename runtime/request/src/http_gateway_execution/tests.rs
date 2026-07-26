use skiff_artifact_model::{
    AssemblyIdentity, GatewayAdapterKind, GatewayDispatchMode, GatewayEntryIdentity,
};
use skiff_runtime_transport::{
    protocol::RUNTIME_FRAME_SCHEMA_VERSION,
    runtime_assembly_request::{
        RuntimeAssemblyHttpRequestFrameHeader, RuntimeAssemblyRequestCallerFrameHeader,
        RuntimeAssemblyRequestIngressFrameHeader, RuntimeAssemblyRequestIngressProtocol,
        RuntimeAssemblyRequestRoutingFrameHeader, RuntimeAssemblyRequestStartFrameHeader,
        RuntimeAssemblyRequestTraceFrameHeader,
    },
};

use super::{
    validate_request_facts, HttpGatewayRequestValidationFacts, RequestError, RequestResult,
};

const ASSEMBLY_GENERATION: u64 = 17;
const GATEWAY_ENTRY_KEY: &str = "http:users.create";

struct ValidationFixture {
    assembly_identity: AssemblyIdentity,
    gateway_entry_identity: GatewayEntryIdentity,
    request_local_generation: u64,
}

impl ValidationFixture {
    fn new(request_local_generation: u64) -> Self {
        Self {
            assembly_identity: assembly_identity('a'),
            gateway_entry_identity: gateway_entry_identity('b'),
            request_local_generation,
        }
    }

    fn target_facts(&self) -> HttpGatewayRequestValidationFacts<'_> {
        HttpGatewayRequestValidationFacts {
            gateway_entry_key: GATEWAY_ENTRY_KEY,
            assembly_identity: &self.assembly_identity,
            assembly_generation: ASSEMBLY_GENERATION,
            gateway_entry_identity: &self.gateway_entry_identity,
            dispatch_mode: GatewayDispatchMode::Unary,
            surface_adapter_kind: GatewayAdapterKind::TypedJson,
            plan_adapter_kind: GatewayAdapterKind::TypedJson,
        }
    }

    fn header(&self) -> RuntimeAssemblyRequestStartFrameHeader {
        RuntimeAssemblyRequestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "request.start".to_string(),
            request_id: format!("request-{}", self.request_local_generation),
            mode: "unary".to_string(),
            caller: RuntimeAssemblyRequestCallerFrameHeader {
                kind: "gateway".to_string(),
            },
            routing: RuntimeAssemblyRequestRoutingFrameHeader {
                kind: "runtimeAssembly".to_string(),
                assembly_identity: self.assembly_identity.clone(),
                assembly_generation: ASSEMBLY_GENERATION,
                gateway_entry_identity: self.gateway_entry_identity.clone(),
                ingress: RuntimeAssemblyRequestIngressFrameHeader {
                    protocol: RuntimeAssemblyRequestIngressProtocol::Http,
                    host: "api.example.test".to_string(),
                    method: "POST".to_string(),
                    path: "/users".to_string(),
                },
            },
            client_session: None,
            deadline: None,
            trace: RuntimeAssemblyRequestTraceFrameHeader {
                trace_id: format!("trace-{}", self.request_local_generation),
                span_id: "span-http-gateway-validation".to_string(),
                parent_span_id: None,
                sampled: None,
            },
            http_request: RuntimeAssemblyHttpRequestFrameHeader {
                method: "POST".to_string(),
                url: "https://api.example.test/users".to_string(),
                path: "/users".to_string(),
                query: Vec::new(),
                headers: Vec::new(),
            },
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
        validate_request_facts(fixture.target_facts(), &fixture.header())
            .expect("request-local generation must not replace the pinned assembly generation");
    }
}

#[test]
fn runtime_http_gateway_wrong_assembly_generation_fails_closed() {
    let fixture = ValidationFixture::new(803);
    let mut header = fixture.header();
    header.routing.assembly_generation += 1;

    assert_protocol_error(
        validate_request_facts(fixture.target_facts(), &header),
        "HTTP gateway request does not match the pinned assembly activation",
    );
}

#[test]
fn runtime_http_gateway_wrong_assembly_or_gateway_identity_fails_closed() {
    let fixture = ValidationFixture::new(804);

    let mut wrong_assembly = fixture.header();
    wrong_assembly.routing.assembly_identity = assembly_identity('c');
    assert_protocol_error(
        validate_request_facts(fixture.target_facts(), &wrong_assembly),
        "HTTP gateway request does not match the pinned assembly activation",
    );

    let mut wrong_gateway = fixture.header();
    wrong_gateway.routing.gateway_entry_identity = gateway_entry_identity('d');
    assert_protocol_error(
        validate_request_facts(fixture.target_facts(), &wrong_gateway),
        "HTTP gateway request identity does not match the exact linked entry",
    );
}

#[test]
fn runtime_http_gateway_disagreeing_request_metadata_fails_closed() {
    let fixture = ValidationFixture::new(805);

    let mut wrong_method = fixture.header();
    wrong_method.http_request.method = "PUT".to_string();
    assert_protocol_error(
        validate_request_facts(fixture.target_facts(), &wrong_method),
        "HTTP gateway routing metadata and binary HTTP context disagree",
    );

    let mut wrong_path = fixture.header();
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
        "skiff-gateway-entry-v1:sha256:{}",
        digest.to_string().repeat(64)
    ))
    .expect("fixture gateway entry identity")
}
