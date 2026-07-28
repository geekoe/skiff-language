use std::{fs, os::unix::fs::symlink};

use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_compiler_source::prelude_registry::{
    prelude_identity, PreludeRegistryInitializationError,
};
use skiff_deployment::storage::CanonicalArtifactStore;

use super::{
    repository_platform_sources, temporary_path, CanonicalPackageProjectError,
    MinimalPlatformFixture,
};
use crate::canonical_package::{compile_package_project, compile_package_project_for_test};
use crate::{
    canonical_fixture::discover_package_test_cases, canonical_std_seed::seed_canonical_std,
    test_overlay::compile_package_test_overlay,
};

// Explicit F18 identity probe pins refreshed with the current canonical std
// source; the production seed consumes the F27A typed receipt instead.
const EXPECTED_PRELUDE_IDENTITY: &str =
    "skiff-prelude-v1:sha256:2ebbd0569d4baf3d7dccf07c4326ec62deb5707c11a8d0eb0ac0722d1ee9d3bd";
const EXPECTED_STD_PACKAGE_BUILD_ID: &str =
    "skiff-package-build-v4:sha256:18adfaaf021770af47aafddff46e9e9876df0843700f260cea77651eefcb810d";

#[test]
#[ignore = "merge-only F18A/F18B compiler repair probe"]
fn p5_f18_compiler_repair_combined() {
    let escaped = EscapedPlatformFixture::new();
    let unused_package = temporary_path("combined-escaped-package");
    let unused_store = temporary_path("combined-escaped-store");
    let error =
        compile_package_project(&escaped.context(), &unused_package, &unused_store).unwrap_err();
    assert!(error
        .to_string()
        .contains("escapes canonical platform root"));
    assert!(matches!(
        error,
        CanonicalPackageProjectError::PlatformContext(
            PreludeRegistryInitializationError::PlatformSources { .. }
        )
    ));

    let platform_sources = repository_platform_sources();
    let artifacts = temporary_path("combined-golden-store");
    CanonicalArtifactStore::create(&artifacts).unwrap();
    let project =
        compile_package_project(&platform_sources, platform_sources.std_dir(), &artifacts).unwrap();
    assert_eq!(prelude_identity(), EXPECTED_PRELUDE_IDENTITY);
    assert_eq!(
        project.package.artifact.package_build_id.as_str(),
        EXPECTED_STD_PACKAGE_BUILD_ID
    );

    let different_root = MinimalPlatformFixture::new("combined-mismatch");
    let poison_package = temporary_path("combined-poison-package");
    fs::create_dir_all(&poison_package).unwrap();
    fs::write(poison_package.join("package.yml"), "not: a-valid-package\n").unwrap();

    let authoring_store = temporary_path("combined-authoring-store");
    let authoring_error = build_authoring_object(
        &different_root.context(),
        AuthoringObject::Package,
        &poison_package,
        &authoring_store,
        "dev",
        false,
    )
    .unwrap_err();
    assert!(matches!(
        authoring_error.downcast_ref::<PreludeRegistryInitializationError>(),
        Some(PreludeRegistryInitializationError::DifferentPlatformRoot { .. })
    ));
    assert!(!authoring_store.exists());

    let runner_error = compile_package_project(
        &different_root.context(),
        &poison_package,
        different_root.root(),
    )
    .unwrap_err();
    assert!(matches!(
        runner_error,
        CanonicalPackageProjectError::PlatformContext(
            PreludeRegistryInitializationError::DifferentPlatformRoot { .. }
        )
    ));

    for path in [
        unused_package,
        unused_store,
        artifacts,
        poison_package,
        authoring_store,
    ] {
        let _ = fs::remove_dir_all(path);
    }
}

#[test]
#[ignore = "Phase 5 compile-only real-package provenance probe"]
fn p5_f76_contextual_callable_provenance_combined() {
    let packages_root = std::env::var_os("P5_F76_PACKAGES_ROOT")
        .map(std::path::PathBuf::from)
        .expect("P5_F76_PACKAGES_ROOT must name the skiff-packages integration checkout");
    let platform_sources = repository_platform_sources();
    let artifacts = temporary_path("f76-real-packages");
    CanonicalArtifactStore::create(&artifacts).unwrap();
    seed_canonical_std(&platform_sources, &artifacts).unwrap();

    // Compile the shared dependency before its consumers while keeping one
    // canonical store for the complete compile-only graph.
    for package in ["http-session", "aliyunoss", "track", "openai"] {
        let package_root = packages_root.join(package);
        let test_root = packages_root.join("tests").join(package);
        build_authoring_object(
            &platform_sources,
            AuthoringObject::Package,
            &package_root,
            &artifacts,
            "dev",
            true,
        )
        .unwrap();
        let production =
            compile_package_project(&platform_sources, &package_root, &artifacts).unwrap();
        assert_eq!(
            production.package.artifact.package_id,
            format!("skiff.run/{package}")
        );
        let project =
            compile_package_project_for_test(&platform_sources, &test_root, &artifacts).unwrap();
        assert_eq!(
            project
                .test_service_profile
                .as_ref()
                .map(|profile| profile.profile_name.as_str()),
            Some("skiff-test")
        );
        let subject = project
            .package
            .artifact
            .package_requirements
            .iter()
            .find(|requirement| requirement.alias == "subject")
            .unwrap_or_else(|| panic!("{package} test service omitted its subject requirement"));
        assert_eq!(subject.package_id, format!("skiff.run/{package}"));
        assert_eq!(subject.exact_version, "1.0.0");
        assert_eq!(
            subject.expected_package_build.as_ref(),
            Some(&production.package.artifact.package_build_id),
            "{package} test service must bind the exact top-level subject build"
        );
        assert!(project.dependency_packages.iter().any(|dependency| {
            dependency.package_build_id == production.package.artifact.package_build_id
        }));
        let cases = discover_package_test_cases(&test_root, &test_root, false).unwrap();
        assert!(
            !cases.is_empty(),
            "{package} current tests/{package} service root selected zero cases"
        );
        let overlay = compile_package_test_overlay(
            &platform_sources,
            &test_root,
            &artifacts,
            &project,
            &cases,
        )
        .unwrap();
        assert_eq!(
            overlay.bindings.len(),
            cases.len(),
            "{package} current test-service cases must map one-to-one to overlay bindings"
        );
        for (index, binding) in overlay.bindings.iter().enumerate() {
            assert_eq!(binding.public_path, format!("testCases.case{index}"));
            assert!(
                overlay
                    .overlay
                    .artifact
                    .callable_semantic_facts
                    .contains_key(&binding.callable_id),
                "{package} {index} omitted callable semantic facts"
            );
        }
    }
    fs::remove_dir_all(artifacts).unwrap();
}

struct EscapedPlatformFixture {
    base: std::path::PathBuf,
    root: std::path::PathBuf,
}

impl EscapedPlatformFixture {
    fn new() -> Self {
        let base = temporary_path("combined-symlink-escape");
        let root = base.join("platform");
        fs::create_dir_all(root.join("std")).unwrap();
        fs::create_dir_all(root.join("prelude")).unwrap();
        fs::write(
            root.join("std/registry.yml"),
            "schemaVersion: skiff-std-registry-v1\npackages:\n  - id: skiff.run/std\n    path: .\n",
        )
        .unwrap();
        fs::write(
            root.join("std/package.yml"),
            "id: skiff.run/std\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(root.join("prelude/error.skiff"), "").unwrap();
        let outside = base.join("escaped.skiff");
        fs::write(&outside, "type EscapedArtifactType {}\n").unwrap();
        symlink(&outside, root.join("prelude/escaped.skiff")).unwrap();
        Self { base, root }
    }

    fn context(&self) -> CompilerPlatformSources {
        CompilerPlatformSources::new(&self.root).unwrap()
    }
}

impl Drop for EscapedPlatformFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}
