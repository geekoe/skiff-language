//! Per-session frame/byte budgets and the bounded outbound queue
//! (C-session §5.3, authority design §3.8).
//!
//! Owners reserve non-blockingly with `try_send`; the permit transfers to the
//! writer on dequeue and is released exactly once. Queue-full never waits for
//! a close frame: the exact session is aborted through its independent abort
//! handle.

use std::sync::Arc;
use std::sync::Mutex;

use tokio::sync::mpsc;

/// Process-level default budgets and the injectable override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionBudgets {
    pub outbound_frames: usize,
    pub outbound_bytes: usize,
    pub inbound_frames: usize,
    pub inbound_bytes: usize,
}

impl Default for SessionBudgets {
    fn default() -> Self {
        Self {
            outbound_frames: 256,
            outbound_bytes: 4 * 1024 * 1024,
            inbound_frames: 64,
            inbound_bytes: 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundFrameId {
    Bootstrap,
    RegisteredAck,
    Close,
}

#[derive(Debug)]
pub struct QueuedFrame {
    pub id: OutboundFrameId,
    pub bytes: Vec<u8>,
    pub written_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

/// Queue-full outcome: the caller must abort the exact session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundFull {
    pub id: OutboundFrameId,
}

#[derive(Debug)]
struct QueueState {
    frames: usize,
    bytes: usize,
}

/// Bounded outbound frame queue. `try_send` checks both frame and byte
/// permits without blocking; the writer task drains and releases permits.
#[derive(Debug, Clone)]
pub struct OutboundQueue {
    tx: mpsc::Sender<QueuedFrame>,
    state: Arc<Mutex<QueueState>>,
    outbound_frames: usize,
    outbound_bytes: usize,
}

impl OutboundQueue {
    pub fn new(budgets: SessionBudgets) -> (Self, mpsc::Receiver<QueuedFrame>) {
        // Channel capacity mirrors the frame budget; the byte budget is the
        // independent second gate.
        let (tx, rx) = mpsc::channel(budgets.outbound_frames.max(1));
        (
            Self {
                tx,
                state: Arc::new(Mutex::new(QueueState {
                    frames: 0,
                    bytes: 0,
                })),
                outbound_frames: budgets.outbound_frames,
                outbound_bytes: budgets.outbound_bytes,
            },
            rx,
        )
    }

    pub fn try_send(
        &self,
        id: OutboundFrameId,
        bytes: Vec<u8>,
        written_tx: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<(), OutboundFull> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.frames + 1 > self.outbound_frames
            || state.bytes + bytes.len() > self.outbound_bytes
        {
            return Err(OutboundFull { id });
        }
        let frame_bytes = bytes.len();
        self.tx
            .try_send(QueuedFrame {
                id,
                bytes,
                written_tx,
            })
            .map_err(|_| OutboundFull { id })?;
        state.frames += 1;
        state.bytes += frame_bytes;
        Ok(())
    }

    /// Release the byte permit after the writer dequeued the frame.
    pub fn mark_written(&self, frame_bytes: usize) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.frames = state.frames.saturating_sub(1);
        state.bytes = state.bytes.saturating_sub(frame_bytes);
    }
}

/// Writer-side error report: which outbound frame failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriterError {
    pub frame_id: OutboundFrameId,
    pub message: String,
}
