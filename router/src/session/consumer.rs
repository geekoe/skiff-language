//! Static session-keyed consumer manifest, reserved terminal delivery and the
//! ACK barrier contract (C-session §5, authority design §3.6).
//!
//! The manifest is generated once per process composition. New capabilities
//! join the manifest plus tests plus the component together through a process
//! restart; the manifest is never extended at runtime. Uninstalled or
//! stateless sinks hold no permit.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use super::identity::RuntimeSessionEpoch;

/// Installed session-keyed component descriptors from the final composition
/// (C-session §5.1). Only components actually installed in this process hold
/// permits; W-session installs `HealthLedger` by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConsumerKind {
    AdmissionPool,
    HealthLedger,
    RequestDispatcher,
    WebSocketRequestBroker,
    ActorSessionOwner,
}

impl ConsumerKind {
    pub const ALL: [Self; 5] = [
        Self::AdmissionPool,
        Self::HealthLedger,
        Self::RequestDispatcher,
        Self::WebSocketRequestBroker,
        Self::ActorSessionOwner,
    ];
}

/// Terminal frame delivered to every installed session-keyed consumer before
/// the close barrier waits for ACKs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSessionClosed {
    pub session_epoch: RuntimeSessionEpoch,
}

/// A consumer that holds session-keyed state and must clean it up
/// idempotently on session close (exact-fence by session epoch).
pub trait SessionConsumer: Send + Sync + fmt::Debug {
    fn kind(&self) -> ConsumerKind;

    /// Idempotent exact-fence cleanup. `Err` means terminal delivery/cleanup
    /// failed and the Router must fail-stop (C-session §3.2(3)).
    fn on_session_closed(&self, session: &RuntimeSessionEpoch) -> Result<(), String>;
}

/// Static consumer manifest. Every installed session-keyed component must be
/// present; the checker compares manifest against the actually registered
/// consumers at `SessionLayer` construction.
#[derive(Debug, Clone)]
pub struct ConsumerManifest {
    installed: BTreeSet<ConsumerKind>,
}

impl ConsumerManifest {
    pub fn installed(consumers: impl IntoIterator<Item = ConsumerKind>) -> Self {
        Self {
            installed: consumers.into_iter().collect(),
        }
    }

    /// W-session composition: only the health ledger is installed in this
    /// batch. Each future lane atomically extends manifest + component + tests.
    pub fn default_installed() -> Self {
        Self::installed([ConsumerKind::HealthLedger])
    }

    pub fn contains(&self, kind: ConsumerKind) -> bool {
        self.installed.contains(&kind)
    }

    pub fn kinds(&self) -> impl Iterator<Item = ConsumerKind> + '_ {
        self.installed.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.installed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.installed.is_empty()
    }
}

/// Reserved terminal slot failure: the consumer mailbox could not accept
/// `RuntimeSessionClosed` without blocking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalDeliveryError {
    pub consumer: ConsumerKind,
    pub session: RuntimeSessionEpoch,
}

enum ConsumerTerminal {
    SessionClosed {
        session: RuntimeSessionEpoch,
        ack_tx: oneshot::Sender<Result<(), String>>,
    },
}

/// One bounded terminal mailbox per installed consumer. The channel capacity
/// is a dedicated reserved terminal slot: data-capacity saturation never
/// blocks terminal delivery, and delivery is non-blocking (`try_send`).
pub(crate) struct ConsumerMailbox {
    kind: ConsumerKind,
    terminal_tx: mpsc::Sender<ConsumerTerminal>,
    task: JoinHandle<()>,
}

impl fmt::Debug for ConsumerMailbox {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConsumerMailbox")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl ConsumerMailbox {
    pub(crate) fn spawn(consumer: Arc<dyn SessionConsumer>, capacity: usize) -> Self {
        let kind = consumer.kind();
        // The terminal lane is reserved with capacity for the maximum number
        // of concurrently closing sessions (runtime.maxConcurrency), so
        // simultaneous close of the full session set never blocks terminal
        // delivery. A stuck consumer still exhausts the lane and fail-stops.
        let (terminal_tx, mut terminal_rx) = mpsc::channel::<ConsumerTerminal>(capacity.max(1));
        let task = tokio::spawn(async move {
            while let Some(message) = terminal_rx.recv().await {
                let ConsumerTerminal::SessionClosed { session, ack_tx } = message;
                let result = consumer.on_session_closed(&session);
                let _ = ack_tx.send(result);
            }
        });
        Self {
            kind,
            terminal_tx,
            task,
        }
    }

    /// Non-blocking delivery through the reserved terminal slot.
    pub(crate) fn try_deliver_terminal(
        &self,
        session: &RuntimeSessionEpoch,
    ) -> Result<oneshot::Receiver<Result<(), String>>, TerminalDeliveryError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.terminal_tx
            .try_send(ConsumerTerminal::SessionClosed {
                session: session.clone(),
                ack_tx,
            })
            .map_err(|_| TerminalDeliveryError {
                consumer: self.kind,
                session: session.clone(),
            })?;
        Ok(ack_rx)
    }

    pub(crate) fn abort(&self) {
        self.task.abort();
    }
}

/// Process fail-stop record. Delivery/ACK timeout or reserved slot failure
/// transitions the Router to non-zero exit; restart clears all ephemeral
/// session state (C-session §3.2(3)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailStop {
    pub reason: String,
}

impl fmt::Display for FailStop {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.reason)
    }
}

impl std::error::Error for FailStop {}
