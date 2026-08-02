//! Executable corpus for C-router-activation-state: read/CAS/retry/recovery
//! contract frozen by `activation-state-contract-cases.json`.
//!
//! The corpus drives the real `CanonicalArtifactStore` (file adapter) with the
//! same durable DTO and CAS semantics that the Router-owned Mongo adapter must
//! keep (see doc/implementation/router-rust-migration-c-router-activation-state.md).

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Deserialize;
use skiff_artifact_identity::runtime_assembly_ref;
use skiff_artifact_model::{
    AssemblyIdentity, RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
};
use skiff_deployment::{
    fixtures::{empty_runtime_assembly_fixture, runtime_assembly_fixture},
    storage::{
        ActivationRecoveryAction, CanonicalArtifactStore, EcosystemStorageError,
        EnvironmentActivationState,
    },
};

static TEST_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
    steps: Vec<Step>,
    expected: Expected,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Expected {
    terminal: String,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Step {
    Initialize {
        committed_generation: u64,
        assembly: String,
        config: String,
    },
    Prepare {
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: String,
        config: String,
        participants: Vec<String>,
        expected: String,
    },
    Abort {
        activation_id: String,
        expected_generation: u64,
        expected: String,
    },
    Commit {
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        connected: Vec<String>,
        prepared: Vec<String>,
        expected: String,
    },
    Read {
        expected_generation: u64,
        expected_pending_activation_id: Option<String>,
        expected_participants: Option<Vec<String>>,
    },
    Recover {
        connected: Vec<String>,
        prepared: Vec<String>,
        expected_action: String,
        expected_replica_ids: Option<Vec<String>>,
    },
}

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let sequence = TEST_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-activation-state-contract-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create temp root");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Refs {
    committed: RuntimeAssemblyRef,
    candidate: RuntimeAssemblyRef,
    missing: RuntimeAssemblyRef,
    committed_config: RuntimeConfigSnapshotRef,
    candidate_config: RuntimeConfigSnapshotRef,
}

fn config_snapshot_ref(hex: char) -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: RuntimeConfigSnapshotId::parse(format!(
            "skiff-runtime-config-snapshot-v1:{}",
            hex.to_string().repeat(32)
        ))
        .expect("valid config snapshot id"),
    }
}

fn test_store() -> (TestRoot, CanonicalArtifactStore, Refs) {
    let temp = TestRoot::new();
    let store = CanonicalArtifactStore::create(temp.path()).expect("artifact store");
    let committed_assembly = empty_runtime_assembly_fixture().expect("committed fixture");
    let candidate_assembly = runtime_assembly_fixture().expect("candidate fixture");
    store
        .write_runtime_assembly(&committed_assembly)
        .expect("write committed assembly");
    store
        .write_runtime_assembly(&candidate_assembly)
        .expect("write candidate assembly");
    let refs = Refs {
        committed: runtime_assembly_ref(&committed_assembly).expect("committed ref"),
        candidate: runtime_assembly_ref(&candidate_assembly).expect("candidate ref"),
        missing: RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(format!(
                "skiff-runtime-assembly-v3:sha256:{}",
                "c".repeat(64)
            )),
        },
        committed_config: config_snapshot_ref('a'),
        candidate_config: config_snapshot_ref('b'),
    };
    (temp, store, refs)
}

fn run_case(case: &Case) {
    let (_temp, store, refs) = test_store();
    for step in &case.steps {
        run_step(&store, &refs, step);
    }
    assert_eq!(case.expected.terminal, "ok", "{}", case.name);
}

fn run_step(store: &CanonicalArtifactStore, refs: &Refs, step: &Step) {
    match step {
        Step::Initialize {
            committed_generation,
            assembly,
            config,
        } => {
            assert_eq!(assembly, "committed", "initialize assembly must exist");
            let state = EnvironmentActivationState::initial(
                "test",
                *committed_generation,
                refs.committed.clone(),
                config_ref(refs, config),
            );
            store
                .initialize_environment_activation(&state)
                .expect("initialize activation state");
        }
        Step::Prepare {
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config,
            participants,
            expected,
        } => {
            let result = store.prepare_environment_activation(
                "test",
                activation_id,
                *expected_generation,
                *candidate_generation,
                assembly_ref(refs, assembly),
                config_ref(refs, config),
                participants.clone(),
            );
            assert_expected_result(result, expected, "prepare");
        }
        Step::Abort {
            activation_id,
            expected_generation,
            expected,
        } => {
            let result =
                store.abort_environment_activation("test", activation_id, *expected_generation);
            assert_expected_result(result, expected, "abort");
        }
        Step::Commit {
            activation_id,
            expected_generation,
            candidate_generation,
            connected,
            prepared,
            expected,
        } => {
            let result = store.commit_environment_activation(
                "test",
                activation_id,
                *expected_generation,
                *candidate_generation,
                &refs.candidate,
                &refs.candidate_config,
                connected,
                prepared,
            );
            assert_expected_result(result, expected, "commit");
        }
        Step::Read {
            expected_generation,
            expected_pending_activation_id,
            expected_participants,
        } => {
            let state = store
                .read_environment_activation("test")
                .expect("read activation state");
            assert_eq!(
                state.committed.generation, *expected_generation,
                "read committed generation"
            );
            match (state.pending.as_ref(), expected_pending_activation_id) {
                (None, None) => {}
                (Some(pending), Some(activation_id)) => {
                    assert_eq!(&pending.activation_id, activation_id);
                    if let Some(participants) = expected_participants {
                        assert_eq!(&pending.participant_replica_ids, participants);
                    }
                }
                (actual, expected) => panic!("pending mismatch: {actual:?} vs {expected:?}"),
            }
        }
        Step::Recover {
            connected,
            prepared,
            expected_action,
            expected_replica_ids,
        } => {
            let state = store
                .read_environment_activation("test")
                .expect("read activation state for recovery");
            let action = state
                .recovery_action(connected, prepared)
                .expect("recovery action");
            match (expected_action.as_str(), action) {
                ("stableCommitted", ActivationRecoveryAction::StableCommitted) => {}
                ("replayPrepare", ActivationRecoveryAction::ReplayPrepare { replica_ids }) => {
                    assert_eq!(
                        &replica_ids,
                        expected_replica_ids.as_ref().expect("replay replica ids")
                    );
                }
                ("commitPending", ActivationRecoveryAction::CommitPending) => {}
                ("abortPending", ActivationRecoveryAction::AbortPending { .. }) => {}
                (expected, actual) => panic!("recovery action mismatch: {expected} vs {actual:?}"),
            }
        }
    }
}

fn assembly_ref(refs: &Refs, name: &str) -> RuntimeAssemblyRef {
    match name {
        "committed" => refs.committed.clone(),
        "candidate" => refs.candidate.clone(),
        "missing" => refs.missing.clone(),
        other => panic!("unknown assembly fixture {other}"),
    }
}

fn config_ref(refs: &Refs, name: &str) -> RuntimeConfigSnapshotRef {
    match name {
        "a" => refs.committed_config.clone(),
        "b" => refs.candidate_config.clone(),
        other => panic!("unknown config fixture {other}"),
    }
}

fn assert_expected_result<T>(
    result: Result<T, EcosystemStorageError>,
    expected: &str,
    operation: &str,
) {
    match expected {
        "ok" => assert!(result.is_ok(), "{operation} expected ok"),
        "casMismatch" => assert!(
            matches!(result, Err(EcosystemStorageError::CasMismatch { .. })),
            "{operation} expected CAS mismatch"
        ),
        "invalid" => assert!(result.is_err(), "{operation} expected error"),
        other => panic!("unknown expected result {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_state_contract_corpus_passes() {
        let corpus: Corpus = serde_json::from_str(include_str!(
            "../../cross-system-fixtures/package-service-ecosystem/activation-state-contract-cases.json"
        ))
        .expect("activation state contract corpus must parse");
        assert_eq!(
            corpus.schema_version, "skiff-activation-state-contract-corpus-v1",
            "schema version"
        );
        assert!(corpus.cases.len() >= 6, "corpus must stay exhaustive");
        for case in &corpus.cases {
            run_case(case);
        }
    }
}
