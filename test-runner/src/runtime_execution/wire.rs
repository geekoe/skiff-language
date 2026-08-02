use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde_json::{Map, Value};
use skiff_artifact_model::{RuntimeAssemblyRef, RuntimeConfigSnapshotRef};
use skiff_deployment::storage::{
    CommittedActivation, EnvironmentActivationState, PendingActivation,
    ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION,
};
use skiff_runtime_model::service_error::{OpaqueServiceError, ServiceErrorEnvelope};

use crate::canonical_fixture::CanonicalFixtureError;

const RUNTIME_FRAME_SCHEMA_VERSION: &str = "skiff-runtime-frame-v3";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TestDispatchOutcome {
    Passed,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ControlErrorResponse {
    pub(super) code: String,
    pub(super) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ActivationReceipt {
    pub(super) environment: String,
    pub(super) generation: u64,
    pub(super) assembly: RuntimeAssemblyRef,
    pub(super) config_snapshot: RuntimeConfigSnapshotRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HealthSnapshot {
    pub(super) active: ActivationReceipt,
    pub(super) pending_activation: bool,
    pub(super) capability_connections: Vec<CapabilityConnection>,
    pub(super) replicas: Vec<ReplicaSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CapabilityConnection {
    pub(super) runtime_id: String,
    pub(super) connected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReplicaSnapshot {
    pub(super) replica_id: String,
    pub(super) environment: String,
    pub(super) generation: u64,
    pub(super) assembly: RuntimeAssemblyRef,
    pub(super) config_snapshot: RuntimeConfigSnapshotRef,
    pub(super) state: ReplicaState,
    pub(super) connected: bool,
    pub(super) connection_pin_count: u64,
    pub(super) connection_release_ack_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReplicaState {
    Healthy,
    Draining,
    Disconnected,
}

pub(super) fn decode_test_dispatch_response(
    body: &str,
) -> Result<TestDispatchOutcome, CanonicalFixtureError> {
    decode_test_dispatch_response_inner(body)
        .map_err(|message| wire_error("runtime test dispatch response", message))
}

pub(super) fn decode_control_error_response(
    body: &str,
) -> Result<ControlErrorResponse, CanonicalFixtureError> {
    decode_control_error_response_inner(body)
        .map_err(|message| wire_error("control error response", message))
}

pub(super) fn decode_activation_receipt(
    body: &str,
) -> Result<ActivationReceipt, CanonicalFixtureError> {
    decode_activation_receipt_inner(body)
        .map_err(|message| wire_error("assembly activation receipt", message))
}

pub(super) fn decode_health_snapshot(body: &str) -> Result<HealthSnapshot, CanonicalFixtureError> {
    decode_health_snapshot_inner(body)
        .map_err(|message| wire_error("router health response", message))
}

fn decode_test_dispatch_response_inner(body: &str) -> Result<TestDispatchOutcome, String> {
    let value = decode_json(body, "runtime test dispatch response")?;
    let root = exact_object(
        &value,
        &["ok", "header", "payloadBase64"],
        &[],
        "runtime test dispatch response",
    )?;
    require_true(root, "ok", "runtime test dispatch response")?;
    let header_value = field(root, "header", "runtime test dispatch response")?;
    let header = header_value
        .as_object()
        .ok_or_else(|| "runtime test dispatch response.header must be an object".to_string())?;
    match string_field(header, "type", "runtime test dispatch response.header")? {
        "response.end" => decode_test_success(root, header_value),
        "response.error" => decode_test_failure(root, header_value),
        _ => Err(
            "runtime test dispatch response.header.type must be response.end or response.error"
                .to_string(),
        ),
    }
}

fn decode_test_success(
    root: &Map<String, Value>,
    header_value: &Value,
) -> Result<TestDispatchOutcome, String> {
    let header = exact_object(
        header_value,
        &[
            "schemaVersion",
            "type",
            "requestId",
            "payloadPresent",
            "httpResponse",
        ],
        &[],
        "runtime test dispatch response.header",
    )?;
    if string_field(
        header,
        "schemaVersion",
        "runtime test dispatch response.header",
    )? != RUNTIME_FRAME_SCHEMA_VERSION
    {
        return Err(format!(
            "header.schemaVersion must be {RUNTIME_FRAME_SCHEMA_VERSION}"
        ));
    }
    if string_field(header, "type", "runtime test dispatch response.header")? != "response.end" {
        return Err("header.type must be response.end".to_string());
    }
    let request_id = string_field(header, "requestId", "runtime test dispatch response.header")?;
    if request_id.is_empty()
        || request_id.trim() != request_id
        || request_id
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err("header.requestId must be a non-empty canonical token".to_string());
    }
    if !bool_field(
        header,
        "payloadPresent",
        "runtime test dispatch response.header",
    )? {
        return Err("header.payloadPresent must be true for the null payload".to_string());
    }
    validate_dispatch_http_response(header)?;
    if string_field(root, "payloadBase64", "runtime test dispatch response")? != "bnVsbA==" {
        return Err(
            "payloadBase64 must be the canonical Base64 encoding of exact null".to_string(),
        );
    }
    Ok(TestDispatchOutcome::Passed)
}

fn decode_test_failure(
    root: &Map<String, Value>,
    header_value: &Value,
) -> Result<TestDispatchOutcome, String> {
    let header = header_value
        .as_object()
        .ok_or_else(|| "runtime test dispatch response.header must be an object".to_string())?;
    let error_kind = string_field(header, "errorKind", "runtime test dispatch response.header")?;
    match error_kind {
        "control" => decode_control_dispatch_failure(root, header_value),
        "fixedService" => decode_fixed_dispatch_failure(root, header_value),
        _ => Err(
            "runtime test dispatch response.header.errorKind must be control or fixedService"
                .to_string(),
        ),
    }
}

fn decode_control_dispatch_failure(
    root: &Map<String, Value>,
    header_value: &Value,
) -> Result<TestDispatchOutcome, String> {
    let header = exact_object(
        header_value,
        &["schemaVersion", "type", "requestId", "errorKind", "error"],
        &[],
        "runtime test dispatch response.header",
    )?;
    validate_error_header_prefix(header)?;
    let error = exact_object(
        field(header, "error", "runtime test dispatch response.header")?,
        &["code", "message"],
        &["status", "details"],
        "runtime test dispatch response.header.error",
    )?;
    let code =
        canonical_non_empty_string(error, "code", "runtime test dispatch response.header.error")?;
    let message = canonical_non_empty_string(
        error,
        "message",
        "runtime test dispatch response.header.error",
    )?;
    validate_optional_error_status(error, "runtime test dispatch response.header.error")?;
    if string_field(root, "payloadBase64", "runtime test dispatch response")? != "" {
        return Err("control response.error payloadBase64 must be empty".to_string());
    }
    Ok(TestDispatchOutcome::Failed(format!("{code}: {message}")))
}

fn decode_fixed_dispatch_failure(
    root: &Map<String, Value>,
    header_value: &Value,
) -> Result<TestDispatchOutcome, String> {
    let header = exact_object(
        header_value,
        &["schemaVersion", "type", "requestId", "errorKind"],
        &[],
        "runtime test dispatch response.header",
    )?;
    validate_error_header_prefix(header)?;
    let encoded = string_field(root, "payloadBase64", "runtime test dispatch response")?;
    if encoded.is_empty() {
        return Err("fixedService response.error payloadBase64 must be non-empty".to_string());
    }
    let bytes = BASE64_STANDARD
        .decode(encoded)
        .map_err(|error| format!("fixedService payloadBase64 is invalid: {error}"))?;
    if BASE64_STANDARD.encode(&bytes) != encoded {
        return Err("fixedService payloadBase64 must be canonical".to_string());
    }
    let error = OpaqueServiceError::decode(bytes)
        .map_err(|error| format!("fixedService payload is invalid: {error}"))?;
    let message = match error.envelope() {
        ServiceErrorEnvelope::PublicTypedError {
            package_id,
            stable_schema_key,
            ..
        } => format!("fixed service error {package_id}::{stable_schema_key}"),
        ServiceErrorEnvelope::InternalError { payload } => payload.message.clone(),
        ServiceErrorEnvelope::PlatformError {
            builtin_error_identity,
            ..
        } => format!("fixed service error {}", builtin_error_identity.symbol()),
    };
    Ok(TestDispatchOutcome::Failed(message))
}

fn validate_error_header_prefix(header: &Map<String, Value>) -> Result<(), String> {
    if string_field(
        header,
        "schemaVersion",
        "runtime test dispatch response.header",
    )? != RUNTIME_FRAME_SCHEMA_VERSION
    {
        return Err(format!(
            "header.schemaVersion must be {RUNTIME_FRAME_SCHEMA_VERSION}"
        ));
    }
    if string_field(header, "type", "runtime test dispatch response.header")? != "response.error" {
        return Err("header.type must be response.error".to_string());
    }
    canonical_non_empty_string(header, "requestId", "runtime test dispatch response.header")?;
    Ok(())
}

fn decode_control_error_response_inner(body: &str) -> Result<ControlErrorResponse, String> {
    let value = decode_json(body, "control error response")?;
    let root = exact_object(&value, &["error"], &[], "control error response")?;
    let error = exact_object(
        field(root, "error", "control error response")?,
        &["code", "message"],
        &["details"],
        "control error response.error",
    )?;
    Ok(ControlErrorResponse {
        code: canonical_non_empty_string(error, "code", "control error response.error")?
            .to_string(),
        message: canonical_non_empty_string(error, "message", "control error response.error")?
            .to_string(),
    })
}

fn canonical_non_empty_string<'a>(
    object: &'a Map<String, Value>,
    name: &str,
    context: &str,
) -> Result<&'a str, String> {
    let value = string_field(object, name, context)?;
    if value.is_empty() || value.trim() != value {
        return Err(format!(
            "{context}.{name} must be a non-empty canonical string"
        ));
    }
    Ok(value)
}

fn validate_optional_error_status(error: &Map<String, Value>, context: &str) -> Result<(), String> {
    if let Some(status) = error.get("status") {
        let status = status
            .as_u64()
            .filter(|status| (400..=599).contains(status))
            .ok_or_else(|| format!("{context}.status must be an integer from 400 through 599"))?;
        u16::try_from(status)
            .map_err(|_| format!("{context}.status must fit the HTTP status range"))?;
    }
    Ok(())
}

fn validate_dispatch_http_response(header: &Map<String, Value>) -> Result<(), String> {
    let http_response = exact_object(
        field(
            header,
            "httpResponse",
            "runtime test dispatch response.header",
        )?,
        &["status", "headers"],
        &[],
        "runtime test dispatch response.header.httpResponse",
    )?;
    if u64_field(
        http_response,
        "status",
        "runtime test dispatch response.header.httpResponse",
    )? != 200
    {
        return Err("inner HTTP response status must be 200".to_string());
    }
    let headers = array(
        field(
            http_response,
            "headers",
            "runtime test dispatch response.header.httpResponse",
        )?,
        "runtime test dispatch response.header.httpResponse.headers",
    )?;
    let [content_type] = headers else {
        return Err(
            "inner HTTP response must have exactly one canonical content-type header".to_string(),
        );
    };
    let content_type = exact_object(
        content_type,
        &["name", "value"],
        &[],
        "runtime test dispatch response.header.httpResponse.headers[0]",
    )?;
    if string_field(
        content_type,
        "name",
        "runtime test dispatch response.header.httpResponse.headers[0]",
    )? != "content-type"
        || string_field(
            content_type,
            "value",
            "runtime test dispatch response.header.httpResponse.headers[0]",
        )? != "application/json; charset=utf-8"
    {
        return Err(
            "inner HTTP response content-type must be application/json; charset=utf-8".to_string(),
        );
    }
    Ok(())
}

fn decode_activation_receipt_inner(body: &str) -> Result<ActivationReceipt, String> {
    let value = decode_json(body, "activation receipt")?;
    let root = exact_object(
        &value,
        &["ok", "committed", "activeAssembly", "replicas"],
        &[],
        "activation receipt",
    )?;
    require_true(root, "ok", "activation receipt")?;
    let committed = decode_committed(field(root, "committed", "activation receipt")?)?;
    let active = decode_active(
        field(root, "activeAssembly", "activation receipt")?,
        false,
        "activation receipt activeAssembly",
    )?;
    decode_replicas(field(root, "replicas", "activation receipt")?)?;
    if committed.generation != active.generation
        || committed.assembly != active.assembly
        || committed.config_snapshot != active.config_snapshot
    {
        return Err("committed and activeAssembly tuples differ".to_string());
    }
    validate_activation_state(&active, None)?;
    Ok(active)
}

fn decode_health_snapshot_inner(body: &str) -> Result<HealthSnapshot, String> {
    let value = decode_json(body, "router health")?;
    let root = exact_object(
        &value,
        &[
            "ok",
            "activeAssembly",
            "pendingActivation",
            "capabilityConnections",
            "replicas",
        ],
        &["counters"],
        "router health",
    )?;
    require_true(root, "ok", "router health")?;
    if let Some(counters) = root.get("counters") {
        decode_counters(counters)?;
    }
    let active = decode_active(
        field(root, "activeAssembly", "router health")?,
        true,
        "router health activeAssembly",
    )?;
    let pending = decode_pending(field(root, "pendingActivation", "router health")?)?;
    validate_activation_state(&active, pending.clone())?;
    Ok(HealthSnapshot {
        active,
        pending_activation: pending.is_some(),
        capability_connections: decode_capability_connections(field(
            root,
            "capabilityConnections",
            "router health",
        )?)?,
        replicas: decode_replicas(field(root, "replicas", "router health")?)?,
    })
}

fn validate_activation_state(
    active: &ActivationReceipt,
    pending: Option<PendingActivation>,
) -> Result<(), String> {
    EnvironmentActivationState {
        schema_version: ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION.to_string(),
        environment: active.environment.clone(),
        committed: CommittedActivation {
            generation: active.generation,
            assembly: active.assembly.clone(),
            config_snapshot: active.config_snapshot.clone(),
        },
        pending,
    }
    .validate()
    .map_err(|error| format!("activation state validation failed: {error}"))
}

struct CommittedCoordinate {
    generation: u64,
    assembly: RuntimeAssemblyRef,
    config_snapshot: RuntimeConfigSnapshotRef,
}

fn decode_committed(value: &Value) -> Result<CommittedCoordinate, String> {
    let committed = exact_object(
        value,
        &["generation", "assembly", "configSnapshot"],
        &[],
        "committed",
    )?;
    Ok(CommittedCoordinate {
        generation: u64_field(committed, "generation", "committed")?,
        assembly: decode_assembly_ref(field(committed, "assembly", "committed")?, "committed")?,
        config_snapshot: decode_config_snapshot_ref(
            field(committed, "configSnapshot", "committed")?,
            "committed",
        )?,
    })
}

fn decode_active(
    value: &Value,
    with_ingress_count: bool,
    context: &str,
) -> Result<ActivationReceipt, String> {
    let required = if with_ingress_count {
        &[
            "environment",
            "generation",
            "assemblyIdentity",
            "configSnapshotId",
            "ingressCount",
        ][..]
    } else {
        &[
            "environment",
            "generation",
            "assemblyIdentity",
            "configSnapshotId",
        ][..]
    };
    let active = exact_object(value, required, &[], context)?;
    if with_ingress_count {
        u64_field(active, "ingressCount", context)?;
    }
    Ok(ActivationReceipt {
        environment: string_field(active, "environment", context)?.to_string(),
        generation: u64_field(active, "generation", context)?,
        assembly: decode_assembly_identity(field(active, "assemblyIdentity", context)?, context)?,
        config_snapshot: decode_config_snapshot_identity(
            field(active, "configSnapshotId", context)?,
            context,
        )?,
    })
}

fn decode_pending(value: &Value) -> Result<Option<PendingActivation>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let pending = exact_object(
        value,
        &[
            "activationId",
            "expectedGeneration",
            "candidateGeneration",
            "assembly",
            "configSnapshot",
            "participantReplicaIds",
        ],
        &[],
        "pendingActivation",
    )?;
    Ok(Some(PendingActivation {
        activation_id: string_field(pending, "activationId", "pendingActivation")?.to_string(),
        expected_generation: u64_field(pending, "expectedGeneration", "pendingActivation")?,
        candidate_generation: u64_field(pending, "candidateGeneration", "pendingActivation")?,
        assembly: decode_assembly_ref(
            field(pending, "assembly", "pendingActivation")?,
            "pendingActivation",
        )?,
        config_snapshot: decode_config_snapshot_ref(
            field(pending, "configSnapshot", "pendingActivation")?,
            "pendingActivation",
        )?,
        participant_replica_ids: string_array_field(
            pending,
            "participantReplicaIds",
            "pendingActivation",
        )?,
    }))
}

fn decode_capability_connections(value: &Value) -> Result<Vec<CapabilityConnection>, String> {
    array(value, "capabilityConnections")?
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let context = format!("capabilityConnections[{index}]");
            let connection = exact_object(
                value,
                &["runtimeId", "connected", "capabilities"],
                &["registeredAt"],
                &context,
            )?;
            if let Some(registered_at) = connection.get("registeredAt") {
                if !registered_at.is_string() {
                    return Err(format!("{context}.registeredAt must be a string"));
                }
            }
            decode_capabilities(field(connection, "capabilities", &context)?, &context)?;
            Ok(CapabilityConnection {
                runtime_id: string_field(connection, "runtimeId", &context)?.to_string(),
                connected: bool_field(connection, "connected", &context)?,
            })
        })
        .collect()
}

fn decode_capabilities(value: &Value, parent: &str) -> Result<(), String> {
    let context = format!("{parent}.capabilities");
    let capabilities = exact_object(
        value,
        &[],
        &[
            "dispatchModes",
            "packageTestDispatch",
            "requestCancel",
            "runtimeProgram",
        ],
        &context,
    )?;
    if let Some(modes) = capabilities.get("dispatchModes") {
        for mode in array(modes, &format!("{context}.dispatchModes"))? {
            match mode.as_str() {
                Some("unary" | "serverStream") => {}
                _ => return Err(format!("{context}.dispatchModes contains an invalid mode")),
            }
        }
    }
    for name in ["packageTestDispatch", "requestCancel", "runtimeProgram"] {
        if capabilities
            .get(name)
            .is_some_and(|value| !value.is_boolean())
        {
            return Err(format!("{context}.{name} must be a boolean"));
        }
    }
    Ok(())
}

fn decode_replicas(value: &Value) -> Result<Vec<ReplicaSnapshot>, String> {
    array(value, "replicas")?
        .iter()
        .enumerate()
        .map(|(index, value)| decode_replica(value, index))
        .collect()
}

fn decode_replica(value: &Value, index: usize) -> Result<ReplicaSnapshot, String> {
    let context = format!("replicas[{index}]");
    let replica = exact_object(
        value,
        &[
            "replicaId",
            "environment",
            "generation",
            "assemblyIdentity",
            "configSnapshotId",
            "state",
            "connected",
            "inFlightCount",
            "connectionPinCount",
            "connectionReleaseAckCount",
        ],
        &["registeredAt", "lastHealthAt", "healthCounters"],
        &context,
    )?;
    u64_field(replica, "inFlightCount", &context)?;
    let connection_pin_count = safe_u64_field(replica, "connectionPinCount", &context)?;
    let connection_release_ack_count =
        safe_u64_field(replica, "connectionReleaseAckCount", &context)?;
    if let Some(registered_at) = replica.get("registeredAt") {
        if !registered_at.is_string() {
            return Err(format!("{context}.registeredAt must be a string"));
        }
    }
    if replica
        .get("lastHealthAt")
        .is_some_and(|value| !value.is_string())
    {
        return Err(format!("{context}.lastHealthAt must be a string"));
    }
    if let Some(counters) = replica.get("healthCounters") {
        decode_health_counters(counters, &context)?;
    }
    let state = match string_field(replica, "state", &context)? {
        "healthy" => ReplicaState::Healthy,
        "draining" => ReplicaState::Draining,
        "disconnected" => ReplicaState::Disconnected,
        _ => return Err(format!("{context}.state is invalid")),
    };
    Ok(ReplicaSnapshot {
        replica_id: string_field(replica, "replicaId", &context)?.to_string(),
        environment: string_field(replica, "environment", &context)?.to_string(),
        generation: u64_field(replica, "generation", &context)?,
        assembly: decode_assembly_identity(
            field(replica, "assemblyIdentity", &context)?,
            &context,
        )?,
        config_snapshot: decode_config_snapshot_identity(
            field(replica, "configSnapshotId", &context)?,
            &context,
        )?,
        state,
        connected: bool_field(replica, "connected", &context)?,
        connection_pin_count,
        connection_release_ack_count,
    })
}

fn decode_health_counters(value: &Value, parent: &str) -> Result<(), String> {
    let context = format!("{parent}.healthCounters");
    let names = [
        "outboundRequestsPending",
        "outboundStreamLeasesActive",
        "streamRuntimeStreamsActive",
        "flagBackedCancelWaitersActive",
        "spawnedTasksActive",
    ];
    let counters = exact_object(value, &names, &[], &context)?;
    for name in names {
        u64_field(counters, name, &context)?;
    }
    Ok(())
}

/// Canonical §10 counting-surface sections (plan §10; batch 12 health leaf).
///
/// The base TS-compatible health projection carries these counters as the
/// optional top-level `counters` object. Every section is required when the
/// object is present so a missing owner surface fails the wire contract.
const HEALTH_COUNTER_SECTIONS: &[&str] = &[
    "activeRoutingEpoch",
    "bootstrap",
    "blockingLoader",
    "sessions",
    "capabilities",
    "health",
    "barrier",
    "admission",
    "requestPending",
    "terminal",
    "clientConnections",
    "generationLeases",
    "broker",
    "actor",
    "activation",
    "http",
    "mailboxes",
    "writerQueues",
    "spawnedTasks",
    "shutdown",
];

fn decode_counters(value: &Value) -> Result<(), String> {
    let counters = exact_object(
        value,
        HEALTH_COUNTER_SECTIONS,
        &[],
        "router health counters",
    )?;
    for section in HEALTH_COUNTER_SECTIONS {
        let section_value = field(counters, section, "router health counters")?;
        if !section_value.is_object() {
            return Err(format!(
                "router health counters.{section} must be an object"
            ));
        }
    }
    let context = |section: &str| format!("router health counters.{section}");

    let epoch = counters["activeRoutingEpoch"]
        .as_object()
        .expect("section object checked above");
    u64_field(epoch, "publishCount", &context("activeRoutingEpoch"))?;
    if !epoch.get("active").is_none_or(Value::is_object) {
        return Err(
            "router health counters.activeRoutingEpoch.active must be an object or null"
                .to_string(),
        );
    }

    let sessions = counters["sessions"]
        .as_object()
        .expect("section object checked above");
    u64_field(sessions, "preAuthConnections", &context("sessions"))?;
    u64_field(sessions, "registeredSessions", &context("sessions"))?;
    u64_field(sessions, "barrierPending", &context("sessions"))?;

    let capabilities = counters["capabilities"]
        .as_object()
        .expect("section object checked above");
    u64_field(capabilities, "connections", &context("capabilities"))?;

    let health = counters["health"]
        .as_object()
        .expect("section object checked above");
    u64_field(health, "observations", &context("health"))?;

    let barrier = counters["barrier"]
        .as_object()
        .expect("section object checked above");
    u64_field(barrier, "pending", &context("barrier"))?;

    let admission = counters["admission"]
        .as_object()
        .expect("section object checked above");
    u64_field(admission, "permitsHeld", &context("admission"))?;

    let request_pending = counters["requestPending"]
        .as_object()
        .expect("section object checked above");
    u64_field(request_pending, "unary", &context("requestPending"))?;
    u64_field(request_pending, "stream", &context("requestPending"))?;
    u64_field(request_pending, "derivedSpawn", &context("requestPending"))?;
    bool_field(request_pending, "stopped", &context("requestPending"))?;

    let terminal = counters["terminal"]
        .as_object()
        .expect("section object checked above");
    field(terminal, "bySource", &context("terminal"))?;

    let client_connections = counters["clientConnections"]
        .as_object()
        .expect("section object checked above");
    u64_field(
        client_connections,
        "connectionCount",
        &context("clientConnections"),
    )?;

    let generation_leases = counters["generationLeases"]
        .as_object()
        .expect("section object checked above");
    u64_field(
        generation_leases,
        "pinsAcquired",
        &context("generationLeases"),
    )?;

    let broker = counters["broker"]
        .as_object()
        .expect("section object checked above");
    u64_field(broker, "outboundPending", &context("broker"))?;
    u64_field(broker, "inboundPending", &context("broker"))?;

    let actor = counters["actor"]
        .as_object()
        .expect("section object checked above");
    for name in [
        "catalog",
        "ownership",
        "activation",
        "invocation",
        "control",
        "lease",
        "spawn",
    ] {
        if !actor.get(name).is_some_and(Value::is_object) {
            return Err(format!(
                "router health counters.actor.{name} must be an object"
            ));
        }
    }

    let activation = counters["activation"]
        .as_object()
        .expect("section object checked above");
    string_field(activation, "phase", &context("activation"))?;
    bool_field(activation, "readiness", &context("activation"))?;
    let repository = activation
        .get("repository")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            "router health counters.activation.repository must be an object".to_string()
        })?;
    let driver = repository
        .get("driver")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            "router health counters.activation.repository.driver must be an object".to_string()
        })?;
    bool_field(driver, "closed", &context("activation.repository.driver"))?;

    let http = counters["http"]
        .as_object()
        .expect("section object checked above");
    u64_field(http, "requests", &context("http"))?;

    let mailboxes = counters["mailboxes"]
        .as_object()
        .expect("section object checked above");
    if !mailboxes.get("coordinator").is_some_and(Value::is_object) {
        return Err("router health counters.mailboxes.coordinator must be an object".to_string());
    }

    let writer_queues = counters["writerQueues"]
        .as_object()
        .expect("section object checked above");
    u64_field(writer_queues, "wsSlowClientCount", &context("writerQueues"))?;

    let spawned_tasks = counters["spawnedTasks"]
        .as_object()
        .expect("section object checked above");
    u64_field(spawned_tasks, "liveSessionTasks", &context("spawnedTasks"))?;

    let shutdown = counters["shutdown"]
        .as_object()
        .expect("section object checked above");
    bool_field(shutdown, "coordinatorShutdown", &context("shutdown"))?;
    bool_field(shutdown, "dispatcherStopped", &context("shutdown"))?;
    Ok(())
}

fn decode_assembly_identity(value: &Value, context: &str) -> Result<RuntimeAssemblyRef, String> {
    let identity = value
        .as_str()
        .ok_or_else(|| format!("{context}.assemblyIdentity must be a string"))?;
    decode_assembly_ref(
        &serde_json::json!({ "assemblyIdentity": identity }),
        context,
    )
}

fn decode_assembly_ref(value: &Value, context: &str) -> Result<RuntimeAssemblyRef, String> {
    serde_json::from_value(value.clone())
        .map_err(|error| format!("{context}.assembly is invalid: {error}"))
}

fn decode_config_snapshot_identity(
    value: &Value,
    context: &str,
) -> Result<RuntimeConfigSnapshotRef, String> {
    let identity = value
        .as_str()
        .ok_or_else(|| format!("{context}.configSnapshotId must be a string"))?;
    decode_config_snapshot_ref(&serde_json::json!({ "snapshotId": identity }), context)
}

fn decode_config_snapshot_ref(
    value: &Value,
    context: &str,
) -> Result<RuntimeConfigSnapshotRef, String> {
    serde_json::from_value(value.clone())
        .map_err(|error| format!("{context}.configSnapshot is invalid: {error}"))
}

fn decode_json(body: &str, context: &str) -> Result<Value, String> {
    serde_json::from_str(body).map_err(|error| format!("{context} is not valid JSON: {error}"))
}

fn exact_object<'a>(
    value: &'a Value,
    required: &[&str],
    optional: &[&str],
    context: &str,
) -> Result<&'a Map<String, Value>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object"))?;
    for name in required {
        if !object.contains_key(*name) {
            return Err(format!("{context} is missing {name}"));
        }
    }
    if let Some(name) = object
        .keys()
        .find(|name| !required.contains(&name.as_str()) && !optional.contains(&name.as_str()))
    {
        return Err(format!("{context} has unexpected field {name}"));
    }
    Ok(object)
}

fn field<'a>(
    object: &'a Map<String, Value>,
    name: &str,
    context: &str,
) -> Result<&'a Value, String> {
    object
        .get(name)
        .ok_or_else(|| format!("{context} is missing {name}"))
}

fn string_field<'a>(
    object: &'a Map<String, Value>,
    name: &str,
    context: &str,
) -> Result<&'a str, String> {
    field(object, name, context)?
        .as_str()
        .ok_or_else(|| format!("{context}.{name} must be a string"))
}

fn string_array_field(
    object: &Map<String, Value>,
    name: &str,
    context: &str,
) -> Result<Vec<String>, String> {
    array(field(object, name, context)?, &format!("{context}.{name}"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToString::to_string)
                .ok_or_else(|| format!("{context}.{name} must contain only strings"))
        })
        .collect()
}

fn bool_field(object: &Map<String, Value>, name: &str, context: &str) -> Result<bool, String> {
    field(object, name, context)?
        .as_bool()
        .ok_or_else(|| format!("{context}.{name} must be a boolean"))
}

fn u64_field(object: &Map<String, Value>, name: &str, context: &str) -> Result<u64, String> {
    field(object, name, context)?
        .as_u64()
        .ok_or_else(|| format!("{context}.{name} must be a canonical unsigned integer"))
}

fn safe_u64_field(object: &Map<String, Value>, name: &str, context: &str) -> Result<u64, String> {
    const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

    let value = u64_field(object, name, context)?;
    if value > MAX_SAFE_INTEGER {
        return Err(format!(
            "{context}.{name} must be a non-negative safe integer"
        ));
    }
    Ok(value)
}

fn array<'a>(value: &'a Value, context: &str) -> Result<&'a [Value], String> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{context} must be an array"))
}

fn require_true(object: &Map<String, Value>, name: &str, context: &str) -> Result<(), String> {
    if bool_field(object, name, context)? {
        Ok(())
    } else {
        Err(format!("{context}.{name} must be true"))
    }
}

fn wire_error(context: impl Into<String>, message: impl Into<String>) -> CanonicalFixtureError {
    CanonicalFixtureError::Wire {
        context: context.into(),
        message: message.into(),
    }
}

#[cfg(test)]
#[path = "tests/wire.rs"]
mod tests;
