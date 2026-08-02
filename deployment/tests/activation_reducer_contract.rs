//! Executable contract for the adapter-independent activation reducer.
//!
//! The pure reducer (`deployment::activation_state`) must agree with the
//! frozen file adapter (`CanonicalArtifactStore`) on every CAS/validation
//! outcome for the same operation sequence. Reference existence checking is
//! adapter-side (file store / coordinator loader), so the shared
//! `activation-state-contract-cases.json` case that injects a missing assembly
//! is asserted as lexical acceptance at the reducer boundary.

#[cfg(test)]
mod tests {

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
        activation_state::{
            abort as reduce_abort, commit as reduce_commit, prepare as reduce_prepare, AbortInput,
            ActivationStateError, CommitInput, EnvironmentActivationState, PrepareInput,
        },
        fixtures::{empty_runtime_assembly_fixture, runtime_assembly_fixture},
        storage::{ActivationRecoveryAction, CanonicalArtifactStore, EcosystemStorageError},
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

    struct Refs {
        committed: RuntimeAssemblyRef,
        candidate: RuntimeAssemblyRef,
        missing: RuntimeAssemblyRef,
        committed_config: RuntimeConfigSnapshotRef,
        candidate_config: RuntimeConfigSnapshotRef,
    }

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Self {
            let sequence = TEST_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "skiff-activation-reducer-contract-{}-{sequence}",
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

    fn config_snapshot_ref(hex: char) -> RuntimeConfigSnapshotRef {
        RuntimeConfigSnapshotRef {
            snapshot_id: RuntimeConfigSnapshotId::parse(format!(
                "skiff-runtime-config-snapshot-v1:{}",
                hex.to_string().repeat(32)
            ))
            .expect("valid config snapshot id"),
        }
    }

    fn refs() -> Refs {
        let committed_fixture = empty_runtime_assembly_fixture().expect("committed fixture");
        let candidate_fixture = runtime_assembly_fixture().expect("candidate fixture");
        Refs {
            committed: runtime_assembly_ref(&committed_fixture).expect("committed ref"),
            candidate: runtime_assembly_ref(&candidate_fixture).expect("candidate ref"),
            missing: RuntimeAssemblyRef {
                assembly_identity: AssemblyIdentity::new(format!(
                    "skiff-runtime-assembly-v3:sha256:{}",
                    "c".repeat(64)
                )),
            },
            committed_config: config_snapshot_ref('a'),
            candidate_config: config_snapshot_ref('b'),
        }
    }

    fn assembly_ref<'a>(refs: &'a Refs, key: &str) -> &'a RuntimeAssemblyRef {
        match key {
            "committed" => &refs.committed,
            "candidate" => &refs.candidate,
            "missing" => &refs.missing,
            other => panic!("unknown corpus assembly ref {other}"),
        }
    }

    fn config_ref<'a>(refs: &'a Refs, key: &str) -> &'a RuntimeConfigSnapshotRef {
        match key {
            "a" => &refs.committed_config,
            "b" => &refs.candidate_config,
            other => panic!("unknown corpus config ref {other}"),
        }
    }

    fn initial_state(refs: &Refs, generation: u64) -> EnvironmentActivationState {
        EnvironmentActivationState::initial(
            "test",
            generation,
            refs.committed.clone(),
            refs.committed_config.clone(),
        )
    }

    fn reducer_result(
        state: &EnvironmentActivationState,
        refs: &Refs,
        step: &Step,
    ) -> Result<EnvironmentActivationState, ActivationStateError> {
        match step {
            Step::Prepare {
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config,
                participants,
                ..
            } => reduce_prepare(
                state,
                &PrepareInput {
                    environment: "test".to_string(),
                    activation_id: activation_id.clone(),
                    expected_generation: *expected_generation,
                    candidate_generation: *candidate_generation,
                    assembly: assembly_ref(refs, assembly).clone(),
                    config_snapshot: config_ref(refs, config).clone(),
                    participant_replica_ids: participants.clone(),
                },
            ),
            Step::Abort {
                activation_id,
                expected_generation,
                ..
            } => reduce_abort(
                state,
                &AbortInput {
                    environment: "test".to_string(),
                    activation_id: activation_id.clone(),
                    expected_generation: *expected_generation,
                },
            ),
            Step::Commit {
                activation_id,
                expected_generation,
                candidate_generation,
                connected,
                prepared,
                ..
            } => reduce_commit(
                state,
                &CommitInput {
                    environment: "test".to_string(),
                    activation_id: activation_id.clone(),
                    expected_generation: *expected_generation,
                    candidate_generation: *candidate_generation,
                    assembly: refs.candidate.clone(),
                    config_snapshot: refs.candidate_config.clone(),
                    connected_replica_ids: connected.clone(),
                    prepared_replica_ids: prepared.clone(),
                },
            ),
            Step::Initialize { .. } | Step::Read { .. } | Step::Recover { .. } => {
                unreachable!("state-only steps handled by the corpus runner")
            }
        }
    }

    fn corpus_expected_is_missing_assembly(expected: &str, assembly: &str) -> bool {
        expected == "invalid" && assembly == "missing"
    }

    #[test]
    fn shared_corpus_drives_pure_reducer_to_frozen_terminal() {
        let corpus: Corpus = serde_json::from_str(include_str!(
        "../../cross-system-fixtures/package-service-ecosystem/activation-state-contract-cases.json"
    ))
        .expect("corpus JSON");
        assert_eq!(
            corpus.schema_version, "skiff-activation-state-contract-corpus-v1",
            "corpus schema"
        );
        let refs = refs();
        for case in &corpus.cases {
            let mut state = initial_state(&refs, 0);
            for step in &case.steps {
                match step {
                    Step::Initialize {
                        committed_generation,
                        assembly,
                        config,
                    } => {
                        let _ = (assembly, config);
                        state = initial_state(&refs, *committed_generation);
                    }
                    Step::Prepare {
                        assembly, expected, ..
                    } => {
                        if corpus_expected_is_missing_assembly(expected, assembly) {
                            // Reference existence is adapter-side: the file adapter
                            // rejects the missing assembly before mutation, the pure
                            // reducer accepts it lexically (asserted separately in
                            // reducer_lexically_accepts_missing_assembly_boundary).
                            // Keep the corpus state progression identical to the
                            // file adapter by not applying the step.
                            continue;
                        }
                        let result = reducer_result(&state, &refs, step);
                        assert_expected(&result, expected);
                        if result.is_ok() {
                            state = result.expect("prepare ok");
                        }
                    }
                    Step::Abort { expected, .. } => {
                        let result = reducer_result(&state, &refs, step);
                        assert_expected(&result, expected);
                        if result.is_ok() {
                            state = result.expect("abort ok");
                        }
                    }
                    Step::Commit { expected, .. } => {
                        let result = reducer_result(&state, &refs, step);
                        assert_expected(&result, expected);
                        if result.is_ok() {
                            state = result.expect("commit ok");
                        }
                    }
                    Step::Read {
                        expected_generation,
                        expected_pending_activation_id,
                        expected_participants,
                    } => {
                        assert_eq!(state.committed.generation, *expected_generation);
                        match (state.pending.as_ref(), expected_pending_activation_id) {
                            (None, None) => {}
                            (Some(pending), Some(activation_id)) => {
                                assert_eq!(&pending.activation_id, activation_id);
                                if let Some(participants) = expected_participants {
                                    assert_eq!(&pending.participant_replica_ids, participants);
                                }
                            }
                            (actual, expected) => {
                                panic!("pending mismatch: {actual:?} vs {expected:?}")
                            }
                        }
                    }
                    Step::Recover {
                        connected,
                        prepared,
                        expected_action,
                        expected_replica_ids,
                    } => {
                        let action = state
                            .recovery_action(connected, prepared)
                            .expect("recovery action");
                        match (expected_action.as_str(), action) {
                            ("stableCommitted", ActivationRecoveryAction::StableCommitted) => {}
                            (
                                "replayPrepare",
                                ActivationRecoveryAction::ReplayPrepare { replica_ids },
                            ) => {
                                assert_eq!(
                                    expected_replica_ids.as_ref().expect("replica ids"),
                                    &replica_ids
                                );
                            }
                            ("commitPending", ActivationRecoveryAction::CommitPending) => {}
                            (
                                "abortPending",
                                ActivationRecoveryAction::AbortPending { activation_id },
                            ) => {
                                assert_eq!(activation_id, "activation-8");
                            }
                            (expected, actual) => {
                                panic!(
                                    "recovery action mismatch: expected {expected}, got {actual:?}"
                                )
                            }
                        }
                    }
                }
            }
            assert_eq!(case.expected.terminal, "ok", "{}", case.name);
        }
    }

    fn assert_expected(
        result: &Result<EnvironmentActivationState, ActivationStateError>,
        expected: &str,
    ) {
        match (expected, result) {
            ("ok", Ok(_)) => {}
            ("casMismatch", Err(ActivationStateError::CasMismatch { .. })) => {}
            ("invalid", Err(ActivationStateError::InvalidRecord { .. })) => {}
            (other, actual) => panic!("expected {other}, got {actual:?}"),
        }
    }

    #[test]
    fn reducer_lexically_accepts_missing_assembly_boundary() {
        // The pure reducer validates lexical reference shape only; artifact
        // existence is enforced by the file adapter (and by the coordinator's
        // blocking loader before it writes durable state).
        let refs = refs();
        let state = initial_state(&refs, 7);
        let accepted = reduce_prepare(
            &state,
            &PrepareInput {
                environment: "test".to_string(),
                activation_id: "activation-8".to_string(),
                expected_generation: 7,
                candidate_generation: 8,
                assembly: refs.missing.clone(),
                config_snapshot: refs.candidate_config.clone(),
                participant_replica_ids: vec!["runtime-a".to_string()],
            },
        )
        .expect("lexically valid missing ref accepted by reducer");
        assert_eq!(
            accepted.pending.unwrap().assembly.assembly_identity,
            refs.missing.assembly_identity
        );

        let (_temp, store) = file_store_with_assemblies(&refs);
        store
            .initialize_environment_activation(&state)
            .expect("initialize file state");
        assert!(
            store
                .prepare_environment_activation(
                    "test",
                    "activation-8",
                    7,
                    8,
                    refs.missing.clone(),
                    refs.candidate_config.clone(),
                    vec!["runtime-a".to_string()],
                )
                .is_err(),
            "file adapter must reject missing assembly ref"
        );
    }

    #[test]
    fn reducer_and_file_adapter_agree_on_operation_sequences() {
        let refs = refs();
        let (_temp, store) = file_store_with_assemblies(&refs);
        let mut file_state = initial_state(&refs, 7);
        store
            .initialize_environment_activation(&file_state)
            .expect("initialize file state");
        let mut reducer_state = initial_state(&refs, 7);

        let sequence: Vec<Operation> = vec![
            op_prepare("activation-8", 7, 8, vec!["runtime-b", "runtime-a"], "ok"),
            op_prepare("activation-8", 7, 8, vec!["runtime-a", "runtime-b"], "ok"),
            op_prepare("activation-8x", 7, 8, vec!["runtime-a"], "casMismatch"),
            op_abort("activation-8", 7, "ok"),
            op_prepare("activation-9", 7, 8, vec!["runtime-a", "runtime-b"], "ok"),
            op_commit(
                "activation-9",
                7,
                8,
                vec!["runtime-a", "runtime-b"],
                vec!["runtime-a"],
                "casMismatch",
            ),
            op_commit(
                "activation-9",
                7,
                8,
                vec!["runtime-a", "runtime-b"],
                vec!["runtime-a", "runtime-b"],
                "ok",
            ),
            op_commit(
                "activation-9",
                7,
                8,
                vec!["runtime-a", "runtime-b"],
                vec!["runtime-a", "runtime-b"],
                "ok",
            ),
            op_prepare("activation-10", 7, 8, vec!["runtime-a"], "casMismatch"),
            op_abort("activation-11", 8, "ok"),
            op_abort("activation-9", 7, "casMismatch"),
        ];

        for operation in sequence {
            let file_result = operation.run_file(&store, &file_state);
            let reducer_result = operation.run_reducer(&reducer_state);
            match (&file_result, &reducer_result) {
                (Ok(file_next), Ok(reducer_next)) => {
                    assert_eq!(file_next, reducer_next, "{}", operation.label());
                    file_state = file_next.clone();
                    reducer_state = reducer_next.clone();
                }
                (
                    Err(EcosystemStorageError::CasMismatch { .. }),
                    Err(ActivationStateError::CasMismatch { .. }),
                )
                | (
                    Err(EcosystemStorageError::InvalidRecord { .. }),
                    Err(ActivationStateError::InvalidRecord { .. }),
                ) => {}
                (file, reducer) => {
                    panic!(
                        "{} divergence: file {file:?} vs reducer {reducer:?}",
                        operation.label()
                    )
                }
            }
        }
        let file_final = store
            .read_environment_activation("test")
            .expect("read final file state");
        assert_eq!(file_final, reducer_state);
        assert_eq!(file_final.committed.generation, 8);
        assert!(file_final.pending.is_none());
    }

    fn file_store_with_assemblies(refs: &Refs) -> (TestRoot, CanonicalArtifactStore) {
        let temp = TestRoot::new();
        let store = CanonicalArtifactStore::create(temp.path()).expect("artifact store");
        let committed = empty_runtime_assembly_fixture().expect("committed fixture");
        let candidate = runtime_assembly_fixture().expect("candidate fixture");
        store
            .write_runtime_assembly(&committed)
            .expect("write committed assembly");
        store
            .write_runtime_assembly(&candidate)
            .expect("write candidate assembly");
        assert_eq!(
            runtime_assembly_ref(&committed)
                .expect("committed ref")
                .assembly_identity,
            refs.committed.assembly_identity
        );
        assert_eq!(
            runtime_assembly_ref(&candidate)
                .expect("candidate ref")
                .assembly_identity,
            refs.candidate.assembly_identity
        );
        (temp, store)
    }

    enum Operation {
        Prepare {
            activation_id: String,
            expected: u64,
            candidate: u64,
            participants: Vec<String>,
            expected_outcome: &'static str,
        },
        Abort {
            activation_id: String,
            expected: u64,
            expected_outcome: &'static str,
        },
        Commit {
            activation_id: String,
            expected: u64,
            candidate: u64,
            connected: Vec<String>,
            prepared: Vec<String>,
            expected_outcome: &'static str,
        },
    }

    fn op_prepare(
        activation_id: &str,
        expected: u64,
        candidate: u64,
        participants: Vec<&str>,
        expected_outcome: &'static str,
    ) -> Operation {
        Operation::Prepare {
            activation_id: activation_id.to_string(),
            expected,
            candidate,
            participants: participants.into_iter().map(str::to_string).collect(),
            expected_outcome,
        }
    }

    fn op_abort(activation_id: &str, expected: u64, expected_outcome: &'static str) -> Operation {
        Operation::Abort {
            activation_id: activation_id.to_string(),
            expected,
            expected_outcome,
        }
    }

    fn op_commit(
        activation_id: &str,
        expected: u64,
        candidate: u64,
        connected: Vec<&str>,
        prepared: Vec<&str>,
        expected_outcome: &'static str,
    ) -> Operation {
        Operation::Commit {
            activation_id: activation_id.to_string(),
            expected,
            candidate,
            connected: connected.into_iter().map(str::to_string).collect(),
            prepared: prepared.into_iter().map(str::to_string).collect(),
            expected_outcome,
        }
    }

    impl Operation {
        fn label(&self) -> &'static str {
            match self {
                Self::Prepare { .. } => "prepare",
                Self::Abort { .. } => "abort",
                Self::Commit { .. } => "commit",
            }
        }

        fn run_file(
            &self,
            store: &CanonicalArtifactStore,
            _state: &EnvironmentActivationState,
        ) -> Result<EnvironmentActivationState, EcosystemStorageError> {
            let refs = refs();
            match self {
                Self::Prepare {
                    activation_id,
                    expected,
                    candidate,
                    participants,
                    expected_outcome,
                } => {
                    let result = store.prepare_environment_activation(
                        "test",
                        activation_id,
                        *expected,
                        *candidate,
                        refs.candidate.clone(),
                        refs.candidate_config.clone(),
                        participants.clone(),
                    );
                    assert_file_outcome(&result, expected_outcome);
                    result
                }
                Self::Abort {
                    activation_id,
                    expected,
                    expected_outcome,
                } => {
                    let result =
                        store.abort_environment_activation("test", activation_id, *expected);
                    assert_file_outcome(&result, expected_outcome);
                    result
                }
                Self::Commit {
                    activation_id,
                    expected,
                    candidate,
                    connected,
                    prepared,
                    expected_outcome,
                } => {
                    let result = store.commit_environment_activation(
                        "test",
                        activation_id,
                        *expected,
                        *candidate,
                        &refs.candidate,
                        &refs.candidate_config,
                        connected,
                        prepared,
                    );
                    assert_file_outcome(&result, expected_outcome);
                    result
                }
            }
        }

        fn run_reducer(
            &self,
            state: &EnvironmentActivationState,
        ) -> Result<EnvironmentActivationState, ActivationStateError> {
            let refs = refs();
            match self {
                Self::Prepare {
                    activation_id,
                    expected,
                    candidate,
                    participants,
                    expected_outcome,
                } => {
                    let result = reduce_prepare(
                        state,
                        &PrepareInput {
                            environment: "test".to_string(),
                            activation_id: activation_id.clone(),
                            expected_generation: *expected,
                            candidate_generation: *candidate,
                            assembly: refs.candidate.clone(),
                            config_snapshot: refs.candidate_config.clone(),
                            participant_replica_ids: participants.clone(),
                        },
                    );
                    assert_reducer_outcome(&result, expected_outcome);
                    result
                }
                Self::Abort {
                    activation_id,
                    expected,
                    expected_outcome,
                } => {
                    let result = reduce_abort(
                        state,
                        &AbortInput {
                            environment: "test".to_string(),
                            activation_id: activation_id.clone(),
                            expected_generation: *expected,
                        },
                    );
                    assert_reducer_outcome(&result, expected_outcome);
                    result
                }
                Self::Commit {
                    activation_id,
                    expected,
                    candidate,
                    connected,
                    prepared,
                    expected_outcome,
                } => {
                    let result = reduce_commit(
                        state,
                        &CommitInput {
                            environment: "test".to_string(),
                            activation_id: activation_id.clone(),
                            expected_generation: *expected,
                            candidate_generation: *candidate,
                            assembly: refs.candidate.clone(),
                            config_snapshot: refs.candidate_config.clone(),
                            connected_replica_ids: connected.clone(),
                            prepared_replica_ids: prepared.clone(),
                        },
                    );
                    assert_reducer_outcome(&result, expected_outcome);
                    result
                }
            }
        }
    }

    fn assert_file_outcome(
        result: &Result<EnvironmentActivationState, EcosystemStorageError>,
        expected: &str,
    ) {
        match (expected, result) {
            ("ok", Ok(_)) => {}
            ("casMismatch", Err(EcosystemStorageError::CasMismatch { .. })) => {}
            ("invalid", Err(EcosystemStorageError::InvalidRecord { .. })) => {}
            (other, actual) => panic!("file expected {other}, got {actual:?}"),
        }
    }

    fn assert_reducer_outcome(
        result: &Result<EnvironmentActivationState, ActivationStateError>,
        expected: &str,
    ) {
        match (expected, result) {
            ("ok", Ok(_)) => {}
            ("casMismatch", Err(ActivationStateError::CasMismatch { .. })) => {}
            ("invalid", Err(ActivationStateError::InvalidRecord { .. })) => {}
            (other, actual) => panic!("reducer expected {other}, got {actual:?}"),
        }
    }

    #[test]
    fn schema_version_constant_is_shared_with_dto() {
        assert_eq!(
            skiff_deployment::activation_state::ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION,
            "skiff-environment-activation-state-v2"
        );
    }
}
