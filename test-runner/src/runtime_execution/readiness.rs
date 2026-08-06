use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

use crate::canonical_fixture::CanonicalFixtureError;

use super::{http::HttpResponse, wire};

const INITIAL_BACKOFF: Duration = Duration::from_millis(10);
const MAX_BACKOFF: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReadinessTarget {
    profile: String,
    build_ids: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReadinessStatus {
    Ready,
    Waiting(String),
}

pub(super) fn target_for_builds(
    profile: &str,
    build_ids: Vec<String>,
) -> Result<ReadinessTarget, CanonicalFixtureError> {
    let build_ids = build_ids.into_iter().collect::<BTreeSet<_>>();
    if build_ids.is_empty() {
        return Err(readiness_error(
            "a test-service batch must expose at least one deployment build id".to_string(),
        ));
    }
    Ok(ReadinessTarget {
        profile: profile.to_string(),
        build_ids,
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
    snapshot: &wire::HealthSnapshot,
    target: &ReadinessTarget,
) -> Result<ReadinessStatus, CanonicalFixtureError> {
    if snapshot.active.profile != target.profile {
        return Err(readiness_error(format!(
            "router health profile mismatch: expected {}, got {}",
            target.profile, snapshot.active.profile
        )));
    }
    let missing = target
        .build_ids
        .difference(&snapshot.active.build_ids)
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(ReadinessStatus::Ready);
    }
    Ok(ReadinessStatus::Waiting(format!(
        "router health build ids do not yet include {missing:?}; active: {:?}",
        snapshot.active.build_ids
    )))
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
