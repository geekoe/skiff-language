use std::collections::BTreeSet;

use skiff_artifact_identity::package_artifact_ref;
use skiff_compiler::authoring::{build_authoring_object, AuthoringObject};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_config_snapshot::RuntimeConfigSnapshotStore;
use skiff_test_runner::{
    canonical_store::CanonicalBaseAssembly,
    package_service_host_fixture::prepare_package_service_host_fixture,
};

use super::*;

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&source_path, &target_path);
        } else {
            fs::copy(source_path, target_path).unwrap();
        }
    }
}

fn write_package(root: &Path, manifest: &str, source: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("package.yml"), manifest).unwrap();
    fs::write(root.join("api.yml"), "{}\n").unwrap();
    if !source.is_empty() {
        fs::write(root.join("main.skiff"), source).unwrap();
    }
}

fn publish_package(root: &Path, artifacts: &Path) {
    build_authoring_object(
        &platform_sources(),
        AuthoringObject::Package,
        root,
        artifacts,
        "dev",
        true,
    )
    .expect("publish canonical package records and pointer");
}

fn snapshot_tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, output: &mut Vec<(PathBuf, Vec<u8>)>) {
        let mut entries = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for entry in entries {
            if entry.is_dir() {
                visit(root, &entry, output);
            } else {
                output.push((
                    entry.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(entry).unwrap(),
                ));
            }
        }
    }

    let mut snapshot = Vec::new();
    visit(root, root, &mut snapshot);
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_test_service_loads_exact_transitive_store_closure_and_ignores_decoy_source() {
        let root = TestRoot::new("transitive-store");
        let artifacts = root.path().join("artifacts");
        CanonicalArtifactStore::create(&artifacts).expect("create source store");

        let leaf = root.path().join("leaf");
        write_package(
            &leaf,
            "id: example.com/leaf\nversion: 1.0.0\n",
            "type LeafRecord { id: string }\n",
        );
        publish_package(&leaf, &artifacts);

        let helper = root.path().join("helper");
        write_package(
        &helper,
        "id: example.com/helper\nversion: 1.0.0\npackages:\n  - id: example.com/leaf\n    version: 1.0.0\n    alias: leaf\n",
        "function marker() -> string { return \"helper\" }\n",
    );
        publish_package(&helper, &artifacts);

        let service = root.path().join("test-service");
        write_package(
        &service,
        "id: test.skiff/transitive-store\nversion: 1.0.0\npackages:\n  - id: example.com/helper\n    version: 1.0.0\n    alias: helper\n",
        "",
    );
        fs::write(
            service.join("service.yml"),
            "id: test.skiff/transitive-store\nkind: test\n",
        )
        .unwrap();
        fs::write(
            service.join("main.test.skiff"),
            "test \"transitive closure\" { assert true }\n",
        )
        .unwrap();
        let decoy = service.join(".skiff-packages/example.com/helper/1.0.0");
        fs::create_dir_all(&decoy).unwrap();
        fs::write(
            decoy.join("package.yml"),
            "this is deliberately not valid package source",
        )
        .unwrap();

        let project = compile_package_project_for_test(&platform_sources(), &service, &artifacts)
            .expect("dependency records, not nested source, supply the closure");
        assert_eq!(
            project
                .dependency_packages
                .iter()
                .map(|package| package.package_id.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["example.com/helper", "example.com/leaf"])
        );

        let cases =
            discover_test_service_cases(&service, &service, false).expect("discover ordinary case");
        let fixture = assemble_test_service_fixture(
            &project,
            &cases,
            CanonicalBaseAssembly::default(),
            "skiff-test",
        )
        .expect("assemble exact transitive package graph");
        assert_eq!(fixture.cases.len(), 1);
        let leaf = project
            .dependency_packages
            .iter()
            .find(|package| package.package_id == "example.com/leaf")
            .expect("transitive leaf package");
        assert!(fixture
            .records
            .assembly
            .resolved_packages
            .contains(&package_artifact_ref(leaf).expect("leaf package ref")));
        assert!(fixture
            .records
            .assembly
            .package_link_plan
            .package_links
            .iter()
            .any(|binding| {
                binding.key.caller_package_build_id
                    == project
                        .dependency_packages
                        .iter()
                        .find(|package| package.package_id == "example.com/helper")
                        .expect("helper package")
                        .package_build_id
                    && binding.key.package_requirement_alias == "leaf"
                    && binding.package == package_artifact_ref(leaf).expect("leaf package ref")
            }));
    }

    #[test]
    fn ordinary_test_service_uses_exact_base_closure_and_publishes_only_to_runtime_root() {
        let root = TestRoot::new("base-assembly");
        let fixture_root = root.path().join("package-service-host");
        copy_tree(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/package-service-host"),
            &fixture_root,
        );
        let test_service = fixture_root.join("consumer-tests");
        let decoy = test_service.join(".skiff-packages/example.com/consumer/1.0.0");
        fs::create_dir_all(&decoy).unwrap();
        fs::write(
            decoy.join("package.yml"),
            "this is deliberately not valid package source",
        )
        .unwrap();

        let artifacts = root.path().join("artifacts");
        let runtime = root.path().join("runtime-artifacts");
        let receipt = prepare_package_service_host_fixture(
            &platform_sources(),
            &fixture_root,
            &root.path().join("authoring"),
            &artifacts,
            "base-test",
        )
        .expect("publish the production provider closure and base assembly");
        let cross_profile = CanonicalBaseAssembly::load(
            &artifacts,
            Some(receipt.base_assembly.assembly_identity.as_str()),
            Some(receipt.base_config_snapshot.snapshot_id.as_str()),
            "other-test",
        )
        .expect_err("base config snapshot must not cross release profiles")
        .to_string();
        assert!(
            cross_profile.contains("does not match target profile"),
            "{cross_profile}"
        );
        let base = CanonicalBaseAssembly::load(
            &artifacts,
            Some(receipt.base_assembly.assembly_identity.as_str()),
            Some(receipt.base_config_snapshot.snapshot_id.as_str()),
            "base-test",
        )
        .expect("load exact base assembly");
        assert_eq!(
            base.assembly
                .as_ref()
                .expect("base assembly receipt must hydrate an assembly")
                .roots
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                receipt.provider_deployment.clone(),
                receipt.consumer_deployment.clone(),
            ]),
            "fileless base projection must receive both exact provider and consumer roots"
        );

        let project =
            compile_package_project_for_test(&platform_sources(), &test_service, &artifacts)
                .expect("compile only the ordinary test service source");
        assert_eq!(
            project
                .dependency_packages
                .iter()
                .map(|package| package.package_id.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "example.com/consumer",
                "example.com/helper",
                "example.com/payments",
            ]),
            "the store supplies the exact transitive closure; nested dependency source is ignored"
        );

        let subject = project
            .dependency_packages
            .iter()
            .find(|package| package.package_id == "example.com/consumer")
            .expect("exact subject package");
        let helper_requirement = subject
            .package_requirements
            .iter()
            .find(|requirement| requirement.package_id == "example.com/helper")
            .expect("subject helper requirement");
        let payments_requirement = subject
            .service_requirements
            .iter()
            .find(|requirement| {
                requirement.contract_requirement.service_id == "example.com/payments"
            })
            .expect("subject payments requirement");
        let subject_requirements = project
            .package
            .artifact
            .package_requirements
            .iter()
            .filter(|requirement| requirement.package_id == "example.com/consumer")
            .collect::<Vec<_>>();
        let [subject_requirement] = subject_requirements.as_slice() else {
            panic!(
                "alias plus topLevelAlias must compile to one exact subject requirement, found {}",
                subject_requirements.len()
            )
        };
        assert_eq!(subject_requirement.alias, "subject");
        assert_eq!(
            subject_requirement.expected_package_build.as_ref(),
            Some(&subject.package_build_id)
        );

        let cases = discover_test_service_cases(&test_service, &test_service, false)
            .expect("discover ordinary test-service cases");
        let first_case = cases.first().cloned().expect("at least one test case");
        let missing_base = assemble_test_service_fixture(
            &project,
            std::slice::from_ref(&first_case),
            CanonicalBaseAssembly::default(),
            "base-test",
        )
        .expect_err(
            "service requirements must fail before release resolution without a base assembly",
        )
        .to_string();
        assert!(
            missing_base.contains("needs exactly one --base-assembly contract"),
            "{missing_base}"
        );

        let fixture = assemble_test_service_fixture(
            &project,
            std::slice::from_ref(&first_case),
            base,
            "base-test",
        )
        .expect("assemble the ordinary test service with its base closure");
        assert_eq!(fixture.cases.len(), 1);
        let [deployment] = fixture.records.deployments.as_slice() else {
            panic!("one selected case must produce one ordinary deployment")
        };
        let subject_ref = package_artifact_ref(subject).expect("subject package ref");
        let subject_bindings = deployment
            .package_bindings
            .iter()
            .filter(|binding| {
                binding.key.caller_package_build_id == project.package.artifact.package_build_id
                    && binding.key.package_requirement_alias == "subject"
            })
            .collect::<Vec<_>>();
        let [subject_binding] = subject_bindings.as_slice() else {
            panic!(
                "the test service must have one exact subject binding, found {}",
                subject_bindings.len()
            )
        };
        assert_eq!(subject_binding.package, subject_ref);
        assert!(deployment.package_bindings.iter().any(|binding| {
            binding.key.caller_package_build_id == subject.package_build_id
                && binding.key.package_requirement_alias == helper_requirement.alias
                && binding.package == receipt.helper_package
        }));
        assert!(fixture
            .records
            .assembly
            .package_link_plan
            .package_links
            .iter()
            .any(|binding| {
                binding.key.caller_package_build_id == subject.package_build_id
                    && binding.key.package_requirement_alias == helper_requirement.alias
                    && binding.package == receipt.helper_package
            }));
        assert!(deployment.service_selectors.iter().any(|selector| {
            selector.key.caller_package_build_id == subject.package_build_id
                && selector.key.service_requirement_slot
                    == payments_requirement.service_binding_slot
                && selector.contract == receipt.payments_contract
        }));
        assert!(fixture
            .records
            .assembly
            .service_binding_templates
            .iter()
            .flat_map(|template| &template.bindings)
            .any(|binding| {
                binding.key.caller_package_build_id == subject.package_build_id
                    && binding.contract == receipt.payments_contract
                    && binding.provider == receipt.provider_deployment
            }));
        assert_eq!(
            fixture
                .records
                .assembly
                .resolved_deployments
                .iter()
                .filter(|deployment| *deployment == &receipt.provider_deployment)
                .count(),
            1,
            "the shared assembly must resolve the base provider once"
        );

        let source_before_publish = snapshot_tree(&artifacts);
        fixture
            .publish(&artifacts, &runtime)
            .expect("publish only into the isolated runtime root");
        assert_eq!(snapshot_tree(&artifacts), source_before_publish);
        let runtime_store = CanonicalArtifactStore::open(&runtime).expect("runtime store");
        runtime_store
            .read_service_deployment(&receipt.provider_deployment)
            .expect("payment provider deployment copied to runtime root");
        let helper = runtime_store
            .read_package_artifact(&receipt.helper_package)
            .expect("helper package copied to runtime root");
        let helper_schema = runtime_store
            .resolve_package_artifact_schema(&helper)
            .expect("helper schema closure copied to runtime root");
        assert_eq!(
            helper_schema.records.len(),
            helper.package_schema_type_records.len()
        );
        runtime_store
            .read_runtime_assembly(&receipt.base_assembly)
            .expect("base assembly copied to runtime root");
        runtime_store
            .read_runtime_assembly(&skiff_artifact_model::RuntimeAssemblyRef {
                assembly_identity: fixture.records.assembly.assembly_identity.clone(),
            })
            .expect("projected test assembly written to runtime root");
        let combined_snapshot = RuntimeConfigSnapshotStore::open(runtime.join("runtime-config"))
            .unwrap()
            .read(fixture.records.config_snapshot.snapshot_ref())
            .expect("combined config snapshot written to runtime root");
        assert_eq!(
            combined_snapshot.deployments().len(),
            3,
            "one test deployment and two exact base deployments share one snapshot"
        );
        let base_consumer = combined_snapshot
            .deployments()
            .iter()
            .find(|deployment| deployment.deployment() == &receipt.consumer_deployment)
            .expect("base consumer config partition");
        let consumer_config = base_consumer
            .packages()
            .iter()
            .find(|package| {
                package.package_build_id() == &receipt.consumer_package.package_build_id
            })
            .expect("base consumer package config");
        assert_eq!(
            consumer_config.config()["app"]["token"],
            serde_json::json!("owned-by-base")
        );
    }
}
