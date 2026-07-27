use std::{cell::Cell, fs, path::PathBuf};

use skiff_compiler::CompilerPlatformSources;
use skiff_compiler_source::prelude_registry::prelude_registry;

use super::{
    read_test_service_profile, run_after_platform_context_guard, CanonicalPackageProjectError,
    PackageCompileWorkflow,
};

#[cfg(unix)]
#[path = "tests/combined.rs"]
mod combined;

#[test]
fn p5_f18b_runner_mismatch_zero_source_reads() {
    let platform_sources = repository_platform_sources();
    run_after_platform_context_guard(&platform_sources, || Ok(())).unwrap();

    let same_root_reads = Cell::new(0);
    run_after_platform_context_guard(&platform_sources, || {
        let _ = prelude_registry();
        same_root_reads.set(same_root_reads.get() + 1);
        Ok(())
    })
    .unwrap();
    assert_eq!(same_root_reads.get(), 1);

    let different_root = MinimalPlatformFixture::new("runner-mismatch");
    let mismatch_reads = Cell::new(0);
    let error = run_after_platform_context_guard(&different_root.context(), || {
        mismatch_reads.set(mismatch_reads.get() + 1);
        Ok(())
    })
    .unwrap_err();
    assert!(matches!(
        error,
        CanonicalPackageProjectError::PlatformContext(
            skiff_compiler_source::prelude_registry::PreludeRegistryInitializationError::DifferentPlatformRoot { .. }
        )
    ));
    assert_eq!(mismatch_reads.get(), 0);
}

#[test]
fn split_external_manifests_require_and_preserve_the_service_role_marker() {
    for (external_file, source) in [("http.yml", "{}\n"), ("websocket.yml", "path: /socket\n")] {
        let external_only = temporary_path(&format!("external-only-role-{external_file}"));
        fs::create_dir_all(&external_only).unwrap();
        fs::write(
            external_only.join("package.yml"),
            "id: example.com/external-only\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(external_only.join("api.yml"), "{}\n").unwrap();
        fs::write(external_only.join(external_file), source).unwrap();
        let error = read_test_service_profile(&external_only, PackageCompileWorkflow::Test)
            .expect_err("external files must not create a service role");
        assert!(matches!(
            error,
            CanonicalPackageProjectError::ServiceConfig(_)
        ));
        assert!(
            error.to_string().contains("service.yml"),
            "unexpected error: {error}"
        );
        fs::remove_dir_all(external_only).unwrap();
    }

    let split = temporary_path("split-test-service-role");
    fs::create_dir_all(&split).unwrap();
    fs::write(
        split.join("package.yml"),
        "id: example.com/split-test\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(split.join("api.yml"), "{}\n").unwrap();
    fs::write(
        split.join("service.yml"),
        "id: example.com/split-test\nkind: test\n",
    )
    .unwrap();
    fs::write(split.join("http.yml"), "{}\n").unwrap();
    fs::write(split.join("websocket.yml"), "path: /socket\n").unwrap();
    fs::write(split.join("config.skiff-test.yml"), "timeout: 30000\n").unwrap();

    let profile = read_test_service_profile(&split, PackageCompileWorkflow::Test)
        .unwrap()
        .expect("service.yml kind: test should declare the test service role");
    assert_eq!(profile.service_id, "example.com/split-test");
    assert_eq!(profile.profile_name, "skiff-test");

    fs::write(split.join("http.yml"), "http: {}\n").unwrap();
    let error = read_test_service_profile(&split, PackageCompileWorkflow::Test)
        .expect_err("role discovery must use the typed split root reader");
    assert!(matches!(
        error,
        CanonicalPackageProjectError::ServiceConfig(_)
    ));
    assert!(
        error.to_string().contains("http.yml"),
        "unexpected error: {error}"
    );
    fs::remove_dir_all(split).unwrap();
}

pub(super) fn repository_platform_sources() -> CompilerPlatformSources {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    CompilerPlatformSources::new(&root).unwrap()
}

pub(super) struct MinimalPlatformFixture {
    root: PathBuf,
}

impl MinimalPlatformFixture {
    pub(super) fn new(name: &str) -> Self {
        let root = temporary_path(name);
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
        Self { root }
    }

    pub(super) fn context(&self) -> CompilerPlatformSources {
        CompilerPlatformSources::new(&self.root).unwrap()
    }

    pub(super) fn root(&self) -> &std::path::Path {
        &self.root
    }
}

impl Drop for MinimalPlatformFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) fn temporary_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "skiff-p5-f18b-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
