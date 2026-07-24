use serde_json::Value;

use super::*;

#[path = "../../../cross-system-fixtures/package-service-ecosystem/activation_raw_corpus.rs"]
mod activation_raw_corpus;

#[test]
fn cross_language_golden_request_and_control_wire_decode_strictly() {
    let request_fixture = include_str!(
        "../../../cross-system-fixtures/package-service-ecosystem/activation-request.json"
    );
    let request: AssemblyActivationRequest =
        serde_json::from_str(request_fixture).expect("canonical activation request fixture");
    assert_eq!(request.expected_generation, 41);
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        serde_json::from_str::<Value>(request_fixture).unwrap()
    );

    let control_fixture =
        include_str!("../../../cross-system-fixtures/package-service-ecosystem/control-wire.json");
    let controls: Vec<AssemblyActivationControl> =
        serde_json::from_str(control_fixture).expect("canonical control fixture");
    assert_eq!(controls.len(), 6);
    assert_eq!(
        serde_json::to_value(controls).unwrap(),
        serde_json::from_str::<Value>(control_fixture).unwrap()
    );
}

#[test]
fn shared_raw_assembly_activation_request_and_control_corpus_has_exact_outcomes() {
    let cases = activation_raw_corpus::activation_raw_cases();

    let mut checked = 0;
    for case in cases
        .iter()
        .filter(|case| case.target == "request" || case.target == "control")
    {
        let bytes = case.bytes();
        let accepted = match case.target.as_str() {
            "request" => serde_json::from_slice::<AssemblyActivationRequest>(&bytes).is_ok(),
            "control" => serde_json::from_slice::<AssemblyActivationControl>(&bytes).is_ok(),
            _ => unreachable!(),
        };
        assert_eq!(
            accepted,
            case.outcome == "accept",
            "shared raw case {}",
            case.name
        );
        checked += 1;
    }
    assert!(
        checked >= 40,
        "request/control raw corpus must stay exhaustive"
    );
}

#[test]
fn assembly_activation_typed_leafs_reject_non_ascii_and_expected_max() {
    assert!(validate_activation_token("visible!~", "token").is_ok());
    assert!(validate_activation_token("not visible", "token").is_err());
    assert!(validate_activation_token("caf\u{e9}", "token").is_err());
    assert!(validate_activation_environment("prod.us-1",).is_ok());
    assert!(validate_activation_environment(".").is_err());
    assert!(validate_expected_activation_generation(
        crate::MAX_EXPECTED_ACTIVATION_GENERATION,
        "expectedGeneration"
    )
    .is_ok());
    assert!(validate_expected_activation_generation(
        crate::MAX_SAFE_ACTIVATION_GENERATION,
        "expectedGeneration"
    )
    .is_err());
}

#[test]
fn service_db_is_strict_and_only_allowed_on_router_provisioning_controls() {
    let transition = serde_json::json!({
        "type": "prepare",
        "environment": "test",
        "activationId": "activation-1",
        "expectedGeneration": 0,
        "candidateGeneration": 1,
        "assembly": {
            "assemblyIdentity": "skiff-runtime-assembly-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        "replicaId": "runtime-a",
        "serviceDb": { "mongoUrl": "mongodb://127.0.0.1:45123/test?replicaSet=rs0" }
    });
    for kind in ["prepare", "commit"] {
        let mut value = transition.clone();
        value["type"] = Value::String(kind.to_owned());
        let control: AssemblyActivationControl =
            serde_json::from_value(value.clone()).expect("provisioning control");
        assert_eq!(serde_json::to_value(control).unwrap(), value);
    }

    for invalid in [
        serde_json::json!({ "mongoUrl": "" }),
        serde_json::json!({ "mongoUrl": "   " }),
        serde_json::json!({ "mongoUrl": 42 }),
        serde_json::json!({ "mongoUrl": "mongodb://db", "storageNamespace": "legacy" }),
        serde_json::json!({ "mongoUrl": "mongodb://db", "retryWrites": true }),
    ] {
        let mut value = transition.clone();
        value["serviceDb"] = invalid;
        assert!(serde_json::from_value::<AssemblyActivationControl>(value).is_err());
    }

    for kind in ["prepared", "reject", "abort", "register"] {
        let mut value = transition.clone();
        value["type"] = Value::String(kind.to_owned());
        if kind == "reject" {
            value["reason"] = Value::String("admission".to_owned());
        }
        if kind == "register" {
            value.as_object_mut().unwrap().remove("activationId");
            value.as_object_mut().unwrap().remove("expectedGeneration");
            value.as_object_mut().unwrap().remove("candidateGeneration");
            value["generation"] = Value::from(1);
        }
        assert!(
            serde_json::from_value::<AssemblyActivationControl>(value).is_err(),
            "{kind} must reject serviceDb"
        );
    }
}

#[test]
fn public_activation_request_cannot_supply_service_db() {
    let request = serde_json::json!({
        "schemaVersion": ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION,
        "environment": "test",
        "activationId": "activation-1",
        "expectedGeneration": 0,
        "assembly": {
            "assemblyIdentity": "skiff-runtime-assembly-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        "serviceDb": { "mongoUrl": "mongodb://127.0.0.1:45123/test" }
    });
    assert!(serde_json::from_value::<AssemblyActivationRequest>(request).is_err());
}
