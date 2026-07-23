use serde_json::{Map, Value};
use skiff_artifact_model::RuntimeAssemblyRef;
use skiff_deployment::storage::{
    CommittedActivation, EnvironmentActivationState, PendingActivation,
    ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION,
};

use crate::canonical_fixture::CanonicalFixtureError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ActivationReceipt {
    pub(super) environment: String,
    pub(super) generation: u64,
    pub(super) assembly: RuntimeAssemblyRef,
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

pub(super) fn decode_activation_receipt(
    body: &str,
) -> Result<ActivationReceipt, CanonicalFixtureError> {
    decode_activation_receipt_inner(body)
        .map_err(|message| wire_error(format!("invalid assembly activation receipt: {message}")))
}

pub(super) fn decode_health_snapshot(body: &str) -> Result<HealthSnapshot, CanonicalFixtureError> {
    decode_health_snapshot_inner(body)
        .map_err(|message| wire_error(format!("invalid router health response: {message}")))
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
    if committed.generation != active.generation || committed.assembly != active.assembly {
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
        &[],
        "router health",
    )?;
    require_true(root, "ok", "router health")?;
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
        },
        pending,
    }
    .validate()
    .map_err(|error| format!("activation state validation failed: {error}"))
}

struct CommittedCoordinate {
    generation: u64,
    assembly: RuntimeAssemblyRef,
}

fn decode_committed(value: &Value) -> Result<CommittedCoordinate, String> {
    let committed = exact_object(value, &["generation", "assembly"], &[], "committed")?;
    Ok(CommittedCoordinate {
        generation: u64_field(committed, "generation", "committed")?,
        assembly: decode_assembly_ref(field(committed, "assembly", "committed")?, "committed")?,
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
            "ingressCount",
        ][..]
    } else {
        &["environment", "generation", "assemblyIdentity"][..]
    };
    let active = exact_object(value, required, &[], context)?;
    if with_ingress_count {
        u64_field(active, "ingressCount", context)?;
    }
    Ok(ActivationReceipt {
        environment: string_field(active, "environment", context)?.to_string(),
        generation: u64_field(active, "generation", context)?,
        assembly: decode_assembly_identity(field(active, "assemblyIdentity", context)?, context)?,
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
                &["runtimeId", "connected", "registeredAt", "capabilities"],
                &[],
                &context,
            )?;
            string_field(connection, "registeredAt", &context)?;
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
            "state",
            "connected",
            "inFlightCount",
            "connectionPinCount",
            "connectionReleaseAckCount",
            "registeredAt",
        ],
        &["lastHealthAt", "healthCounters"],
        &context,
    )?;
    u64_field(replica, "inFlightCount", &context)?;
    let connection_pin_count = safe_u64_field(replica, "connectionPinCount", &context)?;
    let connection_release_ack_count =
        safe_u64_field(replica, "connectionReleaseAckCount", &context)?;
    string_field(replica, "registeredAt", &context)?;
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

fn wire_error(message: impl Into<String>) -> CanonicalFixtureError {
    CanonicalFixtureError::InvalidInput(format!("runtime readiness failed: {}", message.into()))
}

#[cfg(test)]
#[path = "tests/wire.rs"]
mod tests;
