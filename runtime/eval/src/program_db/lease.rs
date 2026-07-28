use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use tokio::{sync::watch, task::JoinHandle};

use crate::capabilities::{DbCapabilityLeaseHold, DbCapabilityStore};

/// Owns the only lease-renew task. Normal terminals signal and join it; every
/// other drop path synchronously aborts the task so it can never detach.
pub(super) struct LeaseRenewOwner {
    stop: Option<watch::Sender<bool>>,
    task: Option<JoinHandle<()>>,
}

impl LeaseRenewOwner {
    pub(super) fn start(
        store: DbCapabilityStore,
        hold: DbCapabilityLeaseHold,
        period: Duration,
        request_cancelled: Arc<AtomicBool>,
    ) -> Self {
        let (stop, mut stopped) = watch::channel(false);
        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(period);
            loop {
                tokio::select! {
                    biased;
                    changed = stopped.changed() => {
                        if changed.is_err() || *stopped.borrow() {
                            break;
                        }
                    }
                    _ = interval.tick() => {
                        if !handle_renew_result(
                            store.renew_lease(&hold).await,
                            request_cancelled.as_ref(),
                        ) {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            stop: Some(stop),
            task: Some(task),
        }
    }

    pub(super) async fn stop_and_join(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(true);
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for LeaseRenewOwner {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn handle_renew_result<E>(
    result: std::result::Result<bool, E>,
    request_cancelled: &AtomicBool,
) -> bool {
    match result {
        Ok(true) => true,
        Ok(false) | Err(_) => {
            request_cancelled.store(true, Ordering::SeqCst);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use tokio::sync::{oneshot, watch};

    use crate::error::RuntimeError;

    use super::{handle_renew_result, LeaseRenewOwner};

    #[test]
    fn renew_failure_requests_internal_stop() {
        let stopped = AtomicBool::new(false);
        assert!(!handle_renew_result(
            Err(RuntimeError::Decode("renew failed".to_string())),
            &stopped,
        ));
        assert!(stopped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn db_actor_lease_owner_drop_aborts_renew_task() {
        struct DropSignal(Option<oneshot::Sender<()>>);

        impl Drop for DropSignal {
            fn drop(&mut self) {
                if let Some(signal) = self.0.take() {
                    let _ = signal.send(());
                }
            }
        }

        let (stop, _stopped) = watch::channel(false);
        let (started_tx, started_rx) = oneshot::channel();
        let (dropped_tx, dropped_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _drop_signal = DropSignal(Some(dropped_tx));
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        let owner = LeaseRenewOwner {
            stop: Some(stop),
            task: Some(task),
        };

        started_rx.await.expect("renew task should start");
        drop(owner);
        dropped_rx
            .await
            .expect("owner drop must synchronously request task abort");
    }

    #[tokio::test]
    async fn normal_stop_signals_and_joins_renew_task() {
        let (stop, mut stopped) = watch::channel(false);
        let (joined_tx, joined_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            stopped.changed().await.expect("stop owner remains live");
            assert!(*stopped.borrow());
            let _ = joined_tx.send(());
        });
        let owner = LeaseRenewOwner {
            stop: Some(stop),
            task: Some(task),
        };

        owner.stop_and_join().await;
        joined_rx.await.expect("renew task must be joined");
    }
}
