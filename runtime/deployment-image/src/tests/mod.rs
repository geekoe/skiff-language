mod cache_failure;
mod cache_success;
mod contracts;

use std::{fmt, future::Future, sync::Arc, time::Duration};

use skiff_artifact_model::{DeploymentArtifactIdentity, DeploymentRevision, ServiceDeploymentRef};
use tokio::task::JoinHandle;

use crate::{DeploymentImage, DeploymentLoadError, DeploymentLoadFailure, DeploymentOwnerIdentity};

const TEST_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, PartialEq, Eq)]
struct TestProviderError(&'static str);

impl fmt::Display for TestProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for TestProviderError {}

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

fn image(owner: &DeploymentOwnerIdentity, program: &str) -> Arc<DeploymentImage<String>> {
    Arc::new(
        DeploymentImage::try_new(owner.clone(), Arc::new(program.to_string()), [])
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
