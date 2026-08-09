use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc,
};

use tokio::sync::Notify;

use crate::{DeploymentImageCache, DeploymentLoadError, DeploymentLoadFailureReason};

use super::{
    attempt_failure, image, join, owner, owner_with, ready_without_runtime, within, TestProgram,
    TestProviderError,
};

#[test]
fn runtime_unavailable_is_attempt_scoped_and_retryable() {
    let cache = DeploymentImageCache::<TestProgram, TestProviderError>::new();
    let requested_owner = owner("build:no-runtime");
    let loader_calls = Arc::new(AtomicUsize::new(0));

    let first = ready_without_runtime(cache.get_or_load(requested_owner.clone(), {
        let loader_calls = Arc::clone(&loader_calls);
        move |_, _| async move {
            loader_calls.fetch_add(1, Ordering::SeqCst);
            Err(TestProviderError("loader must not run"))
        }
    }))
    .expect_err("starting without a runtime must fail closed");
    let first = attempt_failure(first);
    assert!(matches!(
        first.reason(),
        DeploymentLoadFailureReason::RuntimeUnavailable
    ));
    assert!(ready_without_runtime(cache.loaded(&requested_owner))
        .expect("the exact owner remains valid")
        .is_none());

    let second = ready_without_runtime(cache.get_or_load(requested_owner, {
        let loader_calls = Arc::clone(&loader_calls);
        move |_, _| async move {
            loader_calls.fetch_add(1, Ordering::SeqCst);
            Err(TestProviderError("loader must not run"))
        }
    }))
    .expect_err("a retry without a runtime is a new failed attempt");
    let second = attempt_failure(second);

    assert!(matches!(
        second.reason(),
        DeploymentLoadFailureReason::RuntimeUnavailable
    ));
    assert_eq!(second.attempt_id().get(), first.attempt_id().get() + 1);
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(loader_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn concurrent_failure_waiters_share_one_failure_arc() {
    let cache = DeploymentImageCache::<TestProgram, TestProviderError>::new();
    let requested_owner = owner("build:shared-failure");
    let calls = Arc::new(AtomicUsize::new(0));
    let unexpected_calls = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let mut callers = Vec::new();

    callers.push({
        let cache = cache.clone();
        let requested_owner = requested_owner.clone();
        let calls = Arc::clone(&calls);
        let started = Arc::clone(&started);
        let release = Arc::clone(&release);
        tokio::spawn(async move {
            cache
                .get_or_load(requested_owner, move |_, _| async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    started.notify_one();
                    release.notified().await;
                    Err(TestProviderError("provider failed"))
                })
                .await
        })
    });
    within(started.notified()).await;

    for _ in 1..32 {
        let cache = cache.clone();
        let requested_owner = requested_owner.clone();
        let unexpected_calls = Arc::clone(&unexpected_calls);
        callers.push(tokio::spawn(async move {
            cache
                .get_or_load(requested_owner, move |_, _| async move {
                    unexpected_calls.fetch_add(1, Ordering::SeqCst);
                    Err(TestProviderError("unexpected retry"))
                })
                .await
        }));
        tokio::task::yield_now().await;
    }

    release.notify_one();
    let mut failures = Vec::new();
    for caller in callers {
        failures.push(attempt_failure(
            join(caller).await.expect_err("attempt must fail"),
        ));
    }

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(unexpected_calls.load(Ordering::SeqCst), 0);
    assert!(failures
        .iter()
        .all(|candidate| Arc::ptr_eq(candidate, &failures[0])));
    assert_eq!(failures[0].attempt_id().get(), 1);
    match failures[0].reason() {
        DeploymentLoadFailureReason::Provider { error } => {
            assert_eq!(error.0, "provider failed");
        }
        other => panic!("expected provider failure, got {other:?}"),
    }
}

#[tokio::test]
async fn failed_attempt_is_not_published_and_retry_uses_next_id() {
    let cache = DeploymentImageCache::<TestProgram, TestProviderError>::new();
    let requested_owner = owner("build:retry");
    let first = cache
        .get_or_load(requested_owner.clone(), |_, _| async {
            Err(TestProviderError("first attempt"))
        })
        .await
        .expect_err("first attempt fails");
    let first = attempt_failure(first);

    assert!(cache
        .loaded(&requested_owner)
        .await
        .expect("same owner is valid")
        .is_none());
    let observed_attempt = Arc::new(AtomicU64::new(0));
    let loaded = cache
        .get_or_load(requested_owner.clone(), {
            let observed_attempt = Arc::clone(&observed_attempt);
            move |attempt_id, owner| async move {
                observed_attempt.store(attempt_id.get(), Ordering::SeqCst);
                Ok(image(&owner, "retry-success"))
            }
        })
        .await
        .expect("retry succeeds");

    assert_eq!(first.attempt_id().get(), 1);
    assert_eq!(observed_attempt.load(Ordering::SeqCst), 2);
    assert_eq!(loaded.program().label(), "retry-success");
}

#[tokio::test]
async fn spoofed_owner_conflicts_without_joining_or_publishing() {
    let cache = DeploymentImageCache::<TestProgram, TestProviderError>::new();
    let expected_owner = owner_with("build:claimed", "alpha", "revision:alpha");
    let spoofed_owner = owner_with("build:claimed", "beta", "revision:beta");
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let spoofed_loads = Arc::new(AtomicUsize::new(0));

    let genuine = {
        let cache = cache.clone();
        let expected_owner = expected_owner.clone();
        let started = Arc::clone(&started);
        let release = Arc::clone(&release);
        tokio::spawn(async move {
            cache
                .get_or_load(expected_owner, move |_, owner| async move {
                    started.notify_one();
                    release.notified().await;
                    Ok(image(&owner, "genuine"))
                })
                .await
                .expect("genuine owner loads")
        })
    };
    within(started.notified()).await;

    let first_conflict =
        conflicting_load(&cache, spoofed_owner.clone(), Arc::clone(&spoofed_loads)).await;
    let second_conflict =
        conflicting_load(&cache, spoofed_owner.clone(), Arc::clone(&spoofed_loads)).await;
    assert_eq!(first_conflict, second_conflict);
    assert_eq!(first_conflict.clone(), first_conflict);
    assert_eq!(first_conflict.existing(), &expected_owner);
    assert_eq!(first_conflict.requested(), &spoofed_owner);
    assert_eq!(spoofed_loads.load(Ordering::SeqCst), 0);

    release.notify_one();
    let genuine = join(genuine).await;
    assert_eq!(genuine.program().label(), "genuine");
    let lookup_conflict = cache
        .loaded(&spoofed_owner)
        .await
        .expect_err("loaded lookup validates the full owner");
    assert_eq!(lookup_conflict, first_conflict);
}

#[tokio::test]
async fn mismatched_output_owner_is_shared_failure_and_not_cached() {
    let cache = DeploymentImageCache::<TestProgram, TestProviderError>::new();
    let expected_owner = owner_with("build:output-owner", "alpha", "revision:alpha");
    let actual_owner = owner_with("build:output-owner", "beta", "revision:beta");
    let failure = cache
        .get_or_load(expected_owner.clone(), {
            let actual_owner = actual_owner.clone();
            move |_, _| async move { Ok(image(&actual_owner, "wrong-owner")) }
        })
        .await
        .expect_err("mismatched output owner must fail");
    let failure = attempt_failure(failure);

    match failure.reason() {
        DeploymentLoadFailureReason::OutputOwnerMismatch { expected, actual } => {
            assert_eq!(expected, &expected_owner);
            assert_eq!(actual, &actual_owner);
        }
        other => panic!("expected owner mismatch, got {other:?}"),
    }
    assert!(cache
        .loaded(&expected_owner)
        .await
        .expect("canonical owner remains bound")
        .is_none());
    assert!(cache.loaded_snapshot().await.is_empty());
}

#[tokio::test]
async fn loader_task_panic_becomes_attempt_failure_and_retry_can_succeed() {
    let cache = DeploymentImageCache::<TestProgram, TestProviderError>::new();
    let requested_owner = owner("build:panic");
    let failure = within(cache.get_or_load(requested_owner.clone(), |_, _| panicking_loader()))
        .await
        .expect_err("panic is reported as a load failure");
    let failure = attempt_failure(failure);

    assert!(matches!(
        failure.reason(),
        DeploymentLoadFailureReason::LoaderTaskPanicked
    ));
    let retry = cache
        .get_or_load(requested_owner, |_, owner| async move {
            Ok(image(&owner, "after-panic"))
        })
        .await
        .expect("panic does not publish or wedge the key");
    assert_eq!(retry.program().label(), "after-panic");
}

async fn panicking_loader() -> Result<Arc<crate::DeploymentImage<TestProgram>>, TestProviderError> {
    panic!("loader panic fixture")
}

async fn conflicting_load(
    cache: &DeploymentImageCache<TestProgram, TestProviderError>,
    spoofed_owner: crate::DeploymentOwnerIdentity,
    calls: Arc<AtomicUsize>,
) -> crate::DeploymentOwnerConflict {
    match cache
        .get_or_load(spoofed_owner, move |_, _| async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(TestProviderError("spoofed loader ran"))
        })
        .await
        .expect_err("spoofed owner must conflict")
    {
        DeploymentLoadError::OwnerConflict(conflict) => conflict,
        other => panic!("expected owner conflict, got {other:?}"),
    }
}
