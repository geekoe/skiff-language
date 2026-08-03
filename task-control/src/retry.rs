//! Bounded backoff for transient store failures.
//!
//! Only `TaskStoreError::Transient` is retried; deterministic domain outcomes
//! are returned immediately so a CAS conflict is never masked by retry.

use std::future::Future;
use std::time::Duration;

use crate::clock::TaskClock;
use crate::error::TaskStoreError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub total_deadline: Duration,
}

impl Default for TaskRetryPolicy {
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
pub struct TaskRetryOutcome {
    pub attempts: u32,
    pub retried: u32,
}

impl TaskRetryPolicy {
    pub async fn run<F, Fut, T>(
        &self,
        clock: &dyn TaskClock,
        mut operation: F,
    ) -> (Result<T, TaskStoreError>, TaskRetryOutcome)
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, TaskStoreError>>,
    {
        let started = clock.now_millis();
        let deadline = started + i64::try_from(self.total_deadline.as_millis()).unwrap_or(i64::MAX);
        let mut attempts = 0u32;
        let mut retried = 0u32;
        let mut backoff = self.base_delay;
        loop {
            attempts += 1;
            match operation().await {
                Ok(value) => return (Ok(value), TaskRetryOutcome { attempts, retried }),
                Err(error) if error.is_retryable() => {
                    if attempts >= self.max_attempts || clock.now_millis() >= deadline {
                        return (Err(error), TaskRetryOutcome { attempts, retried });
                    }
                    tokio::time::sleep(backoff).await;
                    retried += 1;
                    backoff = backoff.saturating_mul(2).min(self.max_delay);
                }
                Err(error) => return (Err(error), TaskRetryOutcome { attempts, retried }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::SystemClock;

    #[test]
    fn deterministic_outcomes_are_not_retried() {
        let policy = TaskRetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
            total_deadline: Duration::from_secs(1),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");
        let result: (Result<u32, TaskStoreError>, TaskRetryOutcome) =
            runtime.block_on(policy.run(&SystemClock, || async {
                Err(TaskStoreError::CasMismatch {
                    task_id: "task-1".into(),
                    message: "stale".to_string(),
                })
            }));
        assert_eq!(result.1.attempts, 1);
        assert_eq!(result.1.retried, 0);
        assert!(matches!(result.0, Err(TaskStoreError::CasMismatch { .. })));
    }

    #[test]
    fn transient_failures_are_retried_bounded() {
        let attempts = AtomicU32::new(0);
        let policy = TaskRetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
            total_deadline: Duration::from_secs(1),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");
        let (result, outcome) = runtime.block_on(policy.run(&SystemClock, || {
            let attempts = &attempts;
            async move {
                if attempts.fetch_add(1, Ordering::SeqCst) < 2 {
                    Err(TaskStoreError::Transient {
                        message: "boom".to_string(),
                    })
                } else {
                    Ok(7u32)
                }
            }
        }));
        assert_eq!(result, Ok(7));
        assert_eq!(outcome.attempts, 3);
        assert_eq!(outcome.retried, 2);
    }
}
