use super::*;
use skiff_artifact_model::{
    ExecutableBody, ExprRefIr, FileIrPackageCallValidationError, LiteralIr, NominalTypeRefBaseIr,
    TypeDeclIr, TypeDescriptorIr,
};

#[test]
fn file_ir_identity_omits_storage_identity_and_source_hashes() {
    let mut unit = FileIrUnit::empty("internal.example", "source-ast-hash-a");
    unit.file_ir_identity = "stale-file-ir-identity".to_string();
    unit.source_map.sources.push(SourceMapSource {
        id: 0,
        path: "internal/example.skiff".to_string(),
        module_path: "internal.example".to_string(),
        source_ast_hash: Some("source-map-ast-hash-a".to_string()),
    });

    let value = canonical_file_ir_identity_value(&unit).expect("identity value");

    assert!(value.get("fileIrIdentity").is_none());
    assert!(value.get("sourceAstHash").is_none());
    assert!(value
        .pointer("/sourceMap/sources/0/sourceAstHash")
        .is_none());
    assert_eq!(value["modulePath"], "internal.example");
    assert_eq!(
        value.pointer("/sourceMap/sources/0/path"),
        Some(&json!("internal/example.skiff"))
    );
}

#[test]
fn file_ir_identity_validation_rejects_stale_identity() {
    let mut unit = FileIrUnit::empty("internal.example", "source-ast-hash-a");
    unit.file_ir_identity = "stale-file-ir-identity".to_string();

    let error = validate_file_ir_identity(&unit).expect_err("stale identity must fail");

    assert!(matches!(
        error,
        ArtifactIdentityError::FileIrIdentityMismatch { .. }
    ));
    let computed = file_ir_identity(&unit).expect("computed identity");
    unit.file_ir_identity = computed;
    validate_file_ir_identity(&unit).expect("computed identity should validate");
}

#[test]
fn file_ir_identity_validation_rejects_non_current_generation_even_when_recomputed() {
    for (field, stale) in [
        ("schemaVersion", "skiff-file-ir-v7"),
        ("irFormatVersion", "skiff-file-ir-format-v5"),
        ("opcodeTableVersion", "skiff-opcode-table-v0"),
    ] {
        let mut unit = FileIrUnit::empty("internal.example", "source-ast-hash-a");
        match field {
            "schemaVersion" => unit.schema_version = stale.to_string(),
            "irFormatVersion" => unit.ir_format_version = stale.to_string(),
            "opcodeTableVersion" => unit.opcode_table_version = stale.to_string(),
            _ => unreachable!("closed mutation matrix"),
        }
        unit.file_ir_identity =
            file_ir_identity(&unit).expect("non-current preimage can still be framed");

        assert!(matches!(
            validate_file_ir_identity(&unit),
            Err(ArtifactIdentityError::FileIrGenerationMismatch {
                field: actual_field,
                ..
            }) if actual_field == field
        ));
        assert!(matches!(
            assign_file_ir_identity(&mut unit),
            Err(ArtifactIdentityError::FileIrGenerationMismatch {
                field: actual_field,
                ..
            }) if actual_field == field
        ));
    }
}

#[test]
fn file_ir_identity_validation_rejects_stale_prefix_with_current_preimage() {
    let mut unit = FileIrUnit::empty("internal.example", "source-ast-hash-a");
    let current = file_ir_identity(&unit).expect("current identity");
    unit.file_ir_identity = current.replacen(FILE_IR_IDENTITY_PREFIX, "skiff-file-ir-v8:sha256", 1);

    assert!(matches!(
        validate_file_ir_identity(&unit),
        Err(ArtifactIdentityError::FileIrIdentityMismatch { .. })
    ));
}

#[test]
fn applied_nominal_argument_identity_matrix_changes_file_ir_and_rejects_tampering() {
    let string_box = applied_nominal_file_ir(TypeRefIr::builtin("string"));
    let number_box = applied_nominal_file_ir(TypeRefIr::builtin("number"));
    assert_ne!(
        file_ir_identity(&string_box).unwrap(),
        file_ir_identity(&number_box).unwrap()
    );

    let string_number_pair = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index: 1 },
        arguments: vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("number")],
    };
    let number_string_pair = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index: 1 },
        arguments: vec![TypeRefIr::builtin("number"), TypeRefIr::builtin("string")],
    };
    let nested = applied_nominal_file_ir(string_number_pair);
    let reordered = applied_nominal_file_ir(number_string_pair);
    assert_ne!(
        file_ir_identity(&nested).unwrap(),
        file_ir_identity(&reordered).unwrap()
    );

    let mut assigned = file_ir_with_identity(string_box).unwrap();
    let TypeDescriptorIr::Record { fields } = &mut assigned.type_table[2].descriptor else {
        panic!("fixture use type must be a record")
    };
    let TypeRefIr::AppliedNominal { arguments, .. } = fields.get_mut("value").unwrap() else {
        panic!("fixture value must be applied")
    };
    arguments[0] = TypeRefIr::builtin("number");
    assert!(matches!(
        validate_file_ir_identity(&assigned),
        Err(ArtifactIdentityError::FileIrIdentityMismatch { .. })
    ));

    let mut owner_tampered =
        file_ir_with_identity(applied_nominal_file_ir(TypeRefIr::builtin("string"))).unwrap();
    let TypeDescriptorIr::Record { fields } = &mut owner_tampered.type_table[2].descriptor else {
        panic!("fixture use type must be a record")
    };
    let TypeRefIr::AppliedNominal { base, .. } = fields.get_mut("value").unwrap() else {
        panic!("fixture value must be applied")
    };
    *base = NominalTypeRefBaseIr::LocalType { type_index: 3 };
    assert!(matches!(
        validate_file_ir_identity(&owner_tampered),
        Err(ArtifactIdentityError::FileIrIdentityMismatch { .. })
    ));
}

fn applied_nominal_file_ir(argument: TypeRefIr) -> FileIrUnit {
    let mut unit = FileIrUnit::empty("identity.generic", "source");
    unit.type_table = vec![
        nominal_declaration("Box", &["T"]),
        nominal_declaration("Pair", &["A", "B"]),
        TypeDeclIr {
            name: "Use".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([(
                    "value".to_string(),
                    TypeRefIr::AppliedNominal {
                        base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
                        arguments: vec![argument],
                    },
                )]),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
        nominal_declaration("OtherBox", &["T"]),
    ];
    unit
}

fn nominal_declaration(name: &str, type_params: &[&str]) -> TypeDeclIr {
    TypeDeclIr {
        name: name.to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        type_params: type_params
            .iter()
            .map(|parameter| (*parameter).to_string())
            .collect(),
        implements: Vec::new(),
        source_span: None,
    }
}

#[test]
fn representation_wrap_owner_nested_argument_and_child_enter_file_ir_identity() {
    let baseline = representation_wrap_file_ir(0, "string", 0);
    let owner_changed = representation_wrap_file_ir(1, "string", 0);
    let nested_argument_changed = representation_wrap_file_ir(0, "number", 0);
    let child_changed = representation_wrap_file_ir(0, "string", 1);

    let identities = [
        file_ir_identity(&baseline).unwrap(),
        file_ir_identity(&owner_changed).unwrap(),
        file_ir_identity(&nested_argument_changed).unwrap(),
        file_ir_identity(&child_changed).unwrap(),
    ];
    for left in 0..identities.len() {
        for right in (left + 1)..identities.len() {
            assert_ne!(
                identities[left], identities[right],
                "every exact representation carrier input must enter identity"
            );
        }
    }

    let assigned = file_ir_with_identity(baseline).unwrap();
    let mut owner_tampered = assigned.clone();
    let ExprIr::RepresentationWrap { type_ref, .. } =
        &mut owner_tampered.constants[0].body.expressions[2]
    else {
        panic!("fixture expression must be representationWrap")
    };
    let TypeRefIr::AppliedNominal { base, .. } = type_ref else {
        panic!("fixture target must be applied")
    };
    *base = NominalTypeRefBaseIr::LocalType { type_index: 1 };

    let mut argument_tampered = assigned.clone();
    let ExprIr::RepresentationWrap { type_ref, .. } =
        &mut argument_tampered.constants[0].body.expressions[2]
    else {
        panic!("fixture expression must be representationWrap")
    };
    let TypeRefIr::AppliedNominal { arguments, .. } = type_ref else {
        panic!("fixture target must be applied")
    };
    let TypeRefIr::AppliedNominal {
        arguments: nested_arguments,
        ..
    } = &mut arguments[0]
    else {
        panic!("fixture argument must be nested applied nominal")
    };
    nested_arguments[0] = TypeRefIr::builtin("number");

    let mut child_tampered = assigned.clone();
    let ExprIr::RepresentationWrap { value, .. } =
        &mut child_tampered.constants[0].body.expressions[2]
    else {
        panic!("fixture expression must be representationWrap")
    };
    value.expression = 1;

    for tampered in [owner_tampered, argument_tampered, child_tampered] {
        assert!(matches!(
            validate_file_ir_identity(&tampered),
            Err(ArtifactIdentityError::FileIrIdentityMismatch { .. })
        ));
    }
}

fn representation_wrap_file_ir(
    owner_index: u32,
    nested_argument: &str,
    child_index: u32,
) -> FileIrUnit {
    let mut unit = FileIrUnit::empty("identity.representation", "source");
    unit.type_table = vec![
        representation_declaration("OuterA", "T"),
        representation_declaration("OuterB", "T"),
        representation_declaration("Inner", "U"),
    ];
    unit.constants.push(skiff_artifact_model::ConstIr {
        name: "wrapped".to_string(),
        ty: TypeRefIr::builtin("string"),
        body: ExecutableBody {
            expressions: vec![
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "first".to_string(),
                    },
                },
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "second".to_string(),
                    },
                },
                ExprIr::RepresentationWrap {
                    value: ExprRefIr {
                        expression: child_index,
                    },
                    type_ref: TypeRefIr::AppliedNominal {
                        base: NominalTypeRefBaseIr::LocalType {
                            type_index: owner_index,
                        },
                        arguments: vec![TypeRefIr::AppliedNominal {
                            base: NominalTypeRefBaseIr::LocalType { type_index: 2 },
                            arguments: vec![TypeRefIr::builtin(nested_argument)],
                        }],
                    },
                },
            ],
            ..ExecutableBody::default()
        },
        source_span: None,
    });
    unit
}

fn representation_declaration(name: &str, type_param: &str) -> TypeDeclIr {
    TypeDeclIr {
        name: name.to_string(),
        descriptor: TypeDescriptorIr::Representation {
            representation: TypeRefIr::TypeParam {
                name: type_param.to_string(),
            },
        },
        type_params: vec![type_param.to_string()],
        implements: Vec::new(),
        source_span: None,
    }
}

#[test]
fn encrypted_db_field_storage_participates_in_file_ir_identity() {
    let mut identity = FileIrUnit::empty("internal.example", "source-ast-hash-a");
    identity.declarations.db.insert(
        "Credential".to_string(),
        DbDeclarationIr {
            type_ref: TypeRefIr::builtin("Credential"),
            type_name: "Credential".to_string(),
            collection_name: "credential".to_string(),
            kind: DbObjectKindIr::Object,
            key: DbObjectKeyIr {
                name: "id".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
            fields: vec![DbObjectFieldIr {
                name: "apiKey".to_string(),
                ty: TypeRefIr::builtin("string"),
                storage: DbFieldStorageIr::Identity,
            }],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
    let identity_hash = file_ir_hash(&identity).expect("identity field storage hash");
    identity
        .declarations
        .db
        .get_mut("Credential")
        .unwrap()
        .fields[0]
        .storage = DbFieldStorageIr::Encrypted;
    let encrypted_hash = file_ir_hash(&identity).expect("encrypted field storage hash");

    assert_ne!(identity_hash, encrypted_hash);
}

#[test]
fn actor_declaration_abi_participates_in_file_ir_identity() {
    let mut unit = FileIrUnit::empty("internal.example", "source-ast-hash-a");
    let abi = ActorAbiInput {
        actor_name: "DocHub".to_string(),
        actor_id_type: TypeRefIr::builtin("string"),
        fields: vec![ActorFieldIr {
            name: "nextSeq".to_string(),
            ty: TypeRefIr::builtin("number"),
            encoding: ActorFieldEncodingIr::CanonicalValueV1,
        }],
        public_methods: Vec::new(),
        actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
    };
    unit.actor_declarations.push(ActorDeclarationIr {
        actor_abi_identity: actor_abi_identity(&abi).expect("actor ABI identity"),
        actor_implementation_identity: skiff_artifact_model::ActorImplementationIdentity::new(
            "skiff-actor-implementation-v1:sha256:placeholder",
        ),
        abi,
        method_implementations: Default::default(),
    });
    let baseline = file_ir_identity(&unit).expect("actor File IR identity");

    unit.actor_declarations[0].abi.actor_id_type = TypeRefIr::builtin("integer");
    unit.actor_declarations[0].actor_abi_identity =
        actor_abi_identity(&unit.actor_declarations[0].abi).expect("changed actor ABI identity");
    let changed_id = file_ir_identity(&unit).expect("changed actor id File IR identity");
    assert_ne!(baseline, changed_id);

    unit.actor_declarations[0].abi.fields[0].ty = TypeRefIr::builtin("integer");
    unit.actor_declarations[0].actor_abi_identity =
        actor_abi_identity(&unit.actor_declarations[0].abi).expect("changed actor ABI identity");
    let changed_field = file_ir_identity(&unit).expect("changed actor field File IR identity");
    assert_ne!(changed_id, changed_field);
}

#[test]
fn service_call_table_and_instruction_indices_participate_in_file_ir_identity() {
    let base = service_call_file_ir_fixture();
    let baseline = file_ir_identity(&base).expect("valid service-call File IR identity");
    assert_eq!(
        baseline,
        "skiff-file-ir-v9:sha256:20e92b3da085320be0c3d14b38e33fe99a32cba0f4526c1bba3a8d07004df246"
    );

    let mut changed_ref = base.clone();
    changed_ref.external_refs.service_call_refs[0].contract_operation_id =
        ContractOperationId::new("operation:echo-v2");
    assert_ne!(
        file_ir_identity(&changed_ref).unwrap(),
        file_ir_identity(&base).unwrap()
    );

    let mut changed_indices = base.clone();
    let expressions = &mut changed_indices.constants[0].body.expressions;
    for (expression, index) in expressions.iter_mut().zip([1, 0]) {
        let ExprIr::Call { call } = expression else {
            panic!("fixture call expression")
        };
        call.target = CallTargetIr::ServiceCall {
            service_call_ref_index: ServiceCallRefIndex::new(index),
        };
    }
    assert_ne!(file_ir_identity(&changed_indices).unwrap(), baseline);
}

#[test]
fn package_call_target_and_ref_fields_participate_in_file_ir_identity() {
    let base = package_call_file_ir_fixture();
    let baseline = file_ir_identity(&base).expect("valid package-call File IR identity");

    let mut changed_package_ref = base.clone();
    let replacement_package_ref = PackageRefIr::Dependency {
        dependency_ref: "tools-v2".to_string(),
    };
    let ExprIr::Call { call } = &mut changed_package_ref.constants[0].body.expressions[0] else {
        panic!("fixture call expression")
    };
    let CallTargetIr::PackageCallable { package_ref, .. } = &mut call.target else {
        panic!("fixture package-call target")
    };
    *package_ref = replacement_package_ref.clone();
    changed_package_ref.external_refs.package_callables[0].package_ref = replacement_package_ref;
    assert_ne!(file_ir_identity(&changed_package_ref).unwrap(), baseline);

    let mut changed_callable = base.clone();
    let replacement_callable_id = PackageCallableId::new("callable:other.echo");
    let ExprIr::Call { call } = &mut changed_callable.constants[0].body.expressions[0] else {
        panic!("fixture call expression")
    };
    let CallTargetIr::PackageCallable {
        package_callable_id,
        ..
    } = &mut call.target
    else {
        panic!("fixture package-call target")
    };
    *package_callable_id = replacement_callable_id.clone();
    changed_callable.external_refs.package_callables[0].package_callable_id =
        replacement_callable_id;
    assert_ne!(
        file_ir_identity(&changed_callable).unwrap(),
        baseline,
        "owner-qualified callable identities must not collapse on the shared display suffix"
    );
}

#[test]
fn file_ir_identity_reuses_canonical_package_call_validation() {
    let mut missing = package_call_file_ir_fixture();
    missing.external_refs.package_callables.clear();
    assert!(matches!(
        package_call_identity_validation_error(&missing),
        FileIrPackageCallValidationError::MissingRef { .. }
    ));

    let mut orphan = package_call_file_ir_fixture();
    orphan.constants[0].body.expressions.clear();
    assert!(matches!(
        package_call_identity_validation_error(&orphan),
        FileIrPackageCallValidationError::OrphanRef { .. }
    ));

    let mut mismatch = package_call_file_ir_fixture();
    let ExprIr::Call { call } = &mut mismatch.constants[0].body.expressions[0] else {
        panic!("fixture call expression")
    };
    let CallTargetIr::PackageCallable { package_ref, .. } = &mut call.target else {
        panic!("fixture package-call target")
    };
    *package_ref = PackageRefIr::PackageId {
        package_id: "example.com/tools".to_string(),
    };
    assert!(matches!(
        package_call_identity_validation_error(&mismatch),
        FileIrPackageCallValidationError::FieldMismatch { .. }
    ));

    let mut duplicate = package_call_file_ir_fixture();
    duplicate
        .external_refs
        .package_callables
        .push(duplicate.external_refs.package_callables[0].clone());
    assert!(matches!(
        package_call_identity_validation_error(&duplicate),
        FileIrPackageCallValidationError::DuplicateRef { .. }
    ));
}

#[test]
fn file_ir_identity_reuses_canonical_service_call_validation() {
    let mut orphan = service_call_file_ir_fixture();
    orphan.constants[0].body.expressions.pop();

    assert!(matches!(
        file_ir_identity(&orphan),
        Err(ArtifactIdentityError::InvalidFileIrServiceCalls(
            skiff_artifact_model::FileIrServiceCallValidationError::OrphanRef { .. }
        ))
    ));
}

fn service_call_file_ir_fixture() -> FileIrUnit {
    let mut unit = FileIrUnit::empty("consumer.main", "source-ast-hash");
    unit.external_refs.service_call_refs = vec![
        ServiceCallRef {
            service_requirement_slot: 0,
            contract_operation_id: ContractOperationId::new("operation:echo"),
            expected_protocol_identity: ServiceProtocolIdentity::new("protocol:echo"),
        },
        ServiceCallRef {
            service_requirement_slot: 0,
            contract_operation_id: ContractOperationId::new("operation:health"),
            expected_protocol_identity: ServiceProtocolIdentity::new("protocol:echo"),
        },
    ];
    unit.constants.push(skiff_artifact_model::ConstIr {
        name: "calls".to_string(),
        ty: TypeRefIr::builtin("void"),
        body: skiff_artifact_model::ExecutableBody {
            blocks: Vec::new(),
            statements: Vec::new(),
            expressions: [0, 1]
                .into_iter()
                .map(|index| ExprIr::Call {
                    call: CallIr {
                        target: CallTargetIr::ServiceCall {
                            service_call_ref_index: ServiceCallRefIndex::new(index),
                        },
                        site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                            reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                        },
                        args: Vec::new(),
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                })
                .collect(),
        },
        source_span: None,
    });
    unit
}

fn package_call_file_ir_fixture() -> FileIrUnit {
    let package_ref = PackageRefIr::Dependency {
        dependency_ref: "tools".to_string(),
    };
    let package_callable_id = PackageCallableId::new("callable:tools.echo");
    let mut unit = FileIrUnit::empty("consumer.main", "source-ast-hash");
    unit.external_refs.package_callables = vec![PackageCallableRef {
        package_ref: package_ref.clone(),
        package_callable_id: package_callable_id.clone(),
    }];
    unit.constants.push(skiff_artifact_model::ConstIr {
        name: "package-call".to_string(),
        ty: TypeRefIr::builtin("void"),
        body: skiff_artifact_model::ExecutableBody {
            blocks: Vec::new(),
            statements: Vec::new(),
            expressions: vec![ExprIr::Call {
                call: CallIr {
                    target: CallTargetIr::PackageCallable {
                        package_ref,
                        package_callable_id,
                    },
                    site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                        reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                    },
                    args: Vec::new(),
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                },
            }],
        },
        source_span: None,
    });
    unit
}

fn package_call_identity_validation_error(unit: &FileIrUnit) -> FileIrPackageCallValidationError {
    match file_ir_identity(unit) {
        Err(ArtifactIdentityError::InvalidFileIrPackageCalls(error)) => error,
        result => panic!("expected canonical package-call validation error, got {result:?}"),
    }
}
