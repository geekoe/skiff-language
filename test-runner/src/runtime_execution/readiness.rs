use std::{
    cmp::Ordering,
    time::{Duration, Instant},
};

use skiff_artifact_model::{RuntimeAssemblyRef, RuntimeConfigSnapshotRef};

use crate::canonical_fixture::CanonicalFixtureError;

use super::{
    http::HttpResponse,
    wire::{self, ActivationReceipt, HealthSnapshot, ReplicaState},
};

const INITIAL_BACKOFF: Duration = Duration::from_millis(10);
const MAX_BACKOFF: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReadinessTarget {
    profile: String,
    generation: u64,
    assembly: RuntimeAssemblyRef,
    config_snapshot: RuntimeConfigSnapshotRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReadinessStatus {
    Ready,
    Waiting(String),
}

pub(super) fn target_from_receipt(
    receipt: ActivationReceipt,
    profile: &str,
    generation: u64,
    assembly_identity: &str,
    config_snapshot: &RuntimeConfigSnapshotRef,
) -> Result<ReadinessTarget, CanonicalFixtureError> {
    if receipt.profile != profile {
        return Err(readiness_error(format!(
            "activation receipt profile mismatch: expected {profile}, got {}",
            receipt.profile
        )));
    }
    if receipt.generation != generation {
        return Err(readiness_error(format!(
            "activation receipt generation mismatch: expected {generation}, got {}",
            receipt.generation
        )));
    }
    if receipt.assembly.assembly_identity.as_str() != assembly_identity {
        return Err(readiness_error(format!(
            "activation receipt assembly identity mismatch: expected {assembly_identity}, got {}",
            receipt.assembly.assembly_identity
        )));
    }
    if &receipt.config_snapshot != config_snapshot {
        return Err(readiness_error(format!(
            "activation receipt config snapshot mismatch: expected {}, got {}",
            config_snapshot.snapshot_id, receipt.config_snapshot.snapshot_id
        )));
    }
    Ok(ReadinessTarget {
        profile: receipt.profile,
        generation: receipt.generation,
        assembly: receipt.assembly,
        config_snapshot: receipt.config_snapshot,
    })
}

pub(super) fn poll<Fetch>(
    target: &ReadinessTarget,
    deadline: Instant,
    fetch: Fetch,
) -> Result<(), CanonicalFixtureError>
where
    Fetch: FnMut(Instant) -> Result<HttpResponse, CanonicalFixtureError>,
{
    poll_with(target, deadline, fetch, std::thread::sleep, Instant::now)
}

fn poll_with<Fetch, Wait, Now>(
    target: &ReadinessTarget,
    deadline: Instant,
    mut fetch: Fetch,
    mut wait: Wait,
    mut now: Now,
) -> Result<(), CanonicalFixtureError>
where
    Fetch: FnMut(Instant) -> Result<HttpResponse, CanonicalFixtureError>,
    Wait: FnMut(Duration),
    Now: FnMut() -> Instant,
{
    let started_at = now();
    let timeout = deadline.saturating_duration_since(started_at);
    let mut backoff = INITIAL_BACKOFF;
    let mut last_wait = "no health snapshot received".to_string();
    loop {
        if now() >= deadline {
            return Err(readiness_timeout(timeout, &last_wait));
        }
        let response = fetch(deadline)?;
        if !(200..300).contains(&response.status) {
            let error = wire::decode_control_error_response(&response.body)?;
            return Err(CanonicalFixtureError::RemoteControl {
                status: response.status,
                code: error.code,
                message: error.message,
            });
        }
        let snapshot = wire::decode_health_snapshot(&response.body)?;
        let status = classify(&snapshot, target)?;
        if now() >= deadline {
            return Err(readiness_timeout(timeout, &last_wait));
        }
        match status {
            ReadinessStatus::Ready => return Ok(()),
            ReadinessStatus::Waiting(reason) => last_wait = reason,
        }
        let remaining = deadline.saturating_duration_since(now());
        if remaining.is_zero() {
            return Err(readiness_timeout(timeout, &last_wait));
        }
        wait(backoff.min(remaining));
        backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
    }
}

fn classify(
    snapshot: &HealthSnapshot,
    target: &ReadinessTarget,
) -> Result<ReadinessStatus, CanonicalFixtureError> {
    if snapshot.active.profile != target.profile {
        return Err(readiness_error(format!(
            "router health profile mismatch: expected {}, got {}",
            target.profile, snapshot.active.profile
        )));
    }
    match snapshot.active.generation.cmp(&target.generation) {
        Ordering::Less => {
            return Ok(ReadinessStatus::Waiting(format!(
                "active generation {} is behind target {}",
                snapshot.active.generation, target.generation
            )));
        }
        Ordering::Greater => {
            return Err(readiness_error(format!(
                "router health advanced past target generation {} to {}",
                target.generation, snapshot.active.generation
            )));
        }
        Ordering::Equal => {}
    }
    if snapshot.active.assembly != target.assembly {
        return Err(readiness_error(format!(
            "router health has conflicting assembly identity at generation {}",
            target.generation
        )));
    }
    if snapshot.active.config_snapshot != target.config_snapshot {
        return Err(readiness_error(format!(
            "router health has conflicting config snapshot at generation {}",
            target.generation
        )));
    }
    if snapshot.pending_activation {
        return Ok(ReadinessStatus::Waiting(
            "router health still has a pending activation".to_string(),
        ));
    }

    let matching = snapshot.replicas.iter().filter(|replica| {
        replica.profile == target.profile
            && replica.generation == target.generation
            && replica.assembly == target.assembly
            && replica.config_snapshot == target.config_snapshot
    });
    let mut matching_count = 0;
    let mut dispatch_ready_count = 0;
    for replica in matching {
        matching_count += 1;
        if replica.state != ReplicaState::Healthy || !replica.connected {
            continue;
        }
        dispatch_ready_count += 1;
        if snapshot
            .capability_connections
            .iter()
            .any(|capability| capability.runtime_id == replica.replica_id && capability.connected)
        {
            return Ok(ReadinessStatus::Ready);
        }
    }
    if matching_count == 0 {
        return Ok(ReadinessStatus::Waiting(
            "no replica matches the committed activation tuple".to_string(),
        ));
    }
    if dispatch_ready_count == 0 {
        return Ok(ReadinessStatus::Waiting(
            "matching replicas are not both healthy and connected".to_string(),
        ));
    }
    Ok(ReadinessStatus::Waiting(
        "matching healthy replica has no connected capability identity".to_string(),
    ))
}

fn readiness_timeout(timeout: Duration, last_wait: &str) -> CanonicalFixtureError {
    readiness_error(format!(
        "router readiness timed out after {} ms: {last_wait}",
        timeout.as_millis()
    ))
}

fn readiness_error(message: impl Into<String>) -> CanonicalFixtureError {
    CanonicalFixtureError::InvalidInput(format!("runtime readiness failed: {}", message.into()))
}

#[cfg(test)]
#[path = "tests/readiness.rs"]
mod tests;
