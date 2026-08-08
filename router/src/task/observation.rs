//! Router implementation of the task-control scheduler observability seam
//! (authoritative design "Observability And Retention": scheduled→ready,
//! claim / eligible wait, lease renew / loss, infrastructure recovery,
//! duplicate notification absorption, provable-rejection release).
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

fn correlation(record: &TaskRecord) -> (Map<String, Value>, Option<String>) {
    let mut attrs = Map::new();
    attrs.insert(
        "taskId".to_string(),
        Value::String(record.task_id.as_str().to_string()),
    );
    (attrs, Some(record.trace.trace_id.clone()))
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
    fn on_due_ready(&self, record: &TaskRecord, now: DurableUtcTimestamp) {
        let (mut attrs, trace_id) = correlation(record);
        attrs.insert("dueAtMs".to_string(), json!(record.due_at.millis()));
        attrs.insert("readyAtMs".to_string(), json!(now.millis()));
        attrs.insert(
            "scheduledToReadyMs".to_string(),
            json!((now.millis() - record.due_at.millis()).max(0)),
        );
        let mut event = task_event(
            "task.ready",
            Some(record.task_id.as_str()),
            attrs,
        );
        event.trace_id = trace_id;
        self.telemetry.emit(event);
    }

    fn on_claim(&self, record: &TaskRecord, now: DurableUtcTimestamp) {
        let (mut attrs, trace_id) = correlation(record);
        if let Some(lease) = record.active_lease.as_ref() {
            attrs.insert(
                "attemptId".to_string(),
                Value::String(lease.attempt_id.as_str().to_string()),
            );
            attrs.insert(
                "leaseId".to_string(),
                Value::String(lease.lease_id.as_str().to_string()),
            );
        }
        attrs.insert("dueAtMs".to_string(), json!(record.due_at.millis()));
        attrs.insert("claimedAtMs".to_string(), json!(now.millis()));
        attrs.insert(
            "eligibleWaitMs".to_string(),
            json!((now.millis() - record.due_at.millis()).max(0)),
        );
        let mut event = task_event(
            "task.claim",
            Some(record.task_id.as_str()),
            attrs,
        );
        event.trace_id = trace_id;
        self.telemetry.emit(event);
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

    fn on_renewed(&self, task_id: &TaskId, lease_id: &LeaseId, new_expiry: DurableUtcTimestamp) {
        let mut extra = Map::new();
        extra.insert("renewedAtMs".to_string(), json!(new_expiry.millis()));
        emit_lease_event(
            &self.telemetry,
            "task.lease.renewed",
            task_id,
            lease_id,
            extra,
        );
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
