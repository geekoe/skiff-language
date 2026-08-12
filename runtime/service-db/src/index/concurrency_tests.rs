use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};

use tokio::{
    sync::{mpsc, Barrier, Semaphore},
    time::{timeout, Duration},
};

use super::{
    reconcile_collections_in_order, reconcile_databases_bounded,
    DATABASE_RECONCILIATION_CONCURRENCY,
};

struct DropProbe(Arc<AtomicBool>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn database_reconciliation_is_bounded_and_drains_every_database() {
    assert_eq!(DATABASE_RECONCILIATION_CONCURRENCY, 8);
    assert!(std::hint::black_box(DATABASE_RECONCILIATION_CONCURRENCY) <= 10);

    const DATABASES: usize = 17;
    const TEST_LIMIT: usize = 3;
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let gates = Arc::new(Semaphore::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let task = tokio::spawn({
        let gates = Arc::clone(&gates);
        let completed = Arc::clone(&completed);
        async move {
            reconcile_databases_bounded(0..DATABASES, TEST_LIMIT, move |database| {
                let started_tx = started_tx.clone();
                let gates = Arc::clone(&gates);
                let completed = Arc::clone(&completed);
                async move {
                    started_tx.send(database).expect("start observer");
                    let permit = gates.acquire().await.expect("test semaphore remains open");
                    permit.forget();
                    completed.fetch_add(1, Ordering::SeqCst);
                    Ok::<(), &'static str>(())
                }
            })
            .await
        }
    });

    for _ in 0..TEST_LIMIT {
        timeout(Duration::from_secs(1), started_rx.recv())
            .await
            .expect("initial database should start")
            .expect("start channel remains open");
    }
    tokio::task::yield_now().await;
    assert!(matches!(
        started_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));

    for _ in TEST_LIMIT..DATABASES {
        gates.add_permits(1);
        timeout(Duration::from_secs(1), started_rx.recv())
            .await
            .expect("one released slot should start one database")
            .expect("start channel remains open");
        tokio::task::yield_now().await;
        assert!(matches!(
            started_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    assert!(
        !task.is_finished(),
        "active databases must finish before return"
    );
    gates.add_permits(TEST_LIMIT);
    task.await
        .expect("reconciliation task should not panic")
        .expect("all databases should reconcile");
    assert_eq!(completed.load(Ordering::SeqCst), DATABASES);
}

#[tokio::test]
async fn database_reconciliation_fails_fast_and_drops_a_pending_peer() {
    let barrier = Arc::new(Barrier::new(2));
    let peer_dropped = Arc::new(AtomicBool::new(false));
    let result = timeout(
        Duration::from_secs(1),
        reconcile_databases_bounded(0..2, 2, {
            let barrier = Arc::clone(&barrier);
            let peer_dropped = Arc::clone(&peer_dropped);
            move |database| {
                let barrier = Arc::clone(&barrier);
                let peer_dropped = Arc::clone(&peer_dropped);
                async move {
                    let _drop_probe = (database == 1).then(|| DropProbe(Arc::clone(&peer_dropped)));
                    barrier.wait().await;
                    if database == 0 {
                        Err("database-error")
                    } else {
                        std::future::pending::<std::result::Result<(), &'static str>>().await
                    }
                }
            }
        }),
    )
    .await
    .expect("a database error must not wait for a permanently pending peer");

    assert_eq!(
        result.expect_err("the database error should propagate"),
        "database-error"
    );
    assert!(
        peer_dropped.load(Ordering::SeqCst),
        "fail-fast return must cancel the pending peer future"
    );
}

#[tokio::test]
async fn collections_in_one_database_remain_ordered_and_stop_at_first_error() {
    let active = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let error = reconcile_collections_in_order(0..5, {
        let active = Arc::clone(&active);
        let observed = Arc::clone(&observed);
        move |collection| {
            let active = Arc::clone(&active);
            let observed = Arc::clone(&observed);
            async move {
                assert_eq!(
                    active.fetch_add(1, Ordering::SeqCst),
                    0,
                    "collections from one database must never overlap"
                );
                tokio::task::yield_now().await;
                observed.lock().expect("observation lock").push(collection);
                assert_eq!(active.fetch_sub(1, Ordering::SeqCst), 1);
                if collection == 2 {
                    Err("collection-error")
                } else {
                    Ok(())
                }
            }
        }
    })
    .await
    .expect_err("the first collection failure should stop this database");

    assert_eq!(error, "collection-error");
    assert_eq!(*observed.lock().expect("observation lock"), vec![0, 1, 2]);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}
