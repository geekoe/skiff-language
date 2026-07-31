use serde_json::json;

use super::*;

#[test]
fn service_contract_wire_rejects_missing_and_provider_fields() {
    let minimal = json!({
        "schemaVersion": "skiff-service-contract-v5",
        "serviceId": "example.echo",
        "contractVersion": "1.0.0",
        "serviceProtocolIdentity": "protocol",
        "operations": {},
        "packageTypeRequirements": [],
        "diagnosticText": { "service": "", "operations": {}, "types": {} }
    });
    serde_json::from_value::<ServiceContract>(minimal.clone())
        .expect("complete strict contract wire");

    for field in [
        "serviceProtocolIdentity",
        "operations",
        "packageTypeRequirements",
        "diagnosticText",
    ] {
        let mut missing = minimal.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<ServiceContract>(missing).is_err(),
            "missing {field} must fail closed"
        );
    }

    for forbidden in [
        "providerPackageId",
        "providerBuildId",
        "deploymentRevision",
        "route",
        "runtimeReplica",
        "implementationRequirements",
    ] {
        let mut value = minimal.clone();
        value
            .as_object_mut()
            .unwrap()
            .insert(forbidden.to_string(), json!("forbidden"));
        assert!(
            serde_json::from_value::<ServiceContract>(value).is_err(),
            "{forbidden} must not enter ServiceContract"
        );
    }
}
