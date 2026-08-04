use std::{
    cell::RefCell,
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::*;

#[test]
fn file_first_batches_preserve_discovery_order_without_splitting_small_files() {
    let cases = cases_for_files(&[
        ("a.test.skiff", 10),
        ("b.test.skiff", 8),
        ("c.test.skiff", 4),
    ]);

    let batches = batching::partition_non_live_cases(cases);

    assert_eq!(batch_sizes(&batches), vec![10, 12]);
    assert_eq!(
        batch_paths(&batches),
        vec![vec!["a"; 10], [vec!["b"; 8], vec!["c"; 4]].concat()]
    );
    assert_eq!(
        flatten_names(&batches),
        expected_names(&[("a", 10), ("b", 8), ("c", 4)])
    );
    assert!(batches
        .iter()
        .all(|batch| batch.len() <= batching::MAX_NON_LIVE_CASES_PER_ACTIVATION));
}

#[test]
fn only_an_oversized_file_is_split_at_the_hard_cap() {
    let cases = cases_for_files(&[
        ("large.test.skiff", 20),
        ("next.test.skiff", 10),
        ("last.test.skiff", 7),
    ]);

    let batches = batching::partition_non_live_cases(cases);

    assert_eq!(batch_sizes(&batches), vec![16, 14, 7]);
    assert_eq!(
        batch_paths(&batches),
        vec![
            vec!["large"; 16],
            [vec!["large"; 4], vec!["next"; 10]].concat(),
            vec!["last"; 7],
        ]
    );
    assert_eq!(
        flatten_names(&batches),
        expected_names(&[("large", 20), ("next", 10), ("last", 7)])
    );
}

#[test]
fn exact_multiple_of_the_cap_does_not_create_an_empty_batch() {
    let batches = batching::partition_non_live_cases(cases_for_files(&[("large.test.skiff", 32)]));

    assert_eq!(batch_sizes(&batches), vec![16, 16]);
    assert!(batches.iter().all(|batch| !batch.is_empty()));
}

#[test]
fn live_explicit_file_cases_remain_one_activation_even_above_the_non_live_cap() {
    let batches = batching::partition_cases(cases_for_files(&[("live.test.skiff", 20)]), true);

    assert_eq!(batch_sizes(&batches), vec![20]);
}

#[test]
fn every_batch_scope_is_unique_but_keeps_the_same_run_owner() {
    assert_eq!(
        (0..3)
            .map(|index| batching::batch_execution_scope("run-7", index))
            .collect::<Vec<_>>(),
        ["run-7-batch-0", "run-7-batch-1", "run-7-batch-2"]
    );
}

#[test]
fn every_batch_is_assembled_and_published_before_execution_can_start() {
    let timeline = RefCell::new(Vec::new());
    let batches = prepare_execution_batches_with(
        vec!["first", "second", "third"],
        |index, context| {
            timeline.borrow_mut().push(format!("assemble:{context}"));
            Ok(ExecutionBatch {
                context: (index, context),
                entrypoints: vec![test_entrypoint(context, 0)],
            })
        },
        |batch| {
            timeline
                .borrow_mut()
                .push(format!("publish:{}", batch.context.1));
            Ok(())
        },
    )
    .unwrap();
    for batch in batches {
        timeline
            .borrow_mut()
            .push(format!("dispatch:{}", batch.context.1));
    }

    assert_eq!(
        &*timeline.borrow(),
        &[
            "assemble:first",
            "assemble:second",
            "assemble:third",
            "publish:first",
            "publish:second",
            "publish:third",
            "dispatch:first",
            "dispatch:second",
            "dispatch:third",
        ]
    );
}

#[test]
fn real_batch_fixtures_keep_base_partitions_and_storage_identities_disjoint() {
    let root = BatchTestRoot::new();
    let source_artifacts = root.path().join("source-artifacts");
    let runtime_artifacts = root.path().join("runtime-artifacts");
    let service = root.path().join("service");
    fs::create_dir_all(&service).unwrap();
    fs::write(
        service.join("package.yml"),
        "id: test.skiff/batched-fixture\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(service.join("api.yml"), "{}\n").unwrap();
    fs::write(
        service.join("service.yml"),
        "id: test.skiff/batched-fixture\nkind: test\n",
    )
    .unwrap();
    fs::write(
        service.join("large.test.skiff"),
        (0..17)
            .map(|index| format!("test \"case {index}\" {{ assert true }}\n"))
            .collect::<String>(),
    )
    .unwrap();

    let platform_sources = skiff_compiler::CompilerPlatformSources::new(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repository root"),
    )
    .unwrap();
    crate::canonical_std_seed::seed_canonical_std(&platform_sources, &source_artifacts).unwrap();
    let project = crate::canonical_package::compile_package_project_for_test(
        &platform_sources,
        &service,
        &source_artifacts,
    )
    .unwrap();
    let cases =
        crate::test_discovery::discover_test_service_cases(&service, &service, false).unwrap();
    let case_batches = batching::partition_cases(cases, false);
    assert_eq!(batch_sizes(&case_batches), [16, 1]);
    let run_config =
        load_test_service_run_config(&project, Some("http://127.0.0.1:46100")).unwrap();
    fs::write(
        service.join("config.skiff-test.yml"),
        "this would fail if a batch re-read config after run planning: [\n",
    )
    .unwrap();

    let original_base = empty_base("skiff-test");
    let original_base_identity = original_base
        .assembly
        .as_ref()
        .unwrap()
        .assembly_identity
        .clone();
    let mut publication = CanonicalPublishSession::default();
    let batches = prepare_execution_batches_with(
        case_batches,
        |batch_index, cases| {
            let fixture = assemble_test_service_fixture_for_run_with_config(
                &project,
                &cases,
                original_base.clone(),
                &batching::batch_execution_scope("fixture-run", batch_index),
                &run_config,
                "skiff-test",
            )?;
            Ok(ExecutionBatch {
                context: fixture.records,
                entrypoints: fixture
                    .cases
                    .into_iter()
                    .map(|case| case.entrypoint)
                    .collect(),
            })
        },
        |batch| {
            batch.context.publish_with_session(
                &source_artifacts,
                &runtime_artifacts,
                &mut publication,
            )?;
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(publication.owned_package_publication_count(), 1);
    assert_eq!(batches.len(), 2);
    let deployment_sets = batches
        .iter()
        .map(|batch| {
            let deployments = batch
                .entrypoints
                .iter()
                .map(|entrypoint| entrypoint.deployment.clone())
                .collect::<BTreeSet<_>>();
            assert_eq!(
                batch
                    .context
                    .assembly
                    .resolved_deployments
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>(),
                deployments
            );
            assert_eq!(
                batch
                    .context
                    .config_snapshot
                    .deployments()
                    .iter()
                    .map(|partition| partition.deployment().clone())
                    .collect::<BTreeSet<_>>(),
                deployments
            );
            assert_eq!(
                batch
                    .context
                    .base_assembly
                    .as_ref()
                    .map(|assembly| &assembly.assembly_identity),
                Some(&original_base_identity)
            );
            deployments
        })
        .collect::<Vec<_>>();
    assert!(deployment_sets[0].is_disjoint(&deployment_sets[1]));
    assert_eq!(deployment_sets[0].len(), 16);
    assert_eq!(deployment_sets[1].len(), 1);

    let service_ids = batches
        .iter()
        .flat_map(|batch| &batch.entrypoints)
        .map(|entrypoint| entrypoint.deployment.service_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(service_ids.len(), 17);
    assert_eq!(
        service_ids.iter().copied().collect::<BTreeSet<_>>().len(),
        17
    );
    let database_names = service_ids
        .iter()
        .map(|service_id| service_id.replace('.', "~").replace('/', "~~"))
        .collect::<BTreeSet<_>>();
    assert_eq!(database_names.len(), 17);
    assert_ne!(
        batches[0].context.assembly.assembly_identity,
        batches[1].context.assembly.assembly_identity
    );
    assert_ne!(
        batches[0].context.config_snapshot.snapshot_ref(),
        batches[1].context.config_snapshot.snapshot_ref()
    );
}

#[test]
fn batches_advance_generation_safely_and_dispatch_in_discovery_order() {
    let timeline = RefCell::new(Vec::new());
    let batches = execution_batches(&[("first", 2), ("second", 1)]);

    let summary = execute_batches_with(
        batches,
        40,
        |context, expected, candidate| {
            timeline
                .borrow_mut()
                .push(format!("activate:{context}:{expected}->{candidate}"));
            Ok(active(candidate))
        },
        |active| {
            timeline
                .borrow_mut()
                .push(format!("ready:{}", active.generation));
            Ok(())
        },
        |active, entrypoint| {
            timeline.borrow_mut().push(format!(
                "dispatch:{}:{}",
                active.generation, entrypoint.case.name
            ));
            Ok(DispatchOutcome::Passed)
        },
    )
    .unwrap();

    assert_eq!((summary.passed, summary.failed), (3, 0));
    assert_eq!(
        &*timeline.borrow(),
        &[
            "activate:first:40->41",
            "ready:41",
            "dispatch:41:first-0",
            "dispatch:41:first-1",
            "activate:second:41->42",
            "ready:42",
            "dispatch:42:second-0",
        ]
    );
}

#[test]
fn later_activation_failure_preserves_all_prior_pass_and_fail_results() {
    let batches = execution_batches(&[("first", 2), ("second", 2)]);
    let error = execute_batches_with(
        batches,
        7,
        |context, _, candidate| {
            if *context == "second" {
                return Err(CanonicalFixtureError::RemoteControl {
                    status: 409,
                    code: "ActivationGenerationMismatch".to_string(),
                    message: "stale expected generation".to_string(),
                });
            }
            Ok(active(candidate))
        },
        |_| Ok(()),
        |_, entrypoint| {
            if entrypoint.case.name == "first-1" {
                Ok(DispatchOutcome::Failed("assertion failed".to_string()))
            } else {
                Ok(DispatchOutcome::Passed)
            }
        },
    )
    .unwrap_err();

    let CanonicalFixtureError::SuiteExecution {
        completed,
        module_path,
        name,
        source,
    } = error
    else {
        panic!("activation failure did not preserve the suite ledger");
    };
    assert_eq!(completed.len(), 2);
    assert!(completed[0].passed);
    assert!(!completed[1].passed);
    assert_eq!(completed[1].message.as_deref(), Some("assertion failed"));
    assert_eq!(
        (module_path.as_str(), name.as_str()),
        ("second", "second-0")
    );
    assert!(matches!(
        *source,
        CanonicalFixtureError::RemoteControl { ref code, .. }
            if code == "ActivationGenerationMismatch"
    ));
}

#[test]
fn later_readiness_failure_preserves_ledger_and_stops_current_and_future_dispatch() {
    let activations = RefCell::new(Vec::new());
    let dispatches = RefCell::new(Vec::new());
    let error = execute_batches_with(
        execution_batches(&[("first", 2), ("second", 1), ("never", 1)]),
        30,
        |context, _, candidate| {
            activations.borrow_mut().push(*context);
            Ok(active(candidate))
        },
        |active| {
            if active.generation == 32 {
                Err(CanonicalFixtureError::InvalidInput(
                    "second batch was not ready".to_string(),
                ))
            } else {
                Ok(())
            }
        },
        |_, entrypoint| {
            dispatches.borrow_mut().push(entrypoint.case.name.clone());
            if entrypoint.case.name == "first-1" {
                Ok(DispatchOutcome::Failed("assertion failed".to_string()))
            } else {
                Ok(DispatchOutcome::Passed)
            }
        },
    )
    .unwrap_err();

    assert_eq!(&*activations.borrow(), &["first", "second"]);
    assert_eq!(&*dispatches.borrow(), &["first-0", "first-1"]);
    let CanonicalFixtureError::SuiteExecution {
        completed,
        module_path,
        name,
        source,
    } = error
    else {
        panic!("readiness failure did not preserve the suite ledger");
    };
    assert_eq!(
        completed
            .iter()
            .map(|result| (result.name.as_str(), result.passed))
            .collect::<Vec<_>>(),
        [("first-0", true), ("first-1", false)]
    );
    assert_eq!(
        (module_path.as_str(), name.as_str()),
        ("second", "second-0")
    );
    assert!(matches!(
        *source,
        CanonicalFixtureError::InvalidInput(ref message)
            if message == "second batch was not ready"
    ));
}

#[test]
fn assertion_failure_continues_through_later_batches_once_and_in_order() {
    let dispatches = RefCell::new(Vec::new());
    let summary = execute_batches_with(
        execution_batches(&[("first", 2), ("second", 2)]),
        15,
        |_, _, candidate| Ok(active(candidate)),
        |_| Ok(()),
        |_, entrypoint| {
            dispatches.borrow_mut().push(entrypoint.case.name.clone());
            if entrypoint.case.name == "first-1" {
                Ok(DispatchOutcome::Failed("assertion failed".to_string()))
            } else {
                Ok(DispatchOutcome::Passed)
            }
        },
    )
    .unwrap();

    assert_eq!(
        &*dispatches.borrow(),
        &["first-0", "first-1", "second-0", "second-1"]
    );
    assert_eq!((summary.passed, summary.failed, summary.skipped), (3, 1, 0));
    assert_eq!(
        summary
            .results
            .iter()
            .map(|result| (result.name.as_str(), result.passed))
            .collect::<Vec<_>>(),
        [
            ("first-0", true),
            ("first-1", false),
            ("second-0", true),
            ("second-1", true),
        ]
    );
}

#[test]
fn later_dispatch_failure_preserves_prior_batches_and_current_batch_prefix() {
    let batches = execution_batches(&[("first", 1), ("second", 3)]);
    let error = execute_batches_with(
        batches,
        3,
        |_, _, candidate| Ok(active(candidate)),
        |_| Ok(()),
        |_, entrypoint| {
            if entrypoint.case.name == "second-1" {
                Err(CanonicalFixtureError::InvalidInput(
                    "runtime transport disappeared".to_string(),
                ))
            } else {
                Ok(DispatchOutcome::Passed)
            }
        },
    )
    .unwrap_err();

    let CanonicalFixtureError::SuiteExecution {
        completed,
        module_path,
        name,
        source,
    } = error
    else {
        panic!("dispatch failure did not preserve the suite ledger");
    };
    assert_eq!(
        completed
            .iter()
            .map(|result| result.name.as_str())
            .collect::<Vec<_>>(),
        ["first-0", "second-0"]
    );
    assert_eq!(
        (module_path.as_str(), name.as_str()),
        ("second", "second-1")
    );
    assert!(matches!(
        *source,
        CanonicalFixtureError::InvalidInput(ref message)
            if message == "runtime transport disappeared"
    ));
}

#[test]
fn generation_limit_before_a_later_batch_keeps_the_completed_ledger() {
    let error = execute_batches_with(
        execution_batches(&[("first", 1), ("second", 1)]),
        skiff_artifact_model::MAX_SAFE_ACTIVATION_GENERATION - 1,
        |_, _, candidate| Ok(active(candidate)),
        |_| Ok(()),
        |_, _| Ok(DispatchOutcome::Passed),
    )
    .unwrap_err();

    let CanonicalFixtureError::SuiteExecution {
        completed,
        module_path,
        name,
        source,
    } = error
    else {
        panic!("generation overflow did not preserve the suite ledger");
    };
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].name, "first-0");
    assert_eq!(
        (module_path.as_str(), name.as_str()),
        ("second", "second-0")
    );
    assert!(matches!(
        *source,
        CanonicalFixtureError::InvalidInput(ref message)
            if message.contains("expectedGeneration must be between")
    ));
}

#[test]
fn semantic_generation_limit_fails_locally_before_activation() {
    let activation_calls = std::cell::Cell::new(0);
    let error = execute_batches_with(
        execution_batches(&[("first", 1)]),
        skiff_artifact_model::MAX_SAFE_ACTIVATION_GENERATION,
        |_, _, candidate| {
            activation_calls.set(activation_calls.get() + 1);
            Ok(active(candidate))
        },
        |_| Ok(()),
        |_, _| Ok(DispatchOutcome::Passed),
    )
    .unwrap_err();

    assert_eq!(activation_calls.get(), 0);
    assert!(matches!(
        error,
        CanonicalFixtureError::SuiteExecution { source, .. }
            if matches!(*source, CanonicalFixtureError::InvalidInput(ref message)
                if message.contains("expectedGeneration must be between"))
    ));
}

#[test]
fn non_candidate_generation_is_rejected_before_readiness_or_dispatch() {
    let readiness_calls = std::cell::Cell::new(0);
    let dispatch_calls = std::cell::Cell::new(0);
    let error = execute_batches_with(
        execution_batches(&[("first", 1)]),
        12,
        |_, _, _| Ok(active(99)),
        |_| {
            readiness_calls.set(readiness_calls.get() + 1);
            Ok(())
        },
        |_, _| {
            dispatch_calls.set(dispatch_calls.get() + 1);
            Ok(DispatchOutcome::Passed)
        },
    )
    .unwrap_err();

    assert_eq!(readiness_calls.get(), 0);
    assert_eq!(dispatch_calls.get(), 0);
    assert!(matches!(
        error,
        CanonicalFixtureError::SuiteExecution { source, .. }
            if matches!(*source, CanonicalFixtureError::InvalidInput(ref message)
                if message == "assembly activation returned generation 99, expected 13")
    ));
}

fn cases_for_files(files: &[(&str, usize)]) -> Vec<TestServiceCase> {
    files
        .iter()
        .flat_map(|(path, count)| {
            let stem = path.strip_suffix(".test.skiff").unwrap();
            (0..*count).map(move |index| test_case(stem, path, index))
        })
        .collect()
}

fn test_case(module: &str, relative_path: &str, index: usize) -> TestServiceCase {
    let source = format!("test \"{module}-{index}\" {{ assert true }}\n");
    TestServiceCase {
        case_identity: format!("{module}::test[{index}]"),
        relative_path: relative_path.into(),
        module_path: module.to_string(),
        name: format!("{module}-{index}"),
        function_name: format!("skiffTestCase{index}"),
        test_index: index,
        source_ast: skiff_syntax::parser::parse_source(&source).unwrap(),
        source_text: source,
    }
}

fn batch_sizes(batches: &[Vec<TestServiceCase>]) -> Vec<usize> {
    batches.iter().map(Vec::len).collect()
}

fn batch_paths(batches: &[Vec<TestServiceCase>]) -> Vec<Vec<&str>> {
    batches
        .iter()
        .map(|batch| batch.iter().map(|case| case.module_path.as_str()).collect())
        .collect()
}

fn flatten_names(batches: &[Vec<TestServiceCase>]) -> Vec<String> {
    batches
        .iter()
        .flatten()
        .map(|case| case.name.clone())
        .collect()
}

fn expected_names(files: &[(&str, usize)]) -> Vec<String> {
    files
        .iter()
        .flat_map(|(module, count)| (0..*count).map(move |index| format!("{module}-{index}")))
        .collect()
}

fn execution_batches(spec: &[(&'static str, usize)]) -> Vec<ExecutionBatch<&'static str>> {
    spec.iter()
        .map(|(context, count)| ExecutionBatch {
            context: *context,
            entrypoints: (0..*count)
                .map(|index| test_entrypoint(context, index))
                .collect(),
        })
        .collect()
}

fn test_entrypoint(module: &str, index: usize) -> CanonicalTestServiceEntrypoint {
    let mut entrypoint = super::tests::test_service_entrypoint();
    entrypoint.case = test_case(module, &format!("{module}.test.skiff"), index);
    entrypoint.selector.path = format!("/__skiff/test/{index}");
    entrypoint
}

fn active(generation: u64) -> ActivatedAssembly<()> {
    ActivatedAssembly {
        assembly: RuntimeAssemblyRef {
            assembly_identity: skiff_artifact_model::AssemblyIdentity::new(
                test_support::ASSEMBLY_B,
            ),
        },
        generation,
        readiness: (),
    }
}

static BATCH_TEST_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct BatchTestRoot(PathBuf);

impl BatchTestRoot {
    fn new() -> Self {
        let sequence = BATCH_TEST_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-test-runner-batches-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for BatchTestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn empty_base(profile: &str) -> CanonicalBaseAssembly {
    let mut assembly = skiff_artifact_model::RuntimeAssembly {
        schema_version: skiff_artifact_model::RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: skiff_artifact_model::AssemblyIdentity::new("unassigned"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: Vec::new(),
        package_link_plan: skiff_artifact_model::CanonicalPackageLinkPlan {
            code_slots: Vec::new(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut assembly).unwrap();
    let config_snapshot = skiff_runtime_config_snapshot::RuntimeConfigSnapshot::new(
        profile,
        skiff_runtime_config_snapshot::new_runtime_config_snapshot_ref(),
        Vec::new(),
    )
    .unwrap();
    CanonicalBaseAssembly {
        assembly: Some(assembly),
        config_snapshot: Some(config_snapshot),
        packages: Vec::new(),
        contracts: Vec::new(),
        deployments: Vec::new(),
    }
}
