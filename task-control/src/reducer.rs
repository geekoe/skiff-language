//! Pure state-machine transitions shared by the in-memory fake and the Mongo
//! adapter's classification fallback.
//!
//! The reducer only sees an existing record and store authority now; it never
//! generates ids, talks to storage or consults client clocks. Adapters run the
//! actual conditional write first (Mongo CAS / fake lock) and use the reducer
//! to compute the next record or to classify why a CAS did not match.

use crate::model::{
    AttemptId, DurableUtcTimestamp, LeaseId, TaskLease, TaskRecord, TaskState, TaskTerminal,
};
use crate::store::{
    ClaimInput, ClaimRejection, ReleaseInput, RenewInput, RenewRejection, SettleInput,
    SettleTransition,
};

/// Monotonic merge for the scheduler-owned retry not-before: concurrent
/// recoveries / releases converge on the latest value, never an earlier one.
pub(crate) fn merge_retry_not_before(
    existing: Option<DurableUtcTimestamp>,
    next: DurableUtcTimestamp,
) -> Option<DurableUtcTimestamp> {
    Some(match existing {
        Some(current) if current > next => current,
        _ => next,
    })
}

/// `scheduled` -> `ready` when the durable not-before time has arrived.
/// Idempotent: a record already `ready` is returned unchanged.
pub fn advance_due(record: &TaskRecord, now: DurableUtcTimestamp) -> Option<TaskRecord> {
    match record.state {
        TaskState::Scheduled if record.due_at <= now => Some(TaskRecord {
            state: TaskState::Ready,
            ..record.clone()
        }),
        _ => None,
    }
}

/// `ready` -> `leased`: atomic claim semantics. The caller supplies fresh
/// attempt / lease ids from the adapter's generator.
pub fn claim(
    record: &TaskRecord,
    input: &ClaimInput,
    now: DurableUtcTimestamp,
    attempt_id: AttemptId,
    lease_id: LeaseId,
) -> Result<TaskRecord, ClaimRejection> {
    match record.state {
        TaskState::Scheduled => return Err(ClaimRejection::NotReady),
        TaskState::Ready => {}
        TaskState::Leased => return Err(ClaimRejection::AlreadyLeased),
        _ => return Err(ClaimRejection::Terminal),
    }
    if record.due_at > now {
        return Err(ClaimRejection::NotDue);
    }
    if !input.image_activatable {
        return Err(ClaimRejection::NotActivatable);
    }
    if input.lease_expiry <= now {
        return Err(ClaimRejection::InvalidLeaseExpiry);
    }
    Ok(TaskRecord {
        state: TaskState::Leased,
        attempt_generation: record.attempt_generation + 1,
        active_lease: Some(TaskLease {
            lease_id,
            attempt_id,
            owner: input.owner.clone(),
            expiry: input.lease_expiry,
        }),
        terminal: None,
        ..record.clone()
    })
}

/// Lease heartbeat: only the current lease id, only while unexpired, and only
/// to a future expiry.
pub fn renew(
    record: &TaskRecord,
    input: &RenewInput,
    now: DurableUtcTimestamp,
) -> Result<TaskRecord, RenewRejection> {
    let lease = match &record.state {
        TaskState::Leased => record.active_lease.as_ref(),
        TaskState::Scheduled | TaskState::Ready => return Err(RenewRejection::NotLeased),
        _ => return Err(RenewRejection::Terminal),
    }
    .expect("leased record carries active lease");
    if lease.lease_id != input.lease_id {
        return Err(RenewRejection::StaleLease);
    }
    if lease.expiry <= now {
        return Err(RenewRejection::ExpiredLease);
    }
    if input.new_expiry <= now {
        return Err(RenewRejection::InvalidExpiry);
    }
    Ok(TaskRecord {
        active_lease: Some(TaskLease {
            expiry: input.new_expiry,
            ..lease.clone()
        }),
        ..record.clone()
    })
}

/// Terminal settlement reducer. Idempotent for the exact same outcome,
/// conflicting outcomes and stale / expired leases are rejected.
pub fn settle(
    record: &TaskRecord,
    input: &SettleInput,
    now: DurableUtcTimestamp,
) -> SettleTransition {
    match &record.state {
        TaskState::Leased => {
            let lease = record.active_lease.as_ref().expect("leased lease");
            if lease.lease_id != input.lease_id {
                return SettleTransition::StaleLease;
            }
            if lease.expiry <= now {
                return SettleTransition::ExpiredLease;
            }
            if let Some(existing) = &record.terminal {
                return if existing.same_outcome(&input.terminal) {
                    SettleTransition::AlreadySettled
                } else {
                    SettleTransition::Conflict
                };
            }
            SettleTransition::Settled(TaskRecord {
                state: input.terminal.state(),
                active_lease: None,
                terminal: Some(input.terminal.clone()),
                ..record.clone()
            })
        }
        TaskState::Scheduled | TaskState::Ready => SettleTransition::NotLeased,
        TaskState::Succeeded
        | TaskState::Failed
        | TaskState::PlatformFailed
        | TaskState::Canceled => {
            let existing = record
                .terminal
                .as_ref()
                .expect("terminal state has terminal");
            if existing.same_outcome(&input.terminal) {
                SettleTransition::AlreadySettled
            } else {
                SettleTransition::Conflict
            }
        }
    }
}

/// Before-start cancellation: scheduled / ready -> canceled; leased is
/// already started; terminal is already terminal.
pub fn cancel(
    record: &TaskRecord,
    now: DurableUtcTimestamp,
) -> Result<TaskRecord, CancelRejection> {
    match record.state {
        TaskState::Scheduled | TaskState::Ready => Ok(TaskRecord {
            state: TaskState::Canceled,
            active_lease: None,
            terminal: Some(TaskTerminal {
                settled_at: now,
                outcome: crate::model::TaskOutcome::Canceled,
            }),
            ..record.clone()
        }),
        TaskState::Leased => Err(CancelRejection::AlreadyStarted),
        _ => Err(CancelRejection::AlreadyTerminal),
    }
}

/// Lease-expiry recovery: only `leased` with expired lease; atomically clears
/// the lease and returns to `ready` for a future attempt.
pub fn recover_expired_lease(
    record: &TaskRecord,
    now: DurableUtcTimestamp,
    retry_not_before: DurableUtcTimestamp,
) -> Result<TaskRecord, RecoveryRejection> {
    match record.state {
        TaskState::Leased => {
            let lease = record.active_lease.as_ref().expect("leased lease");
            if lease.expiry > now {
                return Err(RecoveryRejection::NotExpired);
            }
            Ok(TaskRecord {
                state: TaskState::Ready,
                active_lease: None,
                retry_not_before: merge_retry_not_before(record.retry_not_before, retry_not_before),
                ..record.clone()
            })
        }
        TaskState::Scheduled | TaskState::Ready => Err(RecoveryRejection::NotLeased),
        _ => Err(RecoveryRejection::Terminal),
    }
}

/// Provable-rejection release: `leased` with the current lease and an
/// unexpired lease returns to `ready` and atomically applies the
/// scheduler-owned retry not-before. Expired leases lose to recovery.
pub fn release(
    record: &TaskRecord,
    input: &ReleaseInput,
    now: DurableUtcTimestamp,
) -> Result<TaskRecord, ReleaseRejection> {
    match record.state {
        TaskState::Leased => {
            let lease = record.active_lease.as_ref().expect("leased lease");
            if lease.lease_id != input.lease_id {
                return Err(ReleaseRejection::StaleLease);
            }
            if lease.expiry <= now {
                return Err(ReleaseRejection::ExpiredLease);
            }
            Ok(TaskRecord {
                state: TaskState::Ready,
                active_lease: None,
                retry_not_before: merge_retry_not_before(
                    record.retry_not_before,
                    input.retry_not_before,
                ),
                ..record.clone()
            })
        }
        TaskState::Scheduled | TaskState::Ready => Err(ReleaseRejection::NotLeased),
        _ => Err(ReleaseRejection::Terminal),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelRejection {
    AlreadyStarted,
    AlreadyTerminal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryRejection {
    NotExpired,
    NotLeased,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseRejection {
    StaleLease,
    ExpiredLease,
    NotLeased,
    Terminal,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{
        DurableDuration, TaskCancelResultKind, TaskId, TaskOutcome, TaskStatusKind,
    };
    use crate::store::{CancelInput, DueScanInput, LeaseRecoveryInput, StatusInput};

    #[test]
    fn claim_writes_lease_generation_and_fencing() {
        let mut record = fixtures::record(1, 10);
        record.due_at = DurableUtcTimestamp::from_millis(10);
        record.state = TaskState::Ready;
        let input = ClaimInput {
            task_id: record.task_id.clone(),
            owner: "scheduler-a".to_string(),
            lease_expiry: DurableUtcTimestamp::from_millis(100),
            image_activatable: true,
        };
        let claimed = claim(
            &record,
            &input,
            DurableUtcTimestamp::from_millis(20),
            AttemptId::new("attempt-1"),
            LeaseId::new("lease-1"),
        )
        .expect("claim");
        assert_eq!(claimed.state, TaskState::Leased);
        assert_eq!(claimed.attempt_generation, 1);
        let lease = claimed.active_lease.as_ref().expect("lease");
        assert_eq!(lease.lease_id.as_str(), "lease-1");
        assert_eq!(lease.attempt_id.as_str(), "attempt-1");
        assert_eq!(lease.owner, "scheduler-a");
        assert_eq!(lease.expiry, DurableUtcTimestamp::from_millis(100));
    }

    #[test]
    fn claim_rejects_illegal_states_and_timing() {
        let mut record = fixtures::record(1, 10);
        record.due_at = DurableUtcTimestamp::from_millis(10);
        record.state = TaskState::Ready;
        let input = || ClaimInput {
            task_id: record.task_id.clone(),
            owner: "s".to_string(),
            lease_expiry: DurableUtcTimestamp::from_millis(100),
            image_activatable: true,
        };
        assert_eq!(
            claim(
                &record,
                &input(),
                DurableUtcTimestamp::from_millis(5),
                AttemptId::new("a"),
                LeaseId::new("l")
            ),
            Err(ClaimRejection::NotDue)
        );
        record.due_at = DurableUtcTimestamp::from_millis(5);
        let mut not_activatable = input();
        not_activatable.image_activatable = false;
        assert_eq!(
            claim(
                &record,
                &not_activatable,
                DurableUtcTimestamp::from_millis(20),
                AttemptId::new("a"),
                LeaseId::new("l")
            ),
            Err(ClaimRejection::NotActivatable)
        );
        record.state = TaskState::Leased;
        assert_eq!(
            claim(
                &record,
                &input(),
                DurableUtcTimestamp::from_millis(20),
                AttemptId::new("a"),
                LeaseId::new("l")
            ),
            Err(ClaimRejection::AlreadyLeased)
        );
        record.state = TaskState::Succeeded;
        assert_eq!(
            claim(
                &record,
                &input(),
                DurableUtcTimestamp::from_millis(20),
                AttemptId::new("a"),
                LeaseId::new("l")
            ),
            Err(ClaimRejection::Terminal)
        );
    }

    #[test]
    fn settle_rejects_stale_expired_and_conflicting_outcomes() {
        let mut record = fixtures::record(1, 10);
        record.state = TaskState::Leased;
        record.attempt_generation = 1;
        record.active_lease = Some(TaskLease {
            lease_id: LeaseId::new("lease-1"),
            attempt_id: AttemptId::new("attempt-1"),
            owner: "s".to_string(),
            expiry: DurableUtcTimestamp::from_millis(100),
        });
        let terminal = || TaskTerminal {
            settled_at: DurableUtcTimestamp::from_millis(80),
            outcome: TaskOutcome::Succeeded,
        };

        let stale = SettleInput {
            task_id: record.task_id.clone(),
            lease_id: LeaseId::new("lease-other"),
            terminal: terminal(),
        };
        assert_eq!(
            settle(&record, &stale, DurableUtcTimestamp::from_millis(80)),
            SettleTransition::StaleLease
        );

        let expired = SettleInput {
            task_id: record.task_id.clone(),
            lease_id: LeaseId::new("lease-1"),
            terminal: terminal(),
        };
        assert_eq!(
            settle(&record, &expired, DurableUtcTimestamp::from_millis(101)),
            SettleTransition::ExpiredLease
        );

        let won = settle(&record, &expired, DurableUtcTimestamp::from_millis(80));
        let SettleTransition::Settled(settled) = won else {
            panic!("settlement must win before expiry");
        };
        assert_eq!(settled.state, TaskState::Succeeded);
        assert!(settled.active_lease.is_none());

        let replay = SettleInput {
            task_id: record.task_id.clone(),
            lease_id: LeaseId::new("lease-1"),
            terminal: terminal(),
        };
        assert_eq!(
            settle(&settled, &replay, DurableUtcTimestamp::from_millis(80)),
            SettleTransition::AlreadySettled
        );
        let conflict = SettleInput {
            task_id: record.task_id.clone(),
            lease_id: LeaseId::new("lease-1"),
            terminal: TaskTerminal {
                settled_at: DurableUtcTimestamp::from_millis(80),
                outcome: TaskOutcome::TargetFailed {
                    error: "boom".to_string(),
                },
            },
        };
        assert_eq!(
            settle(&settled, &conflict, DurableUtcTimestamp::from_millis(80)),
            SettleTransition::Conflict
        );
    }

    #[test]
    fn expiry_recovery_and_settlement_cas_cannot_both_win() {
        let mut record = fixtures::record(1, 10);
        record.state = TaskState::Leased;
        record.attempt_generation = 1;
        record.active_lease = Some(TaskLease {
            lease_id: LeaseId::new("lease-1"),
            attempt_id: AttemptId::new("attempt-1"),
            owner: "s".to_string(),
            expiry: DurableUtcTimestamp::from_millis(100),
        });
        let now = DurableUtcTimestamp::from_millis(100);
        let settle_input = SettleInput {
            task_id: record.task_id.clone(),
            lease_id: LeaseId::new("lease-1"),
            terminal: TaskTerminal {
                settled_at: now,
                outcome: TaskOutcome::Succeeded,
            },
        };
        assert_eq!(
            settle(&record, &settle_input, now),
            SettleTransition::ExpiredLease
        );
        let recovered = recover_expired_lease(&record, now, now).expect("recovery wins");
        assert_eq!(recovered.state, TaskState::Ready);
        assert!(recovered.active_lease.is_none());
        assert_eq!(
            recovered.retry_not_before,
            Some(now),
            "recovery applies the scheduler-owned retry not-before"
        );
        assert_eq!(
            settle(&recovered, &settle_input, now),
            SettleTransition::NotLeased
        );
    }

    #[test]
    fn release_returns_ready_with_monotonic_retry_not_before() {
        let mut record = fixtures::record(1, 10);
        record.state = TaskState::Leased;
        record.attempt_generation = 1;
        record.active_lease = Some(TaskLease {
            lease_id: LeaseId::new("lease-1"),
            attempt_id: AttemptId::new("attempt-1"),
            owner: "s".to_string(),
            expiry: DurableUtcTimestamp::from_millis(100),
        });
        let now = DurableUtcTimestamp::from_millis(20);

        // Wrong lease is stale.
        assert_eq!(
            release(
                &record,
                &ReleaseInput {
                    task_id: record.task_id.clone(),
                    lease_id: LeaseId::new("lease-other"),
                    retry_not_before: DurableUtcTimestamp::from_millis(50),
                },
                now
            ),
            Err(ReleaseRejection::StaleLease)
        );
        // Expired lease loses to recovery.
        assert_eq!(
            release(
                &record,
                &ReleaseInput {
                    task_id: record.task_id.clone(),
                    lease_id: LeaseId::new("lease-1"),
                    retry_not_before: DurableUtcTimestamp::from_millis(50),
                },
                DurableUtcTimestamp::from_millis(100)
            ),
            Err(ReleaseRejection::ExpiredLease)
        );

        let released = release(
            &record,
            &ReleaseInput {
                task_id: record.task_id.clone(),
                lease_id: LeaseId::new("lease-1"),
                retry_not_before: DurableUtcTimestamp::from_millis(50),
            },
            now,
        )
        .expect("release");
        assert_eq!(released.state, TaskState::Ready);
        assert!(released.active_lease.is_none());
        assert_eq!(
            released.retry_not_before,
            Some(DurableUtcTimestamp::from_millis(50))
        );

        // A later concurrent release keeps the later value, never rewinds it.
        let re_released = release(
            &released,
            &ReleaseInput {
                task_id: released.task_id.clone(),
                lease_id: LeaseId::new("lease-1"),
                retry_not_before: DurableUtcTimestamp::from_millis(40),
            },
            now,
        )
        .expect_err("ready task cannot be released again");
        assert_eq!(re_released, ReleaseRejection::NotLeased);
        assert_eq!(
            released.retry_not_before,
            Some(DurableUtcTimestamp::from_millis(50))
        );

        let mut re_claimed = released.clone();
        re_claimed.state = TaskState::Leased;
        re_claimed.attempt_generation = 2;
        re_claimed.active_lease = Some(TaskLease {
            lease_id: LeaseId::new("lease-2"),
            attempt_id: AttemptId::new("attempt-2"),
            owner: "s".to_string(),
            expiry: DurableUtcTimestamp::from_millis(200),
        });
        let re_released = release(
            &re_claimed,
            &ReleaseInput {
                task_id: re_claimed.task_id.clone(),
                lease_id: LeaseId::new("lease-2"),
                retry_not_before: DurableUtcTimestamp::from_millis(40),
            },
            now,
        )
        .expect("second release");
        assert_eq!(
            re_released.retry_not_before,
            Some(DurableUtcTimestamp::from_millis(50)),
            "release never rewinds an existing later not-before"
        );
    }

    #[test]
    fn cancel_and_claim_compete_bidirectionally() {
        let mut record = fixtures::record(1, 10);
        record.due_at = DurableUtcTimestamp::from_millis(10);
        record.state = TaskState::Ready;
        let now = DurableUtcTimestamp::from_millis(20);
        let canceled = cancel(&record, now).expect("cancel");
        assert_eq!(canceled.state, TaskState::Canceled);
        assert_eq!(
            claim(
                &canceled,
                &ClaimInput {
                    task_id: canceled.task_id.clone(),
                    owner: "s".to_string(),
                    lease_expiry: DurableUtcTimestamp::from_millis(100),
                    image_activatable: true,
                },
                now,
                AttemptId::new("a"),
                LeaseId::new("l")
            ),
            Err(ClaimRejection::Terminal)
        );

        let claimed = claim(
            &record,
            &ClaimInput {
                task_id: record.task_id.clone(),
                owner: "s".to_string(),
                lease_expiry: DurableUtcTimestamp::from_millis(100),
                image_activatable: true,
            },
            now,
            AttemptId::new("a"),
            LeaseId::new("l"),
        )
        .expect("claim");
        assert_eq!(cancel(&claimed, now), Err(CancelRejection::AlreadyStarted));
    }

    #[test]
    fn due_visibility_obeys_not_before_boundary() {
        let mut record = fixtures::record(1, 10);
        record.due_at = DurableUtcTimestamp::from_millis(100);
        assert_eq!(
            advance_due(&record, DurableUtcTimestamp::from_millis(99)),
            None,
            "future task must not become visible"
        );
        assert_eq!(
            advance_due(&record, DurableUtcTimestamp::from_millis(100)),
            Some(TaskRecord {
                state: TaskState::Ready,
                ..record.clone()
            })
        );
        // Wall-clock rollback cannot retroactively reveal the task.
        assert_eq!(
            advance_due(&record, DurableUtcTimestamp::from_millis(50)),
            None
        );
    }

    #[test]
    fn full_state_machine_transition_table() {
        let now = DurableUtcTimestamp::from_millis(50);
        let mut record = fixtures::record(1, 10);
        record.due_at = now;
        let task_id = record.task_id.clone();
        let lease_input = || ClaimInput {
            task_id: task_id.clone(),
            owner: "s".to_string(),
            lease_expiry: DurableUtcTimestamp::from_millis(200),
            image_activatable: true,
        };
        // scheduled -> ready
        record = advance_due(&record, now).expect("due");
        // ready -> leased
        record = claim(
            &record,
            &lease_input(),
            now,
            AttemptId::new("a1"),
            LeaseId::new("l1"),
        )
        .expect("claim");
        // leased -> succeeded
        let succeeded = TaskTerminal {
            settled_at: now,
            outcome: TaskOutcome::Succeeded,
        };
        record = match settle(
            &record,
            &SettleInput {
                task_id: record.task_id.clone(),
                lease_id: LeaseId::new("l1"),
                terminal: succeeded.clone(),
            },
            now,
        ) {
            SettleTransition::Settled(record) => record,
            other => panic!("settle failed: {other:?}"),
        };
        assert_eq!(record.state, TaskState::Succeeded);
        // terminal: no further claim / cancel / recovery
        assert_eq!(
            claim(
                &record,
                &lease_input(),
                now,
                AttemptId::new("a2"),
                LeaseId::new("l2")
            ),
            Err(ClaimRejection::Terminal)
        );
        assert_eq!(cancel(&record, now), Err(CancelRejection::AlreadyTerminal));
        assert_eq!(
            recover_expired_lease(&record, now, now),
            Err(RecoveryRejection::Terminal)
        );
    }

    #[test]
    fn terminal_settlement_is_idempotent_across_retry() {
        let mut record = fixtures::record(1, 10);
        record.state = TaskState::Leased;
        record.attempt_generation = 1;
        record.active_lease = Some(TaskLease {
            lease_id: LeaseId::new("l1"),
            attempt_id: AttemptId::new("a1"),
            owner: "s".to_string(),
            expiry: DurableUtcTimestamp::from_millis(200),
        });
        let now = DurableUtcTimestamp::from_millis(50);
        let input = SettleInput {
            task_id: record.task_id.clone(),
            lease_id: LeaseId::new("l1"),
            terminal: TaskTerminal {
                settled_at: now,
                outcome: TaskOutcome::Succeeded,
            },
        };
        let SettleTransition::Settled(settled) = settle(&record, &input, now) else {
            panic!("first settle");
        };
        let retry = SettleInput {
            terminal: TaskTerminal {
                settled_at: DurableUtcTimestamp::from_millis(60),
                outcome: TaskOutcome::Succeeded,
            },
            ..input
        };
        assert_eq!(
            settle(&settled, &retry, now),
            SettleTransition::AlreadySettled,
            "same outcome replay must be idempotent"
        );
    }

    pub(crate) mod fixtures {
        use super::*;
        use crate::model::{
            ActorActivationSnapshot, DetachedCallTarget, RecoverablePayload, ServiceOwner,
            TaskExecutionImageRef, TaskId, TaskTraceContext,
        };
        use skiff_artifact_model::{
            ActorImplementationIdentity, ActorMethodIdentity, AssemblyIdentity,
            DeploymentArtifactIdentity, DeploymentRevision, PackageCallableId,
            RecoverableExpectedTypePlan, RecoverableExpectedTypeRoot, RuntimeAssemblyRef,
            RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef, ServiceDeploymentRef,
        };
        use skiff_deployment::projection::actor_routing::ActorRoutingRef;

        pub(crate) fn record(seed: u64, due_at_millis: i64) -> TaskRecord {
            let due_at = DurableUtcTimestamp::from_millis(due_at_millis);
            let execution = TaskExecutionImageRef {
                target_environment: "prod".to_string(),
                package_version: "1.0.0".to_string(),
                assembly: RuntimeAssemblyRef {
                    assembly_identity: AssemblyIdentity::new(format!(
                        "skiff-runtime-assembly-v3:sha256:{seed:064x}"
                    )),
                },
                config_snapshot: RuntimeConfigSnapshotRef {
                    snapshot_id: RuntimeConfigSnapshotId::parse(format!(
                        "skiff-runtime-config-snapshot-v1:{seed:032x}"
                    ))
                    .expect("config id"),
                },
                deployment: ServiceDeploymentRef {
                    service_id: format!("svc-{seed}"),
                    contract_version: "1.0.0".to_string(),
                    deployment_revision: DeploymentRevision::new("revision-1"),
                    deployment_artifact_identity: DeploymentArtifactIdentity::new(
                        "deployment-identity",
                    ),
                },
            };
            TaskRecord {
                task_id: TaskId::new(format!("task-{seed}")),
                owner: ServiceOwner::new(format!("svc-{seed}")),
                execution,
                target: DetachedCallTarget::Function {
                    callable: PackageCallableId::new(format!("callable-{seed}")),
                },
                payload: RecoverablePayload::new(vec![seed as u8, 2, 3]),
                due_at,
                state: TaskState::Scheduled,
                attempt_generation: 0,
                active_lease: None,
                terminal: None,
                trace: TaskTraceContext {
                    trace_id: format!("trace-{seed}"),
                    span_id: None,
                },
                created_at: DurableUtcTimestamp::from_millis(1),
                retry_not_before: None,
            }
        }

        pub(crate) fn actor_record(seed: u64, due_at_millis: i64) -> TaskRecord {
            let mut record = record(seed, due_at_millis);
            record.target = DetachedCallTarget::ActorMethod {
                actor: ActorRoutingRef {
                    service_id: format!("svc-{seed}"),
                    actor_abi_identity: skiff_artifact_model::ActorAbiIdentity::new(format!(
                        "actor-abi-{seed}"
                    )),
                },
                activation: ActorActivationSnapshot {
                    key: RecoverablePayload::new(vec![seed as u8, 1]),
                    create_input: RecoverablePayload::new(vec![seed as u8, 2]),
                    expected_type_plan: RecoverableExpectedTypePlan {
                        root: RecoverableExpectedTypeRoot::TypeIdentityRef {
                            type_identity_ref: skiff_artifact_model::RecoverableTypeIdentityRef(
                                format!("type-{seed}"),
                            ),
                        },
                        root_type_identity_ref: None,
                        runtime_carrier_check_required: false,
                        interface_projection_refs: Vec::new(),
                        interface_method_refs: Vec::new(),
                        field_refs: Vec::new(),
                        union_branch_refs: Vec::new(),
                    },
                },
                implementation: ActorImplementationIdentity::new(format!("implementation-{seed}")),
                method: ActorMethodIdentity::new(format!("method-{seed}")),
            };
            record
        }
    }

    #[test]
    fn public_kinds_and_status_map() {
        let record = fixtures::record(1, 10);
        assert_eq!(record.status_kind(), TaskStatusKind::Scheduled);
        assert_eq!(
            crate::model::TaskCancelResultKind::Canceled,
            TaskCancelResultKind::Canceled
        );
        let _ = DurableDuration::from_millis(1000);
        let _ = CancelInput {
            task_id: record.task_id,
        };
        let _ = DueScanInput { limit: 10 };
        let _ = StatusInput {
            task_id: TaskId::new("x"),
            retention: DurableDuration::from_millis(1000),
        };
        let _ = LeaseRecoveryInput {
            task_id: TaskId::new("x"),
            retry_not_before: DurableUtcTimestamp::from_millis(1),
        };
    }
}
