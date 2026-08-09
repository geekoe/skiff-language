use super::*;
use crate::{
    derive_synthetic_callback_callable_id, validate_package_build_authority,
    PackageSyntheticCallbackOwner, PACKAGE_SYNTHETIC_CALLBACK_CALLABLE_IDENTITY_PREFIX,
};

fn add_synthetic(artifact: &mut PackageArtifact, site_ordinal: u32) -> PackageCallableId {
    let callable_id = derive_synthetic_callback_callable_id(
        &artifact.package_id,
        &PackageCallableId::new(IMPLEMENTATION_CALLABLE_ID),
        site_ordinal,
    )
    .unwrap();
    artifact
        .synthetic_callback_owners
        .push(PackageSyntheticCallbackOwner {
            owner: owner_coordinate(),
            site_ordinal,
            package_callable_id: callable_id.clone(),
        });
    artifact
        .callable_semantic_facts
        .insert(callable_id.clone(), facts());
    callable_id
}

#[test]
fn synthetic_callback_identity_has_a_fixed_domain_separated_preimage() {
    let identity = derive_synthetic_callback_callable_id(
        PACKAGE_ID,
        &PackageCallableId::new(IMPLEMENTATION_CALLABLE_ID),
        3,
    )
    .unwrap();
    assert_eq!(
        identity.as_str(),
        "skiff-package-synthetic-callback-callable-v1:sha256:1f88bb17b667dddbb4f6731fed6d0683e96a8e776673cad51127816c0b7f2f43"
    );
    assert!(identity
        .as_str()
        .starts_with(PACKAGE_SYNTHETIC_CALLBACK_CALLABLE_IDENTITY_PREFIX));
}

#[test]
fn synthetic_owner_uses_actual_implementation_id_and_extends_only_semantic_facts() {
    let mut artifact = authority_artifact();
    let synthetic_id = add_synthetic(&mut artifact, 3);
    assert!(validate_package_build_authority(&artifact).is_ok());
    assert!(!artifact.callable_links.contains_key(&synthetic_id));

    let alias_id = PackageCallableId::new("public:alias");
    artifact.package_local_abi.public_symbols.insert(
        "alias".to_string(),
        PackageLocalAbiSymbol::Callable {
            callable_id: alias_id.clone(),
            signature: signature(),
        },
    );
    let mut alias_link =
        artifact.callable_links[&PackageCallableId::new(IMPLEMENTATION_CALLABLE_ID)].clone();
    alias_link.callable_id = alias_id.clone();
    alias_link.target.callable_abi_id = alias_id.to_string();
    artifact.callable_links.insert(alias_id.clone(), alias_link);
    artifact.callable_semantic_facts.insert(alias_id, facts());
    assert!(validate_package_build_authority(&artifact).is_ok());
}

#[test]
fn synthetic_owner_drift_unknown_owner_and_coverage_fail_closed() {
    let mut artifact = authority_artifact();
    let synthetic_id = add_synthetic(&mut artifact, 3);

    artifact.synthetic_callback_owners[0].package_callable_id = PackageCallableId::new("forged");
    assert!(validate_package_build_authority(&artifact).is_err());

    artifact.synthetic_callback_owners[0].package_callable_id = synthetic_id.clone();
    artifact.synthetic_callback_owners[0].owner.executable_index += 1;
    assert!(validate_package_build_authority(&artifact).is_err());

    artifact.synthetic_callback_owners[0].owner = owner_coordinate();
    artifact.callable_semantic_facts.remove(&synthetic_id);
    assert!(validate_package_build_authority(&artifact).is_err());

    artifact
        .callable_semantic_facts
        .insert(synthetic_id.clone(), facts());
    let ordinary_link =
        artifact.callable_links[&PackageCallableId::new(IMPLEMENTATION_CALLABLE_ID)].clone();
    artifact
        .callable_links
        .insert(synthetic_id.clone(), ordinary_link);
    assert!(validate_package_build_authority(&artifact).is_err());
}

#[test]
fn synthetic_rows_are_required_nullable_rejecting_and_canonical() {
    let artifact = authority_artifact();
    let mut wire = serde_json::to_value(&artifact).unwrap();
    for field in ["syntheticCallbackOwners", "bytecodeSchemaRecords"] {
        let mut missing = wire.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<PackageArtifact>(missing).is_err());

        let mut null = wire.clone();
        null[field] = serde_json::Value::Null;
        assert!(serde_json::from_value::<PackageArtifact>(null).is_err());
    }
    assert_eq!(wire["syntheticCallbackOwners"], serde_json::json!([]));
    assert_eq!(wire["bytecodeSchemaRecords"], serde_json::json!({}));

    let mut noncanonical = authority_artifact();
    add_synthetic(&mut noncanonical, 2);
    add_synthetic(&mut noncanonical, 1);
    wire = serde_json::to_value(noncanonical).unwrap();
    assert!(serde_json::from_value::<PackageArtifact>(wire).is_err());
}

#[test]
fn ordinary_owner_coordinate_must_resolve_to_one_implementation_callable() {
    let mut artifact = authority_artifact();
    let second_id = PackageCallableId::new("impl:second");
    artifact.package_local_abi.implementation_symbols.insert(
        "module.second".to_string(),
        PackageLocalAbiSymbol::Callable {
            callable_id: second_id.clone(),
            signature: signature(),
        },
    );
    let mut link =
        artifact.callable_links[&PackageCallableId::new(IMPLEMENTATION_CALLABLE_ID)].clone();
    link.callable_id = second_id.clone();
    link.target.callable_abi_id = second_id.to_string();
    artifact.callable_links.insert(second_id.clone(), link);
    artifact.callable_semantic_facts.insert(second_id, facts());
    assert!(validate_package_build_authority(&artifact).is_err());
}
