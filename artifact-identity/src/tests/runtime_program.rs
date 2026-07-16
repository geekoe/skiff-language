use super::*;

#[test]
fn runtime_program_identity_value_keeps_legacy_projection_defaults() {
    let mut publication_abi = publication_abi_fixture();
    publication_abi.abi_identity =
        publication_abi_identity(&publication_abi).expect("publication ABI identity");
    let publication_abi_value =
        serde_json::to_value(&publication_abi).expect("publication ABI JSON");
    let service = json!({
        "schemaVersion": "skiff-service-unit-v1",
        "service": {
            "id": "example.com/svc",
            "displayName": "Typed Name"
        },
        "version": "1.0.0",
        "protocolIdentity": "protocol",
        "publicationAbi": publication_abi_value,
        "files": [],
        "gateway": {},
        "config": {}
    });

    let identity =
        runtime_program_service_unit_identity_value_from_json(&service).expect("identity");
    assert_eq!(identity["service"]["displayName"], "Typed Name");
    assert_eq!(identity["service"]["revisionId"], Value::Null);
    assert_eq!(identity["service"]["metadata"], json!({}));
    assert_eq!(identity["operations"], json!([]));
    assert_eq!(identity["packageDependencies"], json!([]));
    assert_eq!(identity["publicInstances"], Value::Null);
    assert_eq!(identity["bindingResolutions"], Value::Null);
    assert_eq!(identity["db"], Value::Null);
    assert_eq!(identity["processes"], Value::Null);
    assert_eq!(identity["spawnTargets"], json!([]));
    assert_eq!(identity["actors"], json!([]));
    assert_eq!(identity["timeout"], Value::Null);
}

#[test]
fn runtime_program_timeout_participates_in_dynamic_build_id() {
    let mut base = ServiceUnit::empty("example.com/svc", "1.0.0", "protocol");
    base.publication_abi.abi_identity =
        publication_abi_identity(&base.publication_abi).expect("publication ABI identity");
    let base_identity = runtime_program_service_unit_identity_value(&base).expect("base identity");
    assert_eq!(base_identity["timeout"], Value::Null);
    let base_bytes =
        runtime_program_service_unit_identity_bytes(&base).expect("base identity bytes");
    let base_build_id = runtime_program_dynamic_build_id(&base_bytes, []);

    let mut with_timeout = base.clone();
    with_timeout.timeout.default_ms = Some(5_000);
    with_timeout
        .timeout
        .methods
        .insert("run".to_string(), 1_500);
    let timeout_identity =
        runtime_program_service_unit_identity_value(&with_timeout).expect("timeout identity");
    assert_eq!(
        timeout_identity["timeout"],
        json!({
            "defaultMs": 5000,
            "methods": {
                "run": 1500
            }
        })
    );
    let timeout_bytes =
        runtime_program_service_unit_identity_bytes(&with_timeout).expect("timeout identity bytes");
    let timeout_build_id = runtime_program_dynamic_build_id(&timeout_bytes, []);

    assert_ne!(base_identity, timeout_identity);
    assert_ne!(base_build_id, timeout_build_id);
}

#[test]
fn runtime_program_identity_rejects_snake_case_and_missing_dependency_fields() {
    let mut publication_abi = publication_abi_fixture();
    publication_abi.abi_identity =
        publication_abi_identity(&publication_abi).expect("publication ABI identity");
    let publication_abi_value =
        serde_json::to_value(&publication_abi).expect("publication ABI JSON");
    let service = json!({
        "schema_version": "skiff-service-unit-v1",
        "service": { "id": "example.com/svc" },
        "version": "1.0.0",
        "protocolIdentity": "protocol",
        "publicationAbi": publication_abi_value,
        "files": [],
        "gateway": {},
        "config": {}
    });

    let error = runtime_program_service_unit_identity_value_from_json(&service)
        .expect_err("snake_case must fail closed")
        .to_string();
    assert!(
        error.contains("schema_version"),
        "unexpected snake_case error: {error}"
    );

    let mut service = service;
    service["schemaVersion"] = json!("skiff-service-unit-v1");
    service
        .as_object_mut()
        .expect("object")
        .remove("schema_version");
    service["packageDependencies"] = json!([{
        "id": "example.com/pkg",
        "version": "1.0.0"
    }]);
    let error = runtime_program_service_unit_identity_value_from_json(&service)
        .expect_err("missing package dependency alias must fail closed")
        .to_string();
    assert!(
        error.contains("alias"),
        "unexpected missing alias error: {error}"
    );
}

#[test]
fn runtime_program_service_dependency_identity_uses_publication_abi_projection() {
    let mut publication_abi = publication_abi_fixture();
    publication_abi.abi_identity =
        publication_abi_identity(&publication_abi).expect("publication ABI identity");
    let publication_abi_value =
        serde_json::to_value(&publication_abi).expect("publication ABI JSON");
    let service = json!({
        "schemaVersion": "skiff-service-unit-v1",
        "service": { "id": "example.com/svc" },
        "version": "1.0.0",
        "protocolIdentity": "protocol",
        "publicationAbi": publication_abi_value.clone(),
        "files": [],
        "serviceDependencies": [{
            "id": "example.com/upstream",
            "version": "1.0.0",
            "alias": "upstream",
            "buildId": "build:upstream",
            "serviceProtocolIdentity": "protocol:upstream",
            "publicationAbi": publication_abi_value
        }],
        "operations": [],
        "gateway": {},
        "config": {}
    });

    let identity =
        runtime_program_service_unit_identity_value_from_json(&service).expect("identity");
    let dependency_publication_abi = &identity["serviceDependencies"][0]["publicationAbi"];
    assert_eq!(
        dependency_publication_abi["operationExports"][0]["operationAbiId"],
        "operation:run:string"
    );
    assert!(dependency_publication_abi
        .pointer("/operationExports/0/displayName")
        .is_none());
    assert!(dependency_publication_abi
        .pointer("/operationAbi/0/operation/displayName")
        .is_none());

    let mut renamed_display = service.clone();
    renamed_display["serviceDependencies"][0]["publicationAbi"]["operationAbi"][0]["operation"]
        ["displayName"] = json!("renamed");
    let renamed_identity = runtime_program_service_unit_identity_value_from_json(&renamed_display)
        .expect("renamed identity");
    assert_eq!(identity, renamed_identity);

    let mut signature_changed = service;
    signature_changed["serviceDependencies"][0]["publicationAbi"]["operationAbi"][0]
        ["publicSignature"]["returnType"]["name"] = json!("number");
    let signature_identity =
        runtime_program_service_unit_identity_value_from_json(&signature_changed)
            .expect("signature identity");
    assert_ne!(identity, signature_identity);
}

#[test]
fn runtime_program_top_level_publication_abi_identity_uses_publication_abi_projection() {
    let mut publication_abi = publication_abi_fixture();
    publication_abi.abi_identity =
        publication_abi_identity(&publication_abi).expect("publication ABI identity");
    let mut publication_abi_value =
        serde_json::to_value(&publication_abi).expect("publication ABI JSON");
    publication_abi_value["operationAbi"][0]["publicSignature"]["params"] = json!([]);
    let service = json!({
        "schemaVersion": "skiff-service-unit-v1",
        "service": { "id": "example.com/svc" },
        "version": "1.0.0",
        "protocolIdentity": "protocol",
        "publicationAbi": publication_abi_value,
        "files": [],
        "operations": [],
        "gateway": {},
        "config": {}
    });

    let identity =
        runtime_program_service_unit_identity_value_from_json(&service).expect("identity");
    let service_publication_abi = &identity["publicationAbi"];
    assert_eq!(
        service_publication_abi["operationExports"][0]["operationAbiId"],
        "operation:run:string"
    );
    assert!(service_publication_abi
        .pointer("/operationExports/0/displayName")
        .is_none());
    assert!(service_publication_abi
        .pointer("/operationAbi/0/operation/displayName")
        .is_none());
    assert!(service_publication_abi
        .pointer("/operationAbi/0/publicSignature/params")
        .is_none());

    let mut renamed_display = service.clone();
    renamed_display["publicationAbi"]["operationAbi"][0]["operation"]["displayName"] =
        json!("renamed");
    let renamed_identity = runtime_program_service_unit_identity_value_from_json(&renamed_display)
        .expect("renamed identity");
    assert_eq!(identity, renamed_identity);

    let mut signature_changed = service;
    signature_changed["publicationAbi"]["operationAbi"][0]["publicSignature"]["returnType"]
        ["name"] = json!("number");
    let signature_identity =
        runtime_program_service_unit_identity_value_from_json(&signature_changed)
            .expect("signature identity");
    assert_ne!(identity, signature_identity);
}

#[test]
fn runtime_program_top_level_publication_abi_is_required() {
    let service = json!({
        "schemaVersion": "skiff-service-unit-v1",
        "service": { "id": "example.com/svc" },
        "version": "1.0.0",
        "protocolIdentity": "protocol",
        "files": [],
        "operations": [],
        "gateway": {},
        "config": {}
    });

    let error = runtime_program_service_unit_identity_value_from_json(&service)
        .expect_err("missing top-level publicationAbi must fail closed")
        .to_string();
    assert!(
        error.contains("publicationAbi"),
        "unexpected missing publicationAbi error: {error}"
    );

    let mut null_publication_abi = service;
    null_publication_abi["publicationAbi"] = Value::Null;
    let error = runtime_program_service_unit_identity_value_from_json(&null_publication_abi)
        .expect_err("null top-level publicationAbi must fail closed")
        .to_string();
    assert!(
        error.contains("PublicationAbiUnit"),
        "unexpected null publicationAbi error: {error}"
    );
}

#[test]
fn runtime_program_service_dependency_identity_requires_publication_abi() {
    let mut publication_abi = publication_abi_fixture();
    publication_abi.abi_identity =
        publication_abi_identity(&publication_abi).expect("publication ABI identity");
    let publication_abi_value =
        serde_json::to_value(&publication_abi).expect("publication ABI JSON");
    let service = json!({
        "schemaVersion": "skiff-service-unit-v1",
        "service": { "id": "example.com/svc" },
        "version": "1.0.0",
        "protocolIdentity": "protocol",
        "publicationAbi": publication_abi_value,
        "files": [],
        "serviceDependencies": [{
            "id": "example.com/upstream",
            "version": "1.0.0",
            "alias": "upstream",
            "buildId": "build:upstream",
            "serviceProtocolIdentity": "protocol:upstream"
        }],
        "operations": [],
        "gateway": {},
        "config": {}
    });

    let error = runtime_program_service_unit_identity_value_from_json(&service)
        .expect_err("missing dependency publicationAbi must fail closed")
        .to_string();
    assert!(
        error.contains("publicationAbi"),
        "unexpected missing publicationAbi error: {error}"
    );
}
