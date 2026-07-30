use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};

use skiff_compiler_core::id::PublicationId;
use skiff_compiler_input::PublicationApiSpec;

use crate::{
    input::{
        CompilerPlatformSources, ManifestOwner, ManifestProvenance, PackageCompileInput,
        PackageSourceInput, PublicationManifest, PublicationSourceGraph, SourceTree,
    },
    pipeline::compile_package,
    source_compile::{reset_test_compile_count, test_compile_count},
};

const CHILD_ENV: &str = "SKIFF_P5_F18A_REAL_COMPILER_CHILD";
const TEST_NAME: &str = "pipeline::tests::p5_f18a::p5_f18a_real_compiler_symlink_escape";

#[test]
fn p5_f18a_real_compiler_symlink_escape() {
    if std::env::var_os(CHILD_ENV).is_some() {
        run_real_compiler_probe();
        return;
    }

    let status = Command::new(std::env::current_exe().unwrap())
        .arg(TEST_NAME)
        .arg("--exact")
        .arg("--test-threads=1")
        .env(CHILD_ENV, "1")
        .status()
        .unwrap();
    assert!(status.success(), "isolated real compiler probe failed");
}

fn run_real_compiler_probe() {
    let fixture = CompilerFixture::new();
    let platform_sources = CompilerPlatformSources::new(&fixture.root).unwrap();
    let package_root = fixture.base.join("package");
    let package_id = PublicationId::parse("example.com/p5-f18a-probe").unwrap();
    let package = PackageSourceInput::new(
        PublicationManifest::new(
            package_id,
            "1.0.0".to_string(),
            PublicationApiSpec::empty(),
            Vec::new(),
            ManifestProvenance::file(
                package_root.join("package.yml"),
                ManifestOwner::UserOrBuiltinPackage,
            ),
        ),
        SourceTree {
            root: package_root,
            sources: Vec::new(),
        },
        PublicationSourceGraph::from_compiler_sources(Vec::new()),
        Vec::new(),
    );
    let aliases = BTreeMap::new();
    let input = PackageCompileInput::new(
        &platform_sources,
        &package,
        &aliases,
        "example.com/p5-f18a-probe",
    );

    reset_test_compile_count();
    let error = match compile_package(input) {
        Ok(_) => panic!("compiler published an artifact from escaped platform source input"),
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(
        message.contains("invalid compiler platform source layout")
            && message.contains("escapes canonical platform root"),
        "unexpected compiler error: {message}"
    );
    assert_eq!(
        test_compile_count(),
        0,
        "platform containment must fail before package compilation or publication"
    );
}

struct CompilerFixture {
    base: PathBuf,
    root: PathBuf,
}

impl CompilerFixture {
    fn new() -> Self {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!(
            "skiff-p5-f18a-real-compiler-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
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
}

impl Drop for CompilerFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}
