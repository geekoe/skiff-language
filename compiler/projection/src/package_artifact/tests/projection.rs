use skiff_artifact_identity::validate_package_artifact_identities;
use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryUnavailableReason, PackageLocalAbiSymbol,
};

use super::fixtures::{callable_id, project_fixture, SignatureSet};

#[test]
fn package_api_callables_have_exact_local_abi_and_boundary_coverage() {
    let artifact = project_fixture(SignatureSet::Complete, "async").unwrap();
    validate_package_artifact_identities(&artifact).unwrap();

    let callable_paths = artifact
        .package_local_abi
        .public_symbols
        .iter()
        .filter_map(|(path, symbol)| {
            matches!(symbol, PackageLocalAbiSymbol::Callable { .. }).then_some(path.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(callable_paths, vec!["mutate", "run", "worker.handle"]);
    assert_eq!(artifact.callable_links.len(), 3);
    assert_eq!(artifact.callable_semantic_facts.len(), 3);
    assert_eq!(artifact.boundary_projections.len(), 3);
    assert_eq!(artifact.package_requirements.len(), 1);
    assert_eq!(artifact.contract_requirements.len(), 1);
    assert_eq!(artifact.service_requirements.len(), 1);
    assert_eq!(artifact.service_call_refs.len(), 1);
    assert_eq!(artifact.service_call_refs[0].service_requirement_slot, 3);

    let PackageLocalAbiSymbol::PublicInstance { methods, .. } =
        &artifact.package_local_abi.public_symbols["worker"]
    else {
        panic!("public instance must remain in Local ABI");
    };
    assert_eq!(methods.len(), 1);
    let mutate_id = callable_id(&artifact, "mutate");
    assert!(matches!(
        &artifact.boundary_projections[&mutate_id],
        BoundaryCallableProjection::Unavailable { reasons }
            if reasons.contains(&BoundaryUnavailableReason::WritesCallerReachable)
    ));
    assert!(artifact
        .implementation_links
        .functions
        .contains_key("mutate"));
    assert!(artifact.callable_links.contains_key(&mutate_id));

    let wire = serde_json::to_string(&artifact).unwrap();
    for forbidden in [
        "publicationAbi",
        "packageUnit",
        "serviceUnit",
        "providerBuildId",
        "deploymentRevision",
        "route",
    ] {
        assert!(!wire.contains(forbidden), "forbidden field {forbidden}");
    }
}

#[test]
fn canonical_signature_set_rejects_missing_and_extra_api_entries() {
    let missing = project_fixture(SignatureSet::Missing, "async")
        .unwrap_err()
        .to_string();
    assert!(missing.contains("missing="), "unexpected error: {missing}");
    assert!(missing.contains("mutate"), "unexpected error: {missing}");

    let extra = project_fixture(SignatureSet::Extra, "async")
        .unwrap_err()
        .to_string();
    assert!(extra.contains("extra="), "unexpected error: {extra}");
    assert!(extra.contains("internal"), "unexpected error: {extra}");
}

#[test]
fn implementation_requirements_change_build_not_local_abi_or_descriptor() {
    let first = project_fixture(SignatureSet::Complete, "async").unwrap();
    let second = project_fixture(SignatureSet::Complete, "async-v2").unwrap();
    assert_eq!(
        first.package_local_abi.local_abi_identity,
        second.package_local_abi.local_abi_identity
    );
    assert_ne!(first.package_build_id, second.package_build_id);

    let first_id = callable_id(&first, "run");
    let second_id = callable_id(&second, "run");
    let BoundaryCallableProjection::Available {
        descriptor: first_descriptor,
        ..
    } = &first.boundary_projections[&first_id]
    else {
        panic!("run must be available");
    };
    let BoundaryCallableProjection::Available {
        descriptor: second_descriptor,
        ..
    } = &second.boundary_projections[&second_id]
    else {
        panic!("run must be available");
    };
    assert_eq!(first_descriptor, second_descriptor);
}
