use std::{future::Future, sync::Arc};

use tokio::{runtime::Handle, sync::Mutex, task::JoinError};

use crate::attempt::{LoadAttempt, SharedAttemptResult};
use crate::state::{BeginLoad, CacheState};
use crate::{
    DeploymentImage, DeploymentLoadError, DeploymentLoadFailure, DeploymentLoadResult,
    DeploymentOwnerConflict, DeploymentOwnerIdentity, LoadAttemptId,
};

/// Exact-build cache for immutable deployment images.
pub struct DeploymentImageCache<P, E> {
    inner: Arc<CacheInner<P, E>>,
}

struct CacheInner<P, E> {
    // Tokio mutexes are deliberately non-poisoning. A loader panic is isolated
    // in its own task and cannot make cache bookkeeping recover optimistically.
    state: Mutex<CacheState<P, E>>,
}

impl<P, E> DeploymentImageCache<P, E> {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(CacheInner {
                state: Mutex::new(CacheState::new()),
            }),
        }
    }

    pub async fn loaded(
        &self,
        owner: &DeploymentOwnerIdentity,
    ) -> Result<Option<Arc<DeploymentImage<P>>>, DeploymentOwnerConflict> {
        self.inner.state.lock().await.loaded(owner)
    }

    pub async fn loaded_snapshot(&self) -> Box<[Arc<DeploymentImage<P>>]> {
        self.inner.state.lock().await.loaded_snapshot()
    }
}

impl<P, E> Default for DeploymentImageCache<P, E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P, E> DeploymentImageCache<P, E>
where
    P: Send + Sync + 'static,
    E: Send + Sync + 'static,
{
    pub async fn get_or_load<L, F>(
        &self,
        owner: DeploymentOwnerIdentity,
        loader: L,
    ) -> DeploymentLoadResult<P, E>
    where
        L: FnOnce(LoadAttemptId, DeploymentOwnerIdentity) -> F + Send + 'static,
        F: Future<Output = Result<Arc<DeploymentImage<P>>, E>> + Send + 'static,
    {
        let runtime = Handle::try_current().ok();
        let begin = {
            let mut state = self.inner.state.lock().await;
            state.begin_load(owner)?
        };

        let attempt = match begin {
            BeginLoad::Loaded(image) => return Ok(image),
            BeginLoad::Join(attempt) => attempt,
            BeginLoad::Start(attempt) => {
                let Some(runtime) = runtime else {
                    let failure = DeploymentLoadFailure::runtime_unavailable(attempt.id());
                    return self
                        .inner
                        .finish_attempt(attempt, Err(failure))
                        .await
                        .map_err(DeploymentLoadError::Attempt);
                };
                spawn_loader(
                    runtime,
                    Arc::clone(&self.inner),
                    Arc::clone(&attempt),
                    loader,
                );
                attempt
            }
        };

        attempt.wait().await.map_err(DeploymentLoadError::Attempt)
    }
}

impl<P, E> Clone for DeploymentImageCache<P, E> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<P, E> CacheInner<P, E> {
    async fn finish_attempt(
        &self,
        attempt: Arc<LoadAttempt<P, E>>,
        result: SharedAttemptResult<P, E>,
    ) -> SharedAttemptResult<P, E> {
        let mut state = self.state.lock().await;
        let result = if state.is_current(&attempt) {
            result
        } else {
            Err(DeploymentLoadFailure::attempt_state_unavailable(
                attempt.id(),
            ))
        };
        let stored = attempt.store(result).await;
        state.publish_attempt(&attempt, &stored);
        drop(state);
        attempt.notify_waiters();
        stored
    }
}

fn spawn_loader<P, E, L, F>(
    runtime: Handle,
    cache: Arc<CacheInner<P, E>>,
    attempt: Arc<LoadAttempt<P, E>>,
    loader: L,
) where
    P: Send + Sync + 'static,
    E: Send + Sync + 'static,
    L: FnOnce(LoadAttemptId, DeploymentOwnerIdentity) -> F + Send + 'static,
    F: Future<Output = Result<Arc<DeploymentImage<P>>, E>> + Send + 'static,
{
    let attempt_id = attempt.id();
    let expected_owner = attempt.owner().clone();
    let loader_owner = expected_owner.clone();
    let load_task = runtime.spawn(async move { loader(attempt_id, loader_owner).await });
    drop(runtime.spawn(async move {
        let result = loader_result(attempt_id, expected_owner, load_task.await);
        drop(cache.finish_attempt(attempt, result).await);
    }));
}

fn loader_result<P, E>(
    attempt_id: LoadAttemptId,
    expected_owner: DeploymentOwnerIdentity,
    task_result: Result<Result<Arc<DeploymentImage<P>>, E>, JoinError>,
) -> SharedAttemptResult<P, E> {
    match task_result {
        Ok(Ok(image)) if image.owner() == &expected_owner => Ok(image),
        Ok(Ok(image)) => Err(DeploymentLoadFailure::output_owner_mismatch(
            attempt_id,
            expected_owner,
            image.owner().clone(),
        )),
        Ok(Err(error)) => Err(DeploymentLoadFailure::provider(attempt_id, error)),
        Err(error) if error.is_panic() => {
            Err(DeploymentLoadFailure::loader_task_panicked(attempt_id))
        }
        Err(_) => Err(DeploymentLoadFailure::loader_task_cancelled(attempt_id)),
    }
}
