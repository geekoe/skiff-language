use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::Poll,
    time::Duration,
};

use tokio::sync::Notify;

const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(1);

#[derive(Clone, Debug)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
    notify: Option<Arc<Notify>>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            notify: Some(Arc::new(Notify::new())),
        }
    }

    pub fn from_flag(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            cancelled,
            notify: None,
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(notify) = &self.notify {
            notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }

    pub async fn wait_cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }
            let Some(notify) = &self.notify else {
                tokio::time::sleep(CANCEL_POLL_INTERVAL).await;
                continue;
            };
            let notified = notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            tokio::select! {
                _ = &mut notified => {}
                _ = tokio::time::sleep(CANCEL_POLL_INTERVAL) => {}
            }
        }
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
            Self::BorrowedFlag(cancelled) => cancelled.load(Ordering::SeqCst),
            Self::Token(token) => token.is_cancelled(),
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
        while !self.is_cancelled() {
            tokio::select! {
                _ = wait_for_any_token(&self.signals), if self.signals.iter().any(RequestAbortSignal::is_token) => {},
                _ = tokio::time::sleep(CANCEL_POLL_INTERVAL) => {},
            }
        }
    }
}

impl RequestAbortSignal<'_> {
    fn is_token(&self) -> bool {
        matches!(self, Self::Token(_))
    }
}

async fn wait_for_any_token(signals: &[RequestAbortSignal<'_>]) {
    let mut notified = signals
        .iter()
        .filter_map(|signal| match signal {
            RequestAbortSignal::BorrowedFlag(_) => None,
            RequestAbortSignal::Token(token) => token
                .notify
                .as_ref()
                .map(|notify| Box::pin(notify.notified())),
        })
        .collect::<Vec<_>>();
    for notified in &mut notified {
        notified.as_mut().enable();
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

impl CancellationSignals<'static> {
    pub fn from_tokens(tokens: impl IntoIterator<Item = CancellationToken>) -> Self {
        Self::from_signals(tokens.into_iter().map(RequestAbortSignal::from_token))
    }

    pub fn from_flags(cancelled: impl IntoIterator<Item = Arc<AtomicBool>>) -> Self {
        Self::from_tokens(cancelled.into_iter().map(CancellationToken::from_flag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    async fn token_waits_for_flag_backed_cancel() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let token = CancellationToken::from_flag(cancelled.clone());
        let waiter = tokio::spawn(async move { token.wait_cancelled().await });

        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        cancelled.store(true, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("flag-backed cancellation should be polled")
            .expect("wait task should succeed");
    }

    #[tokio::test]
    async fn signals_wait_for_borrowed_flag() {
        let cancelled = AtomicBool::new(false);
        let signals = CancellationSignals::from_borrowed_flag(Some(&cancelled));

        assert!(!signals.is_cancelled());
        cancelled.store(true, Ordering::SeqCst);

        tokio::time::timeout(Duration::from_secs(1), signals.wait_cancelled())
            .await
            .expect("borrowed flag should wake through polling");
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
}
