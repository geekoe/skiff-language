use std::{
    fmt,
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    task::Poll,
    time::Duration,
};

use tokio::sync::Notify;

const FLAG_BACKED_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(10);

static FLAG_BACKED_CANCEL_WAITERS_ACTIVE: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CancellationPollingFallbackAllowlistEntry {
    pub file: &'static str,
    pub function: &'static str,
    pub reason: &'static str,
    pub bound: &'static str,
    pub counter: &'static str,
    pub removal: &'static str,
}

pub const FLAG_BACKED_CANCELLATION_POLLING_FALLBACK_ALLOWLIST:
    &[CancellationPollingFallbackAllowlistEntry] = &[
    CancellationPollingFallbackAllowlistEntry {
        file: "runtime/capability-context/src/cancellation.rs",
        function: "CancellationToken::from_flag / CancellationSignals::from_flags",
        reason: "compatibility constructors for callers that can only provide Arc<AtomicBool>",
        bound: "one waiter per explicit legacy flag-backed wait",
        counter: "cancellation.flag_backed_waiters.active",
        removal: "replace remaining flag-only capability APIs with CancellationToken",
    },
    CancellationPollingFallbackAllowlistEntry {
        file: "runtime/host/src/capability_context/stream_runtime.rs",
        function: "pull_stream / next_with_cancel / send_with_cancel",
        reason: "legacy stream capability surface accepts cancel flag arrays",
        bound: "bounded by stream poll/send calls in the current request",
        counter: "cancellation.flag_backed_waiters.active",
        removal: "move stream capability adapters to CancellationSignals::from_tokens",
    },
    CancellationPollingFallbackAllowlistEntry {
        file: "runtime/host/src/host/http_runtime/request.rs",
        function: "request_inner",
        reason: "public std.http request helper accepts a borrowed AtomicBool",
        bound: "one HTTP request wait per capability invocation",
        counter: "cancellation.flag_backed_waiters.active",
        removal: "thread request CancellationToken into std.http request helpers",
    },
    CancellationPollingFallbackAllowlistEntry {
        file: "runtime/host/src/host/http_runtime/stream.rs",
        function: "open_stream_with_cancel_flags_and_options / open_body_stream_with_cancel_flags_and_options",
        reason: "HTTP stream compatibility helpers accept owned cancel flags",
        bound: "one HTTP stream wait per opened stream",
        counter: "cancellation.flag_backed_waiters.active",
        removal: "replace cancel flag helpers with token-backed stream constructors",
    },
    CancellationPollingFallbackAllowlistEntry {
        file: "runtime/host/src/host/http_runtime/sse.rs",
        function: "open_sse_with_cancel_flags_and_options",
        reason: "SSE compatibility helper accepts owned cancel flags",
        bound: "one SSE stream wait per opened stream",
        counter: "cancellation.flag_backed_waiters.active",
        removal: "replace cancel flag helpers with token-backed SSE constructors",
    },
    CancellationPollingFallbackAllowlistEntry {
        file: "runtime/host/src/capability_context/actor.rs",
        function: "wait_request_cancelled",
        reason: "actor context currently receives a borrowed request cancellation flag",
        bound: "one actor control RPC wait per actor operation",
        counter: "cancellation.flag_backed_waiters.active",
        removal: "thread request CancellationToken into ActorClientContext",
    },
];

#[derive(Clone, Debug)]
pub struct CancellationSource {
    inner: Arc<CancellationState>,
}

#[derive(Clone, Debug)]
pub struct CancellationToken {
    inner: CancellationTokenInner,
}

#[derive(Clone, Debug)]
pub struct CompletionSignal {
    completed: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

#[derive(Clone, Debug)]
enum CancellationTokenInner {
    NotifyBacked(Arc<CancellationState>),
    CompatibilityFlag(Arc<AtomicBool>),
}

struct CancellationState {
    cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl fmt::Debug for CancellationState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationState")
            .field("cancelled", &self.cancelled.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl CancellationSource {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(CancellationState {
                cancelled: Arc::new(AtomicBool::new(false)),
                notify: Arc::new(Notify::new()),
            }),
        }
    }

    pub fn token(&self) -> CancellationToken {
        CancellationToken {
            inner: CancellationTokenInner::NotifyBacked(self.inner.clone()),
        }
    }

    pub fn cancel(&self) {
        cancel_notify_backed(&self.inner);
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.inner.cancelled.clone()
    }
}

impl Default for CancellationSource {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        CancellationSource::new().token()
    }

    pub fn source() -> CancellationSource {
        CancellationSource::new()
    }

    /// Compatibility fallback for legacy callers that can only provide a flag.
    ///
    /// The returned token is not waitable by notification. `wait_cancelled` uses
    /// a bounded polling fallback and is tracked by
    /// `flag_backed_cancel_waiters_active`.
    pub fn from_flag(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            inner: CancellationTokenInner::CompatibilityFlag(cancelled),
        }
    }

    pub fn cancel(&self) {
        match &self.inner {
            CancellationTokenInner::NotifyBacked(state) => cancel_notify_backed(state),
            CancellationTokenInner::CompatibilityFlag(cancelled) => {
                cancelled.store(true, Ordering::Release);
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        match &self.inner {
            CancellationTokenInner::NotifyBacked(state) => state.cancelled.load(Ordering::Acquire),
            CancellationTokenInner::CompatibilityFlag(cancelled) => {
                cancelled.load(Ordering::Acquire)
            }
        }
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        match &self.inner {
            CancellationTokenInner::NotifyBacked(state) => state.cancelled.clone(),
            CancellationTokenInner::CompatibilityFlag(cancelled) => cancelled.clone(),
        }
    }

    pub async fn wait_cancelled(&self) {
        match &self.inner {
            CancellationTokenInner::NotifyBacked(state) => {
                wait_notify_backed_cancelled(state).await;
            }
            CancellationTokenInner::CompatibilityFlag(cancelled) => {
                let _guard = FlagBackedWaiterGuard::new();
                while !cancelled.load(Ordering::Acquire) {
                    tokio::time::sleep(FLAG_BACKED_CANCEL_POLL_INTERVAL).await;
                }
            }
        }
    }

    fn notify(&self) -> Option<&Notify> {
        match &self.inner {
            CancellationTokenInner::NotifyBacked(state) => Some(state.notify.as_ref()),
            CancellationTokenInner::CompatibilityFlag(_) => None,
        }
    }

    fn requires_polling_fallback(&self) -> bool {
        matches!(self.inner, CancellationTokenInner::CompatibilityFlag(_))
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Arc<AtomicBool>> for CancellationToken {
    fn from(cancelled: Arc<AtomicBool>) -> Self {
        Self::from_flag(cancelled)
    }
}

fn cancel_notify_backed(state: &CancellationState) {
    if state
        .cancelled
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        state.notify.notify_waiters();
    }
}

async fn wait_notify_backed_cancelled(state: &CancellationState) {
    loop {
        if state.cancelled.load(Ordering::Acquire) {
            return;
        }
        let notified = state.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if state.cancelled.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }
}

pub fn flag_backed_cancel_waiters_active() -> usize {
    FLAG_BACKED_CANCEL_WAITERS_ACTIVE.load(Ordering::Acquire)
}

struct FlagBackedWaiterGuard;

impl FlagBackedWaiterGuard {
    fn new() -> Self {
        FLAG_BACKED_CANCEL_WAITERS_ACTIVE.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for FlagBackedWaiterGuard {
    fn drop(&mut self) {
        FLAG_BACKED_CANCEL_WAITERS_ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

impl CompletionSignal {
    pub fn new() -> Self {
        Self {
            completed: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }

    pub fn mark_completed(&self) {
        if self
            .completed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.notify.notify_waiters();
        }
    }

    pub fn is_completed(&self) -> bool {
        self.completed.load(Ordering::Acquire)
    }

    pub async fn wait_completed(&self) {
        loop {
            if self.is_completed() {
                return;
            }
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_completed() {
                return;
            }
            notified.await;
        }
    }
}

impl Default for CompletionSignal {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub enum RequestAbortSignal<'a> {
    BorrowedFlag(&'a AtomicBool),
    Token(CancellationToken),
}

impl<'a> RequestAbortSignal<'a> {
    pub fn from_borrowed_flag(cancelled: &'a AtomicBool) -> Self {
        Self::BorrowedFlag(cancelled)
    }

    pub fn from_token(token: CancellationToken) -> Self {
        Self::Token(token)
    }

    pub fn is_cancelled(&self) -> bool {
        match self {
            Self::BorrowedFlag(cancelled) => cancelled.load(Ordering::Acquire),
            Self::Token(token) => token.is_cancelled(),
        }
    }

    fn is_notify_token(&self) -> bool {
        matches!(self, Self::Token(token) if token.notify().is_some())
    }

    fn requires_polling_fallback(&self) -> bool {
        match self {
            Self::BorrowedFlag(_) => true,
            Self::Token(token) => token.requires_polling_fallback(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CancellationSignals<'a> {
    signals: Vec<RequestAbortSignal<'a>>,
}

impl<'a> CancellationSignals<'a> {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn from_borrowed_flag(cancelled: Option<&'a AtomicBool>) -> Self {
        Self {
            signals: cancelled
                .map(RequestAbortSignal::from_borrowed_flag)
                .into_iter()
                .collect(),
        }
    }

    pub fn from_signals(signals: impl IntoIterator<Item = RequestAbortSignal<'a>>) -> Self {
        Self {
            signals: signals.into_iter().collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.signals.is_empty()
    }

    pub fn is_cancelled(&self) -> bool {
        self.signals.iter().any(RequestAbortSignal::is_cancelled)
    }

    pub async fn wait_cancelled(&self) {
        if self.signals.is_empty() {
            std::future::pending::<()>().await;
            return;
        }
        if self
            .signals
            .iter()
            .any(RequestAbortSignal::requires_polling_fallback)
        {
            let _guard = FlagBackedWaiterGuard::new();
            while !self.is_cancelled() {
                tokio::select! {
                    _ = wait_for_any_token(&self.signals), if self.signals.iter().any(RequestAbortSignal::is_notify_token) => {},
                    _ = tokio::time::sleep(FLAG_BACKED_CANCEL_POLL_INTERVAL) => {},
                }
            }
            return;
        }

        wait_for_any_token(&self.signals).await;
    }
}

async fn wait_for_any_token(signals: &[RequestAbortSignal<'_>]) {
    if !signals.iter().any(RequestAbortSignal::is_notify_token) {
        std::future::pending::<()>().await;
        return;
    }
    loop {
        if signals.iter().any(RequestAbortSignal::is_cancelled) {
            return;
        }
        let mut notified = signals
            .iter()
            .filter_map(|signal| match signal {
                RequestAbortSignal::BorrowedFlag(_) => None,
                RequestAbortSignal::Token(token) => {
                    token.notify().map(|notify| Box::pin(notify.notified()))
                }
            })
            .collect::<Vec<_>>();
        for notified in &mut notified {
            notified.as_mut().enable();
        }
        if signals.iter().any(RequestAbortSignal::is_cancelled) {
            return;
        }
        std::future::poll_fn(|context| {
            if signals.iter().any(RequestAbortSignal::is_cancelled) {
                return Poll::Ready(());
            }
            for notified in &mut notified {
                if notified.as_mut().poll(context).is_ready() {
                    return Poll::Ready(());
                }
            }
            Poll::Pending
        })
        .await;
    }
}

impl CancellationSignals<'static> {
    pub fn from_tokens(tokens: impl IntoIterator<Item = CancellationToken>) -> Self {
        Self::from_signals(tokens.into_iter().map(RequestAbortSignal::from_token))
    }

    /// Compatibility fallback for legacy flag-only cancellation adapters.
    pub fn from_flags(cancelled: impl IntoIterator<Item = Arc<AtomicBool>>) -> Self {
        Self::from_tokens(cancelled.into_iter().map(CancellationToken::from_flag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn wait_until_flag_backed_waiters(expected: usize) {
        tokio::time::timeout(Duration::from_secs(1), async move {
            while flag_backed_cancel_waiters_active() != expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("flag-backed waiter count should settle");
    }

    #[tokio::test]
    async fn token_waits_for_notify_backed_cancel() {
        let token = CancellationToken::new();
        let waiter = {
            let token = token.clone();
            tokio::spawn(async move { token.wait_cancelled().await })
        };

        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        token.cancel();
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("notify-backed cancellation should wake")
            .expect("wait task should succeed");
    }

    #[tokio::test]
    async fn pre_cancelled_token_wait_returns_immediately() {
        let token = CancellationToken::new();
        token.cancel();

        tokio::time::timeout(Duration::from_millis(50), token.wait_cancelled())
            .await
            .expect("pre-cancelled token should not wait");
    }

    #[tokio::test]
    async fn cancel_racing_with_waiter_registration_wakes() {
        for _ in 0..100 {
            let token = CancellationToken::new();
            let waiter = {
                let token = token.clone();
                tokio::spawn(async move { token.wait_cancelled().await })
            };
            tokio::task::yield_now().await;
            token.cancel();
            tokio::time::timeout(Duration::from_secs(1), waiter)
                .await
                .expect("racing cancellation should wake waiter")
                .expect("wait task should succeed");
        }
    }

    #[tokio::test]
    async fn cancel_wakes_multiple_waiters() {
        let token = CancellationToken::new();
        let waiters = (0..16)
            .map(|_| {
                let token = token.clone();
                tokio::spawn(async move { token.wait_cancelled().await })
            })
            .collect::<Vec<_>>();

        tokio::task::yield_now().await;
        token.cancel();

        for waiter in waiters {
            tokio::time::timeout(Duration::from_secs(1), waiter)
                .await
                .expect("cancel should wake every waiter")
                .expect("wait task should succeed");
        }
    }

    #[tokio::test]
    async fn token_waits_for_flag_backed_cancel_with_tracked_fallback() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let token = CancellationToken::from_flag(cancelled.clone());
        let waiter = tokio::spawn(async move { token.wait_cancelled().await });

        wait_until_flag_backed_waiters(1).await;

        cancelled.store(true, Ordering::Release);
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("flag-backed cancellation should be polled")
            .expect("wait task should succeed");
        wait_until_flag_backed_waiters(0).await;
    }

    #[tokio::test]
    async fn completion_signal_waits_for_mark_completed() {
        let completed = CompletionSignal::new();
        let waiter = {
            let completed = completed.clone();
            tokio::spawn(async move { completed.wait_completed().await })
        };

        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        completed.mark_completed();
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("completion signal should wake")
            .expect("wait task should succeed");
    }

    #[tokio::test]
    async fn signals_wait_for_borrowed_flag() {
        let cancelled = AtomicBool::new(false);
        let signals = CancellationSignals::from_borrowed_flag(Some(&cancelled));

        assert!(!signals.is_cancelled());
        cancelled.store(true, Ordering::Release);

        tokio::time::timeout(Duration::from_secs(1), signals.wait_cancelled())
            .await
            .expect("borrowed flag should wake through compatibility polling");
    }

    #[tokio::test]
    async fn signals_wait_for_notify_token_without_poll_delay() {
        let token = CancellationToken::new();
        let signals = CancellationSignals::from_tokens([token.clone()]);
        let waiter = tokio::spawn(async move { signals.wait_cancelled().await });

        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        token.cancel();
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("notify-backed token signal should wake")
            .expect("wait task should succeed");
    }

    #[tokio::test]
    async fn signal_set_wakes_when_any_token_cancelled() {
        let first = CancellationToken::new();
        let second = CancellationToken::new();
        let signals = CancellationSignals::from_tokens([first, second.clone()]);
        let waiter = tokio::spawn(async move { signals.wait_cancelled().await });

        tokio::task::yield_now().await;
        second.cancel();

        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("any token cancellation should wake signal set")
            .expect("wait task should succeed");
    }

    #[test]
    fn polling_fallback_allowlist_entries_have_counter_and_removal_owner() {
        assert!(!FLAG_BACKED_CANCELLATION_POLLING_FALLBACK_ALLOWLIST.is_empty());
        for entry in FLAG_BACKED_CANCELLATION_POLLING_FALLBACK_ALLOWLIST {
            assert_eq!(entry.counter, "cancellation.flag_backed_waiters.active");
            assert!(!entry.bound.is_empty());
            assert!(!entry.removal.is_empty());
        }
    }
}
