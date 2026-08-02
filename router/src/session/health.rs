//! Minimal `RuntimeHealthLedger` observation projection (C-session §7.1/§7.6,
//! C-model-registration §2.4).
//!
//! The ledger only keeps current observations and counters; it never holds
//! permits, sockets or eligibility. Health frames are observations only after
//! the registered ACK is written; earlier frames are dropped and counted as
//! `health_before_ack`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use skiff_runtime_transport::protocol::RuntimeHealthFrameHeader;

use super::consumer::{ConsumerKind, SessionConsumer};
use super::identity::RuntimeSessionEpoch;

#[derive(Debug, Default)]
pub struct RuntimeHealthLedger {
    observations: Mutex<HashMap<RuntimeSessionEpoch, RuntimeHealthFrameHeader>>,
    observed_total: AtomicU64,
    dropped_before_ack_total: AtomicU64,
}

impl RuntimeHealthLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a registered observation (only reachable after ACK; the caller
    /// already validated `runtime_id` against the bound replica).
    pub fn record_observation(
        &self,
        session: &RuntimeSessionEpoch,
        header: RuntimeHealthFrameHeader,
    ) {
        self.observations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session.clone(), header);
        self.observed_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Drop a health frame that arrived before the ACK; counted, never an
    /// observation.
    pub fn drop_before_ack(&self, _session: &RuntimeSessionEpoch) {
        self.dropped_before_ack_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn observed_total(&self) -> u64 {
        self.observed_total.load(Ordering::Relaxed)
    }

    pub fn dropped_before_ack_total(&self) -> u64 {
        self.dropped_before_ack_total.load(Ordering::Relaxed)
    }

    pub fn current_observations(&self) -> usize {
        self.observations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

impl SessionConsumer for RuntimeHealthLedger {
    fn kind(&self) -> ConsumerKind {
        ConsumerKind::HealthLedger
    }

    fn on_session_closed(&self, session: &RuntimeSessionEpoch) -> Result<(), String> {
        self.observations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(session);
        Ok(())
    }
}
