use serde_json::Value;

pub(super) const ENVIRONMENT: &str = "package-tests";
pub(super) const ASSEMBLY_A: &str = concat!(
    "skiff-runtime-assembly-v2:sha256:",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
);
pub(super) const ASSEMBLY_B: &str = concat!(
    "skiff-runtime-assembly-v2:sha256:",
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
);
pub(super) const REPLICA: &str = "runtime-two";

pub(super) fn activation_receipt_body() -> String {
    serde_json::json!({
        "ok": true,
        "committed": {
            "generation": 2,
            "assembly": { "assemblyIdentity": ASSEMBLY_B },
        },
        "activeAssembly": {
            "environment": ENVIRONMENT,
            "generation": 2,
            "assemblyIdentity": ASSEMBLY_B,
        },
        "replicas": [replica(1, ASSEMBLY_A, "draining", true)],
    })
    .to_string()
}

pub(super) fn health_body(
    environment: &str,
    generation: u64,
    assembly_identity: &str,
    pending: Value,
    replicas: Vec<Value>,
    capabilities: Vec<Value>,
) -> String {
    serde_json::json!({
        "ok": true,
        "activeAssembly": {
            "environment": environment,
            "generation": generation,
            "assemblyIdentity": assembly_identity,
            "ingressCount": 1,
        },
        "pendingActivation": pending,
        "capabilityConnections": capabilities,
        "replicas": replicas,
    })
    .to_string()
}

pub(super) fn valid_pending() -> Value {
    pending(
        Value::String("activation-three".to_string()),
        serde_json::json!(2),
        serde_json::json!(3),
        ASSEMBLY_A,
        serde_json::json!(["runtime-a", "runtime-b"]),
    )
}

pub(super) fn pending(
    activation_id: Value,
    expected_generation: Value,
    candidate_generation: Value,
    assembly_identity: &str,
    participants: Value,
) -> Value {
    serde_json::json!({
        "activationId": activation_id,
        "expectedGeneration": expected_generation,
        "candidateGeneration": candidate_generation,
        "assembly": { "assemblyIdentity": assembly_identity },
        "participantReplicaIds": participants,
    })
}

pub(super) fn replica(
    generation: u64,
    assembly_identity: &str,
    state: &str,
    connected: bool,
) -> Value {
    serde_json::json!({
        "replicaId": REPLICA,
        "environment": ENVIRONMENT,
        "generation": generation,
        "assemblyIdentity": assembly_identity,
        "state": state,
        "connected": connected,
        "inFlightCount": 0,
        "connectionPinCount": 0,
        "connectionReleaseAckCount": 0,
        "registeredAt": "2026-07-22T00:00:00.000Z",
    })
}

pub(super) fn capability(runtime_id: &str, connected: bool) -> Value {
    serde_json::json!({
        "runtimeId": runtime_id,
        "connected": connected,
        "registeredAt": "2026-07-22T00:00:00.000Z",
        "capabilities": {
            "dispatchModes": ["unary"],
            "runtimeProgram": true,
        },
    })
}
