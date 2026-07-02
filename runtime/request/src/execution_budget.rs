use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

use serde_json::{Map, Value};
use skiff_runtime_capability_context::ExecutionBudgetReason;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

pub const DEFAULT_INSTRUCTION_LIMIT: u64 = 10_000_000;
pub const DEFAULT_POLL_INTERVAL: u64 = 1024;

#[derive(Clone, Debug)]
pub struct ExecutionBudgetConfig {
    pub enabled: bool,
    pub instruction_limit: Option<u64>,
    pub poll_interval: u64,
}

impl ExecutionBudgetConfig {
    pub fn runtime_default() -> Self {
        Self {
            enabled: true,
            instruction_limit: Some(DEFAULT_INSTRUCTION_LIMIT),
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }

    #[allow(dead_code)]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            instruction_limit: None,
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExecutionStats {
    pub instruction_count: u64,
    pub budget_limit: Option<u64>,
    pub poll_count: u64,
    pub elapsed_ms: f64,
    pub budget_exceeded: bool,
    pub budget_reason: Option<ExecutionBudgetReason>,
}

#[derive(Debug)]
pub struct ExecutionBudget {
    config: ExecutionBudgetConfig,
    deadline: Option<Instant>,
    started_at: Instant,
    instruction_count: AtomicU64,
    poll_count: AtomicU64,
    finished_at: Mutex<Option<Instant>>,
    budget_reason: Mutex<Option<ExecutionBudgetReason>>,
}

impl ExecutionBudget {
    pub fn new(config: ExecutionBudgetConfig, deadline: Option<Instant>) -> Self {
        Self {
            config,
            deadline,
            started_at: Instant::now(),
            instruction_count: AtomicU64::new(0),
            poll_count: AtomicU64::new(0),
            finished_at: Mutex::new(None),
            budget_reason: Mutex::new(None),
        }
    }

    pub fn for_runtime_request(extra: &Map<String, Value>) -> Self {
        Self::new(
            ExecutionBudgetConfig::runtime_default(),
            deadline_from_request_extra(extra),
        )
    }

    #[allow(dead_code)]
    pub fn disabled() -> Self {
        Self::new(ExecutionBudgetConfig::disabled(), None)
    }

    pub fn add_units(&self, units: u64) -> bool {
        if !self.config.enabled || units == 0 {
            return false;
        }

        let previous = self.instruction_count.fetch_add(units, Ordering::Relaxed);
        let current = previous.saturating_add(units);
        let interval = self.config.poll_interval.max(1);
        self.config
            .instruction_limit
            .is_some_and(|limit| current >= limit)
            || current / interval != previous / interval
    }

    pub fn poll(&self, cancelled: bool, now: Instant) -> Result<(), ExecutionBudgetReason> {
        if cancelled {
            return self.fail(ExecutionBudgetReason::Cancelled);
        }
        if !self.config.enabled {
            return Ok(());
        }

        self.poll_count.fetch_add(1, Ordering::Relaxed);

        if self.deadline.is_some_and(|deadline| now >= deadline) {
            return self.fail(ExecutionBudgetReason::DeadlineExceeded);
        }

        if self
            .config
            .instruction_limit
            .is_some_and(|limit| self.instruction_count.load(Ordering::Relaxed) >= limit)
        {
            return self.fail(ExecutionBudgetReason::InstructionLimitExceeded);
        }

        Ok(())
    }

    pub fn record_cancelled(&self) {
        let _ = self.fail(ExecutionBudgetReason::Cancelled);
    }

    pub fn finish(&self, now: Instant) {
        if let Ok(mut finished_at) = self.finished_at.lock() {
            finished_at.get_or_insert(now);
        }
    }

    pub fn stats_snapshot(&self) -> ExecutionStats {
        let reason = self.budget_reason.lock().ok().and_then(|reason| *reason);
        let finished_at = self
            .finished_at
            .lock()
            .ok()
            .and_then(|finished_at| *finished_at);
        let elapsed_ms = finished_at
            .unwrap_or_else(Instant::now)
            .duration_since(self.started_at)
            .as_secs_f64()
            * 1000.0;

        ExecutionStats {
            instruction_count: self.instruction_count.load(Ordering::Relaxed),
            budget_limit: self.config.instruction_limit,
            poll_count: self.poll_count.load(Ordering::Relaxed),
            elapsed_ms,
            budget_exceeded: matches!(
                reason,
                Some(
                    ExecutionBudgetReason::DeadlineExceeded
                        | ExecutionBudgetReason::InstructionLimitExceeded,
                )
            ),
            budget_reason: reason,
        }
    }

    fn fail(&self, reason: ExecutionBudgetReason) -> Result<(), ExecutionBudgetReason> {
        if let Ok(mut stored) = self.budget_reason.lock() {
            stored.get_or_insert(reason);
        }
        Err(reason)
    }
}

pub fn deadline_from_request_extra(extra: &Map<String, Value>) -> Option<Instant> {
    let deadline = extra.get("deadline")?.as_object()?;
    let now = Instant::now();
    let mut candidates = Vec::new();

    if let Some(timeout_ms) = deadline.get("timeoutMs").and_then(Value::as_u64) {
        if let Some(deadline) = now.checked_add(Duration::from_millis(timeout_ms)) {
            candidates.push(deadline);
        }
    }

    if let Some(expires_at) = deadline.get("expiresAt").and_then(Value::as_str) {
        if let Ok(expires_at) = OffsetDateTime::parse(expires_at, &Rfc3339) {
            let wall_now = OffsetDateTime::now_utc();
            if expires_at <= wall_now {
                candidates.push(now);
            } else if let Some(deadline) = now.checked_add((expires_at - wall_now).unsigned_abs()) {
                candidates.push(deadline);
            }
        }
    }

    candidates.into_iter().min()
}

#[cfg(test)]
mod tests;
