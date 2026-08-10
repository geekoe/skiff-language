use skiff_artifact_model::{PackageBuildId, PackageCallableId, PackageExecutableCoordinate};
use skiff_runtime_linked_bytecode::{ArtifactFunctionKey, SpecializationKey};

pub(super) fn specialization(build: &PackageBuildId, ordinal: usize) -> SpecializationKey {
    SpecializationKey::new(
        build.clone(),
        ArtifactFunctionKey::parse(function_key(ordinal)).unwrap(),
        callable(ordinal),
        Box::new([]),
        None,
    )
}

pub(super) fn function_key(ordinal: usize) -> String {
    format!("fixture::effect_{ordinal:04}")
}

pub(super) fn symbol(ordinal: usize) -> String {
    format!("fixture.effect_{ordinal:04}")
}

pub(super) fn callable(ordinal: usize) -> PackageCallableId {
    PackageCallableId::new(format!(
        "pkg-callable:example.effect-graph:top-level:fixture.effect_{ordinal:04}"
    ))
}

pub(super) fn coordinate(ordinal: usize) -> PackageExecutableCoordinate {
    PackageExecutableCoordinate {
        file_ir_identity: "file-ir:effect-graph".to_string(),
        module_path: "fixture".to_string(),
        executable_index: ordinal as u32,
    }
}
