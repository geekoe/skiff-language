use super::*;
use skiff_artifact_model::{
    ContractTypeDescriptor, ContractTypeNameability, NamedUnionBranchIr, NominalTypeRefBaseIr,
    PackageBuildId, PackageLocalAbiIdentity, PackageSchemaCanonicalDescriptor, PackageSchemaIndex,
    PackageSchemaIndexEntry, PackageSchemaTypeId, PackageSchemaTypeRecord, TypeDeclIr,
    TypeDeclarationIr, TypeDescriptorIr,
};

fn schema_type(package_id: &str, stable_key: &str, type_id: &str) -> PackageTypeRef {
    PackageTypeRef::PackageSchema {
        package_id: package_id.to_string(),
        stable_schema_key: stable_key.to_string(),
        package_schema_type_id: PackageSchemaTypeId::new(type_id),
    }
}

fn callback_type(package_id: &str, stable_key: &str, type_id: &str) -> PackageTypeRef {
    PackageTypeRef::AnyInterface {
        interface: Box::new(schema_type(package_id, stable_key, type_id)),
        arguments: Vec::new(),
    }
}

fn operation_contract(
    parameters: Vec<PackageTypeRef>,
    return_type: PackageTypeRef,
) -> BoundaryOperationContract {
    let signature = PackageCallableSignature {
        type_params: Vec::new(),
        parameters: parameters
            .into_iter()
            .enumerate()
            .map(
                |(index, ty)| skiff_artifact_model::PackageCallableParameter {
                    name: format!("p{index}"),
                    ty,
                },
            )
            .collect(),
        return_type,
        may_suspend: true,
    };
    let mut reasons = Vec::new();
    let contract =
        project_operation_contract("api", &signature, &[], &BTreeMap::new(), &[], &mut reasons)
            .expect("fixture must project");
    assert!(reasons.is_empty());
    skiff_artifact_model::validate_boundary_operation_contract(&contract)
        .expect("compiler projection must satisfy canonical boundary validation");
    contract
}

fn unavailable_reason(parameter_type: PackageTypeRef) -> Vec<BoundaryUnavailableReason> {
    let signature = PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![skiff_artifact_model::PackageCallableParameter {
            name: "value".to_string(),
            ty: parameter_type,
        }],
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("void"),
        },
        may_suspend: true,
    };
    let mut reasons = Vec::new();
    assert_eq!(
        project_operation_contract("api", &signature, &[], &BTreeMap::new(), &[], &mut reasons,),
        None
    );
    reasons
}

fn canonical_unavailable_reasons(parameter_type: PackageTypeRef) -> Vec<BoundaryUnavailableReason> {
    let mut reasons = unavailable_reason(parameter_type);
    super::super::eligibility::normalize_reasons(&mut reasons);
    reasons
}

#[test]
fn unary_callback_parameters_and_return_use_request_capabilities() {
    let reader = callback_type("example.interfaces", "api.Reader", "type:reader");
    let writer = callback_type("example.interfaces", "api.Writer", "type:writer");
    let contract = operation_contract(vec![writer.clone(), reader.clone(), writer.clone()], reader);

    for parameter in &contract.parameters {
        assert_eq!(
            parameter.value_plan,
            callback_plan(BoundaryValueLifetime::Request)
        );
    }
    assert_eq!(
        contract.return_value.value_plan,
        callback_plan(BoundaryValueLifetime::Request)
    );
    assert_eq!(
        contract.callbacks,
        BoundaryCallbackContract::RequestScoped {
            interface_types: vec![
                PackageSchemaTypeRef {
                    package_id: "example.interfaces".to_string(),
                    stable_schema_key: "api.Reader".to_string(),
                    package_schema_type_id: PackageSchemaTypeId::new("type:reader"),
                },
                PackageSchemaTypeRef {
                    package_id: "example.interfaces".to_string(),
                    stable_schema_key: "api.Writer".to_string(),
                    package_schema_type_id: PackageSchemaTypeId::new("type:writer"),
                },
            ],
            lifetime: BoundaryCallbackLifetime::TopLevelRequest,
            expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
        }
    );
}

#[test]
fn server_stream_callback_parameters_and_items_live_for_the_stream() {
    let reader = callback_type("example.interfaces", "api.Reader", "type:reader");
    let contract = operation_contract(
        vec![reader.clone()],
        PackageTypeRef::Container {
            name: "Stream".to_string(),
            arguments: vec![reader],
        },
    );

    assert_eq!(
        contract.parameters[0].value_plan,
        callback_plan(BoundaryValueLifetime::Stream)
    );
    assert_eq!(
        contract.return_value.value_plan,
        linkable_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Call)
    );
    let BoundaryStreamContract::ServerStream {
        item_value_plan, ..
    } = contract.stream
    else {
        panic!("expected server stream")
    };
    assert_eq!(
        item_value_plan,
        callback_plan(BoundaryValueLifetime::Stream)
    );
    assert!(matches!(
        contract.callbacks,
        BoundaryCallbackContract::RequestScoped {
            lifetime: BoundaryCallbackLifetime::Stream,
            expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
            ..
        }
    ));
}

#[test]
fn direct_package_schema_remains_detached_data() {
    let schema = schema_type("example.interfaces", "api.Reader", "type:reader");
    let contract = operation_contract(vec![schema.clone()], schema);

    assert_eq!(
        contract.parameters[0].value_plan,
        linkable_plan(BoundaryValueOwner::Caller, BoundaryValueLifetime::Call)
    );
    assert_eq!(
        contract.return_value.value_plan,
        linkable_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Call)
    );
    assert_eq!(contract.callbacks, BoundaryCallbackContract::None);
}

#[test]
fn unsupported_callback_shapes_are_structured_unavailable() {
    let exact = callback_type("example.interfaces", "api.Reader", "type:reader");
    let generic = PackageTypeRef::AnyInterface {
        interface: Box::new(schema_type(
            "example.interfaces",
            "api.Reader",
            "type:reader",
        )),
        arguments: vec![PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("string"),
        }],
    };
    let local_any = PackageTypeRef::Local {
        local_type: TypeRefIr::AnyInterface {
            interface: skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: "interface:reader".to_string(),
                canonical_type_args: Vec::new(),
            },
        },
    };
    let raw_function = PackageTypeRef::Local {
        local_type: TypeRefIr::Function {
            params: Vec::new(),
            return_type: Box::new(TypeRefIr::builtin("void")),
        },
    };

    for ty in [
        PackageTypeRef::Container {
            name: "Array".to_string(),
            arguments: vec![exact],
        },
        generic,
        local_any,
        raw_function,
    ] {
        assert_eq!(
            unavailable_reason(ty),
            vec![BoundaryUnavailableReason::CallbackAdapterUnavailable]
        );
    }
    assert_eq!(
        unavailable_reason(PackageTypeRef::Local {
            local_type: TypeRefIr::Builtin {
                name: "NativeHandle".to_string(),
                args: Vec::new(),
            },
        }),
        vec![BoundaryUnavailableReason::NativeAdapterUnavailable]
    );
}

#[test]
fn local_type_closure_saturates_nested_boundary_reasons() {
    let unsupported = || TypeRefIr::TypeParam {
        name: "T".to_string(),
    };
    let raw_callback = || TypeRefIr::Function {
        params: vec![skiff_artifact_model::FunctionTypeParamIr {
            name: "value".to_string(),
            ty: unsupported(),
        }],
        return_type: Box::new(TypeRefIr::builtin("void")),
    };
    let nested_any = || TypeRefIr::AnyInterface {
        interface: skiff_artifact_model::InterfaceInstantiationRef {
            interface_abi_id: "interface:generic".to_string(),
            canonical_type_args: vec![unsupported()],
        },
    };
    let expected = vec![
        BoundaryUnavailableReason::CallbackAdapterUnavailable,
        BoundaryUnavailableReason::NativeAdapterUnavailable,
        BoundaryUnavailableReason::UnsupportedBoundaryType,
    ];

    for local_type in [
        TypeRefIr::Union {
            items: vec![
                raw_callback(),
                TypeRefIr::Builtin {
                    name: "NativeHandle".to_string(),
                    args: vec![nested_any()],
                },
                raw_callback(),
            ],
        },
        TypeRefIr::Union {
            items: vec![
                TypeRefIr::Builtin {
                    name: "NativeHandle".to_string(),
                    args: vec![nested_any()],
                },
                raw_callback(),
            ],
        },
    ] {
        assert_eq!(
            canonical_unavailable_reasons(PackageTypeRef::Local { local_type }),
            expected
        );
    }
}

#[test]
fn generic_callback_closure_keeps_callback_and_unsupported_reasons() {
    let applied = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
        arguments: vec![TypeRefIr::Function {
            params: Vec::new(),
            return_type: Box::new(TypeRefIr::TypeParam {
                name: "T".to_string(),
            }),
        }],
    };

    assert_eq!(
        canonical_unavailable_reasons(PackageTypeRef::Local {
            local_type: applied,
        }),
        vec![
            BoundaryUnavailableReason::CallbackAdapterUnavailable,
            BoundaryUnavailableReason::UnsupportedBoundaryType,
        ]
    );
}

#[test]
fn concrete_suspension_summary_does_not_enter_operation_contract_shape() {
    let signature = PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![skiff_artifact_model::PackageCallableParameter {
            name: "input".to_string(),
            ty: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            },
        }],
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("string"),
        },
        may_suspend: false,
    };
    let mut suspending = signature.clone();
    suspending.may_suspend = true;
    let project = |signature| {
        project_operation_contract(
            "api",
            signature,
            &[],
            &BTreeMap::new(),
            &[],
            &mut Vec::new(),
        )
        .expect("builtin signature is boundary-projectable")
    };

    let non_suspending_contract = project(&signature);
    let suspending_contract = project(&suspending);
    assert_eq!(non_suspending_contract, suspending_contract);
    let wire = serde_json::to_value(non_suspending_contract).unwrap();
    assert!(wire.get("maySuspend").is_none());
    assert!(wire.get("cancellation").is_none());
}

#[test]
fn operation_value_plans_use_call_lifetime_except_for_server_stream_items() {
    let signature = |return_type| PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![skiff_artifact_model::PackageCallableParameter {
            name: "input".to_string(),
            ty: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            },
        }],
        return_type,
        may_suspend: false,
    };
    let project = |signature: &PackageCallableSignature| {
        project_operation_contract(
            "api",
            signature,
            &[],
            &BTreeMap::new(),
            &[],
            &mut Vec::new(),
        )
        .expect("builtin signature is boundary-projectable")
    };

    let unary = project(&signature(PackageTypeRef::Local {
        local_type: TypeRefIr::builtin("string"),
    }));
    assert_eq!(
        unary.parameters[0].value_plan,
        linkable_plan(BoundaryValueOwner::Caller, BoundaryValueLifetime::Call)
    );
    assert_eq!(
        unary.return_value.value_plan,
        linkable_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Call,)
    );
    assert_eq!(unary.stream, BoundaryStreamContract::Unary);

    for stream_return in [
        PackageTypeRef::Container {
            name: "Stream".to_string(),
            arguments: vec![PackageTypeRef::Container {
                name: "string".to_string(),
                arguments: Vec::new(),
            }],
        },
        PackageTypeRef::Local {
            local_type: TypeRefIr::Builtin {
                name: "Stream".to_string(),
                args: vec![TypeRefIr::builtin("string")],
            },
        },
    ] {
        let stream = project(&signature(stream_return));
        assert_eq!(
            stream.parameters[0].value_plan,
            linkable_plan(BoundaryValueOwner::Caller, BoundaryValueLifetime::Call)
        );
        assert_eq!(
            stream.return_value.value_plan,
            linkable_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Call,)
        );
        let BoundaryStreamContract::ServerStream {
            item_value_plan, ..
        } = stream.stream
        else {
            panic!("Stream<T> return must project as a server stream");
        };
        assert_eq!(
            item_value_plan,
            linkable_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Stream,)
        );
    }
}

#[test]
fn package_any_interface_projects_exact_contract_target_recursively() {
    let type_id = PackageSchemaTypeId::new("package-type:reader");
    let projected = project_package_type(
        "api",
        &PackageTypeRef::Nullable {
            inner: Box::new(PackageTypeRef::AnyInterface {
                interface: Box::new(PackageTypeRef::PackageSchema {
                    package_id: "example.interfaces".to_string(),
                    stable_schema_key: "Reader".to_string(),
                    package_schema_type_id: type_id.clone(),
                }),
                arguments: Vec::new(),
            }),
        },
        &[],
        &BTreeMap::new(),
        &[],
    )
    .expect("package existential should retain an exact contract representation");

    assert_eq!(
        projected,
        ContractTypeRef::Nullable {
            inner: Box::new(ContractTypeRef::AnyInterface {
                interface: Box::new(ContractTypeRef::PackageSchema {
                    package_id: "example.interfaces".to_string(),
                    stable_schema_key: "Reader".to_string(),
                    package_schema_type_id: type_id,
                }),
                arguments: Vec::new(),
            }),
        }
    );
}

fn public_literal_union_fixture() -> (
    Vec<skiff_artifact_model::FileIrUnit>,
    BTreeMap<(String, String), ContractTypeRef>,
    ContractTypeRef,
) {
    let mut unit = skiff_artifact_model::FileIrUnit::empty("api", "source-hash");
    unit.declarations.types.insert(
        "Result".to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: "Result".to_string(),
            source_span: None,
        },
    );
    unit.type_table.push(TypeDeclIr {
        name: "Result".to_string(),
        descriptor: TypeDescriptorIr::Union {
            branches: vec![
                NamedUnionBranchIr::Literal {
                    value: LiteralIr::String {
                        value: "complete".to_string(),
                    },
                },
                NamedUnionBranchIr::Literal {
                    value: LiteralIr::String {
                        value: "incomplete".to_string(),
                    },
                },
            ],
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    let projected = ContractTypeRef::package_schema(
        "example.llm",
        "ResponsesMaterializationResult",
        PackageSchemaTypeId::new("package-schema-type:result"),
    );
    (
        vec![unit],
        BTreeMap::from([(("api".to_string(), "Result".to_string()), projected.clone())]),
        projected,
    )
}

fn dependency_schema() -> ResolvedPackageSchema {
    let type_id = PackageSchemaTypeId::new("package-schema-type:http-request");
    let record = PackageSchemaTypeRecord {
        package_id: "skiff.run/std".to_string(),
        stable_schema_key: "std.http.HttpRequest".to_string(),
        package_schema_type_id: type_id.clone(),
        canonical_descriptor: PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "method".to_string(),
                    ContractTypeRef::builtin("string"),
                )]),
            },
        },
    };
    ResolvedPackageSchema::new(
        "std".to_string(),
        "skiff.run/std".to_string(),
        "1.0.0".to_string(),
        PackageBuildId::new("package-build:std"),
        PackageLocalAbiIdentity::new("package-local-abi:std"),
        PackageSchemaIndex {
            package_id: "skiff.run/std".to_string(),
            package_schema_index_identity: skiff_artifact_model::PackageSchemaIndexIdentity::new(
                "package-schema-index:std",
            ),
            types: BTreeMap::from([(
                "std.http.HttpRequest".to_string(),
                PackageSchemaIndexEntry {
                    package_schema_type_id: type_id.clone(),
                    public_path: Some("std.http.HttpRequest".to_string()),
                    nameability: ContractTypeNameability::PublicNameable,
                },
            )]),
        },
        BTreeMap::from([(type_id, record)]),
    )
    .unwrap()
}

fn package_symbol(package: PackageRefIr, symbol_path: &str) -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package,
            symbol_path: symbol_path.to_string(),
            abi_expectation: None,
        },
    }
}

#[test]
fn verified_public_package_symbol_projects_as_package_schema() {
    let schema = dependency_schema();
    let projected = project_local_type(
        "api",
        &package_symbol(
            PackageRefIr::Dependency {
                dependency_ref: "std".to_string(),
            },
            "std.http.HttpRequest",
        ),
        &[],
        &BTreeMap::new(),
        &[schema],
    )
    .unwrap();
    assert!(matches!(
        projected,
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } if package_id == "skiff.run/std"
            && stable_schema_key == "std.http.HttpRequest"
    ));
}

#[test]
fn applied_nominal_is_unavailable_at_service_boundary_without_losing_local_shape() {
    let applied = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
        arguments: vec![TypeRefIr::builtin("string")],
    };

    assert_eq!(
        project_local_type("api", &applied, &[], &BTreeMap::new(), &[]),
        Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
    );
    assert_eq!(
        applied,
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
            arguments: vec![TypeRefIr::builtin("string")],
        }
    );
}

#[test]
fn package_schema_applied_nominal_callable_is_structured_unavailable() {
    let applied = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
        arguments: vec![TypeRefIr::builtin("string")],
    };
    let signature = PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![skiff_artifact_model::PackageCallableParameter {
            name: "value".to_string(),
            ty: PackageTypeRef::Local {
                local_type: applied.clone(),
            },
        }],
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("string"),
        },
        may_suspend: true,
    };
    let mut reasons = Vec::new();

    assert_eq!(
        project_operation_contract("api", &signature, &[], &BTreeMap::new(), &[], &mut reasons,),
        None
    );
    assert_eq!(
        reasons,
        vec![BoundaryUnavailableReason::UnsupportedBoundaryType]
    );
    assert_eq!(
        signature.parameters[0].ty,
        PackageTypeRef::Local {
            local_type: applied
        }
    );
}

#[test]
fn package_schema_generic_return_stream_and_callback_are_unsupported() {
    let applied = || TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
        arguments: vec![TypeRefIr::builtin("string")],
    };
    let cases = [
        PackageCallableSignature {
            type_params: Vec::new(),
            parameters: Vec::new(),
            return_type: PackageTypeRef::Local {
                local_type: applied(),
            },
            may_suspend: false,
        },
        PackageCallableSignature {
            type_params: Vec::new(),
            parameters: Vec::new(),
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::Builtin {
                    name: "Stream".to_string(),
                    args: vec![applied()],
                },
            },
            may_suspend: true,
        },
        PackageCallableSignature {
            type_params: Vec::new(),
            parameters: vec![skiff_artifact_model::PackageCallableParameter {
                name: "callback".to_string(),
                ty: PackageTypeRef::Local {
                    local_type: TypeRefIr::Function {
                        params: vec![skiff_artifact_model::FunctionTypeParamIr {
                            name: "value".to_string(),
                            ty: applied(),
                        }],
                        return_type: Box::new(TypeRefIr::builtin("void")),
                    },
                },
            }],
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("void"),
            },
            may_suspend: false,
        },
    ];

    for (index, signature) in cases.into_iter().enumerate() {
        let mut reasons = Vec::new();
        assert_eq!(
            project_operation_contract("api", &signature, &[], &BTreeMap::new(), &[], &mut reasons,),
            None
        );
        super::super::eligibility::normalize_reasons(&mut reasons);
        assert_eq!(
            reasons,
            match index {
                1 => vec![
                    BoundaryUnavailableReason::UnsupportedBoundaryType,
                    BoundaryUnavailableReason::UnsupportedStream,
                ],
                2 => vec![
                    BoundaryUnavailableReason::CallbackAdapterUnavailable,
                    BoundaryUnavailableReason::UnsupportedBoundaryType,
                ],
                _ => vec![BoundaryUnavailableReason::UnsupportedBoundaryType],
            }
        );
    }
}

#[test]
fn package_schema_callback_transitively_referencing_generic_owner_is_unsupported() {
    let mut unit = skiff_artifact_model::FileIrUnit::empty("api", "source-hash");
    unit.declarations.types.insert(
        "Cell".to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: "Cell".to_string(),
            source_span: None,
        },
    );
    unit.declarations.types.insert(
        "Envelope".to_string(),
        TypeDeclarationIr {
            type_index: 1,
            symbol: "Envelope".to_string(),
            source_span: None,
        },
    );
    unit.type_table.push(TypeDeclIr {
        name: "Cell".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::from([(
                "value".to_string(),
                TypeRefIr::TypeParam {
                    name: "T".to_string(),
                },
            )]),
        },
        type_params: vec!["T".to_string()],
        implements: Vec::new(),
        source_span: None,
    });
    unit.type_table.push(TypeDeclIr {
        name: "Envelope".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::from([(
                "value".to_string(),
                TypeRefIr::AppliedNominal {
                    base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
                    arguments: vec![TypeRefIr::builtin("string")],
                },
            )]),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    let signature = PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![skiff_artifact_model::PackageCallableParameter {
            name: "callback".to_string(),
            ty: PackageTypeRef::Local {
                local_type: TypeRefIr::Function {
                    params: vec![skiff_artifact_model::FunctionTypeParamIr {
                        name: "value".to_string(),
                        ty: TypeRefIr::LocalType { type_index: 1 },
                    }],
                    return_type: Box::new(TypeRefIr::builtin("void")),
                },
            },
        }],
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("void"),
        },
        may_suspend: false,
    };
    let mut reasons = Vec::new();

    assert_eq!(
        project_operation_contract(
            "api",
            &signature,
            &[unit],
            &BTreeMap::new(),
            &[],
            &mut reasons,
        ),
        None
    );
    super::super::eligibility::normalize_reasons(&mut reasons);
    assert_eq!(
        reasons,
        vec![
            BoundaryUnavailableReason::CallbackAdapterUnavailable,
            BoundaryUnavailableReason::UnsupportedBoundaryType,
        ]
    );
}

#[test]
fn websocket_package_types_have_no_builtin_name_based_boundary_admission() {
    for name in [
        "std.websocket.WebSocketConnectRequest",
        "std.websocket.WebSocketConnectResult",
    ] {
        assert_eq!(
            project_local_type(
                "api",
                &TypeRefIr::Builtin {
                    name: name.to_string(),
                    args: vec![TypeRefIr::builtin("string")],
                },
                &[],
                &BTreeMap::new(),
                &[],
            ),
            Err(BoundaryUnavailableReason::NativeAdapterUnavailable),
            "{name}"
        );
    }
}

#[test]
fn package_symbol_projection_is_exact_and_fail_closed() {
    let schema = dependency_schema();
    for ty in [
        package_symbol(
            PackageRefIr::Dependency {
                dependency_ref: "missing".to_string(),
            },
            "std.http.HttpRequest",
        ),
        package_symbol(
            PackageRefIr::Dependency {
                dependency_ref: "std".to_string(),
            },
            "std.http.NotPublic",
        ),
    ] {
        assert_eq!(
            project_local_type("api", &ty, &[], &BTreeMap::new(), &[schema.clone()]),
            Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
        );
    }
    assert_eq!(
        project_local_type(
            "api",
            &TypeRefIr::builtin(skiff_artifact_model::http_boundary::HTTP_REQUEST_TYPE),
            &[],
            &BTreeMap::new(),
            &[],
        ),
        Err(BoundaryUnavailableReason::NativeAdapterUnavailable)
    );
    assert_eq!(
        project_local_type(
            "api",
            &package_symbol(
                PackageRefIr::PackageId {
                    package_id: "skiff.run/std".to_string(),
                },
                "std.http.HttpRequest",
            ),
            &[],
            &BTreeMap::new(),
            &[schema.clone(), schema],
        ),
        Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
    );
}

#[test]
fn published_local_nominal_projects_without_expanding_literal_union() {
    let (units, public_types, expected) = public_literal_union_fixture();
    let local = TypeRefIr::LocalType { type_index: 0 };
    assert_eq!(
        project_local_type("api", &local, &units, &public_types, &[]),
        Ok(expected.clone())
    );
    assert_eq!(
        project_local_type(
            "api",
            &TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![local.clone()],
            },
            &units,
            &public_types,
            &[],
        ),
        Ok(ContractTypeRef::Builtin {
            name: "Array".to_string(),
            arguments: vec![expected],
        })
    );
    assert_eq!(
        validate_local_type_closure("api", &local, &units, &public_types, &[]),
        Ok(())
    );
}

#[test]
fn unpublished_missing_and_ambiguous_local_nominals_fail_closed() {
    let (units, public_types, _) = public_literal_union_fixture();
    let local = TypeRefIr::LocalType { type_index: 0 };
    assert_eq!(
        project_local_type("api", &local, &units, &BTreeMap::new(), &[]),
        Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
    );
    assert_eq!(
        project_local_type("missing", &local, &units, &public_types, &[]),
        Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
    );
    assert_eq!(
        project_local_type(
            "api",
            &local,
            &[units[0].clone(), units[0].clone()],
            &public_types,
            &[],
        ),
        Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
    );

    let mut forged = units;
    forged[0].type_table[0].name = "Other".to_string();
    assert_eq!(
        project_local_type("api", &local, &forged, &public_types, &[]),
        Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
    );
}

#[test]
fn string_literals_project_exactly_while_callbacks_still_require_an_adapter() {
    let (units, public_types, _) = public_literal_union_fixture();
    assert_eq!(
        project_local_type(
            "api",
            &TypeRefIr::Literal {
                value: LiteralIr::String {
                    value: "complete".to_string(),
                },
            },
            &units,
            &public_types,
            &[],
        ),
        Ok(ContractTypeRef::string_literal("complete"))
    );
    assert_eq!(
        project_local_type(
            "api",
            &TypeRefIr::Function {
                params: vec![skiff_artifact_model::FunctionTypeParamIr {
                    name: "value".to_string(),
                    ty: TypeRefIr::LocalType { type_index: 0 },
                }],
                return_type: Box::new(TypeRefIr::LocalType { type_index: 0 }),
            },
            &units,
            &public_types,
            &[],
        ),
        Err(BoundaryUnavailableReason::CallbackAdapterUnavailable)
    );
}
