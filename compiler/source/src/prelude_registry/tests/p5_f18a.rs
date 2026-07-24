use std::{fs, path::PathBuf};

use skiff_compiler_input::{CompilerPlatformSources, CompilerPlatformSourcesError};

use super::super::{
    initialization::load_prelude_registry, PreludeRegistry, PreludeRegistryInitializationError,
};
use super::test_platform_sources;

#[test]
fn p5_f18a_prelude_loader_snapshot() {
    use std::os::unix::fs::symlink;

    let escaped = CopiedPlatformFixture::new("typed-snapshot-error");
    let outside = escaped.base.join("outside.skiff");
    fs::write(&outside, "type EscapedPreludeType {}\n").unwrap();
    symlink(&outside, escaped.root.join("prelude/escaped.skiff")).unwrap();
    let error = load_prelude_registry(&escaped.context()).unwrap_err();
    assert!(matches!(
        error,
        PreludeRegistryInitializationError::PlatformSources {
            source: CompilerPlatformSourcesError::InvalidLayout { .. },
            ..
        }
    ));

    let fixture = CopiedPlatformFixture::new("immutable-snapshot");
    for relative in ["prelude/actor.skiff", "std/actor.skiff"] {
        let logical_path = fixture.root.join(relative);
        let contained_target = logical_path.with_extension("snapshot-source");
        fs::rename(&logical_path, &contained_target).unwrap();
        symlink(&contained_target, &logical_path).unwrap();
    }
    let context = fixture.context();
    let snapshot = context.prelude_registry_snapshot().unwrap();
    fs::write(fixture.root.join("prelude/actor.skiff"), "@ invalid now\n").unwrap();
    fs::write(fixture.root.join("std/actor.skiff"), "@ invalid now\n").unwrap();

    let registry = PreludeRegistry::try_from_platform_sources(&context, &snapshot)
        .expect("loader must consume the captured source texts without re-reading their paths");
    assert_eq!(
        registry.identity(),
        // The copied snapshot includes c277e45's
        // std.websocket.WebSocketIngressEvent platform source/API fingerprint.
        "skiff-prelude-v1:sha256:06b874079b74b70aab0092e4e2ffb9781fb34cd57238f3ee90d2789f5eb6019c"
    );
}

struct CopiedPlatformFixture {
    base: PathBuf,
    root: PathBuf,
}

impl CopiedPlatformFixture {
    fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "skiff-prelude-snapshot-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = base.join("platform");
        let canonical = test_platform_sources();
        for directory in ["std", "prelude"] {
            fs::create_dir_all(root.join(directory)).unwrap();
            for entry in fs::read_dir(canonical.root().join(directory)).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_file() {
                    fs::copy(entry.path(), root.join(directory).join(entry.file_name())).unwrap();
                }
            }
        }
        Self { base, root }
    }

    fn context(&self) -> CompilerPlatformSources {
        CompilerPlatformSources::new(&self.root).unwrap()
    }
}

impl Drop for CopiedPlatformFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}
