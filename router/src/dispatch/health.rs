//! Health snapshot fields (C-dispatch §7.6).
//!
//! Health never exposes payload bytes, request ids or session secrets.

use std::collections::BTreeMap;

/// Frozen 11-class terminal source vocabulary (C-dispatch §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TerminalSource {
    RuntimeResponseEnd,
    RuntimeResponseError,
    RuntimeRequestCancel,
    Timeout,
    CallerAbort,
    ClientDisconnect,
    Backpressure,
    ProtocolError,
    CallbackError,
    RuntimeDisconnect,
    RouterShutdown,
}

impl TerminalSource {
    pub const ALL: [Self; 11] = [
        Self::RuntimeResponseEnd,
        Self::RuntimeResponseError,
        Self::RuntimeRequestCancel,
        Self::Timeout,
        Self::CallerAbort,
        Self::ClientDisconnect,
        Self::Backpressure,
        Self::ProtocolError,
        Self::CallbackError,
        Self::RuntimeDisconnect,
        Self::RouterShutdown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeResponseEnd => "runtime_response_end",
            Self::RuntimeResponseError => "runtime_response_error",
            Self::RuntimeRequestCancel => "runtime_request_cancel",
            Self::Timeout => "timeout",
            Self::CallerAbort => "caller_abort",
            Self::ClientDisconnect => "client_disconnect",
            Self::Backpressure => "backpressure",
            Self::ProtocolError => "protocol_error",
            Self::CallbackError => "callback_error",
            Self::RuntimeDisconnect => "runtime_disconnect",
            Self::RouterShutdown => "router_shutdown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PendingHealth {
    pub unary: u64,
    pub stream: u64,
    /// Durable task attempts executing as ordinary unary requests.
    pub task_attempt: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalHealth {
    pub by_source: BTreeMap<TerminalSource, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdmissionHealth {
    pub permits_held: usize,
    pub releases: u64,
    pub queue_full_rejects: u64,
    pub revalidate_failures: u64,
    pub reselects: u64,
    pub no_candidate_rejects: u64,
    pub duplicate_request_id_rejects: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaskHealth {
    pub task_attempts_accepted: u64,
    pub task_attempts_rejected: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DispatcherHealthSnapshot {
    pub pending: PendingHealth,
    pub terminal: TerminalHealth,
    pub admission: AdmissionHealth,
    pub task: TaskHealth,
    pub stopped: bool,
}
