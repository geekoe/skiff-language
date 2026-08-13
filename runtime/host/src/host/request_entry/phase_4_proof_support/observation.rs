use std::sync::Mutex;

use skiff_runtime_model::bytecode_execution_observation::{
    BytecodeExecutionEvent, BytecodeExecutionEventSink, BytecodeExecutionObservation,
    BytecodeRequestTerminal, RequestCleanupComplete, RequestExecutionOwnerInventorySnapshot,
    RequestTerminalClaimed,
};

/// Recording production observation sink. It implements the exact production
/// `BytecodeExecutionEventSink` contract (failure-isolated, no execution
/// authority); the Phase 4 harness installs it on the host before ingress and
/// reads the frozen terminal/cleanup facts afterwards. It never selects a
/// route, resumes work, or writes a verdict.
#[derive(Default)]
pub(in crate::host::request_entry) struct RecordingSink {
    observations: Mutex<Vec<BytecodeExecutionObservation>>,
}

impl BytecodeExecutionEventSink for RecordingSink {
    fn observe(&self, observation: BytecodeExecutionObservation) {
        self.observations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(observation);
    }
}

impl RecordingSink {
    pub(in crate::host::request_entry) fn snapshot(&self) -> Vec<BytecodeExecutionObservation> {
        self.observations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The terminal facts observed for one correlation, in ordinal order.
    pub(in crate::host::request_entry) fn terminals(
        &self,
        correlation: &super::Correlation,
    ) -> Vec<BytecodeRequestTerminal> {
        self.snapshot()
            .into_iter()
            .filter(|observation| {
                observation.correlation.router_session_id == correlation.router_session_id
                    && observation.correlation.request_id == correlation.request_id
            })
            .filter_map(|observation| match observation.event {
                BytecodeExecutionEvent::RequestTerminalClaimed(RequestTerminalClaimed {
                    terminal,
                }) => Some(terminal),
                _ => None,
            })
            .collect()
    }

    /// The cleanup inventory facts observed for one correlation, in order.
    pub(in crate::host::request_entry) fn cleanup_inventories(
        &self,
        correlation: &super::Correlation,
    ) -> Vec<RequestExecutionOwnerInventorySnapshot> {
        self.snapshot()
            .into_iter()
            .filter(|observation| {
                observation.correlation.router_session_id == correlation.router_session_id
                    && observation.correlation.request_id == correlation.request_id
            })
            .filter_map(|observation| match observation.event {
                BytecodeExecutionEvent::RequestCleanupComplete(RequestCleanupComplete {
                    owner_inventory,
                }) => Some(owner_inventory),
                _ => None,
            })
            .collect()
    }
}
