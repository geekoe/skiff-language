use serde_json::json;

use super::*;

#[test]
fn service_contract_wire_rejects_missing_and_provider_fields() {
    let minimal = json!({
        "schemaVersion": "skiff-service-contract-v6",
        "serviceId": "example.echo",
        "contractVersion": "1.0.0",
        "serviceProtocolIdentity": "protocol",
        "operations": {},
        "publicInstances": {},
        "packageTypeRequirements": [],
        "diagnosticText": { "service": "", "operations": {}, "types": {} }
    });
    serde_json::from_value::<ServiceContract>(minimal.clone())
        .expect("complete strict contract wire");

    for field in [
        "serviceProtocolIdentity",
        "operations",
        "publicInstances",
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

#[test]
fn public_instance_tables_round_trip_without_provider_authority() {
    let value = json!({
        "interfaces": [{
            "interface": {
                "interfaceAbiId": "interface:reader"
            },
            "methods": [{
                "methodAbiId": "method:reader:read",
                "contractOperationId": "operation:reader:read"
            }]
        }]
    });
    let instance: ContractPublicInstance =
        serde_json::from_value(value.clone()).expect("exact public-instance table");
    assert_eq!(serde_json::to_value(instance).unwrap(), value);

    for (path, field) in [
        ("instance", "providerBuildId"),
        ("interface", "packageCallableId"),
        ("method", "functionKey"),
    ] {
        let mut invalid = value.clone();
        match path {
            "instance" => invalid[field] = json!("forbidden"),
            "interface" => invalid["interfaces"][0][field] = json!("forbidden"),
            "method" => invalid["interfaces"][0]["methods"][0][field] = json!("forbidden"),
            _ => unreachable!(),
        }
        assert!(
            serde_json::from_value::<ContractPublicInstance>(invalid).is_err(),
            "{field} must not enter the public-instance contract"
        );
    }
}

#[test]
fn public_instance_interfaces_are_a_canonical_exact_set() {
    let interface = |abi: &str| {
        json!({
            "interface": {
                "interfaceAbiId": abi
            },
            "methods": []
        })
    };
    for interfaces in [
        vec![interface("interface:z"), interface("interface:a")],
        vec![interface("interface:a"), interface("interface:a")],
    ] {
        assert!(serde_json::from_value::<ContractPublicInstance>(json!({
            "interfaces": interfaces
        }))
        .is_err());
    }

    let canonical = json!({
        "interfaces": [interface("interface:a"), interface("interface:z")]
    });
    let decoded: ContractPublicInstance = serde_json::from_value(canonical.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), canonical);
}
