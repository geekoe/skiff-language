mod cache_failure;
mod cache_success;
mod contracts;
mod entry;

use std::{
    fmt,
    future::Future,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};

use skiff_artifact_model::{DeploymentArtifactIdentity, DeploymentRevision, ServiceDeploymentRef};
use tokio::task::JoinHandle;

use crate::{
    DeploymentImage, DeploymentLoadError, DeploymentLoadFailure, DeploymentOwnerIdentity,
    DeploymentProgramFacts, ServiceDependencySlot,
};

const TEST_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, PartialEq, Eq)]
struct TestProviderError(&'static str);

impl fmt::Display for TestProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for TestProviderError {}

#[derive(Debug)]
struct TestProgram {
    owner: DeploymentOwnerIdentity,
    dependency_slots: Box<[ServiceDependencySlot]>,
    label: String,
}

impl TestProgram {
    fn label(&self) -> &str {
        &self.label
    }
}

impl DeploymentProgramFacts for TestProgram {
    fn owner(&self) -> &DeploymentOwnerIdentity {
        &self.owner
    }

    fn dependency_slots(&self) -> &[ServiceDependencySlot] {
        &self.dependency_slots
    }
}

fn owner(build_id: &str) -> DeploymentOwnerIdentity {
    owner_with(build_id, "consumer", "revision:consumer")
}

fn owner_with(build_id: &str, service_id: &str, revision: &str) -> DeploymentOwnerIdentity {
    DeploymentOwnerIdentity::new(ServiceDeploymentRef {
        service_id: service_id.to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new(revision),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(build_id),
    })
}

fn program(
    owner: DeploymentOwnerIdentity,
    label: &str,
    dependency_slots: impl IntoIterator<Item = ServiceDependencySlot>,
) -> Arc<TestProgram> {
    Arc::new(TestProgram {
        owner,
        dependency_slots: dependency_slots
            .into_iter()
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        label: label.to_string(),
    })
}

fn image(owner: &DeploymentOwnerIdentity, label: &str) -> Arc<DeploymentImage<TestProgram>> {
    Arc::new(
        DeploymentImage::try_new(program(owner.clone(), label, []))
            .expect("empty dependency set is valid"),
    )
}

fn attempt_failure(
    error: DeploymentLoadError<TestProviderError>,
) -> Arc<DeploymentLoadFailure<TestProviderError>> {
    match error {
        DeploymentLoadError::Attempt(failure) => failure,
        other => panic!("expected shared attempt failure, got {other:?}"),
    }
}

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(TEST_TIMEOUT, future)
        .await
        .expect("operation must not hang")
}

async fn join<T>(handle: JoinHandle<T>) -> T {
    within(handle).await.expect("spawned caller must not panic")
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn ready_without_runtime<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("no-runtime fixture unexpectedly became pending"),
    }
}
