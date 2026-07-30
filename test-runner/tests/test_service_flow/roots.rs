use skiff_artifact_model::PackageLocalAbiSymbol;
use skiff_test_runner::{
    canonical_fixture::discover_test_service_cases,
    canonical_package::compile_package_project_for_test, canonical_std_seed::seed_canonical_std,
    test_service_fixture::assemble_test_service_fixture,
};

use super::*;

#[test]
fn ordinary_test_service_resolves_public_private_and_test_local_roots_together() {
    let root = TestRoot::new("ordinary-root-paths");
    let artifacts = root.path().join("artifacts");
    seed_canonical_std(&platform_sources(), &artifacts).expect("seed std");
    let service = root.path().join("service");
    write_test_service(
        &service,
        "example.com/ordinary-test-roots",
        "public: main.publicHelper\n",
        "function publicHelper() -> bool { return true }\n\
         function privateHelper() -> bool { return true }\n",
        "function testLocalHelper() -> bool { return true }\n\
         test \"root visibility\" {\n\
           assert root.main.publicHelper()\n\
           assert root.main.privateHelper()\n\
           assert root.main.__test.testLocalHelper()\n\
         }\n",
    );

    let project = compile_package_project_for_test(&platform_sources(), &service, &artifacts)
        .expect("one ordinary test-service compilation must resolve all three root surfaces");
    let artifact = &project.package.artifact;

    assert!(
        artifact.package_requirements.iter().all(|requirement| {
            requirement.package_id != artifact.package_id
                || requirement.exact_version != artifact.package_version
        }),
        "root.* must not emit a self package requirement"
    );
    assert!(artifact
        .package_local_abi
        .public_symbols
        .contains_key("public"));
    for selector in [
        "main.publicHelper",
        "main.privateHelper",
        "main.__test.testLocalHelper",
    ] {
        assert!(
            artifact
                .package_local_abi
                .implementation_symbols
                .contains_key(selector),
            "ordinary test service omitted exact implementation symbol {selector}"
        );
    }

    let cases = discover_test_service_cases(&service, &service, false).expect("discover test case");
    let fixture = assemble_test_service_fixture(&project, &cases, Default::default())
        .expect("root-resolved test service must assemble through the ordinary service path");
    assert_eq!(fixture.cases.len(), 1);
}

#[test]
fn ordinary_test_service_unknown_root_target_fails_closed_without_self_dependency_fallback() {
    let root = TestRoot::new("ordinary-missing-root");
    let artifacts = root.path().join("artifacts");
    seed_canonical_std(&platform_sources(), &artifacts).expect("seed std");
    let service = root.path().join("service");
    write_test_service(
        &service,
        "example.com/ordinary-missing-root",
        "{}\n",
        "function privateHelper() -> bool { return true }\n",
        "test \"missing root\" { assert root.main.missingHelper() }\n",
    );

    let error = compile_package_project_for_test(&platform_sources(), &service, &artifacts)
        .expect_err("an unknown root target must not become a self dependency")
        .to_string();
    assert!(error.contains("invalid root reference"), "{error}");
    assert!(error.contains("root.main.missingHelper"), "{error}");
    assert!(
        !error.contains("dependency pointer") && !error.contains("package requirement"),
        "unknown root resolution escaped into dependency fallback: {error}"
    );
}

#[test]
fn runner_owned_private_gateway_rejects_missing_wrong_signature_and_public_leak() {
    let root = TestRoot::new("runner-private-gateway");
    let artifacts = root.path().join("artifacts");
    seed_canonical_std(&platform_sources(), &artifacts).expect("seed std");
    let service = root.path().join("service");
    write_test_service(
        &service,
        "example.com/runner-private-gateway",
        "{}\n",
        "function privateHelper() -> bool { return true }\n",
        "test \"gateway owner\" { assert root.main.privateHelper() }\n",
    );

    let project = compile_package_project_for_test(&platform_sources(), &service, &artifacts)
        .expect("compile ordinary test service");
    let cases = discover_test_service_cases(&service, &service, false).expect("discover test case");
    let gateway_selector = format!("{}.{}Gateway", cases[0].module_path, cases[0].function_name);

    let mut missing = project.clone();
    missing
        .package
        .artifact
        .package_local_abi
        .implementation_symbols
        .remove(&gateway_selector);
    let error = assemble_test_service_fixture(&missing, &cases, Default::default())
        .expect_err("missing runner gateway must fail closed")
        .to_string();
    assert!(
        error.contains("has no exact private gateway handler"),
        "{error}"
    );

    let mut wrong_signature = project.clone();
    let PackageLocalAbiSymbol::Callable { signature, .. } = wrong_signature
        .package
        .artifact
        .package_local_abi
        .implementation_symbols
        .get_mut(&gateway_selector)
        .expect("generated runner gateway")
    else {
        panic!("generated runner gateway must be callable");
    };
    signature.parameters.clear();
    let error = assemble_test_service_fixture(&wrong_signature, &cases, Default::default())
        .expect_err("wrong runner gateway signature must fail closed")
        .to_string();
    assert!(error.contains("must have exact signature"), "{error}");

    let mut public_leak = project;
    let leaked = public_leak
        .package
        .artifact
        .package_local_abi
        .implementation_symbols[&gateway_selector]
        .clone();
    public_leak
        .package
        .artifact
        .package_local_abi
        .public_symbols
        .insert("leakedGateway".to_string(), leaked);
    let error = assemble_test_service_fixture(&public_leak, &cases, Default::default())
        .expect_err("a public runner gateway must fail closed")
        .to_string();
    assert!(
        error.contains("must not enter the package public API"),
        "{error}"
    );
}

fn write_test_service(root: &Path, service_id: &str, api: &str, production: &str, tests: &str) {
    fs::create_dir_all(root).expect("create test service");
    fs::write(
        root.join("package.yml"),
        format!("id: {service_id}\nversion: 1.0.0\n"),
    )
    .expect("write package.yml");
    fs::write(root.join("api.yml"), api).expect("write api.yml");
    fs::write(
        root.join("service.yml"),
        format!("id: {service_id}\nkind: test\n"),
    )
    .expect("write service.yml");
    fs::write(
        root.join("config.skiff-test.yml"),
        format!(
            "timeout: 30000\nquota:\n  cpuMillis: 100\n  memoryBytes: 67108864\nprincipal: service:{service_id}\nlifecycle:\n  maxConcurrency: 1\n"
        ),
    )
    .expect("write config.skiff-test.yml");
    fs::write(root.join("main.skiff"), production).expect("write production source");
    fs::write(root.join("main.test.skiff"), tests).expect("write test source");
}
