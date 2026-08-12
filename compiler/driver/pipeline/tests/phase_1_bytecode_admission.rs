use std::collections::BTreeMap;

use super::*;

#[test]
fn phase_1_bytecode_admission_rejects_string_shape_with_typed_error() {
    let error = compile_phase_1_source(
        "example.com/bytecode-phase1-string",
        "function run() -> string { return \"disabled\" }\n",
    )
    .unwrap_err();

    let PackageCompileError::BytecodeEmission {
        source:
            crate::BytecodeEmissionError::UnsupportedPhase1Capability {
                capability,
                module_path,
                function_key,
                location,
            },
    } = error
    else {
        panic!("expected typed Phase 1 capability rejection, got {error:?}");
    };
    assert_eq!(capability, crate::Phase1UnsupportedCapability::ValueShape);
    assert_eq!(module_path, "main");
    assert_eq!(function_key.as_deref(), Some("main::run"));
    assert_eq!(location, "return type");
}

#[test]
fn phase_1_bytecode_admission_rejects_tail_local_call_with_typed_error() {
    let error = compile_phase_1_source(
        "example.com/bytecode-phase1-tail",
        "function helper(value: number) -> number { return value + 1 }\nfunction run(value: number) -> number { return helper(value) }\n",
    )
    .unwrap_err();

    let PackageCompileError::BytecodeEmission {
        source:
            crate::BytecodeEmissionError::UnsupportedPhase1Capability {
                capability,
                module_path,
                function_key,
                location,
            },
    } = error
    else {
        panic!("expected typed Phase 1 capability rejection, got {error:?}");
    };
    assert_eq!(capability, crate::Phase1UnsupportedCapability::TailCall);
    assert_eq!(module_path, "main");
    assert_eq!(function_key.as_deref(), Some("main::run"));
    assert_eq!(location, "statement 0");
}

fn compile_phase_1_source(
    package_id: &str,
    text: &str,
) -> Result<PackageCompileOutput, PackageCompileError> {
    let repository_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compiler manifest must have a repository parent")
        .to_path_buf();
    let platform_sources =
        crate::CompilerPlatformSources::new(&repository_root).expect("repository platform sources");
    let temp = std::env::temp_dir().join(format!(
        "skiff-phase1-admission-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp).expect("create temporary source root");
    let source_path = temp.join("main.skiff");
    std::fs::write(&source_path, text).expect("write temporary source");
    let source_tree = crate::SourceTree {
        root: temp.clone(),
        sources: vec![crate::SourceTreeFile {
            module_path: "main".to_string(),
            file_path: std::path::PathBuf::from("main.skiff"),
            is_test_file: false,
            byte_len: text.len() as u64,
        }],
    };
    let compiler_source = skiff_compiler_source::source_graph::CompilerSourceFile::parse(
        std::path::PathBuf::from("main.skiff"),
        "main".to_string(),
        false,
        false,
        text.to_string(),
        source_path.display().to_string(),
    )
    .expect("parse Phase 1 source fixture");
    let package = crate::PackageSourceInput::new(
        crate::PublicationManifest::new(
            skiff_compiler_core::id::PublicationId::parse(package_id)
                .expect("valid fixture package id"),
            "1.0.0".to_string(),
            skiff_compiler_input::PublicationApiSpec::empty(),
            Vec::new(),
            crate::ManifestProvenance {
                owner: crate::ManifestOwner::UserOrBuiltinPackage,
                path: std::path::PathBuf::new(),
                synthetic: true,
            },
        ),
        source_tree,
        crate::PublicationSourceGraph::from_compiler_sources(vec![compiler_source]),
        Vec::new(),
    );
    let aliases = BTreeMap::new();
    let result = compile_package(PackageCompileInput::new(
        &platform_sources,
        &package,
        &aliases,
        package_id,
        true,
    ));
    std::fs::remove_dir_all(temp).expect("remove temporary source root");
    result
}
