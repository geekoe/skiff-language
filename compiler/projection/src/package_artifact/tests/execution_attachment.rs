use skiff_artifact_identity::{
    package_artifact_local_abi_identity_projection, validate_package_artifact_identities,
    BYTECODE_IDENTITY_PREFIX,
};
use skiff_artifact_model::{
    derive_bytecode_statement_manifest_identity, BytecodeArtifactRef,
    BytecodeStatementManifestIdentity, BYTECODE_STATEMENT_MANIFEST_IDENTITY_PREFIX,
};

use crate::package_artifact::{
    attach_package_execution, PackageExecutionAttachment, ProjectedPackageArtifact,
};

use super::fixtures::projected_fixture;

#[test]
fn initial_projection_has_canonical_empty_execution_state() {
    let projected = projected_fixture().unwrap();
    let expected =
        derive_bytecode_statement_manifest_identity(&projected.artifact.package_id, &[]).unwrap();

    assert!(projected.artifact.bytecode.is_none());
    assert_eq!(
        projected.artifact.bytecode_statement_manifest_identity,
        expected
    );
    validate_package_artifact_identities(&projected.artifact).unwrap();
}

#[test]
fn enabled_attachment_preserves_exact_facts_and_local_abi() {
    let projected = projected_fixture().unwrap();
    let source_artifact = projected.artifact.clone();
    let source_local_projection =
        package_artifact_local_abi_identity_projection(&source_artifact).unwrap();
    let attachment = PackageExecutionAttachment {
        bytecode: bytecode_ref('a'),
        statement_manifest_identity: derive_bytecode_statement_manifest_identity(
            &source_artifact.package_id,
            &[],
        )
        .unwrap(),
    };

    let attached = attach_package_execution(&projected, attachment.clone()).unwrap();

    assert_eq!(
        attached.artifact.bytecode.as_ref(),
        Some(&attachment.bytecode)
    );
    assert_eq!(
        attached.artifact.bytecode_statement_manifest_identity,
        attachment.statement_manifest_identity
    );
    assert_eq!(projected.artifact, source_artifact);
    assert_ne!(
        attached.artifact.package_build_id,
        projected.artifact.package_build_id
    );
    assert_eq!(
        attached.artifact.package_local_abi.local_abi_identity,
        projected.artifact.package_local_abi.local_abi_identity
    );
    assert_eq!(
        package_artifact_local_abi_identity_projection(&attached.artifact).unwrap(),
        source_local_projection
    );
    assert_projection_sidecars_unchanged(&projected, &attached);
    validate_package_artifact_identities(&attached.artifact).unwrap();
}

#[test]
fn failed_identity_assignment_leaves_source_projection_unchanged() {
    let projected = projected_fixture().unwrap();
    let source_artifact = projected.artifact.clone();
    let error = attach_package_execution(
        &projected,
        PackageExecutionAttachment {
            bytecode: BytecodeArtifactRef::new("not-a-bytecode-identity"),
            statement_manifest_identity: manifest_identity('a'),
        },
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("failed to assign"),
        "unexpected error: {error}"
    );
    assert_eq!(projected.artifact, source_artifact);
}

#[test]
fn manifest_identity_changes_build_but_not_local_abi() {
    let projected = projected_fixture().unwrap();
    let source_local_projection =
        package_artifact_local_abi_identity_projection(&projected.artifact).unwrap();
    let bytecode = bytecode_ref('a');
    let first = attach_package_execution(
        &projected,
        PackageExecutionAttachment {
            bytecode: bytecode.clone(),
            statement_manifest_identity: manifest_identity('a'),
        },
    )
    .unwrap();
    let second = attach_package_execution(
        &projected,
        PackageExecutionAttachment {
            bytecode,
            statement_manifest_identity: manifest_identity('b'),
        },
    )
    .unwrap();

    assert_ne!(
        first.artifact.package_build_id,
        second.artifact.package_build_id
    );
    assert_eq!(
        first.artifact.package_local_abi.local_abi_identity,
        projected.artifact.package_local_abi.local_abi_identity
    );
    assert_eq!(
        second.artifact.package_local_abi.local_abi_identity,
        projected.artifact.package_local_abi.local_abi_identity
    );
    assert_eq!(
        package_artifact_local_abi_identity_projection(&first.artifact).unwrap(),
        source_local_projection
    );
    assert_eq!(
        package_artifact_local_abi_identity_projection(&second.artifact).unwrap(),
        source_local_projection
    );
}

#[test]
fn attachment_rejects_paths_and_noncanonical_source_states() {
    let projected = projected_fixture().unwrap();
    let mut path_bearing = bytecode_ref('a');
    path_bearing.artifact_path = Some("units/bytecode/example.json".to_string());
    let path_error = attach_package_execution(
        &projected,
        PackageExecutionAttachment {
            bytecode: path_bearing,
            statement_manifest_identity: manifest_identity('a'),
        },
    )
    .unwrap_err()
    .to_string();
    assert!(
        path_error.contains("must be path-free"),
        "unexpected error: {path_error}"
    );

    let mut bytecode_only = projected.clone();
    bytecode_only.artifact.bytecode = Some(bytecode_ref('b'));
    let bytecode_only_error = attach_package_execution(&bytecode_only, attachment('c', 'c'))
        .unwrap_err()
        .to_string();
    assert!(
        bytecode_only_error.contains("exact canonical empty statement manifest"),
        "unexpected error: {bytecode_only_error}"
    );

    let mut manifest_only = projected;
    manifest_only.artifact.bytecode_statement_manifest_identity = manifest_identity('d');
    let manifest_only_error = attach_package_execution(&manifest_only, attachment('e', 'e'))
        .unwrap_err()
        .to_string();
    assert!(
        manifest_only_error.contains("exact canonical empty statement manifest"),
        "unexpected error: {manifest_only_error}"
    );
}

fn attachment(bytecode_digest: char, manifest_digest: char) -> PackageExecutionAttachment {
    PackageExecutionAttachment {
        bytecode: bytecode_ref(bytecode_digest),
        statement_manifest_identity: manifest_identity(manifest_digest),
    }
}

fn bytecode_ref(digest: char) -> BytecodeArtifactRef {
    BytecodeArtifactRef::new(format!(
        "{BYTECODE_IDENTITY_PREFIX}:{}",
        std::iter::repeat_n(digest, 64).collect::<String>()
    ))
}

fn manifest_identity(digest: char) -> BytecodeStatementManifestIdentity {
    BytecodeStatementManifestIdentity::parse(format!(
        "{BYTECODE_STATEMENT_MANIFEST_IDENTITY_PREFIX}:{}",
        std::iter::repeat_n(digest, 64).collect::<String>()
    ))
    .unwrap()
}

fn assert_projection_sidecars_unchanged(
    source: &ProjectedPackageArtifact,
    attached: &ProjectedPackageArtifact,
) {
    assert_eq!(attached.package_schema_index, source.package_schema_index);
    assert_eq!(
        attached.package_schema_type_records,
        source.package_schema_type_records
    );
    assert_eq!(
        attached.resolved_package_schema_type_records,
        source.resolved_package_schema_type_records
    );
    assert_eq!(attached.file_ir_units, source.file_ir_units);
    assert_eq!(attached.resources, source.resources);
}
