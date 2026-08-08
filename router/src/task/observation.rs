//! Router implementation of the task-control scheduler observability seam
//! (authoritative design "Observability And Retention": lease loss /
//! infrastructure recovery, duplicate notification absorption,
//! provable-rejection release). High-frequency trend transitions
//! (scheduled→ready, claim, lease renew) are deliberately not emitted: queue
//! depth and latency live in the router health counters, not the telemetry
//! stream.
//!
//! The observer is strictly read-only and never influences scheduler
//! semantics; all events reuse the `skiff-telemetry-v1` event schema with
//! `TaskId` correlation.

use std::sync::Arc;

use serde_json::{json, Map, Value};
use skiff_task_control::model::{DurableUtcTimestamp, LeaseId, TaskId, TaskRecord};
use skiff_task_control::scheduler::SchedulerObservation;
use skiff_task_control::store::{ClaimRejection, RenewRejection};

use crate::telemetry::{task_event, TaskTelemetrySink};

/// Router task scheduler observation: maps scheduler transitions to task
/// telemetry events (source Router, `task.*` names).
#[derive(Debug)]
pub struct RouterTaskSchedulerObservation {
    telemetry: Arc<dyn TaskTelemetrySink>,
}

impl RouterTaskSchedulerObservation {
    pub fn new(telemetry: Arc<dyn TaskTelemetrySink>) -> Self {
        Self { telemetry }
    }
}

fn lease_attrs(task_id: &TaskId, lease_id: &LeaseId) -> Map<String, Value> {
    let mut attrs = Map::new();
    attrs.insert(
        "taskId".to_string(),
        Value::String(task_id.as_str().to_string()),
    );
    attrs.insert(
        "leaseId".to_string(),
        Value::String(lease_id.as_str().to_string()),
    );
    attrs
}

fn emit_lease_event(
    telemetry: &Arc<dyn TaskTelemetrySink>,
    name: &str,
    task_id: &TaskId,
    lease_id: &LeaseId,
    extra: Map<String, Value>,
) {
    let mut attrs = lease_attrs(task_id, lease_id);
    attrs.extend(extra);
    telemetry.emit(task_event(name, Some(task_id.as_str()), attrs));
}

impl SchedulerObservation for RouterTaskSchedulerObservation {
    fn on_due_ready(&self, _record: &TaskRecord, _now: DurableUtcTimestamp) {
        // High-frequency trend event: queue depth / oldest eligible age are
        // served by the router health counters, not the telemetry stream.
    }

    fn on_claim(&self, _record: &TaskRecord, _now: DurableUtcTimestamp) {
        // High-frequency trend event: see `on_due_ready`.
    }

    fn on_claim_duplicate(&self, task_id: &TaskId, reason: &ClaimRejection) {
        let mut attrs = Map::new();
        attrs.insert("reason".to_string(), Value::String(format!("{reason:?}")));
        self.telemetry.emit(task_event(
            "task.duplicate.absorbed",
            Some(task_id.as_str()),
            attrs,
        ));
    }

    fn on_renewed(&self, _task_id: &TaskId, _lease_id: &LeaseId, _new_expiry: DurableUtcTimestamp) {
        // High-frequency trend event: lease renewal is a heartbeat; see
        // `on_due_ready`.
    }

    fn on_renew_lost(&self, task_id: &TaskId, lease_id: &LeaseId, rejection: RenewRejection) {
        let mut extra = Map::new();
        extra.insert(
            "reason".to_string(),
            Value::String(format!("{rejection:?}")),
        );
        emit_lease_event(
            &self.telemetry,
            "task.lease.lost",
            task_id,
            lease_id,
            extra,
        );
    }

    fn on_recover(&self, task_id: &TaskId, lease_id: &LeaseId) {
        emit_lease_event(
            &self.telemetry,
            "task.recovered",
            task_id,
            lease_id,
            Map::new(),
        );
    }

    fn on_release(
        &self,
        task_id: &TaskId,
        lease_id: &LeaseId,
        retry_not_before: DurableUtcTimestamp,
    ) {
        let mut extra = Map::new();
        extra.insert(
            "retryNotBeforeMs".to_string(),
            json!(retry_not_before.millis()),
        );
        emit_lease_event(
            &self.telemetry,
            "task.lease.released",
            task_id,
            lease_id,
            extra,
        );
    }
}
