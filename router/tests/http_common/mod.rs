//! Shared fixtures and real-socket client helpers for W-http probes.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentGatewayEntry, DeploymentIngressBinding, DeploymentRevision, GatewayAdapterKind,
    GatewayAdapterPlan, GatewayDispatchMode, GatewayEntryIdentity, GatewayEntryKey,
    GatewayEntryProtocolSurface, GatewayExternalErrorProjection, GatewayHttpProtocolSurface,
    GatewayIngressBinding, GatewayProtocolSurface, IngressProtocol, IngressSelector,
    PackageArtifactRef, PackageBuildId, PackageLocalAbiIdentity, RuntimeAssembly,
    RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef, ServiceContractRef, ServiceDeployment,
    ServiceDeploymentRef, ServiceProtocolIdentity, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
    RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_artifact_model::GatewayAdapterSource;
use skiff_artifact_model::GatewayExternalSchema;
use skiff_artifact_identity::assign_service_deployment_identity;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_deployment::storage::{CanonicalArtifactStore, ReleasePointer};
use skiff_router::artifact::ActorRoutingCatalog;
use skiff_router::bootstrap::RoutingEpoch;
use skiff_router::http::{EpochHttpIngressResolver, HttpGatewaySurfaceView, HttpIngressResolver};
use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;

pub const SERVICE_ID: &str = "example.com/service-1";
pub const CONTRACT_VERSION: &str = "1.0.0";
pub const DEPLOYMENT_REVISION: &str = "deployment-1";
pub const ASSEMBLY_IDENTITY: &str =
    "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const CONFIG_SNAPSHOT_ID: &str =
    "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub const GATEWAY_ITEMS_IDENTITY: &str =
    "skiff-gateway-entry-v2:sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
pub const GATEWAY_EVENTS_IDENTITY: &str =
    "skiff-gateway-entry-v2:sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
pub const GATEWAY_ITEMS_OPTIONS_IDENTITY: &str =
    "skiff-gateway-entry-v2:sha256:1111111111111111111111111111111111111111111111111111111111111111";

/// The canonical fixture deployment record (content-addressed identity).
fn fixture_service_deployment() -> ServiceDeployment {
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: ServiceContractRef {
            service_id: SERVICE_ID.to_string(),
            contract_version: CONTRACT_VERSION.to_string(),
            service_protocol_identity: ServiceProtocolIdentity::new("protocol"),
        },
        deployment_revision: DeploymentRevision::new(DEPLOYMENT_REVISION.to_string()),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("placeholder"),
        implementation: PackageArtifactRef {
            package_id: SERVICE_ID.to_string(),
            package_version: "0.1.0".to_string(),
            package_build_id: PackageBuildId::new("build"),
            package_local_abi_identity: PackageLocalAbiIdentity::new("abi"),
        },
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::from([
            (
                GatewayEntryKey::parse("items").expect("key"),
                surface_entry(
                    GatewayDispatchMode::Unary,
                    GatewayAdapterKind::TypedJson,
                    GATEWAY_ITEMS_IDENTITY,
                ),
            ),
            (
                GatewayEntryKey::parse("events").expect("key"),
                surface_entry(
                    GatewayDispatchMode::ServerStream,
                    GatewayAdapterKind::RawHttp,
                    GATEWAY_EVENTS_IDENTITY,
                ),
            ),
            (
                GatewayEntryKey::parse("items-options").expect("key"),
                surface_entry(
                    GatewayDispatchMode::Unary,
                    GatewayAdapterKind::TypedJson,
                    GATEWAY_ITEMS_OPTIONS_IDENTITY,
                ),
            ),
        ]),
        ingress: vec![
            DeploymentIngressBinding {
                selector: IngressSelector {
                    protocol: IngressProtocol::Http,
                    method: Some("POST".to_string()),
                    path: "/items".to_string(),
                },
                gateway_entry_key: GatewayEntryKey::parse("items").expect("key"),
            },
            DeploymentIngressBinding {
                selector: IngressSelector {
                    protocol: IngressProtocol::Http,
                    method: Some("GET".to_string()),
                    path: "/events".to_string(),
                },
                gateway_entry_key: GatewayEntryKey::parse("events").expect("key"),
            },
            DeploymentIngressBinding {
                selector: IngressSelector {
                    protocol: IngressProtocol::Http,
                    method: Some("OPTIONS".to_string()),
                    path: "/items".to_string(),
                },
                gateway_entry_key: GatewayEntryKey::parse("items-options").expect("key"),
            },
        ],
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "http-gateway-fixture".to_string(),
            notes: BTreeMap::new(),
        },
    };
    assign_service_deployment_identity(&mut deployment).expect("assign deployment identity");
    deployment
}

pub fn fixture_deployment() -> ServiceDeploymentRef {
    skiff_artifact_identity::service_deployment_ref(&fixture_service_deployment())
}

pub fn fixture_deployment_identity() -> String {
    fixture_deployment()
        .deployment_artifact_identity
        .as_str()
        .to_string()
}

/// Process-wide canonical store holding the fixture deployment record and
/// its release pointer (profile `prod`).
fn fixture_store() -> Arc<CanonicalArtifactStore> {
    static STORE: OnceLock<Arc<CanonicalArtifactStore>> = OnceLock::new();
    Arc::clone(STORE.get_or_init(|| {
        let path = std::env::temp_dir().join(format!(
            "skiff-http-gateway-fixture-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let store = CanonicalArtifactStore::create(&path).expect("create fixture artifact store");
        let deployment = fixture_service_deployment();
        store
            .write_service_deployment(&deployment)
            .expect("write fixture deployment record");
        let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
        let pointer = ReleasePointer::new("prod", reference).expect("fixture release pointer");
        store
            .write_release_pointer(&pointer)
            .expect("write fixture release pointer");
        Arc::new(store)
    }))
}

pub fn fixture_epoch() -> Arc<RoutingEpoch> {
    let deployment = fixture_deployment();
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new(ASSEMBLY_IDENTITY),
        roots: vec![deployment.clone()],
        resolved_deployments: vec![deployment.clone()],
        resolved_contracts: Vec::new(),
        resolved_packages: Vec::new(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: Vec::new(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: vec![
            GatewayIngressBinding {
                selector: IngressSelector {
                    protocol: IngressProtocol::Http,
                    method: Some("POST".to_string()),
                    path: "/items".to_string(),
                },
                deployment: deployment.clone(),
                gateway_entry_key: GatewayEntryKey::parse("items").expect("key"),
                gateway_entry_identity: GatewayEntryIdentity::parse(GATEWAY_ITEMS_IDENTITY)
                    .expect("identity"),
            },
            GatewayIngressBinding {
                selector: IngressSelector {
                    protocol: IngressProtocol::Http,
                    method: Some("GET".to_string()),
                    path: "/events".to_string(),
                },
                deployment: deployment.clone(),
                gateway_entry_key: GatewayEntryKey::parse("events").expect("key"),
                gateway_entry_identity: GatewayEntryIdentity::parse(GATEWAY_EVENTS_IDENTITY)
                    .expect("identity"),
            },
            GatewayIngressBinding {
                selector: IngressSelector {
                    protocol: IngressProtocol::Http,
                    method: Some("OPTIONS".to_string()),
                    path: "/items".to_string(),
                },
                deployment: deployment.clone(),
                gateway_entry_key: GatewayEntryKey::parse("items-options").expect("key"),
                gateway_entry_identity: GatewayEntryIdentity::parse(GATEWAY_ITEMS_OPTIONS_IDENTITY)
                    .expect("identity"),
            },
        ],
    };
    let snapshot = RuntimeConfigSnapshot::new(
        "prod",
        RuntimeConfigSnapshotRef {
            snapshot_id: RuntimeConfigSnapshotId::parse(CONFIG_SNAPSHOT_ID).expect("snapshot id"),
        },
        Vec::new(),
    )
    .expect("snapshot fixture");
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection");
    let catalog = Arc::new(ActorRoutingCatalog::from_projection(Arc::new(projection)));
    Arc::new(
        RoutingEpoch::new("prod", 42, Arc::new(assembly), Arc::new(snapshot), catalog)
            .expect("epoch fixture"),
    )
}

fn surface_entry(
    mode: GatewayDispatchMode,
    adapter_kind: GatewayAdapterKind,
    identity: &str,
) -> DeploymentGatewayEntry {
    let external_sources = if adapter_kind == GatewayAdapterKind::RawHttp {
        vec![GatewayAdapterSource::HttpRequest]
    } else {
        Vec::new()
    };
    let stream_item_schema = if mode == GatewayDispatchMode::ServerStream {
        Some(GatewayExternalSchema::String)
    } else {
        None
    };
    DeploymentGatewayEntry {
        gateway_entry_identity: GatewayEntryIdentity::parse(identity).expect("identity"),
        protocol_surface: GatewayEntryProtocolSurface {
            protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
                adapter_kind,
                dispatch_mode: mode,
                external_sources,
                request_body_schema: None,
                response_schema: None,
                stream_item_schema,
            }),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        },
        handler: None,
        pre: None,
        guard: None,
        adapter_plan: GatewayAdapterPlan {
            kind: adapter_kind,
            args: Vec::new(),
        },
    }
}

pub fn fixture_resolver() -> Arc<dyn HttpIngressResolver> {
    let deployment = fixture_deployment();
    let entries = BTreeMap::from([
        (
            (
                deployment.clone(),
                GatewayEntryKey::parse("items").expect("key"),
            ),
            surface_entry(
                GatewayDispatchMode::Unary,
                GatewayAdapterKind::TypedJson,
                GATEWAY_ITEMS_IDENTITY,
            ),
        ),
        (
            (
                deployment.clone(),
                GatewayEntryKey::parse("events").expect("key"),
            ),
            surface_entry(
                GatewayDispatchMode::ServerStream,
                GatewayAdapterKind::RawHttp,
                GATEWAY_EVENTS_IDENTITY,
            ),
        ),
        (
            (
                deployment.clone(),
                GatewayEntryKey::parse("items-options").expect("key"),
            ),
            surface_entry(
                GatewayDispatchMode::Unary,
                GatewayAdapterKind::TypedJson,
                GATEWAY_ITEMS_OPTIONS_IDENTITY,
            ),
        ),
    ]);
    Arc::new(EpochHttpIngressResolver::new_with_live_artifact_store(
        Arc::new(
            HttpGatewaySurfaceView::from_deployment_gateway_entries(&entries)
                .expect("surface view fixture"),
        ),
        (*fixture_store()).clone(),
    ))
}

#[derive(Debug, Clone)]
pub struct RawResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RawResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub fn send_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> io::Result<RawResponse> {
    let mut stream = TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut request = format!("{method} {path} HTTP/1.1\r\nhost: {addr}\r\nconnection: close\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!("content-length: {}\r\n\r\n", body.len()));
    stream.write_all(request.as_bytes())?;
    if !body.is_empty() {
        stream.write_all(body)?;
    }
    let mut raw = Vec::new();
    match stream.read_to_end(&mut raw) {
        Ok(_) => {}
        // macOS/BSD stacks can deliver RST after the server already wrote a
        // complete response; if the bytes we received form a complete
        // response, treat it as success (curl-compatible behavior).
        Err(_) if complete_response(&raw) => {}
        Err(error) => return Err(error),
    }
    parse_response(&raw)
}

fn complete_response(raw: &[u8]) -> bool {
    let Some(header_end) = raw.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(&raw[..header_end]);
    let content_length = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        (name.trim().eq_ignore_ascii_case("content-length"))
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    });
    if let Some(length) = content_length {
        return raw.len() >= header_end + 4 + length;
    }
    let chunked = head.lines().any(|line| {
        line.to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
    });
    if !chunked {
        return true;
    }
    let body = &raw[header_end + 4..];
    body.windows(5).any(|window| window == b"0\r\n\r\n")
        || body.windows(7).any(|window| window == b"0\r\n\r\n")
}

fn parse_response(raw: &[u8]) -> io::Result<RawResponse> {
    let header_end = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing header terminator"))?;
    let head = String::from_utf8_lossy(&raw[..header_end]);
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing status"))?
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid status"))?;
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    let body = &raw[header_end + 4..];
    let chunked = headers
        .iter()
        .any(|(name, value)| name == "transfer-encoding" && value.contains("chunked"));
    let body = if chunked {
        decode_chunked(body)?
    } else {
        body.to_vec()
    };
    Ok(RawResponse {
        status,
        headers,
        body,
    })
}

fn decode_chunked(mut input: &[u8]) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    loop {
        let line_end = input
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "chunk size line missing"))?;
        let size_text = String::from_utf8_lossy(&input[..line_end]);
        let size_text = size_text.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid chunk size"))?;
        input = &input[line_end + 2..];
        if size == 0 {
            return Ok(output);
        }
        if input.len() < size + 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk data truncated",
            ));
        }
        output.extend_from_slice(&input[..size]);
        input = &input[size + 2..];
    }
}

/// Streaming client: reads the response head, then chunked frames one at a
/// time (used for ordering, ceiling and disconnect probes).
pub struct StreamClient {
    reader: BufReader<TcpStream>,
}

impl StreamClient {
    pub fn connect(
        addr: SocketAddr,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
    ) -> io::Result<(Self, RawResponseHead)> {
        let stream = TcpStream::connect(addr)?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let mut request =
            format!("{method} {path} HTTP/1.1\r\nhost: {addr}\r\nconnection: close\r\n");
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("content-length: 0\r\n\r\n");
        let mut reader = BufReader::new(stream);
        reader.get_mut().write_all(request.as_bytes())?;
        let mut head_bytes = Vec::new();
        loop {
            let byte = reader.fill_buf()?;
            if byte.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "connection closed before response head",
                ));
            }
            let read = byte.len().min(1);
            head_bytes.extend_from_slice(&byte[..read]);
            reader.consume(read);
            if head_bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let head = parse_response_head(&head_bytes)?;
        Ok((Self { reader }, head))
    }

    /// Reads one chunked frame payload; `None` at the terminating frame.
    pub fn read_chunk(&mut self) -> io::Result<Option<Vec<u8>>> {
        let line = read_line(&mut self.reader)?;
        let size_text = line.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid chunk size"))?;
        if size == 0 {
            // Skip trailers until the blank line.
            loop {
                let trailer = read_line(&mut self.reader)?;
                if trailer.is_empty() {
                    return Ok(None);
                }
            }
        }
        let mut data = vec![0u8; size];
        self.reader.read_exact(&mut data)?;
        let mut crlf = [0u8; 2];
        self.reader.read_exact(&mut crlf)?;
        Ok(Some(data))
    }
}

pub struct RawResponseHead {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

impl RawResponseHead {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, value)| value.as_str())
    }
}

fn parse_response_head(raw: &[u8]) -> io::Result<RawResponseHead> {
    let header_end = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing header terminator"))?;
    let head = String::from_utf8_lossy(&raw[..header_end]);
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing status"))?
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid status"))?;
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    Ok(RawResponseHead { status, headers })
}

fn read_line(reader: &mut BufReader<TcpStream>) -> io::Result<String> {
    let mut line = Vec::new();
    loop {
        let byte = reader.fill_buf()?;
        if byte.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed mid-line",
            ));
        }
        let byte = byte[0];
        reader.consume(1);
        if byte == b'\n' {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return String::from_utf8(line)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "line is not UTF-8"));
        }
        line.push(byte);
    }
}

pub fn service_headers() -> Vec<(&'static str, &'static str)> {
    vec![
        ("x-skiff-service", SERVICE_ID),
        ("x-skiff-version", CONTRACT_VERSION),
    ]
}
