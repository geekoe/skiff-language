//! Bounded exponential backoff retry for infrastructure-only failures.
//!
//! C-router-activation-state §5: only transient driver/connection/backoff/write
//! conflict errors are retried; `CasMismatch` and `InvalidRecord` are returned
//! immediately (retrying can only mask a concurrent conflict). Retries are
//! bounded by attempt count and total deadline; the outcome (attempts, backoff,
//! remaining deadline) feeds repository health.

use std::{
    future::Future,
    pin::Pin,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::error::RepositoryError;

pub trait ActivationClock: Send + Sync {
    fn now_millis(&self) -> i64;
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl ActivationClock for SystemClock {
    fn now_millis(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep(duration))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub total_deadline: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_delay: Duration::from_millis(25),
            max_delay: Duration::from_millis(500),
            total_deadline: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryOutcome {
    pub attempts: u32,
    pub retried: u32,
    pub next_backoff_ms: u64,
    pub deadline_remaining_ms: Option<i64>,
}

impl RetryPolicy {
    pub async fn run<F, Fut, T>(
        &self,
        clock: &dyn ActivationClock,
        mut operation: F,
    ) -> (Result<T, RepositoryError>, RetryOutcome)
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, RepositoryError>>,
    {
        let started = clock.now_millis();
        let deadline = started + i64::try_from(self.total_deadline.as_millis()).unwrap_or(i64::MAX);
        let mut attempts = 0u32;
        let mut retried = 0u32;
        let mut backoff = self.base_delay;
        loop {
            attempts += 1;
            let result = operation().await;
            match result {
                Ok(value) => {
                    return (
                        Ok(value),
                        self.outcome(clock, attempts, retried, backoff, deadline),
                    )
                }
                Err(error) if error.is_retryable() => {
                    if attempts >= self.max_attempts || clock.now_millis() >= deadline {
                        return (
                            Err(error),
                            self.outcome(clock, attempts, retried, backoff, deadline),
                        );
                    }
                    clock.sleep(backoff).await;
                    retried += 1;
                    backoff = backoff.saturating_mul(2).min(self.max_delay);
                }
                Err(error) => {
                    return (
                        Err(error),
                        self.outcome(clock, attempts, retried, backoff, deadline),
                    )
                }
            }
        }
    }

    fn outcome(
        &self,
        clock: &dyn ActivationClock,
        attempts: u32,
        retried: u32,
        next_backoff: Duration,
        deadline_millis: i64,
    ) -> RetryOutcome {
        let remaining = deadline_millis - clock.now_millis();
        RetryOutcome {
            attempts,
            retried,
            next_backoff_ms: next_backoff.as_millis() as u64,
            deadline_remaining_ms: (remaining > 0).then_some(remaining),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicI64, AtomicU32, Ordering},
        Arc,
    };

    use super::*;
    use crate::activation::error::cas_mismatch;

    #[derive(Debug, Default)]
    struct FakeClock {
        now: AtomicI64,
        slept: std::sync::Mutex<Vec<Duration>>,
    }

    impl FakeClock {
        fn advance(&self, millis: i64) {
            self.now.fetch_add(millis, Ordering::SeqCst);
        }

        fn slept(&self) -> Vec<Duration> {
            self.slept.lock().expect("slept lock").clone()
        }
    }

    impl ActivationClock for FakeClock {
        fn now_millis(&self) -> i64 {
            self.now.load(Ordering::SeqCst)
        }

        fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.slept.lock().expect("slept lock").push(duration);
            self.now
                .fetch_add(duration.as_millis() as i64, Ordering::SeqCst);
            Box::pin(async {})
        }
    }

    async fn run_with_failures(
        failures: u32,
        max_attempts: u32,
        base_ms: u64,
    ) -> (
        Result<u32, RepositoryError>,
        RetryOutcome,
        Vec<Duration>,
        Arc<FakeClock>,
    ) {
        let clock = Arc::new(FakeClock::default());
        let policy = RetryPolicy {
            max_attempts,
            base_delay: Duration::from_millis(base_ms),
            max_delay: Duration::from_millis(base_ms * 4),
            total_deadline: Duration::from_secs(60),
        };
        let failed = Arc::new(AtomicU32::new(0));
        let (result, outcome) = policy
            .run(clock.as_ref(), {
                let failed = failed.clone();
                move || {
                    let failed = failed.clone();
                    async move {
                        let attempt = failed.fetch_add(1, Ordering::SeqCst) + 1;
                        if attempt <= failures {
                            Err(RepositoryError::Transient {
                                message: format!("injected failure {attempt}"),
                            })
                        } else {
                            Ok(42u32)
                        }
                    }
                }
            })
            .await;
        (result, outcome, clock.slept(), clock)
    }

    #[tokio::test]
    async fn transient_failures_are_retried_with_exponential_backoff() {
        let (result, outcome, sleeps, clock) = run_with_failures(2, 4, 25).await;
        assert_eq!(result.expect("eventual success"), 42);
        assert_eq!(outcome.attempts, 3);
        assert_eq!(outcome.retried, 2);
        assert_eq!(outcome.next_backoff_ms, 100);
        assert_eq!(
            sleeps,
            vec![Duration::from_millis(25), Duration::from_millis(50)]
        );
        clock.advance(5);
        assert!(outcome.deadline_remaining_ms.unwrap() > 0);
    }

    #[tokio::test]
    async fn max_attempts_is_bounded() {
        let (result, outcome, sleeps, _) = run_with_failures(99, 3, 10).await;
        assert!(matches!(result, Err(RepositoryError::Transient { .. })));
        assert_eq!(outcome.attempts, 3);
        assert_eq!(outcome.retried, 2);
        assert_eq!(sleeps.len(), 2);
    }

    #[tokio::test]
    async fn cas_and_invalid_errors_are_never_retried() {
        let clock = FakeClock::default();
        let policy = RetryPolicy::default();
        let calls = Arc::new(AtomicU32::new(0));
        let (result, outcome) = policy
            .run(&clock, {
                let calls = calls.clone();
                move || {
                    let calls = calls.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Err::<u32, _>(cas_mismatch("test", "stale generation"))
                    }
                }
            })
            .await;
        assert!(matches!(result, Err(RepositoryError::CasMismatch { .. })));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(outcome.attempts, 1);
        assert_eq!(outcome.retried, 0);
        assert!(clock.slept().is_empty());
    }

    #[tokio::test]
    async fn total_deadline_stops_retries() {
        let clock = Arc::new(FakeClock::default());
        let policy = RetryPolicy {
            max_attempts: 100,
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(10),
            total_deadline: Duration::from_millis(25),
        };
        let calls = Arc::new(AtomicU32::new(0));
        let (result, outcome) = policy
            .run(clock.as_ref(), {
                let calls = calls.clone();
                move || {
                    let calls = calls.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Err::<u32, _>(RepositoryError::Transient {
                            message: "deadline test".to_string(),
                        })
                    }
                }
            })
            .await;
        assert!(matches!(result, Err(RepositoryError::Transient { .. })));
        // First attempt at t=0, sleeps 10ms twice (t=10, t=20), third attempt
        // at t=20 still before the 25ms deadline, then sleep to t=30: the
        // fourth attempt observes the deadline and stops.
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert_eq!(outcome.attempts, 4);
        assert_eq!(outcome.retried, 3);
    }
}
