//! `/__router/health` wire projection (batch 12 health leaf).
//!
//! The base payload keeps the TS `AssemblyControlPlane` shape
//! (`ok`/`activeAssembly`/`pendingActivation`/`capabilityConnections`/
//! `replicas`) and adds the §10 `counters` object; `?detail=loop-risk` adds
//! the TS-parity `loopRisk` object. Health never contains Mongo URLs,
//! secrets, business payloads or complete query URLs.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use serde_json::{json, to_value, Value};
use skiff_runtime_transport::protocol::{
    RuntimeHealthCountersFrameHeader, RuntimeHealthFrameHeader,
};

use crate::routing::DispatchCapabilities;
use crate::session::directory::RuntimeRegistrationDirectory;
use crate::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};

use super::counters::HealthCounters;
use super::time::parse_iso_utc_millis;

/// One current session's directory facts (projection input; never mutated).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFacts {
    pub session: RuntimeSessionEpoch,
    pub tuple: RegisteredAssemblyTuple,
    pub registered: bool,
    pub cancelled: bool,
}

/// `activeAssembly` (TS shape; `ingressCount` from the immutable epoch).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveAssemblyProjection {
    pub environment: String,
    pub generation: u64,
    pub assembly_identity: String,
    pub config_snapshot_id: String,
    pub ingress_count: usize,
}

/// `capabilityConnections[]` (TS shape; `registeredAt` is omitted because the
/// session directory does not retain registration timestamps).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityConnectionProjection {
    pub runtime_id: String,
    pub connected: bool,
    pub capabilities: CapabilitiesProjection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitiesProjection {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dispatch_modes: Vec<String>,
}

/// `replicas[]` (TS shape; `registeredAt` omitted, see leaf §5b).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplicaProjection {
    pub replica_id: String,
    pub environment: String,
    pub generation: u64,
    pub assembly_identity: String,
    pub config_snapshot_id: String,
    pub state: String,
    pub connected: bool,
    pub in_flight_count: u64,
    pub connection_pin_count: u64,
    pub connection_release_ack_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_health_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_counters: Option<RuntimeHealthCountersFrameHeader>,
}

/// `loopRisk` (TS `AssemblyControlPlane.loopRiskHealthSnapshot` parity).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopRiskProjection {
    pub observed_at: String,
    pub router: LoopRiskRouterProjection,
    pub runtimes: Vec<LoopRiskRuntimeProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopRiskRouterProjection {
    pub dispatcher: LoopRiskDispatcherProjection,
    pub http_stream: LoopRiskHttpStreamProjection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopRiskDispatcherProjection {
    pub pending_unary: u64,
    pub pending_stream: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopRiskHttpStreamProjection {
    /// The Rust HTTP gateway does not publish a waiter count; the field is
    /// kept for TS parity and is zero (leaf §5d).
    pub backpressure_waiters: u64,
    pub backpressure_cancels: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopRiskRuntimeProjection {
    pub runtime_id: String,
    pub connected: bool,
    pub fresh: bool,
    pub counters: RuntimeHealthCountersFrameHeader,
}

/// Builds the base health JSON (TS shape + `counters`).
pub fn render_base(
    ok: bool,
    active: Option<&ActiveAssemblyProjection>,
    pending: Option<&skiff_deployment::activation_state::PendingActivation>,
    capability_connections: &[CapabilityConnectionProjection],
    replicas: &[ReplicaProjection],
    counters: &HealthCounters,
) -> Value {
    json!({
        "ok": ok,
        "activeAssembly": active.map(|active| to_value(active).expect("active projection serializes")),
        "pendingActivation": pending.map(|pending| to_value(pending).expect("pending DTO serializes")),
        "capabilityConnections": capability_connections
            .iter()
            .map(|connection| to_value(connection).expect("capability projection serializes"))
            .collect::<Vec<_>>(),
        "replicas": replicas
            .iter()
            .map(|replica| to_value(replica).expect("replica projection serializes"))
            .collect::<Vec<_>>(),
        "counters": to_value(counters).expect("counters serialize"),
    })
}

/// Projects the current registered/cancelled sessions into the TS `replicas`
/// array. Pending (not yet ACKed) sessions are excluded (TS registry parity).
/// `in_flight_by_replica` comes from the dispatcher permit ledger; pin counts
/// are not published per session by the WS ledger and stay zero (leaf §5c).
pub fn project_replicas(
    facts: &[SessionFacts],
    observations: &HashMap<RuntimeSessionEpoch, RuntimeHealthFrameHeader>,
    connected: &HashSet<RuntimeSessionEpoch>,
    in_flight_by_replica: &HashMap<String, u64>,
) -> Vec<ReplicaProjection> {
    let mut replicas = facts
        .iter()
        .filter(|fact| fact.registered || fact.cancelled)
        .map(|fact| {
            let observation = observations.get(&fact.session);
            let state = if fact.cancelled {
                "disconnected"
            } else if fact.registered {
                "healthy"
            } else {
                "draining"
            };
            ReplicaProjection {
                replica_id: fact.session.replica_id.clone(),
                environment: fact.tuple.environment.clone(),
                generation: fact.tuple.generation,
                assembly_identity: fact.tuple.assembly_identity().to_string(),
                config_snapshot_id: fact.tuple.snapshot_id().to_string(),
                state: state.to_string(),
                connected: !fact.cancelled && connected.contains(&fact.session),
                in_flight_count: in_flight_by_replica
                    .get(&fact.session.replica_id)
                    .copied()
                    .unwrap_or(0),
                connection_pin_count: 0,
                connection_release_ack_count: 0,
                last_health_at: observation.map(|header| header.observed_at.clone()),
                health_counters: observation.map(|header| header.counters.clone()),
            }
        })
        .collect::<Vec<_>>();
    replicas.sort_by(|left, right| left.replica_id.cmp(&right.replica_id));
    replicas
}

/// Projects capability bindings into the TS `capabilityConnections` array.
pub fn project_capability_connections(
    facts: &[SessionFacts],
    capabilities: &HashMap<RuntimeSessionEpoch, DispatchCapabilities>,
    connected: &HashSet<RuntimeSessionEpoch>,
) -> Vec<CapabilityConnectionProjection> {
    let mut connections = facts
        .iter()
        .filter(|fact| fact.registered || fact.cancelled)
        .filter_map(|fact| {
            let dispatch_modes = capabilities.get(&fact.session)?;
            let mut modes = Vec::new();
            if dispatch_modes.unary {
                modes.push("unary".to_string());
            }
            if dispatch_modes.server_stream {
                modes.push("serverStream".to_string());
            }
            Some(CapabilityConnectionProjection {
                runtime_id: fact.session.replica_id.clone(),
                connected: !fact.cancelled && connected.contains(&fact.session),
                capabilities: CapabilitiesProjection {
                    dispatch_modes: modes,
                },
            })
        })
        .collect::<Vec<_>>();
    connections.sort_by(|left, right| left.runtime_id.cmp(&right.runtime_id));
    connections
}

/// Projects `loopRisk.runtimes` (TS parity): registered sessions with a
/// health observation, `fresh` within a 5-second observation window.
pub fn project_loop_risk_runtimes(
    facts: &[SessionFacts],
    observations: &HashMap<RuntimeSessionEpoch, RuntimeHealthFrameHeader>,
    connected: &HashSet<RuntimeSessionEpoch>,
    now_millis: u64,
) -> Vec<LoopRiskRuntimeProjection> {
    let mut runtimes = facts
        .iter()
        .filter(|fact| fact.registered || fact.cancelled)
        .filter_map(|fact| {
            let observation = observations.get(&fact.session)?;
            let connected = !fact.cancelled && connected.contains(&fact.session);
            let fresh = connected
                && parse_iso_utc_millis(&observation.observed_at)
                    .is_some_and(|observed| now_millis.saturating_sub(observed) <= 5_000);
            Some(LoopRiskRuntimeProjection {
                runtime_id: fact.session.replica_id.clone(),
                connected,
                fresh,
                counters: observation.counters.clone(),
            })
        })
        .collect::<Vec<_>>();
    runtimes.sort_by(|left, right| left.runtime_id.cmp(&right.runtime_id));
    runtimes
}

/// Read-only facts snapshot from the registration directory (caller must
/// hold the directory lock; see `SessionLayer::directory_lock`).
pub fn session_facts(directory: &RuntimeRegistrationDirectory) -> Vec<SessionFacts> {
    directory
        .current_by_replica()
        .values()
        .filter_map(|session| {
            let record = directory.record(session)?;
            Some(SessionFacts {
                session: session.clone(),
                tuple: record.registered_tuple.clone()?,
                registered: record.routable,
                cancelled: record.cancelled,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::{RuntimeAssemblyRef, RuntimeConfigSnapshotRef};

    #[test]
    fn session_facts_excludes_records_without_a_tuple() {
        let directory = RuntimeRegistrationDirectory::new(
            &crate::session::consumer::ConsumerManifest::default_installed(),
        );
        let facts = session_facts(&directory);
        assert!(facts.is_empty());
    }

    #[test]
    fn time_window_uses_five_second_freshness() {
        let now = 1_785_628_800_123;
        let mut observations = HashMap::new();
        observations.insert(
            RuntimeSessionEpoch {
                replica_id: "runtime-a".to_string(),
                connection_generation: 1,
            },
            RuntimeHealthFrameHeader {
                schema_version: "skiff-runtime-frame-v3".to_string(),
                envelope_type: "runtime.health".to_string(),
                runtime_id: "runtime-a".to_string(),
                observed_at: "2026-08-02T00:00:00.000Z".to_string(),
                counters: RuntimeHealthCountersFrameHeader {
                    outbound_requests_pending: 0,
                    outbound_stream_leases_active: 0,
                    stream_runtime_streams_active: 0,
                    flag_backed_cancel_waiters_active: 0,
                    spawned_tasks_active: 0,
                },
            },
        );
        let facts = vec![SessionFacts {
            session: RuntimeSessionEpoch {
                replica_id: "runtime-a".to_string(),
                connection_generation: 1,
            },
            tuple: RegisteredAssemblyTuple {
                environment: "prod".to_string(),
                generation: 7,
                assembly: RuntimeAssemblyRef {
                    assembly_identity: skiff_artifact_model::AssemblyIdentity::new(
                        "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    ),
                },
                config_snapshot: RuntimeConfigSnapshotRef {
                    snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
                        "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    )
                    .expect("snapshot"),
                },
            },
            registered: true,
            cancelled: false,
        }];
        let connected = facts
            .iter()
            .map(|fact| fact.session.clone())
            .collect::<HashSet<_>>();
        let runtimes = project_loop_risk_runtimes(&facts, &observations, &connected, now);
        assert_eq!(runtimes.len(), 1);
        assert!(runtimes[0].fresh);
        let stale = project_loop_risk_runtimes(&facts, &observations, &connected, now + 6_000);
        assert!(!stale[0].fresh);
    }
}
