use serde_json::Value;
use skiff_artifact_model::RuntimeConfigSnapshotRef;

pub(super) const ENVIRONMENT: &str = "package-tests";
pub(super) const ASSEMBLY_A: &str = concat!(
    "skiff-runtime-assembly-v3:sha256:",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
);
pub(super) const ASSEMBLY_B: &str = concat!(
    "skiff-runtime-assembly-v3:sha256:",
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
);
pub(super) const SNAPSHOT_A: &str =
    "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub(super) const SNAPSHOT_B: &str =
    "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub(super) const REPLICA: &str = "runtime-two";

pub(super) fn snapshot_ref(snapshot_id: &str) -> RuntimeConfigSnapshotRef {
    serde_json::from_value(serde_json::json!({ "snapshotId": snapshot_id }))
        .expect("canonical test config snapshot ref")
}

pub(super) fn activation_receipt_body() -> String {
    serde_json::json!({
        "ok": true,
        "committed": {
            "generation": 2,
            "assembly": { "assemblyIdentity": ASSEMBLY_B },
            "configSnapshot": { "snapshotId": SNAPSHOT_B },
        },
        "activeAssembly": {
            "environment": ENVIRONMENT,
            "generation": 2,
            "assemblyIdentity": ASSEMBLY_B,
            "configSnapshotId": SNAPSHOT_B,
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
            "configSnapshotId": snapshot_for_assembly(assembly_identity),
            "ingressCount": 1,
        },
        "pendingActivation": pending,
        "capabilityConnections": capabilities,
        "replicas": replicas,
        "counters": counters(),
    })
    .to_string()
}

/// Canonical zero §10 counters fixture (batch 12 health leaf contract).
pub(super) fn counters() -> Value {
    let actor_activation = serde_json::json!({
        "pendingClaims": 0,
        "pendingWaiters": 0,
        "dedupJoins": 0,
        "lineageConflicts": 0,
        "commits": 0,
        "aborts": 0,
        "timeouts": 0,
        "lateAcks": 0,
        "wrongCorrelation": 0,
        "saturated": 0,
        "tombstones": 0,
    });
    let actor_invocation = serde_json::json!({
        "pending": 0,
        "capacity": 0,
        "settled": 0,
        "rejected": 0,
        "terminals": 0,
        "deadlineCancels": 0,
        "saturated": 0,
        "tombstones": 0,
    });
    let actor_control = serde_json::json!({
        "pending": 0,
        "accepted": 0,
        "rejected": 0,
        "lateAcks": 0,
        "timeouts": 0,
        "wrongCorrelation": 0,
        "disconnects": 0,
        "saturated": 0,
        "tombstones": 0,
    });
    let actor = serde_json::json!({
        "catalog": { "captures": 0, "hits": 0, "misses": 0 },
        "ownership": {
            "currentFences": 0,
            "inFlightReservations": 0,
            "commits": 0,
            "aborts": 0,
            "conflicts": 0,
            "renewals": 0,
            "releases": 0,
            "expired": 0,
            "epochMismatches": 0,
            "rejectedCommits": 0,
            "rejectedAborts": 0,
        },
        "activation": actor_activation,
        "invocation": actor_invocation,
        "control": actor_control,
        "lease": {
            "sweepCount": 0,
            "expired": 0,
            "idleCandidates": 0,
            "evictionPending": 0,
            "evictionAcked": 0,
            "evictionRetries": 0,
            "evictionExhausted": 0,
        },
    });
    let activation = serde_json::json!({
        "phase": "idle",
        "environment": null,
        "activationId": null,
        "expectedGeneration": null,
        "candidateGeneration": null,
        "participantBindings": 0,
        "preparedAcks": 0,
        "rejectAcks": 0,
        "staleAcks": 0,
        "sessionAborts": 0,
        "decision": "idle",
        "recoveryActive": false,
        "reboundParticipants": 0,
        "waitingReplicas": [],
        "readiness": false,
        "mailboxOccupancy": 0,
        "mailboxCapacity": 0,
        "mailboxSaturation": 0,
        "shutdown": false,
        "lastFailure": null,
        "repository": {
            "environment": null,
            "committedGeneration": null,
            "pendingActivationId": null,
            "lastOutcome": null,
            "lastOutcomeOperation": null,
            "retry": {
                "attempts": 0,
                "retried": 0,
                "nextBackoffMs": 0,
                "deadlineRemainingMs": null,
            },
            "audit": {
                "lastEventId": null,
                "lastEventOperation": null,
                "lastEventTimestamp": null,
                "failedWrites": 0,
            },
            "driver": {
                "connected": false,
                "reconnecting": false,
                "closed": false,
                "shutdownResidue": 0,
            },
        },
    });
    serde_json::json!({
        "activeRoutingEpoch": {
            "publishCount": 1,
            "active": {
                "environment": ENVIRONMENT,
                "generation": 1,
                "assemblyIdentity": ASSEMBLY_A,
                "configSnapshotId": SNAPSHOT_A,
            },
        },
        "bootstrap": {
            "reader": {
                "missing": 0,
                "malformed": 0,
                "identityMismatch": 0,
                "pending": 0,
                "repository": 0,
            },
        },
        "blockingLoader": {
            "concurrency": 8,
            "occupancy": 0,
            "queued": 0,
            "saturated": 0,
            "deadlineAborts": 0,
            "shutdownRefusals": 0,
            "shutdown": false,
        },
        "sessions": {
            "preAuthConnections": 0,
            "preAuthRefused": 0,
            "registeredSessions": 0,
            "pendingSessions": 0,
            "cancelledSessions": 0,
            "barrierPending": 0,
            "consumerPermitsHeld": 0,
            "liveSessionTasks": 0,
        },
        "capabilities": { "connections": 0 },
        "health": { "observations": 0, "observedTotal": 0, "healthBeforeAck": 0 },
        "barrier": { "pending": 0, "permitsHeld": 0, "failStop": null },
        "admission": { "permitsHeld": 0, "releases": 0, "queueFullRejects": 0, "revalidateFailures": 0, "reselects": 0, "noCandidateRejects": 0, "duplicateRequestIdRejects": 0 },
        "requestPending": {
            "unary": 0,
            "stream": 0,
            "taskAttempt": 0,
            "httpPending": 0,
            "httpOverflowTerminals": 0,
            "stopped": false,
        },
        "terminal": { "bySource": {
            "runtime_response_end": 0,
            "runtime_response_error": 0,
            "runtime_request_cancel": 0,
            "timeout": 0,
            "caller_abort": 0,
            "client_disconnect": 0,
            "backpressure": 0,
            "protocol_error": 0,
            "callback_error": 0,
            "runtime_disconnect": 0,
            "router_shutdown": 0,
        } },
        "clientConnections": { "connectionCount": 0, "openConnections": [], "finalizerPending": 0, "finalizerCount": 0, "finalizerFailures": [], "slowClientCount": 0 },
        "generationLeases": { "pinsAcquired": 0, "pinsPendingRelease": 0, "cachedAcquireCount": 0, "releaseAcks": 0, "releaseFailures": [], "runtimeClosed": [] },
        "broker": { "generationCount": 0, "outboundPending": 0, "inboundPending": 0, "outboundTombstones": 0, "inboundTombstones": 0, "timerCount": 0, "protocolViolations": 0, "runtimeDisconnectDetached": 0 },
        "actor": actor,
        "activation": activation,
        "http": { "requests": 0, "unaryDispatches": 0, "streamDispatches": 0, "corsPreflights": 0, "serviceManagedCors": 0, "selectorRejects": 0, "ingressMisses": 0, "requestTooLarge": 0, "responseTooLarge": 0, "backpressureCancels": 0, "clientDisconnectCancels": 0, "timeouts": 0, "platformErrors": 0 },
        "mailboxes": { "coordinator": { "occupancy": 0, "capacity": 64, "saturation": 0 } },
        "writerQueues": { "wsSlowClientCount": 0, "wsObservedWriteBytesTotal": 0 },
        "tasks": { "liveSessionTasks": 0, "actorTaskCapacityInUse": 0, "actorTaskAccepted": 0, "actorTaskRejected": 0 },
        "shutdown": { "sessionFailStop": null, "coordinatorShutdown": false, "repositoryDriverClosed": false, "repositoryDriverShutdownResidue": 0, "dispatcherStopped": false, "wsFailStopReason": null },
    })
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
        "configSnapshot": {
            "snapshotId": snapshot_for_assembly(assembly_identity),
        },
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
        "configSnapshotId": snapshot_for_assembly(assembly_identity),
        "state": state,
        "connected": connected,
        "inFlightCount": 0,
        "connectionPinCount": 0,
        "connectionReleaseAckCount": 0,
        "registeredAt": "2026-07-22T00:00:00.000Z",
    })
}

fn snapshot_for_assembly(assembly_identity: &str) -> &'static str {
    if assembly_identity == ASSEMBLY_A {
        SNAPSHOT_A
    } else {
        SNAPSHOT_B
    }
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
