//! `/__router/health` wire projection (batch 12 health leaf; M4 shape).
//!
//! The base payload keeps the TS `AssemblyControlPlane` shape
//! (`ok`/`activeAssembly`/`capabilityConnections`/`replicas`) and adds the
//! §10 `counters` object; `?detail=loop-risk` adds the TS-parity `loopRisk`
//! object. M4: `pendingActivation` is retired and `activeAssembly` is the
//! release pointer table projection (`{ profile, releaseCount, buildIds }`).
//! Health never contains Mongo URLs, secrets, business payloads or complete
//! query URLs.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use serde_json::{json, to_value, Value};
use skiff_runtime_transport::protocol::{
    RuntimeHealthCountersFrameHeader, RuntimeHealthFrameHeader,
};

use crate::routing::DispatchCapabilities;
use crate::session::directory::RuntimeRegistrationDirectory;
use crate::session::identity::RuntimeSessionEpoch;

use super::counters::HealthCounters;
use super::time::parse_iso_utc_millis;

/// One current session's directory facts (projection input; never mutated).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFacts {
    pub session: RuntimeSessionEpoch,
    pub registered: bool,
    pub cancelled: bool,
    pub registered_build_ids: Vec<String>,
    pub lazy_load: bool,
    pub artifact_root: Option<String>,
}

/// `activeAssembly` (M4 pointer-table projection): the profile plus the
/// number of published release pointers and the resolved build id set.
/// `loadedBuildIds` / `routerArtifactRoot` are the lazy-load deployment
/// extension (integration-contract-v2 §3/health): the union of build ids
/// currently registered by connected capability-holders plus the router's
/// own artifact store root used by the candidate rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveAssemblyProjection {
    pub profile: String,
    pub release_count: usize,
    pub build_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loaded_build_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub router_artifact_root: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_root: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub lazy_load: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub loaded_build_ids: Vec<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// `replicas[]` (M4: no tuple fields).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplicaProjection {
    pub replica_id: String,
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

/// Builds the base health JSON (TS shape + `counters`; M4: no
/// `pendingActivation`).
pub fn render_base(
    ok: bool,
    active: Option<&ActiveAssemblyProjection>,
    capability_connections: &[CapabilityConnectionProjection],
    replicas: &[ReplicaProjection],
    counters: &HealthCounters,
) -> Value {
    json!({
        "ok": ok,
        "activeAssembly": active.map(|active| to_value(active).expect("active projection serializes")),
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
                    artifact_root: fact.artifact_root.clone(),
                    lazy_load: fact.lazy_load,
                    loaded_build_ids: fact.registered_build_ids.clone(),
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
                registered: record.routable,
                cancelled: record.cancelled,
                registered_build_ids: record.registration_facts.registered_build_ids.clone(),
                lazy_load: record.registration_facts.lazy_load,
                artifact_root: record.registration_facts.artifact_root.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::consumer::ConsumerManifest;

    #[test]
    fn session_facts_excludes_records_without_a_tuple() {
        let directory = RuntimeRegistrationDirectory::new(&ConsumerManifest::default_installed());
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
                schema_version: "skiff-runtime-frame-v4".to_string(),
                envelope_type: "runtime.health".to_string(),
                runtime_id: "runtime-a".to_string(),
                observed_at: "2026-08-02T00:00:00.000Z".to_string(),
                counters: RuntimeHealthCountersFrameHeader {
                    outbound_requests_pending: 0,
                    outbound_stream_leases_active: 0,
                    stream_runtime_streams_active: 0,
                    flag_backed_cancel_waiters_active: 0,
                    task_requests_active: 0,
                },
            },
        );
        let facts = vec![SessionFacts {
            session: RuntimeSessionEpoch {
                replica_id: "runtime-a".to_string(),
                connection_generation: 1,
            },
            registered: true,
            cancelled: false,
            registered_build_ids: Vec::new(),
            lazy_load: false,
            artifact_root: None,
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

    #[test]
    fn loop_risk_freshness_accepts_real_runtime_six_and_nine_digit_timestamps() {
        let facts = vec![SessionFacts {
            session: RuntimeSessionEpoch {
                replica_id: "runtime-a".to_string(),
                connection_generation: 1,
            },
            registered: true,
            cancelled: false,
            registered_build_ids: Vec::new(),
            lazy_load: false,
            artifact_root: None,
        }];
        let connected = facts
            .iter()
            .map(|fact| fact.session.clone())
            .collect::<HashSet<_>>();
        let now = 1_785_628_800_123;
        for observed_at in [
            "2026-08-02T00:00:00.123456Z",
            "2026-08-02T00:00:00.123456789Z",
        ] {
            let observations = HashMap::from([(
                RuntimeSessionEpoch {
                    replica_id: "runtime-a".to_string(),
                    connection_generation: 1,
                },
                RuntimeHealthFrameHeader {
                    schema_version: "skiff-runtime-frame-v4".to_string(),
                    envelope_type: "runtime.health".to_string(),
                    runtime_id: "runtime-a".to_string(),
                    observed_at: observed_at.to_string(),
                    counters: RuntimeHealthCountersFrameHeader {
                        outbound_requests_pending: 0,
                        outbound_stream_leases_active: 0,
                        stream_runtime_streams_active: 0,
                        flag_backed_cancel_waiters_active: 0,
                        task_requests_active: 0,
                    },
                },
            )]);
            let runtimes = project_loop_risk_runtimes(&facts, &observations, &connected, now);
            assert_eq!(runtimes.len(), 1);
            assert!(
                runtimes[0].fresh,
                "real runtime timestamp {observed_at} must be fresh"
            );
        }
    }
}
