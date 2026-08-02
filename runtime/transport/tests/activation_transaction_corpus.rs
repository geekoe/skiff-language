//! Executable corpus for C-model-activation + C-activation-coordinator.
//!
//! The corpus fixture lives in the shared cross-system fixtures directory:
//! `activation-transaction-cases.json`. Every wire event passes through the real
//! `skiff-runtime-transport` codec; coordinator behavior is driven by the
//! test-only fake harness described in the C-activation-coordinator contract
//! (owner/invariant, decision terminals, stale ACK rejection, queue-full,
//! disconnect/replacement/timeout/shutdown, cold recovery rebind).

use serde::Deserialize;
use skiff_artifact_model::{
    AssemblyActivationControl, AssemblyActivationRejectReason, AssemblyIdentity,
    RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
};
use skiff_runtime_transport::assembly_activation::{
    decode_assembly_activation_frame, encode_assembly_activation_frame,
    AssemblyActivationFrameDirection,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Corpus {
    schema_version: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Case {
    name: String,
    contract: String,
    tx: Option<TxFixture>,
    steps: Option<Vec<Step>>,
    runs: Option<Vec<Run>>,
    expected: Option<Expected>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Run {
    tx: Option<TxFixture>,
    steps: Vec<Step>,
    expected: Expected,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TxFixture {
    environment: String,
    activation_id: String,
    expected_generation: u64,
    candidate_generation: u64,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PendingFixture {
    activation_id: String,
    expected_generation: u64,
    candidate_generation: u64,
    participant_replica_ids: Vec<String>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LeaseFixture {
    replica_id: String,
    session_epoch: u64,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Expected {
    terminal: String,
    durable_state: DurableStateFixture,
    published: bool,
    #[serde(default)]
    listener_open: Option<bool>,
    #[serde(default)]
    readiness: Option<bool>,
    #[serde(default)]
    session_aborts: Vec<String>,
    #[serde(default)]
    enqueues: Vec<Vec<String>>,
    #[serde(default)]
    stale_acks: u64,
    #[serde(default)]
    recovery: Option<bool>,
    #[serde(default)]
    active_epoch: Option<u64>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DurableStateFixture {
    committed_generation: u64,
    pending: Option<PendingFixture>,
}

#[derive(Deserialize, Clone)]
#[serde(
    tag = "step",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Step {
    ReadState {
        committed_generation: u64,
        pending: Option<PendingFixture>,
    },
    CaptureActiveEpoch {
        generation: u64,
    },
    LoadCandidate {
        result: String,
    },
    QueryCandidates {
        leases: Vec<LeaseFixture>,
    },
    Revalidate {
        result: String,
    },
    DurablePrepare {
        expected: String,
    },
    EnqueuePrepare {
        replica_id: String,
        result: String,
    },
    Ack {
        kind: String,
        replica_id: String,
        session_epoch: u64,
        expected: String,
    },
    DurableCommit {
        expected: String,
        #[serde(default)]
        durable_outcome: Option<String>,
    },
    PublishEpoch {
        expected: String,
    },
    PublishCommitted,
    EnqueueCommit {
        replica_id: String,
        result: String,
    },
    EnqueueAbort {
        replica_id: String,
        result: String,
    },
    DurableAbort {
        expected: String,
        #[serde(default)]
        queue_full_for: Vec<String>,
    },
    SessionAbort {
        replica_id: String,
    },
    Disconnect {
        replica_id: String,
    },
    Replacement {
        replica_id: String,
        session_epoch: u64,
    },
    Timeout {
        after: String,
    },
    Register {
        replica_id: String,
        session_epoch: u64,
    },
    Shutdown,
    ProcessExit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pending {
    activation_id: String,
    expected_generation: u64,
    candidate_generation: u64,
    participant_replica_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Durable {
    committed_generation: u64,
    pending: Option<Pending>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    replica_id: String,
    session_epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StagedSession {
    replica_id: String,
    session_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Decision {
    Commit,
    Abort,
}

#[derive(Clone, Debug)]
struct Tx {
    environment: String,
    activation_id: String,
    expected_generation: u64,
    candidate_generation: u64,
}

struct Harness {
    tx: Tx,
    durable: Durable,
    active_epoch: Option<u64>,
    published: bool,
    listener_open: bool,
    readiness: bool,
    bindings: Vec<Binding>,
    candidate_set: Vec<String>,
    staged: Vec<StagedSession>,
    prepared: Vec<String>,
    rejected: Vec<String>,
    decision: Option<Decision>,
    terminal: Option<String>,
    enqueues: Vec<(String, String)>,
    session_aborts: Vec<String>,
    stale_acks: u64,
    recovery_active: bool,
}

impl Harness {
    fn new(tx: TxFixture) -> Self {
        Self {
            tx: Tx {
                environment: tx.environment,
                activation_id: tx.activation_id,
                expected_generation: tx.expected_generation,
                candidate_generation: tx.candidate_generation,
            },
            durable: Durable {
                committed_generation: 0,
                pending: None,
            },
            active_epoch: None,
            published: false,
            listener_open: false,
            readiness: false,
            bindings: Vec::new(),
            candidate_set: Vec::new(),
            staged: Vec::new(),
            prepared: Vec::new(),
            rejected: Vec::new(),
            decision: None,
            terminal: None,
            enqueues: Vec::new(),
            session_aborts: Vec::new(),
            stale_acks: 0,
            recovery_active: false,
        }
    }

    fn run_steps(&mut self, steps: &[Step]) -> Result<(), String> {
        for step in steps {
            self.apply_step(step)?;
        }
        Ok(())
    }

    fn apply_step(&mut self, step: &Step) -> Result<(), String> {
        match step {
            Step::ReadState {
                committed_generation,
                pending,
            } => {
                self.durable = Durable {
                    committed_generation: *committed_generation,
                    pending: pending.clone().map(Into::into),
                };
                self.recovery_active = self.durable.pending.is_some();
                Ok(())
            }
            Step::CaptureActiveEpoch { generation } => {
                if self.durable.committed_generation != *generation {
                    return Err(format!(
                        "active epoch {generation} != committed {}",
                        self.durable.committed_generation
                    ));
                }
                self.active_epoch = Some(*generation);
                self.listener_open = true;
                Ok(())
            }
            Step::LoadCandidate { result } => match result.as_str() {
                "ok" => {
                    if self.durable.committed_generation + 1 != self.tx.candidate_generation {
                        return Err("candidate generation must be committed + 1".to_string());
                    }
                    Ok(())
                }
                "missing" | "malformed" => {
                    if self.decision.is_some() {
                        return Err("candidate load failure after decision".to_string());
                    }
                    if self.durable.pending.is_some() {
                        self.auto_abort()?;
                    } else {
                        self.terminal = Some("failed".to_string());
                    }
                    Ok(())
                }
                other => Err(format!("unknown loadCandidate result {other}")),
            },
            Step::QueryCandidates { leases } => {
                self.candidate_set = leases
                    .iter()
                    .map(|lease| lease.replica_id.clone())
                    .collect();
                self.bindings = leases
                    .iter()
                    .map(|lease| Binding {
                        replica_id: lease.replica_id.clone(),
                        session_epoch: lease.session_epoch,
                    })
                    .collect();
                Ok(())
            }
            Step::Revalidate { result } => match result.as_str() {
                "ok" => Ok(()),
                "stale" => {
                    self.terminal = Some("failed".to_string());
                    Ok(())
                }
                other => Err(format!("unknown revalidate result {other}")),
            },
            Step::DurablePrepare { expected } => {
                if self.candidate_set.is_empty() {
                    return Err("durable prepare without candidate set".to_string());
                }
                if self.durable.committed_generation != self.tx.expected_generation
                    || self.durable.pending.is_some()
                {
                    if expected == "casMismatch" {
                        self.terminal = Some("failed".to_string());
                        return Ok(());
                    }
                    return Err("durable prepare unexpected CAS mismatch".to_string());
                }
                self.durable.pending = Some(Pending {
                    activation_id: self.tx.activation_id.clone(),
                    expected_generation: self.tx.expected_generation,
                    candidate_generation: self.tx.candidate_generation,
                    participant_replica_ids: self.candidate_set.clone(),
                });
                Ok(())
            }
            Step::EnqueuePrepare { replica_id, result } => {
                if !self.candidate_set.iter().any(|r| r == replica_id) {
                    return Err(format!("prepare enqueue for non-candidate {replica_id}"));
                }
                match result.as_str() {
                    "ok" => {
                        assert_prepare_wire(&self.tx, replica_id);
                        self.staged.push(StagedSession {
                            replica_id: replica_id.clone(),
                            session_epoch: self.binding_epoch(replica_id).ok_or_else(|| {
                                format!("prepare enqueue without binding {replica_id}")
                            })?,
                        });
                        self.enqueues
                            .push(("prepare".to_string(), replica_id.clone()));
                        Ok(())
                    }
                    "queueFull" => {
                        self.session_aborts.push(replica_id.clone());
                        self.auto_abort()
                    }
                    other => Err(format!("unknown enqueue result {other}")),
                }
            }
            Step::Ack {
                kind,
                replica_id,
                session_epoch,
                expected,
            } => {
                let accepted = self.decision.is_none()
                    && self.participant_set().iter().any(|r| r == replica_id)
                    && self
                        .staged
                        .iter()
                        .any(|s| &s.replica_id == replica_id && s.session_epoch == *session_epoch)
                    && self.binding_epoch(replica_id) == Some(*session_epoch)
                    && !self.prepared.iter().any(|r| r == replica_id)
                    && !self.rejected.iter().any(|r| r == replica_id);
                match expected.as_str() {
                    "accepted" => {
                        if !accepted {
                            return Err(format!(
                                "expected accepted ACK was stale for {replica_id}"
                            ));
                        }
                        match kind.as_str() {
                            "prepared" => {
                                assert_prepared_wire(&self.tx, replica_id);
                                self.prepared.push(replica_id.clone());
                                Ok(())
                            }
                            "reject" => {
                                assert_reject_wire(&self.tx, replica_id);
                                self.rejected.push(replica_id.clone());
                                self.auto_abort()
                            }
                            other => Err(format!("unknown ACK kind {other}")),
                        }
                    }
                    "staleRejected" => {
                        if accepted {
                            return Err(format!(
                                "expected stale ACK was accepted for {replica_id}"
                            ));
                        }
                        self.stale_acks += 1;
                        Ok(())
                    }
                    other => Err(format!("unknown ACK expectation {other}")),
                }
            }
            Step::DurableCommit {
                expected,
                durable_outcome,
            } => {
                if self.decision.is_some() {
                    return Err("durable commit after decision".to_string());
                }
                let participants = self.participant_set();
                let all_prepared = participants.iter().all(|r| self.prepared.contains(r))
                    && self.prepared.len() == participants.len();
                let bindings_ok = participants
                    .iter()
                    .all(|r| self.bindings.iter().any(|b| &b.replica_id == r));
                let pending_matches = self
                    .durable
                    .pending
                    .as_ref()
                    .map(|p| {
                        p.activation_id == self.tx.activation_id
                            && p.expected_generation == self.tx.expected_generation
                            && p.candidate_generation == self.tx.candidate_generation
                    })
                    .unwrap_or(false);
                if !all_prepared
                    || !bindings_ok
                    || !pending_matches
                    || self.durable.committed_generation != self.tx.expected_generation
                {
                    return Err("durable commit preconditions failed".to_string());
                }
                match expected.as_str() {
                    "ok" => {
                        self.durable.committed_generation = self.tx.candidate_generation;
                        self.durable.pending = None;
                        self.decision = Some(Decision::Commit);
                        self.recovery_active = false;
                        Ok(())
                    }
                    "casMismatch" => match durable_outcome.as_deref() {
                        Some("committed") => {
                            self.durable.committed_generation = self.tx.candidate_generation;
                            self.durable.pending = None;
                            self.decision = Some(Decision::Commit);
                            self.recovery_active = false;
                            self.publish_candidate()?;
                            self.enqueue_commit_to_exact()?;
                            Ok(())
                        }
                        Some("aborted") => {
                            self.durable.pending = None;
                            self.decision = Some(Decision::Abort);
                            self.recovery_active = false;
                            self.enqueue_abort_to_staged()?;
                            Ok(())
                        }
                        other => Err(format!(
                            "durable commit mismatch requires outcome, got {other:?}"
                        )),
                    },
                    other => Err(format!("unknown durable commit expectation {other}")),
                }
            }
            Step::PublishEpoch { expected } => {
                if self.decision != Some(Decision::Commit) {
                    return Err("publish without commit decision".to_string());
                }
                if expected != "ok" {
                    return Err("epoch publish must be infallible".to_string());
                }
                self.publish_candidate()
            }
            Step::PublishCommitted => {
                self.active_epoch = Some(self.durable.committed_generation);
                self.published = true;
                self.listener_open = true;
                Ok(())
            }
            Step::EnqueueCommit { replica_id, result } => {
                if self.decision != Some(Decision::Commit) {
                    return Err("commit enqueue without commit decision".to_string());
                }
                if !self.is_staged_exact(replica_id) {
                    return Ok(());
                }
                match result.as_str() {
                    "ok" => {
                        assert_commit_wire(&self.tx, replica_id);
                        self.enqueues
                            .push(("commit".to_string(), replica_id.clone()));
                        Ok(())
                    }
                    "queueFull" => {
                        self.session_aborts.push(replica_id.clone());
                        Ok(())
                    }
                    other => Err(format!("unknown enqueue result {other}")),
                }
            }
            Step::EnqueueAbort { replica_id, result } => {
                if self.decision != Some(Decision::Abort) {
                    return Err("abort enqueue without abort decision".to_string());
                }
                match result.as_str() {
                    "ok" => {
                        assert_abort_wire(&self.tx, replica_id);
                        self.enqueues
                            .push(("abort".to_string(), replica_id.clone()));
                        Ok(())
                    }
                    "queueFull" => {
                        self.session_aborts.push(replica_id.clone());
                        Ok(())
                    }
                    other => Err(format!("unknown enqueue result {other}")),
                }
            }
            Step::DurableAbort {
                expected,
                queue_full_for,
            } => {
                if expected != "ok" {
                    return Err("durable abort expected to succeed".to_string());
                }
                if self.durable.pending.is_none() {
                    return Err("durable abort without pending".to_string());
                }
                self.durable.pending = None;
                self.decision = Some(Decision::Abort);
                self.recovery_active = false;
                self.enqueue_abort_to_staged_with_failures(queue_full_for)?;
                self.terminal = Some("aborted".to_string());
                Ok(())
            }
            Step::SessionAbort { replica_id } => {
                self.session_aborts.push(replica_id.clone());
                Ok(())
            }
            Step::Disconnect { replica_id } => {
                self.bindings.retain(|b| &b.replica_id != replica_id);
                if self.decision.is_none() && self.durable.pending.is_some() {
                    self.auto_abort()
                } else {
                    Ok(())
                }
            }
            Step::Replacement {
                replica_id,
                session_epoch,
            } => {
                if let Some(binding) = self
                    .bindings
                    .iter_mut()
                    .find(|b| &b.replica_id == replica_id)
                {
                    binding.session_epoch = *session_epoch;
                }
                if self.decision.is_none() && self.durable.pending.is_some() {
                    self.auto_abort()
                } else {
                    Ok(())
                }
            }
            Step::Timeout { after } => {
                if after != "prepared" {
                    return Err(format!("unknown timeout phase {after}"));
                }
                if self.decision.is_none() && self.durable.pending.is_some() {
                    self.auto_abort()
                } else {
                    Ok(())
                }
            }
            Step::Register {
                replica_id,
                session_epoch,
            } => {
                self.bindings.retain(|b| &b.replica_id != replica_id);
                self.bindings.push(Binding {
                    replica_id: replica_id.clone(),
                    session_epoch: *session_epoch,
                });
                if self.durable.pending.is_some() {
                    let participants = self.participant_set();
                    if participants.iter().any(|r| r == replica_id) {
                        self.staged.retain(|s| &s.replica_id != replica_id);
                        assert_prepare_wire(&self.tx, replica_id);
                        self.staged.push(StagedSession {
                            replica_id: replica_id.clone(),
                            session_epoch: *session_epoch,
                        });
                        self.enqueues
                            .push(("prepare".to_string(), replica_id.clone()));
                    }
                }
                Ok(())
            }
            Step::Shutdown => {
                if self.decision == Some(Decision::Commit) {
                    self.publish_candidate()?;
                    self.enqueue_commit_to_exact()?;
                } else if self.decision == Some(Decision::Abort) {
                    self.terminal = Some("aborted".to_string());
                } else if self.durable.pending.is_some() {
                    self.auto_abort()?;
                } else {
                    self.terminal = Some("shutdown".to_string());
                }
                Ok(())
            }
            Step::ProcessExit => {
                self.terminal = Some("exited".to_string());
                Ok(())
            }
        }
    }

    fn participant_set(&self) -> Vec<String> {
        match &self.durable.pending {
            Some(pending) => pending.participant_replica_ids.clone(),
            None => self.candidate_set.clone(),
        }
    }

    fn binding_epoch(&self, replica_id: &str) -> Option<u64> {
        self.bindings
            .iter()
            .find(|b| b.replica_id == replica_id)
            .map(|b| b.session_epoch)
    }

    fn is_staged_exact(&self, replica_id: &str) -> bool {
        self.staged.iter().any(|s| {
            s.replica_id == replica_id && self.binding_epoch(replica_id) == Some(s.session_epoch)
        })
    }

    fn auto_abort(&mut self) -> Result<(), String> {
        if self.decision.is_some() {
            return Err("auto abort after decision".to_string());
        }
        if self.durable.pending.is_none() {
            return Ok(());
        }
        self.durable.pending = None;
        self.decision = Some(Decision::Abort);
        self.recovery_active = false;
        self.enqueue_abort_to_staged()?;
        self.terminal = Some("aborted".to_string());
        Ok(())
    }

    fn enqueue_abort_to_staged(&mut self) -> Result<(), String> {
        self.enqueue_abort_to_staged_with_failures(&[])
    }

    fn enqueue_abort_to_staged_with_failures(
        &mut self,
        queue_full_for: &[String],
    ) -> Result<(), String> {
        let staged: Vec<(String, u64)> = self
            .staged
            .iter()
            .filter(|s| {
                !self.rejected.iter().any(|r| r == &s.replica_id)
                    && self.binding_epoch(&s.replica_id) == Some(s.session_epoch)
            })
            .map(|s| (s.replica_id.clone(), s.session_epoch))
            .collect();
        for (replica_id, _) in staged {
            if queue_full_for.iter().any(|r| r == &replica_id) {
                self.session_aborts.push(replica_id);
            } else {
                assert_abort_wire(&self.tx, &replica_id);
                self.enqueues.push(("abort".to_string(), replica_id));
            }
        }
        Ok(())
    }

    fn enqueue_commit_to_exact(&mut self) -> Result<(), String> {
        let staged: Vec<String> = self
            .staged
            .iter()
            .filter(|s| self.binding_epoch(&s.replica_id) == Some(s.session_epoch))
            .map(|s| s.replica_id.clone())
            .collect();
        for replica_id in staged {
            assert_commit_wire(&self.tx, &replica_id);
            self.enqueues.push(("commit".to_string(), replica_id));
        }
        Ok(())
    }

    fn publish_candidate(&mut self) -> Result<(), String> {
        self.active_epoch = Some(self.tx.candidate_generation);
        self.published = true;
        self.listener_open = true;
        Ok(())
    }

    fn assert_expected(&self, case: &str, expected: &Expected) {
        let terminal = self
            .terminal
            .clone()
            .unwrap_or_else(|| self.infer_terminal());
        assert_eq!(terminal, expected.terminal, "{case} terminal");
        assert_eq!(
            self.durable.committed_generation, expected.durable_state.committed_generation,
            "{case} committed generation"
        );
        match (&self.durable.pending, &expected.durable_state.pending) {
            (None, None) => {}
            (Some(actual), Some(expected_pending)) => {
                assert_eq!(
                    actual.activation_id, expected_pending.activation_id,
                    "{case} pending activation id"
                );
                assert_eq!(
                    actual.expected_generation, expected_pending.expected_generation,
                    "{case} pending expected generation"
                );
                assert_eq!(
                    actual.candidate_generation, expected_pending.candidate_generation,
                    "{case} pending candidate generation"
                );
                assert_eq!(
                    actual.participant_replica_ids, expected_pending.participant_replica_ids,
                    "{case} pending participants"
                );
            }
            (actual, expected_pending) => {
                panic!("{case} pending mismatch: {actual:?} != {expected_pending:?}")
            }
        }
        assert_eq!(self.published, expected.published, "{case} published");
        if let Some(listener_open) = expected.listener_open {
            assert_eq!(self.listener_open, listener_open, "{case} listener open");
        }
        if let Some(readiness) = expected.readiness {
            assert_eq!(self.readiness, readiness, "{case} readiness");
        }
        let mut aborts = self.session_aborts.clone();
        aborts.sort();
        let mut expected_aborts = expected.session_aborts.clone();
        expected_aborts.sort();
        assert_eq!(aborts, expected_aborts, "{case} session aborts");
        let expected_enqueues: Vec<(String, String)> = expected
            .enqueues
            .iter()
            .map(|pair| (pair[0].clone(), pair[1].clone()))
            .collect();
        assert_eq!(self.enqueues, expected_enqueues, "{case} enqueues");
        assert_eq!(self.stale_acks, expected.stale_acks, "{case} stale acks");
        if let Some(recovery) = expected.recovery {
            assert_eq!(self.recovery_active, recovery, "{case} recovery active");
        }
        if let Some(active_epoch) = expected.active_epoch {
            assert_eq!(self.active_epoch, Some(active_epoch), "{case} active epoch");
        }
    }

    fn infer_terminal(&self) -> String {
        match self.decision {
            Some(Decision::Commit) => "committed".to_string(),
            Some(Decision::Abort) => "aborted".to_string(),
            None if self.durable.pending.is_some() => "waitingRecovery".to_string(),
            None if self.published && self.active_epoch.is_some() => "committed".to_string(),
            None => "inProgress".to_string(),
        }
    }
}

impl From<PendingFixture> for Pending {
    fn from(fixture: PendingFixture) -> Self {
        Self {
            activation_id: fixture.activation_id,
            expected_generation: fixture.expected_generation,
            candidate_generation: fixture.candidate_generation,
            participant_replica_ids: fixture.participant_replica_ids,
        }
    }
}

fn assembly_ref(byte: char) -> RuntimeAssemblyRef {
    RuntimeAssemblyRef {
        assembly_identity: AssemblyIdentity::new(format!(
            "skiff-runtime-assembly-v3:sha256:{}",
            byte.to_string().repeat(64)
        )),
    }
}

fn config_ref(byte: char) -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: RuntimeConfigSnapshotId::parse(format!(
            "skiff-runtime-config-snapshot-v1:{}",
            byte.to_string().repeat(32)
        ))
        .expect("valid config snapshot id"),
    }
}

fn router_to_runtime(control: &AssemblyActivationControl) {
    let bytes = encode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RouterToRuntime,
        control,
    )
    .expect("router to runtime encode");
    assert_eq!(
        decode_assembly_activation_frame(AssemblyActivationFrameDirection::RouterToRuntime, &bytes)
            .expect("router to runtime decode"),
        *control
    );
    assert!(
        encode_assembly_activation_frame(
            AssemblyActivationFrameDirection::RuntimeToRouter,
            control
        )
        .is_err(),
        "reverse direction must fail"
    );
}

fn runtime_to_router(control: &AssemblyActivationControl) {
    let bytes = encode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RuntimeToRouter,
        control,
    )
    .expect("runtime to router encode");
    assert_eq!(
        decode_assembly_activation_frame(AssemblyActivationFrameDirection::RuntimeToRouter, &bytes)
            .expect("runtime to router decode"),
        *control
    );
    assert!(
        encode_assembly_activation_frame(
            AssemblyActivationFrameDirection::RouterToRuntime,
            control
        )
        .is_err(),
        "reverse direction must fail"
    );
}

fn assert_prepare_wire(tx: &Tx, replica_id: &str) {
    router_to_runtime(&AssemblyActivationControl::Prepare {
        environment: tx.environment.clone(),
        activation_id: tx.activation_id.clone(),
        expected_generation: tx.expected_generation,
        candidate_generation: tx.candidate_generation,
        assembly: assembly_ref('a'),
        config_snapshot: config_ref('a'),
        replica_id: replica_id.to_string(),
        service_db: None,
    });
}

fn assert_prepared_wire(tx: &Tx, replica_id: &str) {
    runtime_to_router(&AssemblyActivationControl::Prepared {
        environment: tx.environment.clone(),
        activation_id: tx.activation_id.clone(),
        expected_generation: tx.expected_generation,
        candidate_generation: tx.candidate_generation,
        assembly: assembly_ref('a'),
        config_snapshot: config_ref('a'),
        replica_id: replica_id.to_string(),
    });
}

fn assert_reject_wire(tx: &Tx, replica_id: &str) {
    runtime_to_router(&AssemblyActivationControl::Reject {
        environment: tx.environment.clone(),
        activation_id: tx.activation_id.clone(),
        expected_generation: tx.expected_generation,
        candidate_generation: tx.candidate_generation,
        assembly: assembly_ref('a'),
        config_snapshot: config_ref('a'),
        replica_id: replica_id.to_string(),
        reason: AssemblyActivationRejectReason::Admission,
    });
}

fn assert_commit_wire(tx: &Tx, replica_id: &str) {
    router_to_runtime(&AssemblyActivationControl::Commit {
        environment: tx.environment.clone(),
        activation_id: tx.activation_id.clone(),
        expected_generation: tx.expected_generation,
        candidate_generation: tx.candidate_generation,
        assembly: assembly_ref('a'),
        config_snapshot: config_ref('a'),
        replica_id: replica_id.to_string(),
        service_db: None,
    });
}

fn assert_abort_wire(tx: &Tx, replica_id: &str) {
    router_to_runtime(&AssemblyActivationControl::Abort {
        environment: tx.environment.clone(),
        activation_id: tx.activation_id.clone(),
        expected_generation: tx.expected_generation,
        candidate_generation: tx.candidate_generation,
        assembly: assembly_ref('a'),
        config_snapshot: config_ref('a'),
        replica_id: replica_id.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_transaction_corpus_live_and_cold_recovery_contracts() {
        let corpus: Corpus = serde_json::from_str(include_str!(
            "../../../cross-system-fixtures/package-service-ecosystem/activation-transaction-cases.json"
        ))
        .expect("activation transaction corpus must parse");
        assert_eq!(
            corpus.schema_version,
            "skiff-activation-transaction-corpus-v1"
        );
        let mut live_cases = 0;
        let mut cold_cases = 0;
        for case in &corpus.cases {
            match (&case.runs, &case.steps, &case.expected) {
                (Some(runs), None, None) => {
                    assert_eq!(case.contract, "coldRecovery", "{}", case.name);
                    let mut carried: Option<Durable> = None;
                    for run in runs {
                        let mut harness =
                            Harness::new(run.tx.clone().expect("run tx must be present"));
                        if let Some(durable) = &carried {
                            harness.durable = durable.clone();
                        }
                        harness
                            .run_steps(&run.steps)
                            .unwrap_or_else(|error| panic!("{}: {error}", case.name));
                        harness.assert_expected(&case.name, &run.expected);
                        carried = Some(harness.durable.clone());
                    }
                    cold_cases += 1;
                }
                (None, Some(steps), Some(expected)) => {
                    let mut harness =
                        Harness::new(case.tx.clone().expect("case tx must be present"));
                    harness
                        .run_steps(steps)
                        .unwrap_or_else(|error| panic!("{}: {error}", case.name));
                    harness.assert_expected(&case.name, expected);
                    if case.contract == "live" {
                        live_cases += 1;
                    } else {
                        cold_cases += 1;
                    }
                }
                _ => panic!("{} must have steps+expected or runs", case.name),
            }
        }
        assert!(live_cases >= 15, "live corpus must stay exhaustive");
        assert!(cold_cases >= 5, "cold recovery corpus must stay exhaustive");
    }

    #[test]
    fn activation_transaction_wire_subset_stays_frozen() {
        let frame_corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../../../cross-system-fixtures/package-service-ecosystem/runtime-wire.json"
        ))
        .expect("runtime wire corpus must parse");
        let controls: Vec<AssemblyActivationControl> = serde_json::from_str(include_str!(
            "../../../cross-system-fixtures/package-service-ecosystem/control-wire.json"
        ))
        .expect("control wire corpus must parse");
        let frames = frame_corpus["assemblyActivationFrames"]
            .as_array()
            .expect("assemblyActivationFrames");
        let transaction_frames: Vec<&serde_json::Value> = frames
            .iter()
            .filter(|frame| frame["name"].as_str() != Some("register"))
            .collect();
        assert_eq!(transaction_frames.len(), 5, "transaction wire subset");
        for frame in &transaction_frames {
            let name = frame["name"].as_str().expect("frame name");
            let direction = frame["direction"].as_str().expect("frame direction");
            let expected_direction = match name {
                "prepare" | "commit" | "abort" => "routerToRuntime",
                "prepared" | "reject" => "runtimeToRouter",
                other => panic!("unexpected transaction frame {other}"),
            };
            assert_eq!(direction, expected_direction, "{name} direction");
            let control_index = frame["controlIndex"].as_u64().expect("control index") as usize;
            let bytes = decode_hex(frame["frameHex"].as_str().expect("frame hex"));
            let decoded = decode_assembly_activation_frame(
                if direction == "routerToRuntime" {
                    AssemblyActivationFrameDirection::RouterToRuntime
                } else {
                    AssemblyActivationFrameDirection::RuntimeToRouter
                },
                &bytes,
            )
            .unwrap_or_else(|error| panic!("{name} decode: {error}"));
            assert_eq!(decoded, controls[control_index], "{name} golden control");
        }
    }

    fn decode_hex(value: &str) -> Vec<u8> {
        assert_eq!(value.len() % 2, 0, "byte hex must have whole octets");
        (0..value.len())
            .step_by(2)
            .map(|offset| u8::from_str_radix(&value[offset..offset + 2], 16).expect("valid hex"))
            .collect()
    }
}
