use super::*;
use crate::{
    addr::ExecutableAddr,
    runtime_value::{
        InterfaceMethodSignature, InterfaceMethodSlot, InterfaceMethodTable, InterfaceMethodTarget,
        RuntimeValue,
    },
};
use skiff_artifact_model::PackageSchemaTypeId;

fn local_interface(method_name: &str, method_abi_id: &str) -> InterfaceValue {
    InterfaceValue::new(
        "interface-abi:observer".to_string(),
        InterfaceCarrier::Local {
            concrete_type: "local:Observer".to_string(),
            method_table: InterfaceMethodTable::new(
                "table:observer".to_string(),
                "interface-abi:observer".to_string(),
                vec![InterfaceMethodSlot::from_admitted_metadata(
                    0,
                    method_name.to_string(),
                    method_abi_id.to_string(),
                    InterfaceMethodSignature::new(
                        vec![InterfaceMethodType::builtin("Self")],
                        InterfaceMethodType::builtin("bool"),
                    ),
                    InterfaceMethodTarget::LocalExecutable {
                        executable: ExecutableAddr::service(0, 1),
                        receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
                    },
                )],
            ),
            payload: RuntimeValue::Bool(true),
        },
    )
}

fn operations() -> BTreeMap<String, BoundaryCallbackOperation> {
    BTreeMap::from([(
        "invoke".to_string(),
        BoundaryCallbackOperation {
            parameters: Vec::new(),
            return_type: ContractTypeRef::builtin("bool"),
        },
    )])
}

fn package_schema_type(
    package_id: &str,
    stable_schema_key: &str,
    type_id: &str,
) -> PackageSchemaTypeRef {
    PackageSchemaTypeRef {
        package_id: package_id.to_string(),
        stable_schema_key: stable_schema_key.to_string(),
        package_schema_type_id: PackageSchemaTypeId::new(type_id),
    }
}

#[test]
fn callback_contract_projection_separates_identity_domains_and_maps_by_name() {
    let projection = CallbackContractProjection::build(
        package_schema_type(
            "skiff.run/observer",
            "api.Observer",
            "package-schema-type:observer",
        ),
        &operations(),
        &local_interface("invoke", "method-abi:callback-probe:invoke"),
    )
    .expect("typed admitted method metadata should project");

    assert_eq!(
        projection.canonical_package_schema_type(),
        &package_schema_type(
            "skiff.run/observer",
            "api.Observer",
            "package-schema-type:observer",
        )
    );
    assert_eq!(
        projection.local_interface_abi_id(),
        "interface-abi:observer"
    );
    assert_eq!(
        projection.operations()[0].method_abi_id(),
        "method-abi:callback-probe:invoke"
    );
}

#[test]
fn callback_contract_projection_rejects_order_or_string_identity_shortcuts() {
    assert!(matches!(
        CallbackContractProjection::build(
            package_schema_type(
                "skiff.run/observer",
                "api.Observer",
                "package-schema-type:observer",
            ),
            &operations(),
            &local_interface("different", "method-abi:callback-probe:invoke"),
        ),
        Err(CallbackContractProjectionError::MissingLocalMethod { .. })
    ));
    assert!(matches!(
        CallbackContractProjection::build(
            package_schema_type(
                "skiff.run/observer",
                "api.Observer",
                "package-schema-type:observer",
            ),
            &operations(),
            &local_interface("invoke", ""),
        ),
        Err(CallbackContractProjectionError::MissingAdmittedMethodMetadata { .. })
    ));
}

#[test]
fn callback_contract_projection_rejects_package_nominal_without_exact_execution_identity() {
    let callback_type = package_schema_type(
        "skiff.run/observer",
        "api.Observer",
        "package-schema-type:observer",
    );
    let interface = InterfaceValue::new(
        "interface-abi:observer".to_string(),
        InterfaceCarrier::Local {
            concrete_type: "local:Observer".to_string(),
            method_table: InterfaceMethodTable::new(
                "table:observer".to_string(),
                "interface-abi:observer".to_string(),
                vec![InterfaceMethodSlot::from_admitted_metadata(
                    0,
                    "invoke".to_string(),
                    "method-abi:invoke".to_string(),
                    InterfaceMethodSignature::new(
                        vec![
                            InterfaceMethodType::builtin("Self"),
                            InterfaceMethodType::builtin("unknown"),
                        ],
                        InterfaceMethodType::builtin("unknown"),
                    ),
                    InterfaceMethodTarget::LocalExecutable {
                        executable: ExecutableAddr::service(0, 1),
                        receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
                    },
                )],
            ),
            payload: RuntimeValue::Bool(true),
        },
    );
    for payload_type in [
        ContractTypeRef::package_schema(
            "skiff.run/types",
            "api.Payload",
            PackageSchemaTypeId::new("package-schema-type:payload"),
        ),
        ContractTypeRef::package_schema(
            "skiff.run/other-types",
            "api.Payload",
            PackageSchemaTypeId::new("package-schema-type:payload"),
        ),
        ContractTypeRef::package_schema(
            "skiff.run/types",
            "api.OtherPayload",
            PackageSchemaTypeId::new("package-schema-type:payload"),
        ),
        ContractTypeRef::package_schema(
            "skiff.run/types",
            "api.Payload",
            PackageSchemaTypeId::new("package-schema-type:other-payload"),
        ),
    ] {
        let operations = BTreeMap::from([(
            "invoke".to_string(),
            BoundaryCallbackOperation {
                parameters: vec![payload_type.clone()],
                return_type: payload_type,
            },
        )]);
        assert!(matches!(
            CallbackContractProjection::build(callback_type.clone(), &operations, &interface),
            Err(CallbackContractProjectionError::SignatureMismatch { .. })
        ));
    }
}
