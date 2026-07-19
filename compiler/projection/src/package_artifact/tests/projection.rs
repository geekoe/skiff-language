use skiff_artifact_identity::validate_package_artifact_identities;
use skiff_artifact_model::{
    config_shape_from_package_requirements, BoundaryCallableProjection, BoundaryUnavailableReason,
    PackageLocalAbiSymbol, PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

use super::fixtures::{
    callable_id, exact_typed_signature, project_fixture, project_fixture_with_runtime_requirements,
    runtime_requirements, SignatureSet,
};

#[test]
fn package_api_callables_have_exact_local_abi_and_boundary_coverage() {
    let artifact = project_fixture(SignatureSet::Complete, "async").unwrap();
    validate_package_artifact_identities(&artifact).unwrap();
    assert_eq!(artifact.schema_version, PACKAGE_ARTIFACT_SCHEMA_VERSION);
    assert_eq!(artifact.schema_version, "skiff-package-artifact-v2");
    assert!(artifact
        .package_build_id
        .as_str()
        .starts_with("skiff-package-build-v4:sha256:"));

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
    let config_shape =
        config_shape_from_package_requirements(&artifact.runtime_requirements.config).unwrap();
    assert_eq!(config_shape.entries.len(), 1);
    assert_eq!(config_shape.entries[0].path, "app.token");
    assert_eq!(artifact.runtime_requirements.resources.len(), 1);
    assert_eq!(artifact.runtime_requirements.runtime_capabilities.len(), 1);

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
    assert_eq!(
        artifact.implementation_links.constants["worker"].const_index,
        0
    );
    let worker_handle_id = callable_id(&artifact, "worker.handle");
    assert_eq!(
        artifact.callable_links[&worker_handle_id]
            .target
            .executable_index,
        2
    );
    assert!(artifact.callable_links.contains_key(&mutate_id));

    let wire = serde_json::to_string(&artifact).unwrap();
    for forbidden in [
        "publicationAbi",
        "packageUnit",
        "serviceUnit",
        "providerBuildId",
        "deploymentRevision",
        "route",
        "operationAbiId",
        "methodAbiId",
    ] {
        assert!(!wire.contains(forbidden), "forbidden field {forbidden}");
    }
}

#[test]
fn exact_typed_signatures_reach_local_abi_and_public_instance_receiver_is_trimmed() {
    let artifact = project_fixture(SignatureSet::ExactTyped, "async").unwrap();
    let PackageLocalAbiSymbol::Callable {
        signature: run_signature,
        ..
    } = &artifact.package_local_abi.public_symbols["run"]
    else {
        panic!("run must be a Local ABI callable");
    };
    assert_eq!(run_signature, &exact_typed_signature());

    let PackageLocalAbiSymbol::Callable {
        signature: instance_signature,
        ..
    } = &artifact.package_local_abi.public_symbols["worker.handle"]
    else {
        panic!("public-instance operation must be a Local ABI callable");
    };
    assert_eq!(instance_signature.parameters.len(), 1);
    assert_eq!(instance_signature.parameters[0].name, "value");
}

#[test]
fn canonical_projection_rejects_invalid_or_duplicate_config_requirements() {
    let mut invalid_type = runtime_requirements("async");
    invalid_type.config[0].value_type = "bytes".to_string();
    let error = project_fixture_with_runtime_requirements(SignatureSet::Complete, invalid_type)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("canonical runtime config requirements are invalid"),
        "unexpected error: {error}"
    );
    assert!(error.contains("app.token"), "unexpected error: {error}");

    let mut duplicate = runtime_requirements("async");
    duplicate.config.push(duplicate.config[0].clone());
    let error = project_fixture_with_runtime_requirements(SignatureSet::Complete, duplicate)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("declared more than once"),
        "unexpected error: {error}"
    );
}

#[test]
fn missing_signature_is_not_reconstructed_from_executable_ir() {
    let missing = project_fixture(SignatureSet::Missing, "async")
        .unwrap_err()
        .to_string();
    assert!(missing.contains("missing="), "unexpected error: {missing}");
    assert!(missing.contains("mutate"), "unexpected error: {missing}");
}

#[test]
fn canonical_signature_set_rejects_extra_and_target_mismatched_entries() {
    let extra = project_fixture(SignatureSet::Extra, "async")
        .unwrap_err()
        .to_string();
    assert!(extra.contains("extra="), "unexpected error: {extra}");
    assert!(extra.contains("internal"), "unexpected error: {extra}");

    let target_mismatch = project_fixture(SignatureSet::TargetMismatch, "async")
        .unwrap_err()
        .to_string();
    assert!(
        target_mismatch.contains("api#0") && target_mismatch.contains("api#9"),
        "unexpected error: {target_mismatch}"
    );
}

#[test]
fn implementation_requirements_change_build_not_local_abi_or_operation_contract() {
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
        operation_contract: first_contract,
        ..
    } = &first.boundary_projections[&first_id]
    else {
        panic!("run must be available");
    };
    let BoundaryCallableProjection::Available {
        operation_contract: second_contract,
        ..
    } = &second.boundary_projections[&second_id]
    else {
        panic!("run must be available");
    };
    assert_eq!(first_contract, second_contract);
}
