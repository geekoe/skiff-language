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
use crate::canonical_package::compile_package_project;

const EXPECTED_PRELUDE_IDENTITY: &str =
    "skiff-prelude-v1:sha256:aae18f07de6746b8cc769ca3bd9db6b65b6c292fc75016549b58cd253b3f3f0d";
const EXPECTED_STD_PACKAGE_BUILD_ID: &str =
    "skiff-package-build-v4:sha256:3bbab8df662b54826dfbd3112c960446dd8b429f3018e7b0a5f27ffc314b7fa4";

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
