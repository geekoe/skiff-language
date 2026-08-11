//! `POST /__skiff/test-dispatch` control handler (plan §7 E-http).
//!
//! TS parity with `router/src/router/assemblyControlPlane.ts`
//! (`decodeRuntimeAssemblyTestDispatch` / `exactTestDispatchBinding` /
//! `assemblyTestHttpRequestHeader`): exact field decode, canonical payload
//! Base64, positive safe timeout, exact active-assembly generation and
//! gateway-binding match, and the canonical test `request.start` frame
//! (`testEffectsEnabled: true` + fresh `testCaseCapability`). Dispatch uses
//! the production [`HttpDispatchPort`] seam; the control response re-emits
//! the runtime `response.end` / `response.error` frame with TS wire parity.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Method, Request, Response, StatusCode};
use serde_json::{json, Value};
use skiff_artifact_model::IngressProtocol;
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_transport::protocol::{
    ResponseEndFrameHeader, ResponseEndFrameMetadata, RuntimeHttpResponseFrameHeader,
    RUNTIME_FRAME_SCHEMA_VERSION,
};
use skiff_runtime_transport::runtime_assembly_request::{
    decode_runtime_assembly_request_start_frame, RuntimeAssemblyHttpRequestFrameHeader,
    RuntimeAssemblyRequestCallerFrameHeader, RuntimeAssemblyRequestDeadlineFrameHeader,
    RuntimeAssemblyRequestRoutingFrameHeader, RuntimeAssemblyRequestStartFrameHeader,
    RuntimeAssemblyRequestTraceFrameHeader,
};

use crate::http::dispatch::{
    cancel_channel, DispatchRequest, HttpDispatchError, HttpDispatchPort, TestDispatchOutcome,
};
use crate::http::frame::{
    deadline_parts, encode_request_start_frame, new_request_id, new_span_id,
    new_test_case_capability, new_trace_id,
};
use crate::http::ingress::HttpGatewaySurfaceView;

/// Canonical control endpoint for runtimeAssembly test dispatch
/// (`POST /__skiff/test-dispatch`).
pub const TEST_DISPATCH_CONTROL_PATH: &str = "/__skiff/test-dispatch";

/// Body cap for the test-dispatch request (TS `readBody` parity: 1 MiB).
pub const TEST_DISPATCH_REQUEST_BODY_CAP: usize = 1024 * 1024;

const BODY_TOO_LARGE_MESSAGE: &str = "assembly activation request body exceeds 1 MiB";

/// `Number.MAX_SAFE_INTEGER` (TS `Number.isSafeInteger` parity).
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

const TEST_DISPATCH_FIELDS: [&str; 6] = [
    "kind",
    "routing",
    "mode",
    "httpRequest",
    "payloadBase64",
    "timeoutMs",
];

pub type TestDispatchHttpResponse = Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error>;

/// Production test-dispatch handler (test-dispatch lane).
#[derive(Clone)]
pub struct TestDispatchHttpHandler {
    options: Arc<TestDispatchHttpHandlerOptions>,
}

impl fmt::Debug for TestDispatchHttpHandler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TestDispatchHttpHandler")
            .field("dispatcher", &"HttpDispatchPort")
            .finish_non_exhaustive()
    }
}

/// Handler dependencies (supervisor composition injects the profile, the
/// static surface view and the artifact store for pointer-table surface
/// rebuilds; M4: no epoch).
#[derive(Clone)]
pub struct TestDispatchHttpHandlerOptions {
    pub profile: String,
    pub surfaces: Arc<HttpGatewaySurfaceView>,
    pub artifact_store: CanonicalArtifactStore,
    pub dispatcher: Arc<dyn HttpDispatchPort>,
}

impl fmt::Debug for TestDispatchHttpHandlerOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TestDispatchHttpHandlerOptions")
            .field("profile", &self.profile)
            .field("surfaces", &self.surfaces)
            .field("dispatcher", &"HttpDispatchPort")
            .finish_non_exhaustive()
    }
}

impl TestDispatchHttpHandler {
    pub fn new(options: TestDispatchHttpHandlerOptions) -> Self {
        Self {
            options: Arc::new(options),
        }
    }

    /// Routes one control request (listener already matched the canonical
    /// path). Reads the body with the 1 MiB cap, then performs the exact
    /// test dispatch and returns the TS-parity JSON outcome.
    pub async fn handle(&self, request: Request<Incoming>) -> TestDispatchHttpResponse {
        let method = request.method().clone();
        if method != Method::POST {
            return method_not_allowed();
        }
        let body = match read_body_capped(request.into_body(), TEST_DISPATCH_REQUEST_BODY_CAP).await
        {
            Ok(body) => body,
            Err(message) => {
                return control_json_error(classify_activation_error(&message), &message)
            }
        };
        self.handle_parts(&method, &body).await
    }

    /// Pure request handling over an already-read body (unit-test seam; the
    /// wire path additionally enforces the cap while reading).
    pub async fn handle_parts(&self, method: &Method, body: &[u8]) -> TestDispatchHttpResponse {
        if method != &Method::POST {
            return method_not_allowed();
        }
        if body.len() > TEST_DISPATCH_REQUEST_BODY_CAP {
            return control_json_error(
                classify_activation_error(BODY_TOO_LARGE_MESSAGE),
                BODY_TOO_LARGE_MESSAGE,
            );
        }
        let decoded = match decode_test_dispatch(body) {
            Ok(decoded) => decoded,
            Err(message) => {
                return control_json_error(classify_activation_error(&message), &message);
            }
        };
        let surfaces = match self.current_surfaces() {
            Ok(surfaces) => surfaces,
            Err(message) => {
                return control_json_error(classify_activation_error(&message), &message);
            }
        };
        let _binding = match exact_test_dispatch_binding(
            &self.options.artifact_store,
            &self.options.profile,
            &surfaces,
            &decoded,
        ) {
            Ok(binding) => binding,
            Err(message) => {
                return control_json_error(classify_activation_error(&message), &message);
            }
        };
        let header = match build_test_dispatch_header(&decoded) {
            Ok(header) => header,
            Err(message) => {
                return control_json_error(classify_activation_error(&message), &message);
            }
        };
        let request_id = header.request_id.clone();
        let timeout = Duration::from_millis(decoded.timeout_ms);
        let (_cancel_signal, cancel_watch) = cancel_channel();
        let result = self
            .options
            .dispatcher
            .dispatch_test(DispatchRequest {
                header,
                payload_bytes: decoded.payload_bytes,
                timeout,
                client_disconnect: cancel_watch,
            })
            .await;
        dispatch_outcome_response(&request_id, result)
    }

    fn current_surfaces(&self) -> Result<Arc<HttpGatewaySurfaceView>, String> {
        crate::http::ingress::http_surface_view_from_pointers(
            &self.options.artifact_store,
            &self.options.profile,
        )
        .map(Arc::new)
        .map_err(|message| {
            format!("runtime assembly test dispatch surface load failed: {message}")
        })
    }
}

/// One strictly decoded runtimeAssembly test-dispatch request.
#[derive(Debug, Clone, PartialEq)]
struct DecodedTestDispatch {
    routing: RuntimeAssemblyRequestRoutingFrameHeader,
    mode: String,
    http_request: RuntimeAssemblyHttpRequestFrameHeader,
    payload_bytes: Bytes,
    timeout_ms: u64,
}

/// TS `decodeRuntimeAssemblyTestDispatch` parity: exact object/fields,
/// `kind: "test"`, strict canonical routing/mode/httpRequest decode,
/// canonical standard Base64 payload, positive safe timeout.
fn decode_test_dispatch(bytes: &[u8]) -> Result<DecodedTestDispatch, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        format!("runtime assembly test dispatch request body is not valid JSON: {error}")
    })?;
    let root = value
        .as_object()
        .ok_or_else(|| "runtime assembly test dispatch must be an object".to_string())?;
    for field in TEST_DISPATCH_FIELDS {
        if !root.contains_key(field) {
            return Err(format!("runtime assembly test dispatch requires {field}"));
        }
    }
    for field in root.keys() {
        if !TEST_DISPATCH_FIELDS.contains(&field.as_str()) {
            return Err(format!(
                "runtime assembly test dispatch does not support {field}"
            ));
        }
    }
    if root.get("kind").and_then(Value::as_str) != Some("test") {
        return Err("runtime assembly test dispatch kind must be test".to_string());
    }
    let routing = serde_json::from_value::<RuntimeAssemblyRequestRoutingFrameHeader>(
        root.get("routing").expect("field presence checked").clone(),
    )
    .map_err(|error| {
        format!("runtime assembly test dispatch has invalid canonical fields: {error}")
    })?;
    let mode = root.get("mode").and_then(Value::as_str).ok_or_else(|| {
        "runtime assembly test dispatch mode must be unary or serverStream".to_string()
    })?;
    if mode != "unary" && mode != "serverStream" {
        return Err(
            "runtime assembly test dispatch mode must be unary or serverStream".to_string(),
        );
    }
    let http_request = serde_json::from_value::<RuntimeAssemblyHttpRequestFrameHeader>(
        root.get("httpRequest")
            .expect("field presence checked")
            .clone(),
    )
    .map_err(|error| {
        format!("runtime assembly test dispatch has invalid canonical fields: {error}")
    })?;
    validate_http_request_metadata(&http_request)?;
    let encoded = root
        .get("payloadBase64")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            "runtime assembly test dispatch payloadBase64 must be a string".to_string()
        })?;
    let payload_bytes = BASE64_STANDARD.decode(encoded).map_err(|_| {
        "runtime assembly test dispatch payloadBase64 must be canonical standard Base64".to_string()
    })?;
    if BASE64_STANDARD.encode(&payload_bytes) != encoded {
        return Err(
            "runtime assembly test dispatch payloadBase64 must be canonical standard Base64"
                .to_string(),
        );
    }
    let timeout_ms = root
        .get("timeoutMs")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0 && *value <= MAX_SAFE_INTEGER)
        .ok_or_else(|| {
            "runtime assembly test dispatch timeoutMs must be a positive safe integer".to_string()
        })?;
    Ok(DecodedTestDispatch {
        routing,
        mode: mode.to_string(),
        http_request,
        payload_bytes: Bytes::from(payload_bytes),
        timeout_ms,
    })
}

/// TS canonical-field parity for the HTTP request metadata projected into
/// the test `request.start` frame.
fn validate_http_request_metadata(
    metadata: &RuntimeAssemblyHttpRequestFrameHeader,
) -> Result<(), String> {
    let canonical = |value: &str| {
        !value.is_empty() && !value.chars().any(|c| c.is_control() || c.is_whitespace())
    };
    if !canonical(&metadata.method) || !canonical(&metadata.url) || !canonical(&metadata.path) {
        return Err(
            "runtime assembly test dispatch has invalid canonical fields: httpRequest method/url/path must be non-empty canonical strings"
                .to_string(),
        );
    }
    for pair in metadata.query.iter().chain(metadata.headers.iter()) {
        if !canonical(&pair.name) {
            return Err(
                "runtime assembly test dispatch has invalid canonical fields: httpRequest name must be a non-empty canonical token"
                    .to_string(),
            );
        }
        if pair.value.is_empty() || pair.value.chars().any(|c| c.is_control()) {
            return Err(
                "runtime assembly test dispatch has invalid canonical fields: httpRequest value must be a non-empty canonical string"
                    .to_string(),
            );
        }
    }
    Ok(())
}

/// TS `exactTestDispatchBinding` parity against the current epoch and the
/// live/static HTTP gateway surface view.
fn exact_test_dispatch_binding(
    artifact_store: &CanonicalArtifactStore,
    profile: &str,
    surfaces: &HttpGatewaySurfaceView,
    decoded: &DecodedTestDispatch,
) -> Result<(), String> {
    let routing = &decoded.routing;
    // M4: the test dispatch routing names an exact deployment; it must be
    // currently published by the release pointer table and carry the
    // matching build id.
    let release: Arc<dyn crate::release::ReleaseResolver> =
        Arc::new(crate::release::StoreReleaseResolver::new(artifact_store.clone()));
    let published = release
        .resolve(profile, &routing.deployment.service_id, &routing.deployment.contract_version)
        .map_err(|error| format!("runtime assembly test dispatch release resolve failed: {error}"))?
        .ok_or_else(|| {
            "runtime assembly test dispatch does not match a published release pointer".to_string()
        })?;
    if published != routing.deployment {
        return Err(
            "runtime assembly test dispatch does not match the exact published deployment"
                .to_string(),
        );
    }
    let record = artifact_store
        .read_service_deployment(&routing.deployment)
        .map_err(|error| {
            format!("runtime assembly test dispatch deployment read failed: {error}")
        })?;
    let mut candidates = record
        .ingress
        .iter()
        .filter(|binding| {
            binding.selector.protocol == IngressProtocol::Http
                && binding.selector.method.as_deref() == Some(routing.ingress.method.as_str())
                && binding.selector.path == routing.ingress.path
        })
        .collect::<Vec<_>>();
    candidates.retain(|binding| {
        record
            .gateway_entries
            .get(&binding.gateway_entry_key)
            .is_some_and(|entry| {
                entry.gateway_entry_identity == routing.gateway_entry_identity
            })
    });
    let Some(binding) = candidates.first() else {
        return Err(
            "runtime assembly test dispatch does not match the exact active gateway binding"
                .to_string(),
        );
    };
    if candidates.len() != 1 {
        return Err(
            "runtime assembly test dispatch does not match the exact active gateway binding"
                .to_string(),
        );
    }
    let surface = surfaces
        .get(&routing.deployment, &binding.gateway_entry_key)
        .ok_or_else(|| {
            "runtime assembly test dispatch does not match the exact active gateway binding"
                .to_string()
        })?;
    if surface.mode.as_wire_str() != decoded.mode {
        return Err(
            "runtime assembly test dispatch does not match the exact active gateway binding"
                .to_string(),
        );
    }
    Ok(())
}

/// TS `assemblyTestHttpRequestHeader` parity: canonical `request.start` with
/// the exact body routing/mode/httpRequest, test effects enabled and a fresh
/// test-case capability. The encoded frame is re-decoded through the
/// canonical transport validator (TS
/// `validateRuntimeAssemblyRequestStartFrameHeader` equivalent) before
/// dispatch.
fn build_test_dispatch_header(
    decoded: &DecodedTestDispatch,
) -> Result<RuntimeAssemblyRequestStartFrameHeader, String> {
    let (timeout_ms, expires_at) = deadline_parts(Duration::from_millis(decoded.timeout_ms));
    // M4: the dispatched routing header always carries the exact deployment
    // build id (the candidate admission fails closed without it). The
    // caller-provided routing may omit buildId; the exact deployment was
    // already validated against the release pointer table, so its artifact
    // identity is the authoritative build id (same rule as the HTTP ingress
    // resolver).
    let mut routing = decoded.routing.clone();
    if routing.build_id.is_none() {
        routing.build_id = Some(
            decoded
                .routing
                .deployment
                .deployment_artifact_identity
                .as_str()
                .to_string(),
        );
    }
    let header = RuntimeAssemblyRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: new_request_id(),
        mode: decoded.mode.clone(),
        caller: RuntimeAssemblyRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing,
        client_session: None,
        deadline: Some(RuntimeAssemblyRequestDeadlineFrameHeader {
            timeout_ms,
            expires_at,
        }),
        trace: RuntimeAssemblyRequestTraceFrameHeader {
            trace_id: new_trace_id(),
            span_id: new_span_id(),
            parent_span_id: None,
            sampled: None,
        },
        http_request: decoded.http_request.clone(),
        test_effects_enabled: true,
        test_case_capability: Some(new_test_case_capability()),
        test_case_parent_request_id: None,
    };
    let frame = encode_request_start_frame(&header, &decoded.payload_bytes).map_err(|error| {
        format!("runtime assembly test dispatch has invalid canonical fields: {error}")
    })?;
    decode_runtime_assembly_request_start_frame(&frame).map_err(|error| {
        format!("runtime assembly test dispatch has invalid canonical fields: {error}")
    })?;
    Ok(header)
}

/// Maps the dispatch outcome to the TS-parity control JSON: runtime
/// `response.end` / `response.error` frames are re-emitted as HTTP 200 with
/// `ok: true`; dispatcher-level failures become control errors classified by
/// the TS `classifyActivationError` vocabulary.
fn dispatch_outcome_response(
    request_id: &str,
    result: Result<TestDispatchOutcome, HttpDispatchError>,
) -> TestDispatchHttpResponse {
    match result {
        Ok(TestDispatchOutcome::End(response)) => {
            let header = ResponseEndFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "response.end".to_string(),
                request_id: request_id.to_string(),
                payload_present: true,
                metadata: ResponseEndFrameMetadata::Http(RuntimeHttpResponseFrameHeader {
                    status: response.status,
                    headers: response.headers,
                }),
            };
            let header = serde_json::to_value(&header).expect("response.end header serializes");
            json_response(
                StatusCode::OK,
                &json!({
                    "ok": true,
                    "header": header,
                    "payloadBase64": BASE64_STANDARD.encode(response.payload),
                }),
            )
        }
        Ok(TestDispatchOutcome::Error(header, payload)) => {
            let header = serde_json::to_value(&header).expect("response.error header serializes");
            json_response(
                StatusCode::OK,
                &json!({
                    "ok": true,
                    "header": header,
                    "payloadBase64": BASE64_STANDARD.encode(payload),
                }),
            )
        }
        Err(HttpDispatchError::Control {
            code: _,
            message,
            status,
            details: _,
        }) => {
            let status = status.unwrap_or_else(|| classify_activation_error(&message));
            control_json_error(status, &message)
        }
        Err(HttpDispatchError::Timeout { timeout_ms }) => {
            let message = format!("Runtime did not respond within {timeout_ms}ms");
            control_json_error(classify_activation_error(&message), &message)
        }
        Err(HttpDispatchError::Cancelled { message, .. }) => {
            control_json_error(classify_activation_error(&message), &message)
        }
        Err(HttpDispatchError::FixedService(_)) => {
            let message = "runtime returned a fixed service error frame".to_string();
            control_json_error(classify_activation_error(&message), &message)
        }
    }
}

fn method_not_allowed() -> TestDispatchHttpResponse {
    let body = json!({
        "error": {
            "code": "MethodNotAllowed",
            "message": "test dispatch requires POST",
        }
    });
    json_response_with_headers(StatusCode::METHOD_NOT_ALLOWED, &body, &[("allow", "POST")])
}

fn control_json_error(status: u16, message: &str) -> TestDispatchHttpResponse {
    let code = if status == 503 {
        "AssemblyParticipantsUnavailable"
    } else {
        "AssemblyActivationRejected"
    };
    json_response(
        StatusCode::from_u16(status).unwrap_or(StatusCode::CONFLICT),
        &json!({
            "error": {
                "code": code,
                "message": message,
            }
        }),
    )
}

fn json_response(status: StatusCode, body: &Value) -> TestDispatchHttpResponse {
    json_response_with_headers(status, body, &[])
}

fn json_response_with_headers(
    status: StatusCode,
    body: &Value,
    headers: &[(&str, &str)],
) -> TestDispatchHttpResponse {
    let bytes = serde_json::to_vec(body).expect("response body serializes");
    let mut builder = Response::builder()
        .status(status)
        .header("content-type", "application/json; charset=utf-8");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = builder
        .body(
            Full::new(Bytes::from(bytes))
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("static JSON response is valid");
    Ok(response)
}

async fn read_body_capped(mut body: Incoming, cap: usize) -> Result<Vec<u8>, String> {
    let mut chunks = Vec::new();
    let mut size = 0usize;
    while let Some(frame) = body.frame().await {
        let Ok(frame) = frame else {
            return Err("assembly activation request body read failed".to_string());
        };
        if let Some(data) = frame.data_ref() {
            size += data.len();
            if size > cap {
                return Err(BODY_TOO_LARGE_MESSAGE.to_string());
            }
            chunks.extend_from_slice(data);
        }
    }
    Ok(chunks)
}

/// TS `classifyActivationError` parity (assemblyControlPlane.ts): substring
/// classification used for the control response status.
fn classify_activation_error(message: &str) -> u16 {
    if message.contains("healthy participant") || message.contains("disconnected") {
        503
    } else if message.contains("timed out") {
        504
    } else if message.contains("invalid") || message.contains("must be") || message.contains("JSON")
    {
        400
    } else {
        409
    }
}
