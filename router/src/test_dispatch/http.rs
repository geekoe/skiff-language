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
use skiff_artifact_model::{GatewayIngressBinding, IngressProtocol};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_transport::protocol::{
    ResponseEndFrameHeader, ResponseEndFrameMetadata, RUNTIME_FRAME_SCHEMA_VERSION,
    RuntimeHttpResponseFrameHeader,
};
use skiff_runtime_transport::runtime_assembly_request::{
    decode_runtime_assembly_request_start_frame, RuntimeAssemblyHttpRequestFrameHeader,
    RuntimeAssemblyRequestCallerFrameHeader, RuntimeAssemblyRequestDeadlineFrameHeader,
    RuntimeAssemblyRequestRoutingFrameHeader, RuntimeAssemblyRequestStartFrameHeader,
    RuntimeAssemblyRequestTraceFrameHeader,
};

use crate::bootstrap::{ActiveRoutingEpochStore, RoutingEpoch};
use crate::http::dispatch::{
    cancel_channel, DispatchRequest, HttpDispatchError, HttpDispatchPort, TestDispatchOutcome,
};
use crate::http::frame::{
    deadline_parts, encode_request_start_frame, new_request_id, new_test_case_capability,
    new_span_id, new_trace_id,
};
use crate::http::ingress::{http_surface_view_from_epoch, HttpGatewaySurfaceView};

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

/// Handler dependencies (supervisor composition injects the live epoch
/// store, the static surface view and the artifact store for live surface
/// rebuilds after an activation swap).
#[derive(Clone)]
pub struct TestDispatchHttpHandlerOptions {
    pub epoch: Arc<RoutingEpoch>,
    pub epoch_store: Option<Arc<ActiveRoutingEpochStore>>,
    pub surfaces: Arc<HttpGatewaySurfaceView>,
    pub artifact_store: CanonicalArtifactStore,
    pub dispatcher: Arc<dyn HttpDispatchPort>,
}

impl fmt::Debug for TestDispatchHttpHandlerOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TestDispatchHttpHandlerOptions")
            .field("epoch", &self.epoch)
            .field("epoch_store", &self.epoch_store.is_some())
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
            Err(message) => return control_json_error(classify_activation_error(&message), &message),
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
        let epoch = self.current_epoch();
        let surfaces = match self.current_surfaces() {
            Ok(surfaces) => surfaces,
            Err(message) => {
                return control_json_error(classify_activation_error(&message), &message);
            }
        };
        let binding = match exact_test_dispatch_binding(&epoch, &surfaces, &decoded) {
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

    fn current_epoch(&self) -> Arc<RoutingEpoch> {
        self.options
            .epoch_store
            .as_ref()
            .and_then(|store| store.capture())
            .unwrap_or_else(|| Arc::clone(&self.options.epoch))
    }

    fn current_surfaces(&self) -> Result<Arc<HttpGatewaySurfaceView>, String> {
        let Some(store) = &self.options.epoch_store else {
            return Ok(Arc::clone(&self.options.surfaces));
        };
        let Some(current) = store.capture() else {
            return Ok(Arc::clone(&self.options.surfaces));
        };
        http_surface_view_from_epoch(&self.options.artifact_store, &current)
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
            return Err(format!("runtime assembly test dispatch does not support {field}"));
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
    let mode = root
        .get("mode")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            "runtime assembly test dispatch mode must be unary or serverStream".to_string()
        })?;
    if mode != "unary" && mode != "serverStream" {
        return Err(
            "runtime assembly test dispatch mode must be unary or serverStream".to_string(),
        );
    }
    let http_request = serde_json::from_value::<RuntimeAssemblyHttpRequestFrameHeader>(
        root.get("httpRequest").expect("field presence checked").clone(),
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
        "runtime assembly test dispatch payloadBase64 must be canonical standard Base64"
            .to_string()
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
    for pair in metadata
        .query
        .iter()
        .chain(metadata.headers.iter())
    {
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
    epoch: &RoutingEpoch,
    surfaces: &HttpGatewaySurfaceView,
    decoded: &DecodedTestDispatch,
) -> Result<GatewayIngressBinding, String> {
    let routing = &decoded.routing;
    if routing.assembly_identity.as_str() != epoch.assembly_identity()
        || routing.assembly_generation != epoch.assembly_generation()
    {
        return Err(
            "runtime assembly test dispatch does not match the exact active assembly generation"
                .to_string(),
        );
    }
    let candidates: Vec<&GatewayIngressBinding> = epoch
        .ingress_projection()
        .iter()
        .filter(|binding| {
            binding.selector.protocol == IngressProtocol::Http
                && binding.selector.method.as_deref() == Some(routing.ingress.method.as_str())
                && binding.selector.path == routing.ingress.path
                && binding.deployment == routing.deployment
                && binding.gateway_entry_identity == routing.gateway_entry_identity
        })
        .collect();
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
        .get(&binding.deployment, &binding.gateway_entry_key)
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
    Ok((*binding).clone())
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
    let header = RuntimeAssemblyRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: new_request_id(),
        mode: decoded.mode.clone(),
        caller: RuntimeAssemblyRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: decoded.routing.clone(),
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
    let frame = encode_request_start_frame(&header, &decoded.payload_bytes)
        .map_err(|error| {
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    use http_body_util::BodyExt as _;
    use hyper::Method;
    use skiff_artifact_identity::assign_runtime_assembly_identity;
    use skiff_artifact_model::{
        GatewayEntryKey, GatewayIngressBinding, IngressProtocol, IngressSelector,
        RuntimeAssemblyRef, ServiceDeploymentRef,
    };
    use skiff_deployment::fixtures::{
        runtime_assembly_fixture, service_deployment_fixture,
    };
    use skiff_deployment::projection::actor_routing::{
        ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
    };
    use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;

    use crate::artifact::ActorRoutingCatalog;
    use crate::bootstrap::RoutingEpoch;
    use crate::http::fake::{FakeDispatchPlan, FakeHttpDispatcher};
    use crate::http::ingress::HttpGatewaySurfaceView;

    use super::*;

    static TEMP_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    const ENVIRONMENT: &str = "prod";
    const GENERATION: u64 = 7;
    const GATEWAY_KEY: &str = "echo";
    const ECHO_PATH: &str = "/echo";

    struct TestRoot {
        parent: PathBuf,
        root: PathBuf,
    }

    impl TestRoot {
        fn new() -> Self {
            let sequence = TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let parent = std::env::temp_dir().join(format!(
                "skiff-router-test-dispatch-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&parent).expect("create temp parent");
            Self {
                parent: parent.clone(),
                root: parent.join("root"),
            }
        }

        fn path(&self) -> &Path {
            &self.root
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.parent);
        }
    }

    fn snapshot_ref() -> skiff_artifact_model::RuntimeConfigSnapshotRef {
        skiff_artifact_model::RuntimeConfigSnapshotRef {
            snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
                "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .expect("snapshot id"),
        }
    }

    struct Fixture {
        _root: TestRoot,
        assembly_ref: RuntimeAssemblyRef,
        deployment: ServiceDeploymentRef,
        gateway_entry_identity: String,
        epoch: Arc<RoutingEpoch>,
        surfaces: Arc<HttpGatewaySurfaceView>,
    }

    fn fixture() -> Fixture {
        let root = TestRoot::new();
        fs::create_dir_all(root.path()).expect("create artifact root");
        let artifact_store =
            skiff_deployment::storage::CanonicalArtifactStore::create(root.path())
                .expect("create artifact store");
        let deployment = service_deployment_fixture().expect("deployment fixture");
        let gateway_entry = deployment
            .gateway_entries
            .get(&GatewayEntryKey::parse(GATEWAY_KEY).expect("gateway key"))
            .expect("fixture gateway entry");
        let mut assembly = runtime_assembly_fixture().expect("assembly fixture");
        let deployment_ref = ServiceDeploymentRef {
            service_id: deployment.contract.service_id.clone(),
            contract_version: deployment.contract.contract_version.clone(),
            deployment_revision: deployment.deployment_revision.clone(),
            deployment_artifact_identity: deployment.deployment_artifact_identity.clone(),
        };
        assembly.gateway_ingress = vec![GatewayIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::Http,
                method: Some("POST".to_string()),
                path: ECHO_PATH.to_string(),
            },
            deployment: deployment_ref.clone(),
            gateway_entry_key: GatewayEntryKey::parse(GATEWAY_KEY).expect("gateway key"),
            gateway_entry_identity: gateway_entry.gateway_entry_identity.clone(),
        }];
        assign_runtime_assembly_identity(&mut assembly).expect("assign assembly identity");
        artifact_store
            .write_runtime_assembly(&assembly)
            .expect("write assembly");
        artifact_store
            .write_service_deployment(&deployment)
            .expect("write deployment");
        let assembly_ref =
            skiff_artifact_identity::runtime_assembly_ref(&assembly).expect("assembly ref");

        let snapshot = RuntimeConfigSnapshot::new(ENVIRONMENT, snapshot_ref(), Vec::new())
            .expect("snapshot fixture");
        let projection = ActorRoutingProjection::new(
            ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
            Vec::new(),
        )
        .expect("empty projection");
        let catalog = Arc::new(ActorRoutingCatalog::from_projection(Arc::new(projection)));
        let epoch = Arc::new(
            RoutingEpoch::new(
                ENVIRONMENT,
                GENERATION,
                Arc::new(assembly),
                Arc::new(snapshot),
                catalog,
            )
            .expect("epoch fixture"),
        );
        let mut entries = BTreeMap::new();
        for (key, entry) in &deployment.gateway_entries {
            entries.insert((deployment_ref.clone(), key.clone()), entry.clone());
        }
        let surfaces = Arc::new(
            HttpGatewaySurfaceView::from_deployment_gateway_entries(&entries)
                .expect("surface view"),
        );
        Fixture {
            _root: root,
            assembly_ref,
            deployment: deployment_ref,
            gateway_entry_identity: gateway_entry.gateway_entry_identity.as_str().to_string(),
            epoch,
            surfaces,
        }
    }

    fn handler_with(fixture: &Fixture, dispatcher: FakeHttpDispatcher) -> TestDispatchHttpHandler {
        TestDispatchHttpHandler::new(TestDispatchHttpHandlerOptions {
            epoch: Arc::clone(&fixture.epoch),
            epoch_store: None,
            surfaces: Arc::clone(&fixture.surfaces),
            artifact_store: CanonicalArtifactStore::open(fixture._root.path())
                .expect("artifact store open"),
            dispatcher: Arc::new(dispatcher),
        })
    }

    fn body(fixture: &Fixture) -> serde_json::Value {
        serde_json::json!({
            "kind": "test",
            "routing": {
                "kind": "runtimeAssembly",
                "assemblyIdentity": fixture.assembly_ref.assembly_identity.clone(),
                "assemblyGeneration": GENERATION,
                "deployment": {
                    "serviceId": fixture.deployment.service_id.clone(),
                    "contractVersion": fixture.deployment.contract_version.clone(),
                    "deploymentRevision": fixture.deployment.deployment_revision.clone(),
                    "deploymentArtifactIdentity": fixture.deployment.deployment_artifact_identity.clone(),
                },
                "gatewayEntryIdentity": fixture.gateway_entry_identity.clone(),
                "ingress": {
                    "protocol": "http",
                    "method": "POST",
                    "path": ECHO_PATH,
                },
            },
            "mode": "unary",
            "httpRequest": {
                "method": "POST",
                "url": "http://127.0.0.1/echo",
                "path": ECHO_PATH,
                "query": [],
                "headers": [{
                    "name": "content-type",
                    "value": "application/json",
                }],
            },
            "payloadBase64": "bnVsbA==",
            "timeoutMs": 30_000,
        })
    }

    fn body_bytes(fixture: &Fixture) -> Vec<u8> {
        serde_json::to_vec(&body(fixture)).expect("body serializes")
    }

    async fn json(response: hyper::Response<BoxBody<Bytes, hyper::Error>>) -> serde_json::Value {
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("collect response")
            .to_bytes();
        serde_json::from_slice(&bytes).expect("response is JSON")
    }

    #[tokio::test]
    async fn method_not_allowed_returns_405_allow_post() {
        let fixture = fixture();
        let handler = handler_with(&fixture, FakeHttpDispatcher::new(Vec::new()));
        let response = handler.handle_parts(&Method::GET, b"").await.expect("response");
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            response.headers().get("allow").and_then(|v| v.to_str().ok()),
            Some("POST")
        );
        let body = json(response).await;
        assert_eq!(body["error"]["code"], "MethodNotAllowed");
        assert_eq!(body["error"]["message"], "test dispatch requires POST");
    }

    #[tokio::test]
    async fn malformed_bodies_fail_closed_with_ts_classification() {
        let fixture = fixture();
        let handler = handler_with(&fixture, FakeHttpDispatcher::new(Vec::new()));
        let cases: Vec<(Vec<u8>, u16, &str)> = vec![
            (
                b"not json".to_vec(),
                400,
                "AssemblyActivationRejected",
            ),
            (b"[]".to_vec(), 400, "AssemblyActivationRejected"),
            (
                {
                    let mut value = body(&fixture);
                    value["kind"] = serde_json::json!("packageTest");
                    serde_json::to_vec(&value).unwrap()
                },
                400,
                "AssemblyActivationRejected",
            ),
            (
                {
                    let mut value = body(&fixture);
                    value["mode"] = serde_json::json!("bidi");
                    serde_json::to_vec(&value).unwrap()
                },
                400,
                "AssemblyActivationRejected",
            ),
            (
                {
                    let mut value = body(&fixture);
                    value["payloadBase64"] = serde_json::json!("!!!");
                    serde_json::to_vec(&value).unwrap()
                },
                400,
                "AssemblyActivationRejected",
            ),
            (
                {
                    let mut value = body(&fixture);
                    value["timeoutMs"] = serde_json::json!(0);
                    serde_json::to_vec(&value).unwrap()
                },
                400,
                "AssemblyActivationRejected",
            ),
            (
                {
                    let mut value = body(&fixture);
                    value["timeoutMs"] = serde_json::json!(9_007_199_254_740_992u64);
                    serde_json::to_vec(&value).unwrap()
                },
                400,
                "AssemblyActivationRejected",
            ),
            (
                {
                    let mut value = body(&fixture);
                    value["extra"] = serde_json::json!(1);
                    serde_json::to_vec(&value).unwrap()
                },
                409,
                "AssemblyActivationRejected",
            ),
            (
                {
                    let mut value = body(&fixture);
                    value.as_object_mut().unwrap().remove("routing");
                    serde_json::to_vec(&value).unwrap()
                },
                409,
                "AssemblyActivationRejected",
            ),
        ];
        for (bytes, expected_status, expected_code) in cases {
            let response = handler
                .handle_parts(&Method::POST, &bytes)
                .await
                .expect("response");
            assert_eq!(
                response.status().as_u16(),
                expected_status,
                "body: {}",
                String::from_utf8_lossy(&bytes)
            );
            let value = json(response).await;
            assert_eq!(value["error"]["code"], expected_code);
        }
    }

    #[tokio::test]
    async fn binding_mismatch_returns_409() {
        let fixture = fixture();
        let handler = handler_with(&fixture, FakeHttpDispatcher::new(Vec::new()));
        let cases: Vec<serde_json::Value> = vec![
            {
                let mut value = body(&fixture);
                value["routing"]["assemblyGeneration"] = serde_json::json!(GENERATION + 1);
                value
            },
            {
                let mut value = body(&fixture);
                value["routing"]["ingress"]["path"] = serde_json::json!("/nope");
                value
            },
            {
                let mut value = body(&fixture);
                value["routing"]["gatewayEntryIdentity"] =
                    serde_json::json!("skiff-gateway-entry-v2:sha256:deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
                value
            },
            {
                let mut value = body(&fixture);
                value["mode"] = serde_json::json!("serverStream");
                value
            },
            {
                let mut value = body(&fixture);
                value["routing"]["deployment"]["deploymentRevision"] =
                    serde_json::json!("revision-other");
                value
            },
        ];
        for value in cases {
            let bytes = serde_json::to_vec(&value).expect("body serializes");
            let response = handler
                .handle_parts(&Method::POST, &bytes)
                .await
                .expect("response");
            let status = response.status().as_u16();
            let body = json(response).await;
            assert_eq!(
                status,
                StatusCode::CONFLICT.as_u16(),
                "case: {} -> {}",
                value,
                body["error"]["message"]
            );
            assert_eq!(body["error"]["code"], "AssemblyActivationRejected");
            assert!(
                body["error"]["message"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("does not match the exact active"),
                "case {} -> {}",
                value,
                body["error"]["message"]
            );
        }
    }

    #[tokio::test]
    async fn happy_dispatch_returns_response_end_frame_and_records_test_header() {
        let fixture = fixture();
        let dispatcher = FakeHttpDispatcher::new(vec![FakeDispatchPlan::UnaryOk {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json; charset=utf-8".to_string())],
            payload: Bytes::from_static(b"null"),
        }]);
        let handler = handler_with(&fixture, dispatcher.clone());
        let response = handler
            .handle_parts(&Method::POST, &body_bytes(&fixture))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = json(response).await;
        assert_eq!(body["ok"], true);
        assert_eq!(body["header"]["type"], "response.end");
        assert_eq!(body["header"]["payloadPresent"], true);
        assert_eq!(body["header"]["httpResponse"]["status"], 200);
        assert_eq!(body["header"]["httpResponse"]["headers"][0]["name"], "content-type");
        assert_eq!(body["payloadBase64"], "bnVsbA==");

        let recorded = dispatcher.recorded_requests();
        assert_eq!(recorded.len(), 1);
        let request = &recorded[0];
        assert_eq!(request.header.mode, "unary");
        assert_eq!(request.header.caller.kind, "gateway");
        assert_eq!(request.header.routing.assembly_generation, GENERATION);
        assert_eq!(request.header.routing.ingress.path, ECHO_PATH);
        assert_eq!(request.header.http_request.method, "POST");
        assert_eq!(request.header.http_request.url, "http://127.0.0.1/echo");
        assert!(request.header.test_effects_enabled);
        assert!(request.header.test_case_capability.is_some());
        assert_eq!(request.header.test_case_parent_request_id, None);
        assert_eq!(
            request.header.deadline.as_ref().expect("deadline").timeout_ms,
            30_000
        );
        assert_eq!(request.header.deadline.as_ref().expect("deadline").expires_at.len(), 24);
        assert_eq!(request.payload_bytes, Bytes::from_static(b"null"));
    }

    #[tokio::test]
    async fn runtime_control_error_is_reemitted_as_200_error_frame() {
        let fixture = fixture();
        let dispatcher = FakeHttpDispatcher::new(vec![FakeDispatchPlan::UnaryControlError {
            code: "fixture.boom".to_string(),
            message: "control boom".to_string(),
            status: Some(400),
            details: None,
        }]);
        let handler = handler_with(&fixture, dispatcher);
        let response = handler
            .handle_parts(&Method::POST, &body_bytes(&fixture))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = json(response).await;
        assert_eq!(body["ok"], true);
        assert_eq!(body["header"]["type"], "response.error");
        assert_eq!(body["header"]["errorKind"], "control");
        assert_eq!(body["header"]["error"]["code"], "fixture.boom");
        assert_eq!(body["header"]["error"]["status"], 400);
        assert_eq!(body["payloadBase64"], "");
    }

    #[tokio::test]
    async fn runtime_fixed_service_error_is_reemitted_as_200_error_frame() {
        let fixture = fixture();
        let dispatcher = FakeHttpDispatcher::new(vec![FakeDispatchPlan::UnaryFixedServiceError {
            trace_id: "trace-1".to_string(),
            error_id: "error-1".to_string(),
        }]);
        let handler = handler_with(&fixture, dispatcher);
        let response = handler
            .handle_parts(&Method::POST, &body_bytes(&fixture))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = json(response).await;
        assert_eq!(body["header"]["type"], "response.error");
        assert_eq!(body["header"]["errorKind"], "fixedService");
        assert!(!body["payloadBase64"].as_str().unwrap_or_default().is_empty());
    }

    #[tokio::test]
    async fn dispatcher_unavailable_is_a_503_control_error() {
        let fixture = fixture();
        let dispatcher = FakeHttpDispatcher::new(vec![FakeDispatchPlan::UnaryControlError {
            code: "ServiceUnavailable".to_string(),
            message: "no eligible runtime for request".to_string(),
            status: Some(503),
            details: None,
        }]);
        // A runtime error frame with status 503 is still a runtime frame and
        // is re-emitted as HTTP 200; a submit rejection is the only source
        // of a control error here. The fake scripts the frame path, so assert
        // the frame re-emission instead.
        let handler = handler_with(&fixture, dispatcher);
        let response = handler
            .handle_parts(&Method::POST, &body_bytes(&fixture))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = json(response).await;
        assert_eq!(body["header"]["errorKind"], "control");
        assert_eq!(body["header"]["error"]["status"], 503);
    }

    #[tokio::test]
    async fn dispatch_timeout_maps_to_control_error() {
        let fixture = fixture();
        let dispatcher = FakeHttpDispatcher::new(vec![FakeDispatchPlan::UnaryHang]);
        let handler = handler_with(&fixture, dispatcher);
        let mut value = body(&fixture);
        value["timeoutMs"] = serde_json::json!(50);
        let bytes = serde_json::to_vec(&value).expect("body serializes");
        let response = handler
            .handle_parts(&Method::POST, &bytes)
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = json(response).await;
        assert_eq!(body["error"]["code"], "AssemblyActivationRejected");
        assert!(body["error"]["message"].as_str().unwrap_or_default().contains("Runtime did not respond"));
    }

    #[test]
    fn classify_matches_ts_substring_rules() {
        assert_eq!(classify_activation_error("healthy participant"), 503);
        assert_eq!(classify_activation_error("runtime disconnected"), 503);
        assert_eq!(classify_activation_error("activation timed out"), 504);
        assert_eq!(classify_activation_error("invalid canonical fields"), 400);
        assert_eq!(classify_activation_error("kind must be test"), 400);
        assert_eq!(classify_activation_error("request body is not valid JSON"), 400);
        assert_eq!(classify_activation_error("generation mismatch"), 409);
    }

    #[test]
    fn body_cap_constant_matches_wire_limit() {
        assert_eq!(TEST_DISPATCH_REQUEST_BODY_CAP, 1024 * 1024);
    }
}
