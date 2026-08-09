use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use tokio::sync::Notify;

use crate::attempt::LoadAttempt;
use crate::{DeploymentImageCache, LoadAttemptId};

use super::{image, join, owner, within, TestProviderError};

#[tokio::test]
async fn concurrent_success_runs_loader_once_and_shares_image() {
    let cache = DeploymentImageCache::<String, TestProviderError>::new();
    let requested_owner = owner("build:shared-success");
    let calls = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let mut callers = Vec::new();

    for _ in 0..32 {
        let cache = cache.clone();
        let requested_owner = requested_owner.clone();
        let calls = Arc::clone(&calls);
        let started = Arc::clone(&started);
        let release = Arc::clone(&release);
        callers.push(tokio::spawn(async move {
            cache
                .get_or_load(requested_owner, move |_, owner| async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    started.notify_one();
                    release.notified().await;
                    Ok(image(&owner, "shared-program"))
                })
                .await
                .expect("shared load must succeed")
        }));
    }

    within(started.notified()).await;
    release.notify_one();
    let mut images = Vec::new();
    for caller in callers {
        images.push(join(caller).await);
    }

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(images
        .iter()
        .all(|candidate| Arc::ptr_eq(candidate, &images[0])));
    let loaded = cache
        .loaded(&requested_owner)
        .await
        .expect("the exact owner is accepted")
        .expect("successful image is published");
    assert!(Arc::ptr_eq(&loaded, &images[0]));
}

#[tokio::test]
async fn stored_completion_is_observed_without_a_wake() {
    let requested_owner = owner("build:precompleted");
    let attempt = LoadAttempt::<String, TestProviderError>::new(
        LoadAttemptId::new(9),
        requested_owner.clone(),
    );
    let expected = image(&requested_owner, "precompleted");

    let stored = attempt.store(Ok(Arc::clone(&expected))).await;
    assert!(Arc::ptr_eq(
        stored.as_ref().expect("stored success"),
        &expected
    ));
    let observed = within(attempt.wait())
        .await
        .expect("stored success must be visible");
    assert!(Arc::ptr_eq(&observed, &expected));
}

#[tokio::test]
async fn cancelling_leader_does_not_cancel_the_load_attempt() {
    let cache = DeploymentImageCache::<String, TestProviderError>::new();
    let requested_owner = owner("build:cancelled-leader");
    let calls = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());

    let leader = {
        let cache = cache.clone();
        let requested_owner = requested_owner.clone();
        let calls = Arc::clone(&calls);
        let started = Arc::clone(&started);
        let release = Arc::clone(&release);
        tokio::spawn(async move {
            cache
                .get_or_load(requested_owner, move |_, owner| async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    started.notify_one();
                    release.notified().await;
                    Ok(image(&owner, "survived-cancellation"))
                })
                .await
        })
    };

    within(started.notified()).await;
    leader.abort();
    let leader_error = within(leader)
        .await
        .expect_err("leader caller is cancelled");
    assert!(leader_error.is_cancelled());

    let waiter = {
        let cache = cache.clone();
        let requested_owner = requested_owner.clone();
        tokio::spawn(async move {
            cache
                .get_or_load(requested_owner, |_, _| async {
                    Err(TestProviderError("joined loader must not execute"))
                })
                .await
                .expect("detached loader must still publish")
        })
    };
    tokio::task::yield_now().await;
    release.notify_one();
    let loaded = join(waiter).await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(loaded.program().as_str(), "survived-cancellation");
}

#[tokio::test]
async fn different_builds_load_independently_and_snapshot_in_build_order() {
    let cache = DeploymentImageCache::<String, TestProviderError>::new();
    let z_owner = owner("build:z");
    let a_owner = owner("build:a");
    let z_started = Arc::new(Notify::new());
    let a_started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());

    let z_load = spawn_blocked_success(
        cache.clone(),
        z_owner,
        Arc::clone(&z_started),
        Arc::clone(&release),
    );
    let a_load = spawn_blocked_success(
        cache.clone(),
        a_owner,
        Arc::clone(&a_started),
        Arc::clone(&release),
    );

    within(z_started.notified()).await;
    within(a_started.notified()).await;
    release.notify_waiters();
    join(z_load).await;
    join(a_load).await;

    let snapshot = cache.loaded_snapshot().await;
    let build_ids = snapshot
        .iter()
        .map(|image| image.owner().build_id().as_str())
        .collect::<Vec<_>>();
    assert_eq!(build_ids, ["build:a", "build:z"]);
}

fn spawn_blocked_success(
    cache: DeploymentImageCache<String, TestProviderError>,
    requested_owner: crate::DeploymentOwnerIdentity,
    started: Arc<Notify>,
    release: Arc<Notify>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        cache
            .get_or_load(requested_owner, move |_, owner| async move {
                started.notify_one();
                release.notified().await;
                Ok(image(&owner, owner.build_id().as_str()))
            })
            .await
            .expect("independent build must load");
    })
}
