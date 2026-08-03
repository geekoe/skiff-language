//! Shared TaskStore contract (doc/reference/dispatch.md test matrix items
//! 5-14) run against any `TaskStore` implementation. The in-memory fake drives
//! a controllable clock; the Mongo probe drives the same scenarios against
//! the server clock (waiting out short lease windows).

use skiff_task_control::model::{
    DurableDuration, DurableUtcTimestamp, LeaseId, TaskCancelResultKind, TaskId, TaskOutcome,
    TaskState, TaskStatusKind, TaskTerminal,
};
use skiff_task_control::store::{
    CancelInput, ClaimInput, ClaimOutcome, ClaimRejection, DueScanInput, LeaseRecoveryInput,
    LeaseRecoveryOutcome, RenewInput, RenewOutcome, RenewRejection, SettleInput, SettleOutcome,
    StatusInput, TaskStore,
};

use super::{fixtures, TestTime};

const PAST_MILLIS: i64 = 60_000;
const FUTURE_MILLIS: i64 = 3_000;
const SHORT_LEASE_MILLIS: i64 = 2_000;
const LONG_LEASE_MILLIS: i64 = 60_000;

pub async fn run_contract(store: &dyn TaskStore, time: &TestTime) {
    create_idempotency(store, time).await;
    claim_cas(store, time).await;
    lease_expiry_settlement_race(store, time).await;
    renew_heartbeat(store, time).await;
    duplicate_notification_and_reopen(store, time).await;
    cancel_claim_race(store, time).await;
    terminal_settlement_idempotency(store, time).await;
    due_visibility(store, time).await;
    state_machine_illegal_transitions(store, time).await;
    permanent_error_classification(store, time).await;
    status_retention(store, time).await;
}

fn task_id(seed: u64) -> TaskId {
    TaskId::new(format!("task-{seed}"))
}

async fn create_due(store: &dyn TaskStore, seed: u64, time: &TestTime) {
    let mut record = fixtures::record(seed, time.now_millis() - PAST_MILLIS);
    record.created_at = DurableUtcTimestamp::from_millis(time.now_millis() - 1_000);
    store.create(record).await.expect("create due task");
}

async fn claim_ready(
    store: &dyn TaskStore,
    seed: u64,
    time: &TestTime,
    lease_span_millis: i64,
) -> (skiff_task_control::model::TaskRecord, DurableUtcTimestamp) {
    create_due(store, seed, time).await;
    let due = store
        .scan_due(DueScanInput { limit: 100 })
        .await
        .expect("scan due");
    assert!(
        due.iter().any(|record| record.task_id == task_id(seed)),
        "task {seed} must be visible after due"
    );
    let lease_expiry = DurableUtcTimestamp::from_millis(time.now_millis() + lease_span_millis);
    match store
        .claim(ClaimInput {
            task_id: task_id(seed),
            owner: format!("scheduler-{seed}"),
            lease_expiry,
            image_activatable: true,
        })
        .await
        .expect("claim")
    {
        ClaimOutcome::Claimed(record) => (record, lease_expiry),
        other => panic!("claim for task {seed} failed: {other:?}"),
    }
}

async fn create_idempotency(store: &dyn TaskStore, time: &TestTime) {
    let record = fixtures::record(101, time.now_millis() + FUTURE_MILLIS);
    let first = store.create(record.clone()).await.expect("first create");
    let second = store.create(record.clone()).await.expect("retry create");
    assert_eq!(
        first, second,
        "TaskId-idempotent create returns the same record"
    );

    let mut conflicting = record.clone();
    conflicting.payload = skiff_task_control::model::RecoverablePayload::new(vec![9, 9, 9]);
    assert!(
        matches!(
            store.create(conflicting).await,
            Err(skiff_task_control::TaskStoreError::DuplicateTaskId { .. })
        ),
        "same TaskId with a different canonical record must conflict"
    );
}

async fn claim_cas(store: &dyn TaskStore, time: &TestTime) {
    // Future task: not claimable before due.
    let mut future = fixtures::record(102, time.now_millis() + FUTURE_MILLIS);
    future.created_at = DurableUtcTimestamp::from_millis(time.now_millis() - 1_000);
    store.create(future).await.expect("create future");
    assert!(
        matches!(
            store
                .claim(ClaimInput {
                    task_id: task_id(102),
                    owner: "scheduler-102".to_string(),
                    lease_expiry: DurableUtcTimestamp::from_millis(
                        time.now_millis() + LONG_LEASE_MILLIS
                    ),
                    image_activatable: true,
                })
                .await
                .expect("claim future"),
            ClaimOutcome::Rejected(ClaimRejection::NotReady)
        ),
        "scheduled future task must not be claimable"
    );

    // Ready but image not activatable: rejected without state change.
    create_due(store, 103, time).await;
    store
        .scan_due(DueScanInput { limit: 100 })
        .await
        .expect("scan 103");
    assert!(
        matches!(
            store
                .claim(ClaimInput {
                    task_id: task_id(103),
                    owner: "scheduler-103".to_string(),
                    lease_expiry: DurableUtcTimestamp::from_millis(
                        time.now_millis() + LONG_LEASE_MILLIS
                    ),
                    image_activatable: false,
                })
                .await
                .expect("claim 103"),
            ClaimOutcome::Rejected(ClaimRejection::NotActivatable)
        ),
        "claim must require a reactivatable execution image"
    );

    // Valid claim writes state / lease / attempt generation atomically.
    let (claimed, expected_expiry) = claim_ready(store, 104, time, LONG_LEASE_MILLIS).await;
    assert_eq!(claimed.state, TaskState::Leased);
    assert_eq!(claimed.attempt_generation, 1);
    let lease = claimed.active_lease.as_ref().expect("active lease");
    assert_eq!(lease.owner, "scheduler-104");
    assert_eq!(lease.expiry, expected_expiry);
    assert!(!lease.lease_id.as_str().is_empty());
    assert!(!lease.attempt_id.as_str().is_empty());

    // A second consumer cannot obtain a second valid lease.
    assert!(
        matches!(
            store
                .claim(ClaimInput {
                    task_id: task_id(104),
                    owner: "scheduler-104b".to_string(),
                    lease_expiry: DurableUtcTimestamp::from_millis(
                        time.now_millis() + LONG_LEASE_MILLIS
                    ),
                    image_activatable: true,
                })
                .await
                .expect("second claim"),
            ClaimOutcome::Rejected(ClaimRejection::AlreadyLeased)
        ),
        "leased task must reject a second claim"
    );
}

async fn lease_expiry_settlement_race(store: &dyn TaskStore, time: &TestTime) {
    // Settlement wins while the lease is valid; recovery loses.
    let (claimed, _) = claim_ready(store, 105, time, LONG_LEASE_MILLIS).await;
    let lease_id = claimed
        .active_lease
        .as_ref()
        .expect("lease")
        .lease_id
        .clone();
    assert!(
        matches!(
            store
                .settle(SettleInput {
                    task_id: task_id(105),
                    lease_id,
                    terminal: terminal_succeeded(time),
                })
                .await
                .expect("settle 105"),
            SettleOutcome::Settled(_)
        ),
        "settlement must win while the lease is valid"
    );
    assert!(
        matches!(
            store
                .recover_expired_lease(LeaseRecoveryInput {
                    task_id: task_id(105),
                    retry_not_before: DurableUtcTimestamp::from_millis(0),
                })
                .await
                .expect("recover after settle"),
            LeaseRecoveryOutcome::Terminal
        ),
        "terminal task must not be recovered"
    );

    // Recovery wins after expiry; stale settlement loses.
    let (claimed, _) = claim_ready(store, 106, time, SHORT_LEASE_MILLIS).await;
    let lease_id = claimed
        .active_lease
        .as_ref()
        .expect("lease")
        .lease_id
        .clone();
    time.advance(SHORT_LEASE_MILLIS + 500).await;
    assert!(
        matches!(
            store
                .settle(SettleInput {
                    task_id: task_id(106),
                    lease_id: lease_id.clone(),
                    terminal: terminal_succeeded(time),
                })
                .await
                .expect("settle after expiry"),
            SettleOutcome::ExpiredLease
        ),
        "settlement must lose to authority-time expiry"
    );
    let recovered = match store
        .recover_expired_lease(LeaseRecoveryInput {
            task_id: task_id(106),
            retry_not_before: DurableUtcTimestamp::from_millis(0),
        })
        .await
        .expect("recover expired")
    {
        LeaseRecoveryOutcome::Recovered(record) => record,
        other => panic!("recovery must win after expiry: {other:?}"),
    };
    assert_eq!(recovered.state, TaskState::Ready);
    assert!(recovered.active_lease.is_none());
    assert!(
        matches!(
            store
                .settle(SettleInput {
                    task_id: task_id(106),
                    lease_id,
                    terminal: terminal_succeeded(time),
                })
                .await
                .expect("stale settle"),
            SettleOutcome::NotLeased
        ),
        "old lease must never settle after recovery"
    );

    // Recovery enables a new attempt with a new lease / generation.
    match store
        .claim(ClaimInput {
            task_id: task_id(106),
            owner: "scheduler-106b".to_string(),
            lease_expiry: DurableUtcTimestamp::from_millis(time.now_millis() + LONG_LEASE_MILLIS),
            image_activatable: true,
        })
        .await
        .expect("reclaim")
    {
        ClaimOutcome::Claimed(record) => {
            assert_eq!(record.attempt_generation, 2, "attempt generation advances");
        }
        other => panic!("reclaim failed: {other:?}"),
    }
}

async fn renew_heartbeat(store: &dyn TaskStore, time: &TestTime) {
    let (claimed, _) = claim_ready(store, 107, time, LONG_LEASE_MILLIS).await;
    let lease_id = claimed
        .active_lease
        .as_ref()
        .expect("lease")
        .lease_id
        .clone();
    let new_expiry = DurableUtcTimestamp::from_millis(time.now_millis() + 2 * LONG_LEASE_MILLIS);
    match store
        .renew(RenewInput {
            task_id: task_id(107),
            lease_id: lease_id.clone(),
            new_expiry,
        })
        .await
        .expect("renew")
    {
        RenewOutcome::Renewed(record) => {
            assert_eq!(
                record.active_lease.as_ref().expect("lease").expiry,
                new_expiry,
                "renew must extend the current lease"
            );
        }
        other => panic!("renew failed: {other:?}"),
    }
    assert!(
        matches!(
            store
                .renew(RenewInput {
                    task_id: task_id(107),
                    lease_id: LeaseId::new("stale-lease"),
                    new_expiry,
                })
                .await
                .expect("stale renew"),
            RenewOutcome::Rejected(RenewRejection::StaleLease)
        ),
        "renew must carry the current lease id"
    );
    assert!(
        matches!(
            store
                .renew(RenewInput {
                    task_id: task_id(107),
                    lease_id: lease_id.clone(),
                    new_expiry: DurableUtcTimestamp::from_millis(time.now_millis() - 1),
                })
                .await
                .expect("past renew"),
            RenewOutcome::Rejected(RenewRejection::InvalidExpiry)
        ),
        "renew to a past expiry must be rejected"
    );

    // Heartbeat after expiry is stale; recovery takes over.
    let (claimed, _) = claim_ready(store, 108, time, SHORT_LEASE_MILLIS).await;
    let lease_id = claimed
        .active_lease
        .as_ref()
        .expect("lease")
        .lease_id
        .clone();
    store
        .renew(RenewInput {
            task_id: task_id(108),
            lease_id: lease_id.clone(),
            new_expiry: DurableUtcTimestamp::from_millis(
                time.now_millis() + SHORT_LEASE_MILLIS + 1_000,
            ),
        })
        .await
        .expect("renew 108");
    time.advance(SHORT_LEASE_MILLIS + 2_000).await;
    assert!(
        matches!(
            store
                .renew(RenewInput {
                    task_id: task_id(108),
                    lease_id,
                    new_expiry: DurableUtcTimestamp::from_millis(
                        time.now_millis() + LONG_LEASE_MILLIS
                    ),
                })
                .await
                .expect("expired renew"),
            RenewOutcome::Rejected(RenewRejection::ExpiredLease)
        ),
        "renew after authority-time expiry must be stale"
    );
    assert!(matches!(
        store
            .recover_expired_lease(LeaseRecoveryInput {
                task_id: task_id(108),
                retry_not_before: DurableUtcTimestamp::from_millis(0),
            })
            .await
            .expect("recover 108"),
        LeaseRecoveryOutcome::Recovered(_)
    ));
}

async fn duplicate_notification_and_reopen(store: &dyn TaskStore, time: &TestTime) {
    create_due(store, 109, time).await;
    let first = store
        .scan_due(DueScanInput { limit: 100 })
        .await
        .expect("first scan");
    let second = store
        .scan_due(DueScanInput { limit: 100 })
        .await
        .expect("second scan");
    let count = |records: &[skiff_task_control::model::TaskRecord]| {
        records
            .iter()
            .filter(|record| record.task_id == task_id(109))
            .count()
    };
    assert_eq!(
        count(&first),
        1,
        "duplicate notification must not duplicate the task"
    );
    assert_eq!(count(&second), 1);
    let claimed = match store
        .claim(ClaimInput {
            task_id: task_id(109),
            owner: "scheduler-109".to_string(),
            lease_expiry: DurableUtcTimestamp::from_millis(time.now_millis() + LONG_LEASE_MILLIS),
            image_activatable: true,
        })
        .await
        .expect("claim 109")
    {
        ClaimOutcome::Claimed(record) => record,
        other => panic!("claim 109 failed: {other:?}"),
    };
    assert_eq!(claimed.attempt_generation, 1);
    let post_claim = store
        .scan_due(DueScanInput { limit: 100 })
        .await
        .expect("post-claim scan");
    assert_eq!(
        count(&post_claim),
        0,
        "leased task must not be due-visible again"
    );

    // Terminal tasks never reopen via scanner / claim.
    store
        .settle(SettleInput {
            task_id: task_id(109),
            lease_id: claimed
                .active_lease
                .as_ref()
                .expect("lease")
                .lease_id
                .clone(),
            terminal: terminal_succeeded(time),
        })
        .await
        .expect("settle 109");
    let post_terminal = store
        .scan_due(DueScanInput { limit: 100 })
        .await
        .expect("post-terminal scan");
    assert_eq!(count(&post_terminal), 0, "terminal task must not reopen");
    assert!(matches!(
        store
            .claim(ClaimInput {
                task_id: task_id(109),
                owner: "scheduler-109".to_string(),
                lease_expiry: DurableUtcTimestamp::from_millis(
                    time.now_millis() + LONG_LEASE_MILLIS
                ),
                image_activatable: true,
            })
            .await
            .expect("claim terminal"),
        ClaimOutcome::Rejected(ClaimRejection::Terminal)
    ));

    // Canceled tasks never recover to ready.
    create_due(store, 110, time).await;
    store
        .scan_due(DueScanInput { limit: 100 })
        .await
        .expect("scan 110");
    assert_eq!(
        store
            .cancel(CancelInput {
                task_id: task_id(110)
            })
            .await
            .expect("cancel 110")
            .kind,
        TaskCancelResultKind::Canceled
    );
    let post_cancel = store
        .scan_due(DueScanInput { limit: 100 })
        .await
        .expect("post-cancel scan");
    assert_eq!(count(&post_cancel), 0, "canceled task must not reopen");
    assert!(matches!(
        store
            .claim(ClaimInput {
                task_id: task_id(110),
                owner: "scheduler-110".to_string(),
                lease_expiry: DurableUtcTimestamp::from_millis(
                    time.now_millis() + LONG_LEASE_MILLIS
                ),
                image_activatable: true,
            })
            .await
            .expect("claim canceled"),
        ClaimOutcome::Rejected(ClaimRejection::Terminal)
    ));
}

async fn cancel_claim_race(store: &dyn TaskStore, time: &TestTime) {
    // Cancel first: task never starts.
    let scheduled = fixtures::record(111, time.now_millis() + FUTURE_MILLIS);
    store.create(scheduled).await.expect("create 111");
    assert_eq!(
        store
            .cancel(CancelInput {
                task_id: task_id(111)
            })
            .await
            .expect("cancel 111")
            .kind,
        TaskCancelResultKind::Canceled
    );
    assert!(
        matches!(
            store
                .claim(ClaimInput {
                    task_id: task_id(111),
                    owner: "scheduler-111".to_string(),
                    lease_expiry: DurableUtcTimestamp::from_millis(
                        time.now_millis() + LONG_LEASE_MILLIS
                    ),
                    image_activatable: true,
                })
                .await
                .expect("claim canceled 111"),
            ClaimOutcome::Rejected(ClaimRejection::Terminal)
        ),
        "canceled task must never produce an attempt"
    );

    // Claim first: cancel returns AlreadyStarted and does not modify state.
    let (claimed, _) = claim_ready(store, 112, time, LONG_LEASE_MILLIS).await;
    assert_eq!(
        store
            .cancel(CancelInput {
                task_id: task_id(112)
            })
            .await
            .expect("cancel 112")
            .kind,
        TaskCancelResultKind::AlreadyStarted
    );
    let still = store
        .settle(SettleInput {
            task_id: task_id(112),
            lease_id: claimed
                .active_lease
                .as_ref()
                .expect("lease")
                .lease_id
                .clone(),
            terminal: terminal_succeeded(time),
        })
        .await
        .expect("settle after alreadyStarted");
    assert!(
        matches!(still, SettleOutcome::Settled(_)),
        "AlreadyStarted cancel must not stop settlement"
    );
}

async fn terminal_settlement_idempotency(store: &dyn TaskStore, time: &TestTime) {
    let (claimed, _) = claim_ready(store, 113, time, LONG_LEASE_MILLIS).await;
    let lease_id = claimed
        .active_lease
        .as_ref()
        .expect("lease")
        .lease_id
        .clone();
    let terminal = terminal_succeeded(time);
    let input = SettleInput {
        task_id: task_id(113),
        lease_id: lease_id.clone(),
        terminal: terminal.clone(),
    };
    assert!(matches!(
        store.settle(input.clone()).await.expect("settle 113"),
        SettleOutcome::Settled(_)
    ));
    let replay = SettleInput {
        terminal: TaskTerminal {
            settled_at: DurableUtcTimestamp::from_millis(time.now_millis() + 1),
            outcome: TaskOutcome::Succeeded,
        },
        ..input.clone()
    };
    assert!(
        matches!(
            store.settle(replay).await.expect("replay settle"),
            SettleOutcome::AlreadySettled(_)
        ),
        "exact same terminal outcome replay must be idempotent"
    );
    let conflicting = SettleInput {
        terminal: TaskTerminal {
            settled_at: terminal.settled_at,
            outcome: TaskOutcome::TargetFailed {
                error: "boom".to_string(),
            },
        },
        ..input
    };
    assert!(
        matches!(
            store.settle(conflicting).await.expect("conflict settle"),
            SettleOutcome::Conflict(_)
        ),
        "same lease with a conflicting outcome must be rejected"
    );
}

async fn due_visibility(store: &dyn TaskStore, time: &TestTime) {
    let mut future = fixtures::record(114, time.now_millis() + FUTURE_MILLIS);
    future.created_at = DurableUtcTimestamp::from_millis(time.now_millis() - 1_000);
    store.create(future).await.expect("create 114");
    let before = store
        .scan_due(DueScanInput { limit: 100 })
        .await
        .expect("scan before due");
    assert!(
        !before.iter().any(|record| record.task_id == task_id(114)),
        "future task must not be visible before due"
    );
    assert_eq!(
        store
            .status(StatusInput {
                task_id: task_id(114),
                retention: DurableDuration::from_millis(365 * 24 * 3600 * 1000),
            })
            .await
            .expect("status before due")
            .kind,
        TaskStatusKind::Scheduled
    );

    time.advance(FUTURE_MILLIS + 1_000).await;
    let after = store
        .scan_due(DueScanInput { limit: 100 })
        .await
        .expect("scan after due");
    assert!(
        after.iter().any(|record| record.task_id == task_id(114)),
        "task must be visible once due has arrived"
    );
    assert_eq!(
        store
            .status(StatusInput {
                task_id: task_id(114),
                retention: DurableDuration::from_millis(365 * 24 * 3600 * 1000),
            })
            .await
            .expect("status after due")
            .kind,
        TaskStatusKind::Ready
    );

    if time.is_controlled() {
        // Wall-clock rollback cannot reveal the task early. The Mongo adapter
        // keeps this guarantee by comparing `dueAt` against the server clock;
        // a client cannot roll its local clock past it.
        time.rollback(FUTURE_MILLIS + 2_000);
        let rolled_back = store
            .scan_due(DueScanInput { limit: 100 })
            .await
            .expect("scan after rollback");
        assert!(
            !rolled_back
                .iter()
                .any(|record| record.task_id == task_id(114)),
            "rollback must not reveal a future task"
        );
        assert!(
            matches!(
                store
                    .claim(ClaimInput {
                        task_id: task_id(114),
                        owner: "scheduler-114".to_string(),
                        lease_expiry: DurableUtcTimestamp::from_millis(
                            time.now_millis() + LONG_LEASE_MILLIS
                        ),
                        image_activatable: true,
                    })
                    .await
                    .expect("claim after rollback"),
                ClaimOutcome::Rejected(ClaimRejection::NotDue)
            ),
            "a ready task whose due_at is still ahead must not be claimable"
        );
    }
}

async fn state_machine_illegal_transitions(store: &dyn TaskStore, time: &TestTime) {
    create_due(store, 115, time).await;
    assert!(
        matches!(
            store
                .claim(ClaimInput {
                    task_id: task_id(115),
                    owner: "scheduler-115".to_string(),
                    lease_expiry: DurableUtcTimestamp::from_millis(
                        time.now_millis() + LONG_LEASE_MILLIS
                    ),
                    image_activatable: true,
                })
                .await
                .expect("claim scheduled"),
            ClaimOutcome::Rejected(ClaimRejection::NotReady)
        ),
        "claim must require the ready state"
    );
    store
        .scan_due(DueScanInput { limit: 100 })
        .await
        .expect("scan 115");
    assert!(
        matches!(
            store
                .settle(SettleInput {
                    task_id: task_id(115),
                    lease_id: LeaseId::new("never-claimed"),
                    terminal: terminal_succeeded(time),
                })
                .await
                .expect("settle ready"),
            SettleOutcome::NotLeased
        ),
        "settlement requires an active lease"
    );
    assert!(
        matches!(
            store
                .renew(RenewInput {
                    task_id: task_id(115),
                    lease_id: LeaseId::new("never-claimed"),
                    new_expiry: DurableUtcTimestamp::from_millis(
                        time.now_millis() + LONG_LEASE_MILLIS
                    ),
                })
                .await
                .expect("renew ready"),
            RenewOutcome::Rejected(RenewRejection::NotLeased)
        ),
        "renew requires an active lease"
    );
    assert!(
        matches!(
            store
                .recover_expired_lease(LeaseRecoveryInput {
                    task_id: task_id(115),
                    retry_not_before: DurableUtcTimestamp::from_millis(0),
                })
                .await
                .expect("recover ready"),
            LeaseRecoveryOutcome::NotLeased
        ),
        "recovery requires a leased state"
    );

    let (claimed, _) = claim_ready(store, 116, time, LONG_LEASE_MILLIS).await;
    let lease_id = claimed
        .active_lease
        .as_ref()
        .expect("lease")
        .lease_id
        .clone();
    assert!(
        matches!(
            store
                .settle(SettleInput {
                    task_id: task_id(116),
                    lease_id: LeaseId::new("wrong-lease"),
                    terminal: terminal_succeeded(time),
                })
                .await
                .expect("stale settle"),
            SettleOutcome::StaleLease
        ),
        "stale lease settlement must be rejected"
    );
    assert!(matches!(
        store
            .settle(SettleInput {
                task_id: task_id(116),
                lease_id,
                terminal: terminal_succeeded(time),
            })
            .await
            .expect("settle 116"),
        SettleOutcome::Settled(_)
    ));
    assert_eq!(
        store
            .cancel(CancelInput {
                task_id: task_id(116)
            })
            .await
            .expect("cancel terminal")
            .kind,
        TaskCancelResultKind::AlreadyTerminal
    );
    assert!(
        matches!(
            store
                .recover_expired_lease(LeaseRecoveryInput {
                    task_id: task_id(116),
                    retry_not_before: DurableUtcTimestamp::from_millis(0),
                })
                .await
                .expect("recover terminal"),
            LeaseRecoveryOutcome::Terminal
        ),
        "terminal tasks must not recover"
    );
}

async fn permanent_error_classification(store: &dyn TaskStore, time: &TestTime) {
    let (claimed, _) = claim_ready(store, 117, time, LONG_LEASE_MILLIS).await;
    let lease_id = claimed
        .active_lease
        .as_ref()
        .expect("lease")
        .lease_id
        .clone();
    let terminal = TaskTerminal {
        settled_at: DurableUtcTimestamp::from_millis(time.now_millis()),
        outcome: TaskOutcome::PlatformFailed {
            reason: "execution record permanently destroyed".to_string(),
        },
    };
    assert!(
        matches!(
            store
                .settle(SettleInput {
                    task_id: task_id(117),
                    lease_id,
                    terminal,
                })
                .await
                .expect("platform settle"),
            SettleOutcome::Settled(record)
                if record.state == TaskState::PlatformFailed && record.terminal.is_some()
        ),
        "permanent platform errors must converge to platform-failed"
    );
    assert_eq!(
        store
            .status(StatusInput {
                task_id: task_id(117),
                retention: DurableDuration::from_millis(365 * 24 * 3600 * 1000),
            })
            .await
            .expect("platform status")
            .kind,
        TaskStatusKind::PlatformFailed
    );
}

async fn status_retention(store: &dyn TaskStore, time: &TestTime) {
    let now = time.now_millis();
    let mut old = fixtures::record(118, now + FUTURE_MILLIS);
    old.created_at = DurableUtcTimestamp::from_millis(now - 2 * PAST_MILLIS);
    store.create(old).await.expect("create old");
    assert_eq!(
        store
            .status(StatusInput {
                task_id: task_id(118),
                retention: DurableDuration::from_millis(PAST_MILLIS),
            })
            .await
            .expect("expired status")
            .kind,
        TaskStatusKind::Expired,
        "records past retention must report expired"
    );
    assert_eq!(
        store
            .status(StatusInput {
                task_id: task_id(118),
                retention: DurableDuration::from_millis(365 * 24 * 3600 * 1000),
            })
            .await
            .expect("live status")
            .kind,
        TaskStatusKind::Scheduled
    );
    assert_eq!(
        store
            .status(StatusInput {
                task_id: TaskId::new("task-missing"),
                retention: DurableDuration::from_millis(PAST_MILLIS),
            })
            .await
            .expect("missing status")
            .kind,
        TaskStatusKind::Expired,
        "unresolvable TaskId must report expired"
    );
    assert_eq!(
        store
            .cancel(CancelInput {
                task_id: TaskId::new("task-missing")
            })
            .await
            .expect("missing cancel")
            .kind,
        TaskCancelResultKind::Expired
    );
}

pub(crate) fn terminal_succeeded(time: &TestTime) -> TaskTerminal {
    TaskTerminal {
        settled_at: DurableUtcTimestamp::from_millis(time.now_millis()),
        outcome: TaskOutcome::Succeeded,
    }
}
