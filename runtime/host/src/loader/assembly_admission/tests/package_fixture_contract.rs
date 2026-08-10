use skiff_artifact_model::{
    derive_bytecode_statement_manifest_identity, PackageArtifact, PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

#[track_caller]
pub(super) fn assert_bytecode_free_statement_epoch_fixture(package: &PackageArtifact) {
    assert!(
        package.bytecode.is_none(),
        "bytecode-free fixture assertion requires bytecode: None"
    );
    let expected = derive_bytecode_statement_manifest_identity(&package.package_id, &[])
        .expect("empty bytecode statement manifest is canonical");
    assert_eq!(package.bytecode_statement_manifest_identity, expected);
    assert!(package.synthetic_callback_owners.is_empty());
    assert!(package.bytecode_schema_records.is_empty());

    let wire = serde_json::to_value(package).expect("fixture package should serialize");
    assert_eq!(
        wire["schemaVersion"],
        serde_json::json!(PACKAGE_ARTIFACT_SCHEMA_VERSION)
    );
    assert!(wire.get("bytecode").is_none());
    assert_eq!(
        wire["bytecodeStatementManifestIdentity"],
        serde_json::json!(expected.as_str())
    );
    assert_eq!(wire["syntheticCallbackOwners"], serde_json::json!([]));
    assert_eq!(wire["bytecodeSchemaRecords"], serde_json::json!({}));
    assert_eq!(
        serde_json::from_value::<PackageArtifact>(wire)
            .expect("statement-epoch fixture wire should deserialize exactly"),
        *package
    );

    let foreign_package_id = format!("{}.foreign-owner", package.package_id);
    let mut wrong_owner = package.clone();
    wrong_owner.bytecode_statement_manifest_identity =
        derive_bytecode_statement_manifest_identity(&foreign_package_id, &[])
            .expect("foreign empty bytecode statement manifest is canonical");
    let error = skiff_artifact_identity::validate_package_artifact_identities(&wrong_owner)
        .expect_err("empty statement manifest must bind the exact packageId");
    assert!(
        error.to_string().contains(
            "package without bytecode must declare the canonical empty statement manifest"
        ),
        "unexpected wrong-owner statement manifest error: {error}"
    );
}
