//! Typed frame/peer seams for W-dispatch
//! (C-dispatch §7.2/§7.7, C-model-request §2/§4/§5).

use std::fmt;

use skiff_runtime_transport::protocol::ValidatedResponseErrorFrame;

use crate::session::identity::RuntimeSessionEpoch;

use super::types::{DispatchSubmit, RequestDeadline, TaskAttemptSubmit};

/// Runtime-to-Router response/cancel frame already decoded and validated by
/// the shared codec (C-model-request §2/§5). The dispatcher only enforces
/// correlation and sequence semantics.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeResponseFrame {
    Start {
        request_id: String,
    },
    Chunk {
        request_id: String,
        seq: u64,
        payload: Vec<u8>,
    },
    End {
        request_id: String,
        payload_present: bool,
        payload: Vec<u8>,
    },
    Error {
        request_id: String,
        error: ValidatedResponseErrorFrame,
    },
    /// Runtime-initiated `request.cancel` (C-model-request §4).
    Cancel {
        request_id: String,
        reason: String,
    },
}

impl RuntimeResponseFrame {
    pub fn request_id(&self) -> &str {
        match self {
            Self::Start { request_id }
            | Self::Chunk { request_id, .. }
            | Self::End { request_id, .. }
            | Self::Error { request_id, .. }
            | Self::Cancel { request_id, .. } => request_id,
        }
    }
}

/// Non-blocking outbound frame port to one Runtime session writer
/// (C-dispatch §7.7 fake seam; C-session writer contract).
///
/// `Err` means writer queue full / socket failure: the dispatcher terminates
/// `callback_error`, best-effort sends `request.cancel(protocol_error)` and
/// requests an abort of the exact session (C-dispatch §7.4). The production
/// implementation (W-session/W-http integration) encodes typed headers with
/// the shared transport codec; this module never re-implements the wire.
pub trait RuntimePeer: Send + Sync + fmt::Debug {
    fn send_request_start(
        &self,
        session: &RuntimeSessionEpoch,
        request: &DispatchSubmit,
    ) -> Result<(), String>;
    fn send_request_cancel(
        &self,
        session: &RuntimeSessionEpoch,
        request_id: &str,
        reason: &str,
    ) -> Result<(), String>;
    /// Task-attempt `request.start` (task invocation shape + `taskAttempt`
    /// header). Same bounded-writer contract as `send_request_start`.
    fn send_task_attempt_start(
        &self,
        session: &RuntimeSessionEpoch,
        attempt: &TaskAttemptSubmit,
    ) -> Result<(), String>;
}

/// Session abort request emitted on writer failure (C-dispatch §7.4).
///
/// W-session seam: the production implementation aborts the exact session
/// socket without waiting for the writer queue to accept a close frame.
pub trait SessionAbortControl: Send + Sync + fmt::Debug {
    fn abort_session(&self, session: &RuntimeSessionEpoch);
}

/// Definite terminal of one task-attempt request, as proven by the ordinary
/// request lifecycle. The task control plane maps these to
/// `TaskStore.settle` or to lease-loss recovery (uncertain outcomes are
/// neither settled nor released).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskAttemptTerminalOutcome {
    /// `response.end` converged: the target returned normally.
    Succeeded,
    /// `response.error` / contract cancel / ordinary request timeout:
    /// the attempt executed and failed.
    Failed { message: String },
    /// Disconnect, shutdown or protocol loss: the platform cannot prove the
    /// outcome. No settlement; lease expiry drives recovery.
    Uncertain { reason: String },
}

/// Settlement port consumed by the dispatcher for task-attempt terminals
/// (authoritative design "Runtime Admission And Settlement"). The
/// implementation is the task control plane; it owns settlement correlation
/// and pending-attempt bookkeeping.
pub trait TaskAttemptTerminalSink: Send + Sync + fmt::Debug {
    fn on_terminal(
        &self,
        request_id: &str,
        task_id: &str,
        attempt_id: &str,
        lease_id: &str,
        outcome: TaskAttemptTerminalOutcome,
    );
}

/// Default no-op sink used by dispatcher-only tests that do not exercise the
/// task control plane.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopTaskAttemptTerminalSink;

impl TaskAttemptTerminalSink for NoopTaskAttemptTerminalSink {
    fn on_terminal(
        &self,
        _request_id: &str,
        _task_id: &str,
        _attempt_id: &str,
        _lease_id: &str,
        _outcome: TaskAttemptTerminalOutcome,
    ) {
    }
}

/// Deadline expiry check (C-dispatch §3 "发送前 deadline 重检").
///
/// The default [`WireTimeoutCheck`] treats `timeoutMs == 0` as expired;
/// parsing/arithmetic over the ISO `expiresAt` belongs to the caller
/// (W-http), which supplies a richer check in production.
pub trait TimeoutCheck: Send + Sync + fmt::Debug {
    fn is_expired(&self, deadline: &RequestDeadline) -> bool;
}

#[derive(Debug, Clone, Default)]
pub struct WireTimeoutCheck;

impl TimeoutCheck for WireTimeoutCheck {
    fn is_expired(&self, deadline: &RequestDeadline) -> bool {
        deadline.timeout_ms == 0
    }
}
