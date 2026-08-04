//! C-bootstrap corpus consumer.
//!
//! Freezes the repository read port semantics (`CommittedActivationBootstrapReader`),
//! the durable→shared projection, the strict loader fail-closed matrix and the
//! initial `ActiveRoutingEpochStore` publication contract. Only contract docs and
//! corpus tests are written here; `CommittedActivationBootstrapReader`,
//! `RoutingEpoch` and `ActiveRoutingEpochStore` are W-bootstrap production types.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Deserialize;
use serde_json::{json, Value};
use skiff_artifact_identity::{
    runtime_assembly_ref, ProfileActivationStatePath, RuntimeAssemblyRecordPath,
};
use skiff_artifact_model::{
    AssemblyIdentity, RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
};
use skiff_deployment::{
    fixtures::empty_runtime_assembly_fixture,
    storage::{
        ActivationRecoveryAction, CanonicalArtifactStore, CommittedActivation,
        ProfileActivationState,
    },
};

static TEST_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let sequence = TEST_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-bootstrap-chain-corpus-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create temp root");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn test_store() -> (TestRoot, CanonicalArtifactStore) {
    let root = TestRoot::new();
    let store = CanonicalArtifactStore::create(root.path()).expect("create artifact store");
    (root, store)
}

fn assembly_ref(fill: char) -> RuntimeAssemblyRef {
    RuntimeAssemblyRef {
        assembly_identity: AssemblyIdentity::new(format!(
            "skiff-runtime-assembly-v3:sha256:{}",
            fill.to_string().repeat(64)
        )),
    }
}

fn snapshot_ref(fill: char) -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: RuntimeConfigSnapshotId::parse(format!(
            "skiff-runtime-config-snapshot-v1:{}",
            fill.to_string().repeat(32)
        ))
        .expect("snapshot id"),
    }
}

fn profile_path(store: &CanonicalArtifactStore, profile: &str) -> PathBuf {
    store.root().join(
        ProfileActivationStatePath::new(profile)
            .expect("profile path")
            .as_relative_path()
            .as_path(),
    )
}

fn assembly_record_path(store: &CanonicalArtifactStore, reference: &RuntimeAssemblyRef) -> PathBuf {
    store.root().join(
        RuntimeAssemblyRecordPath::new(reference)
            .expect("record path")
            .as_relative_path()
            .as_path(),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChainCorpus {
    schema_version: String,
    states: Vec<StateCase>,
    projection: ProjectionContract,
    epoch: EpochContract,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StateCase {
    id: String,
    kind: String,
    repository_read: String,
    bootstrap_outcome: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProjectionContract {
    input: String,
    output: String,
    pending: String,
    wire: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EpochContract {
    fields: Vec<String>,
    publication: String,
    single_authority: bool,
    pending_never_enters: bool,
    actor_projection_owner: String,
}

fn corpus() -> ChainCorpus {
    serde_json::from_str(include_str!("fixtures/bootstrap-chain-corpus.json"))
        .expect("bootstrap chain corpus must decode")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_schema_and_epoch_publication_contract_are_frozen() {
        let corpus = corpus();
        assert_eq!(
            corpus.schema_version,
            "skiff-router-rust-bootstrap-chain-corpus-v1"
        );

        assert_eq!(
            corpus.epoch.fields,
            [
                "profile",
                "assemblyGeneration",
                "assemblyIdentity",
                "configSnapshotId",
                "immutableIngressDeploymentActorProjection",
            ]
        );
        assert_eq!(corpus.epoch.publication, "atomic-arc-replacement");
        assert!(corpus.epoch.single_authority);
        assert!(corpus.epoch.pending_never_enters);
        assert_eq!(corpus.epoch.actor_projection_owner, "A0");

        assert!(corpus.projection.input.contains("CommittedActivation"));
        assert!(corpus.projection.output.contains("CommittedBootstrapRefs"));
        assert_eq!(corpus.projection.pending, "no projection");
        assert!(corpus
            .projection
            .wire
            .contains("RouterBootstrapActivationFrameHeader"));

        let kinds = corpus
            .states
            .iter()
            .map(|state| state.kind.as_str())
            .collect::<Vec<_>>();
        for required in [
            "committedOnly",
            "pendingPresent",
            "missing",
            "malformed",
            "committedRefMissing",
            "committedRefMismatch",
        ] {
            assert!(
                kinds.contains(&required),
                "corpus must contain state kind {required}"
            );
        }
    }

    #[test]
    fn repository_read_matrix_and_bootstrap_outcomes_are_consistent() {
        let corpus = corpus();
        for state in corpus.states {
            let (_root, store) = test_store();
            let assembly = empty_runtime_assembly_fixture().expect("assembly fixture");
            store
                .write_runtime_assembly(&assembly)
                .expect("write assembly record");
            let committed_ref = runtime_assembly_ref(&assembly).expect("assembly ref");

            match state.kind.as_str() {
                "committedOnly" => {
                    store
                        .initialize_profile_activation(&ProfileActivationState::initial(
                            "prod",
                            1,
                            committed_ref.clone(),
                            snapshot_ref('a'),
                        ))
                        .expect("initialize committed state");
                }
                "pendingPresent" => {
                    store
                        .initialize_profile_activation(&ProfileActivationState::initial(
                            "prod",
                            1,
                            committed_ref.clone(),
                            snapshot_ref('a'),
                        ))
                        .expect("initialize committed state");
                    store
                        .prepare_profile_activation(
                            "prod",
                            "activation-1",
                            1,
                            2,
                            committed_ref.clone(),
                            snapshot_ref('a'),
                            vec!["runtime-a".to_string()],
                        )
                        .expect("prepare pending activation");
                }
                "missing" => {}
                "malformed" => {
                    let path = profile_path(&store, "prod");
                    fs::create_dir_all(path.parent().expect("parent")).expect("create env dir");
                    fs::write(&path, b"{\"schemaVersion\":").expect("write malformed state");
                }
                "committedRefMissing" => {
                    store
                        .initialize_profile_activation(&ProfileActivationState::initial(
                            "prod",
                            1,
                            committed_ref.clone(),
                            snapshot_ref('a'),
                        ))
                        .expect("initialize committed state");
                    fs::remove_file(assembly_record_path(&store, &committed_ref))
                        .expect("remove assembly record");
                }
                "committedRefMismatch" => {
                    let mismatched_ref = assembly_ref('c');
                    let mut state = ProfileActivationState::initial(
                        "prod",
                        1,
                        committed_ref.clone(),
                        snapshot_ref('a'),
                    );
                    state.committed.assembly = mismatched_ref.clone();
                    // The state file is written directly (the repository cannot
                    // initialize a mismatched committed ref by construction).
                    let bytes = skiff_canonical_json::canonical_json_bytes(&state)
                        .expect("canonical state bytes");
                    let mut value: Value = serde_json::from_slice(&bytes).expect("state json");
                    value["committed"]["assembly"]["assemblyIdentity"] =
                        json!(mismatched_ref.assembly_identity.as_str());
                    let path = profile_path(&store, "prod");
                    fs::create_dir_all(path.parent().expect("parent")).expect("create env dir");
                    fs::write(&path, serde_json::to_vec(&value).expect("state bytes"))
                        .expect("write state");
                    // Record at the mismatched ref path carries a different
                    // declared identity, so the strict reader must fail closed.
                    fs::write(
                        assembly_record_path(&store, &mismatched_ref),
                        skiff_canonical_json::canonical_json_bytes(&assembly)
                            .expect("assembly bytes"),
                    )
                    .expect("write mismatched assembly record");
                }
                other => panic!("unsupported state kind {other}"),
            }

            // Uniform repository read assertion driven by the corpus.
            let read = store.read_profile_activation("prod");
            match state.repository_read.as_str() {
                "ok" => {
                    let record =
                        read.unwrap_or_else(|error| panic!("{} must read, got {error}", state.id));
                    match state.kind.as_str() {
                        "committedOnly" => {
                            assert!(record.pending.is_none());
                            assert_eq!(
                                record.committed,
                                CommittedActivation {
                                    generation: 1,
                                    assembly: committed_ref,
                                    config_snapshot: snapshot_ref('a'),
                                }
                            );
                            assert_eq!(
                                record.recovery_action(&[], &[]).expect("recovery action"),
                                ActivationRecoveryAction::StableCommitted
                            );
                        }
                        "pendingPresent" => {
                            assert!(
                                record.pending.is_some(),
                                "{} repository must surface pending to the bootstrap reader",
                                state.id
                            );
                            assert_eq!(
                                record.recovery_action(&[], &[]).expect("recovery action"),
                                ActivationRecoveryAction::AbortPending {
                                    activation_id: "activation-1".to_string()
                                },
                                "{} must surface a recoverable pending activation",
                                state.id
                            );
                        }
                        other => panic!("unexpected ok repository read for kind {other}"),
                    }
                }
                "reject" => {
                    assert!(
                        read.is_err(),
                        "{} repository read must fail closed",
                        state.id
                    );
                }
                other => panic!("unsupported repositoryRead {other}"),
            }

            // Bootstrap outcome mapping: committedOnly is stable; pendingPresent
            // carries recovery semantics (committed published + recovery
            // installed, E-activation §4.2); every other negative kind must
            // fail closed. Pending candidates are never projected as committed.
            match state.bootstrap_outcome.as_str() {
                "stableCommitted" => {
                    assert_eq!(state.kind, "committedOnly");
                }
                "recoverPending" => {
                    assert_eq!(state.kind, "pendingPresent");
                }
                outcome @ ("failClosedMissing"
                | "failClosedMalformed"
                | "failClosedIdentityMismatch") => {
                    assert_ne!(state.kind, "committedOnly", "{outcome} for {}", state.id);
                    assert_ne!(state.kind, "pendingPresent", "{outcome} for {}", state.id);
                }
                other => panic!("unsupported bootstrapOutcome {other}"),
            }
        }
    }

    #[test]
    fn initial_publication_rejects_pending_by_construction() {
        let (_root, store) = test_store();
        let assembly = empty_runtime_assembly_fixture().expect("assembly fixture");
        store
            .write_runtime_assembly(&assembly)
            .expect("write assembly record");

        let mut state =
            ProfileActivationState::initial("prod", 1, assembly_ref('a'), snapshot_ref('a'));
        state.pending = Some(skiff_deployment::storage::PendingActivation {
            activation_id: "activation-1".to_string(),
            expected_generation: 1,
            candidate_generation: 2,
            assembly: assembly_ref('a'),
            config_snapshot: snapshot_ref('a'),
            participant_replica_ids: vec!["runtime-a".to_string()],
        });
        assert!(
            store.initialize_profile_activation(&state).is_err(),
            "initial activation state cannot contain pending (E-bootstrap fail closed)"
        );
    }
}
