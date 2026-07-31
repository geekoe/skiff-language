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
                        let renew = store.renew_lease(&hold);
                        tokio::pin!(renew);
                        let keep_running = loop {
                            tokio::select! {
                                biased;
                                changed = stopped.changed() => {
                                    if changed.is_err() || *stopped.borrow() {
                                        break false;
                                    }
                                }
                                result = &mut renew => {
                                    break handle_renew_result(
                                        result,
                                        request_cancelled.as_ref(),
                                    );
                                }
                            }
                        };
                        if !keep_running {
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
mod tests;
