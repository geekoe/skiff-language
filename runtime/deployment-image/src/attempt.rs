use std::sync::Arc;

use tokio::sync::{Mutex, Notify};

use crate::{DeploymentLoadFailure, DeploymentOwnerIdentity, LoadAttemptId};

pub(crate) type SharedAttemptResult<P, E> = Result<Arc<P>, Arc<DeploymentLoadFailure<E>>>;

pub(crate) struct LoadAttempt<P, E> {
    id: LoadAttemptId,
    owner: DeploymentOwnerIdentity,
    completion: Mutex<Option<SharedAttemptResult<P, E>>>,
    notify: Notify,
}

impl<P, E> LoadAttempt<P, E> {
    pub(crate) fn new(id: LoadAttemptId, owner: DeploymentOwnerIdentity) -> Self {
        Self {
            id,
            owner,
            completion: Mutex::new(None),
            notify: Notify::new(),
        }
    }

    pub(crate) fn id(&self) -> LoadAttemptId {
        self.id
    }

    pub(crate) fn owner(&self) -> &DeploymentOwnerIdentity {
        &self.owner
    }

    pub(crate) async fn wait(&self) -> SharedAttemptResult<P, E> {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            if let Some(result) = self.completion.lock().await.as_ref() {
                return clone_result(result);
            }
            notified.await;
        }
    }

    pub(crate) async fn store(
        &self,
        result: SharedAttemptResult<P, E>,
    ) -> SharedAttemptResult<P, E> {
        let mut completion = self.completion.lock().await;
        if let Some(stored) = completion.as_ref() {
            return clone_result(stored);
        }
        *completion = Some(clone_result(&result));
        result
    }

    pub(crate) fn notify_waiters(&self) {
        self.notify.notify_waiters();
    }
}

pub(crate) fn clone_result<P, E>(result: &SharedAttemptResult<P, E>) -> SharedAttemptResult<P, E> {
    match result {
        Ok(image) => Ok(Arc::clone(image)),
        Err(failure) => Err(Arc::clone(failure)),
    }
}
