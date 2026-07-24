use std::{fs, os::unix::fs::symlink};

use skiff_artifact_model::{
    BoundaryCallableProjection, CallableEffectSummary, CallableProvenanceSummary,
};
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
use crate::canonical_package::compile_package_project;
use crate::{
    canonical_fixture::discover_package_test_cases, canonical_std_seed::seed_canonical_std,
    test_overlay::compile_package_test_overlay,
};

// Explicit F18 identity probe pins refreshed for c277e45's canonical WebSocket
// std surface; the production seed consumes the F27A typed receipt instead.
const EXPECTED_PRELUDE_IDENTITY: &str =
    "skiff-prelude-v1:sha256:5166ba3c306e94624094e0736da821a1b653da5aace1ef8cee2fb654f4106699";
const EXPECTED_STD_PACKAGE_BUILD_ID: &str =
    "skiff-package-build-v4:sha256:4cf082e69e7b95f16494319f1a74bd0c1d6499f75ee45092bcabcb12241be24e";

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
        build_authoring_object(
            &platform_sources,
            AuthoringObject::Package,
            &package_root,
            &artifacts,
            "dev",
            true,
        )
        .unwrap();
        let project =
            compile_package_project(&platform_sources, &package_root, &artifacts).unwrap();
        let cases = discover_package_test_cases(&package_root, &package_root, false).unwrap();
        let overlay = compile_package_test_overlay(
            &platform_sources,
            &package_root,
            &artifacts,
            &project,
            &cases,
        )
        .unwrap();
        let binding = overlay
            .bindings
            .iter()
            .find(|binding| binding.public_path == "testCases.case0")
            .unwrap_or_else(|| panic!("{package} did not emit testCases.case0"));
        let facts = &overlay.overlay.artifact.callable_semantic_facts[&binding.callable_id];
        let CallableEffectSummary::Analyzed { effects } = facts.effects else {
            panic!("{package} case0 retained unknown effects");
        };
        assert!(
            !effects.writes_caller_reachable,
            "{package} case0: {facts:?}"
        );
        assert!(
            !effects.requires_same_heap_identity,
            "{package} case0: {facts:?}"
        );
        assert!(!effects.invokes_unknown_target, "{package} case0");
        assert!(
            matches!(facts.provenance, CallableProvenanceSummary::Analyzed { .. }),
            "{package} case0 retained unknown provenance"
        );
        assert!(
            matches!(
                overlay.overlay.artifact.boundary_projections[&binding.callable_id],
                BoundaryCallableProjection::Available { .. }
            ),
            "{package} case0 is not boundary available"
        );
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
        fs::write(
            root.join("prelude/error.skiff"),
            "native type ErrorPayload\n",
        )
        .unwrap();
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
