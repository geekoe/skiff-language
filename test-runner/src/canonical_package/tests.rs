use std::{cell::Cell, fs, path::PathBuf};

use skiff_compiler::CompilerPlatformSources;
use skiff_compiler_source::prelude_registry::prelude_registry;

use super::{run_after_platform_context_guard, CanonicalPackageProjectError};

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
