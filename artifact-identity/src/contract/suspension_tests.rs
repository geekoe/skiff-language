use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCallbackOperation, BoundaryEffectGuarantee,
    BoundaryOperationContract, BoundaryOperationDescriptor, BoundaryParameter, BoundaryReturn,
    BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, ContractDiagnosticText, ContractTypeDescriptor,
    ContractTypeRef, PackageCallableParameter, PackageCallableSignature,
    PackageSchemaCanonicalDescriptor, PackageSchemaTypeId, PackageSchemaTypeRecord, PackageTypeRef,
    PackageTypeRequirement, ServiceContract, ServiceProtocolIdentity, TypeRefIr,
    SERVICE_CONTRACT_SCHEMA_VERSION,
};

use super::*;

#[test]
fn callback_schema_shape_is_identity_bearing_but_implementor_summary_is_not() {
    let descriptor = callback_descriptor("string", "void");
    let baseline = package_schema_type_id("example.pkg", "Callback", &descriptor).unwrap();
    let non_suspending = provider_signature(false);
    let suspending = provider_signature(true);

    assert_ne!(non_suspending, suspending);
    assert_eq!(
        package_schema_type_id("example.pkg", "Callback", &descriptor).unwrap(),
        baseline,
        "concrete implementor summaries are outside PackageSchemaType"
    );
    assert_ne!(
        package_schema_type_id(
            "example.pkg",
            "Callback",
            &callback_descriptor("integer", "void")
        )
        .unwrap(),
        baseline
    );
    assert_ne!(
        package_schema_type_id(
            "example.pkg",
            "Callback",
            &callback_descriptor("string", "bool")
        )
        .unwrap(),
        baseline
    );
}

#[test]
fn provider_summary_is_outside_service_contract_protocol_and_operation_identity() {
    let type_id = package_schema_type_id("example.pkg", "User", &descriptor("string")).unwrap();
    let contract = service_contract(type_id);
    let contract_body = serde_json::to_value(&contract).unwrap();
    let protocol = service_protocol_identity(&contract).unwrap();
    let operation_id = contract.operations.keys().next().unwrap().clone();
    let non_suspending = provider_signature(false);
    let suspending = provider_signature(true);

    assert_ne!(non_suspending, suspending);
    for _provider in [non_suspending, suspending] {
        assert_eq!(serde_json::to_value(&contract).unwrap(), contract_body);
        assert_eq!(service_protocol_identity(&contract).unwrap(), protocol);
        assert_eq!(contract.operations.keys().next().unwrap(), &operation_id);
    }
    assert!(
        !contract_body.to_string().contains("maySuspend"),
        "provider summary must not enter ServiceContract"
    );
}

#[test]
fn stale_package_schema_type_prefix_fails_closed() {
    let canonical_descriptor = descriptor("string");
    let current = package_schema_type_id("example.pkg", "User", &canonical_descriptor).unwrap();
    let stale = PackageSchemaTypeId::new(current.as_str().replacen(
        PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX,
        "skiff-package-schema-type-v1:sha256",
        1,
    ));
    let records = BTreeMap::from([(
        stale.clone(),
        PackageSchemaTypeRecord {
            package_id: "example.pkg".to_string(),
            stable_schema_key: "User".to_string(),
            package_schema_type_id: stale,
            canonical_descriptor,
        },
    )]);

    assert!(matches!(
        validate_package_schema_records(&records),
        Err(ArtifactIdentityError::InvalidServiceContract { .. })
    ));
}

fn descriptor(target: &str) -> PackageSchemaCanonicalDescriptor {
    PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Representation {
            target: ContractTypeRef::builtin(target),
        },
    }
}

fn callback_descriptor(parameter: &str, returned: &str) -> PackageSchemaCanonicalDescriptor {
    PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::CallbackInterface {
            operations: BTreeMap::from([(
                "invoke".to_string(),
                BoundaryCallbackOperation {
                    parameters: vec![ContractTypeRef::builtin(parameter)],
                    return_type: ContractTypeRef::builtin(returned),
                },
            )]),
        },
    }
}

fn provider_signature(may_suspend: bool) -> PackageCallableSignature {
    PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![PackageCallableParameter {
            name: "value".to_string(),
            ty: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            },
        }],
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("void"),
        },
        may_suspend,
    }
}

fn service_contract(type_id: PackageSchemaTypeId) -> ServiceContract {
    let operation_id = contract_operation_id("example.service", "1.0.0", "get").unwrap();
    let operation = BoundaryOperationDescriptor {
        operation_id: operation_id.clone(),
        stable_key: "get".to_string(),
        contract: BoundaryOperationContract {
            parameters: vec![BoundaryParameter {
                name: "user".to_string(),
                ty: ContractTypeRef::package_schema("example.pkg", "User", type_id.clone()),
                value_plan: value_plan(BoundaryValueOwner::Caller),
            }],
            return_value: BoundaryReturn {
                ty: ContractTypeRef::builtin("void"),
                value_plan: value_plan(BoundaryValueOwner::Provider),
            },
            stream: BoundaryStreamContract::Unary,
            callbacks: BoundaryCallbackContract::None,
            effect_guarantee: BoundaryEffectGuarantee {
                detached_parameters: true,
                detached_return: true,
                detached_error: true,
                no_caller_reachable_mutation: true,
                no_caller_value_escape: true,
                no_same_heap_identity: true,
            },
        },
    };
    ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: "example.service".to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(operation_id, operation)]),
        package_type_requirements: vec![PackageTypeRequirement {
            package_id: "example.pkg".to_string(),
            required_type_ids: vec![type_id],
        }],
        diagnostic_text: ContractDiagnosticText {
            service: String::new(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    }
}

fn value_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}
