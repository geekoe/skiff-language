use std::{cell::Cell, fs, path::PathBuf};

use skiff_compiler_input::CompilerPlatformSources;
use skiff_compiler_source::prelude_registry::{
    prelude_registry, PreludeRegistryInitializationError,
};

use super::{build_authoring_object, run_after_platform_context_guard, AuthoringObject};

#[test]
fn p5_f18b_authoring_mismatch_zero_source_reads() {
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

    let different_root = MinimalPlatformFixture::new("authoring-mismatch");
    let mismatch_reads = Cell::new(0);
    let error = run_after_platform_context_guard(&different_root.context(), || {
        mismatch_reads.set(mismatch_reads.get() + 1);
        Ok(())
    })
    .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<PreludeRegistryInitializationError>(),
        Some(PreludeRegistryInitializationError::DifferentPlatformRoot { .. })
    ));
    assert_eq!(mismatch_reads.get(), 0);

    let hostile_store = different_root.root.join("hostile-authoring-store");
    let authoring_error = build_authoring_object(
        &different_root.context(),
        AuthoringObject::Package,
        &different_root.root.join("missing-package"),
        &hostile_store,
        "dev",
        false,
    )
    .unwrap_err();
    assert!(matches!(
        authoring_error.downcast_ref::<PreludeRegistryInitializationError>(),
        Some(PreludeRegistryInitializationError::DifferentPlatformRoot { .. })
    ));
    assert!(!hostile_store.exists());
}

fn repository_platform_sources() -> CompilerPlatformSources {
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
        let root = std::env::temp_dir().join(format!(
            "skiff-p5-f18b-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
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
        Self { root }
    }

    pub(super) fn context(&self) -> CompilerPlatformSources {
        CompilerPlatformSources::new(&self.root).unwrap()
    }
}

impl Drop for MinimalPlatformFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
