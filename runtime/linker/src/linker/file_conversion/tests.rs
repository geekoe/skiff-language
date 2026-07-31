use super::*;

fn db_target_id() -> DbObjectTargetId {
    DbObjectTargetId {
        package_artifact_ref: artifact::PackageArtifactRef {
            package_id: "example.models".to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: artifact::PackageBuildId::new("build:models"),
            package_local_abi_identity: artifact::PackageLocalAbiIdentity::new("abi:models"),
        },
        file_ir_ref: artifact::FileIrRef {
            file_ir_identity: "file:models".to_string(),
            module_path: "models".to_string(),
            artifact_path: None,
            source_ast_hash: Some("source:models".to_string()),
        },
        type_index: 7,
    }
}

fn artifact_db_target() -> artifact::DbTargetIr {
    artifact::DbTargetIr {
        type_ref: artifact::TypeRefIr::PackageSymbol {
            symbol: artifact::PackageSymbolRef {
                package: artifact::PackageRefIr::Dependency {
                    dependency_ref: "models".to_string(),
                },
                symbol_path: "models.User".to_string(),
                abi_expectation: Some("abi:models".to_string()),
            },
        },
        type_name: "User".to_string(),
    }
}

fn source_site(seed: u32) -> artifact::InstructionSourceSite {
    artifact::InstructionSourceSite::Source {
        span: artifact::SourceSpanRef {
            source_id: u64::from(seed) + 100,
            start: artifact::SourcePosition {
                line: seed,
                column: seed + 1,
                offset: Some(seed + 2),
            },
            end: artifact::SourcePosition {
                line: seed + 3,
                column: seed + 4,
                offset: Some(seed + 5),
            },
        },
    }
}

#[test]
fn linked_db_target_carriers_preserve_one_exact_runtime_identity() {
    let target_id = db_target_id();
    let resolve = |target: &artifact::DbTargetIr| {
        assert_eq!(target, &artifact_db_target());
        Ok(target_id.clone())
    };
    let operation = linked_db_operation(
        &artifact::DbOperationIr {
            op: artifact::DbOpKindIr::Count,
            many: false,
            target: artifact_db_target(),
            selector: None,
            query: None,
            projection: None,
            body: None,
            insert_body: None,
            change: None,
            result_type: artifact::TypeRefIr::builtin("number"),
            source_span: None,
        },
        &resolve,
    )
    .unwrap();
    let query_target = linked_db_target(&artifact_db_target(), &resolve).unwrap();
    let claim = linked_db_lease_claim(
        &artifact::DbLeaseClaimIr {
            target: artifact_db_target(),
            key: artifact::ExprRefIr { expression: 0 },
            slot: "lease".to_string(),
            binding_slot: Some(0),
            body: "claimBody".to_string(),
            result_type: artifact::TypeRefIr::builtin("bool"),
            source_span: None,
        },
        &resolve,
    )
    .unwrap();
    let read = linked_db_lease_read(
        &artifact::DbLeaseReadIr {
            target: artifact_db_target(),
            key: artifact::ExprRefIr { expression: 0 },
            slot: "lease".to_string(),
            result_type: artifact::TypeRefIr::builtin("bool"),
            source_span: None,
        },
        &resolve,
    )
    .unwrap();

    assert_eq!(operation.target.target_id, target_id);
    assert_eq!(query_target.target_id, target_id);
    assert_eq!(claim.target.target_id, target_id);
    assert_eq!(read.target.target_id, target_id);
}

fn artifact_call(
    target: artifact::CallTargetIr,
    site: artifact::InstructionSourceSite,
) -> artifact::CallIr {
    artifact::CallIr {
        target,
        site,
        args: Vec::new(),
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}

#[test]
fn linked_throw_statement_and_expression_preserve_exact_source_sites() {
    let statement_site = source_site(11);
    let statement = linked_stmt(
        &artifact::StmtIr::Throw {
            value: artifact::ExprRefIr { expression: 2 },
            payload_type: artifact::TypeRefIr::builtin("string"),
            site: statement_site.clone(),
        },
        &|_| unreachable!(),
    )
    .unwrap();
    assert!(matches!(
        statement,
        LinkedStmtIr::Throw { site, .. } if site == statement_site
    ));

    let expression_site = source_site(29);
    let expression = linked_expr(
        &artifact::ExprIr::Throw {
            value: artifact::ExprRefIr { expression: 5 },
            payload_type: artifact::TypeRefIr::builtin("number"),
            site: expression_site.clone(),
        },
        &|_| unreachable!(),
        &|_| unreachable!(),
    )
    .unwrap();
    assert!(matches!(
        expression,
        LinkedExprIr::Throw { site, .. } if site == expression_site
    ));
}

#[test]
fn linked_while_statement_preserves_condition_ref_and_body_label() {
    let statement = linked_stmt(
        &artifact::StmtIr::While {
            condition: artifact::ExprRefIr { expression: 4 },
            body: "while_body".to_string(),
        },
        &|_| unreachable!(),
    )
    .unwrap();
    assert!(matches!(
        statement,
        LinkedStmtIr::While { condition, body } if condition.expression == 4 && body == "while_body"
    ));
}

#[test]
fn linked_throw_preserves_exact_synthetic_site() {
    let expected = artifact::InstructionSourceSite::Synthetic {
        reason: artifact::SyntheticInstructionSiteReason::RuntimeControlFlow,
    };
    let linked = linked_expr(
        &artifact::ExprIr::Throw {
            value: artifact::ExprRefIr { expression: 0 },
            payload_type: artifact::TypeRefIr::builtin("unknown"),
            site: expected.clone(),
        },
        &|_| unreachable!(),
        &|_| unreachable!(),
    )
    .unwrap();
    assert!(matches!(
        linked,
        LinkedExprIr::Throw { site, .. } if site == expected
    ));
}

#[test]
fn linked_local_package_service_and_native_calls_preserve_exact_sites() {
    let local_site = source_site(41);
    let local = linked_call(
        &artifact_call(
            artifact::CallTargetIr::LocalExecutable {
                executable_index: 7,
            },
            local_site.clone(),
        ),
        &|_| unreachable!(),
    )
    .unwrap();
    assert_eq!(local.site, local_site);

    let package_site = source_site(51);
    let package = linked_call(
        &artifact_call(
            artifact::CallTargetIr::PackageCallable {
                package_ref: artifact::PackageRefIr::Dependency {
                    dependency_ref: "models".to_string(),
                },
                package_callable_id: artifact::PackageCallableId::new("callable:models.lookup"),
            },
            package_site.clone(),
        ),
        &|target| match target {
            artifact::CallTargetIr::PackageCallable { .. } => Ok(LinkedCallTarget::Builtin {
                op: "resolved-package".to_string(),
            }),
            _ => unreachable!(),
        },
    )
    .unwrap();
    assert_eq!(package.site, package_site);

    let service_site = source_site(61);
    let service = linked_call(
        &artifact_call(
            artifact::CallTargetIr::ServiceCall {
                service_call_ref_index: artifact::ServiceCallRefIndex::new(3),
            },
            service_site.clone(),
        ),
        &|target| match target {
            artifact::CallTargetIr::ServiceCall { .. } => Ok(LinkedCallTarget::Builtin {
                op: "resolved-service".to_string(),
            }),
            _ => unreachable!(),
        },
    )
    .unwrap();
    assert_eq!(service.site, service_site);

    let native_site = source_site(71);
    let native = linked_call(
        &artifact_call(
            artifact::CallTargetIr::Native {
                target: artifact::NativeTarget {
                    namespace: "std.http".to_string(),
                    symbol: "fetch".to_string(),
                    binding_key: Some("std.http.fetch".to_string()),
                    metadata: BTreeMap::new(),
                },
            },
            native_site.clone(),
        ),
        &|_| unreachable!(),
    )
    .unwrap();
    assert_eq!(native.site, native_site);
}

#[test]
fn linked_required_catch_type_preserves_applied_nominal() {
    let linked = linked_expr(
        &artifact::ExprIr::Catch {
            try_expression: artifact::ExprRefIr { expression: 0 },
            catch_slot: 1,
            catch_type: artifact::TypeRefIr::AppliedNominal {
                base: artifact::NominalTypeRefBaseIr::LocalType { type_index: 2 },
                arguments: vec![artifact::TypeRefIr::builtin("string")],
            },
            body: artifact::ExprRefIr { expression: 3 },
        },
        &|_| unreachable!(),
        &|_| unreachable!(),
    )
    .unwrap();
    assert!(matches!(
        linked,
        LinkedExprIr::Catch {
            catch_type: LinkedTypeRef::AppliedNominal {
                base: LinkedNominalTypeRefBase::LocalType { type_index: 2 },
                arguments,
            },
            ..
        } if arguments == vec![LinkedTypeRef::Native {
            name: "string".to_string(),
            args: Vec::new(),
        }]
    ));
}

#[test]
fn linked_representation_wrap_preserves_child_and_plain_target() {
    let linked = linked_expr(
        &artifact::ExprIr::RepresentationWrap {
            value: artifact::ExprRefIr { expression: 11 },
            type_ref: artifact::TypeRefIr::LocalType { type_index: 3 },
        },
        &|_| unreachable!(),
        &|_| unreachable!(),
    )
    .unwrap();

    assert_eq!(
        linked,
        LinkedExprIr::RepresentationWrap {
            value: ExprRefIr { expression: 11 },
            type_ref: LinkedTypeRef::LocalType { type_index: 3 },
        }
    );
}

#[test]
fn linked_representation_wrap_preserves_external_owner_and_nested_arguments() {
    let linked = linked_expr(
        &artifact::ExprIr::RepresentationWrap {
            value: artifact::ExprRefIr { expression: 19 },
            type_ref: artifact::TypeRefIr::AppliedNominal {
                base: artifact::NominalTypeRefBaseIr::PackageSymbol {
                    symbol: artifact::PackageSymbolRef {
                        package: artifact::PackageRefIr::Dependency {
                            dependency_ref: "models".to_string(),
                        },
                        symbol_path: "api.OuterRepresentation".to_string(),
                        abi_expectation: Some("local-abi:models".to_string()),
                    },
                },
                arguments: vec![artifact::TypeRefIr::AppliedNominal {
                    base: artifact::NominalTypeRefBaseIr::LocalType { type_index: 5 },
                    arguments: vec![artifact::TypeRefIr::builtin("string")],
                }],
            },
        },
        &|_| unreachable!(),
        &|_| unreachable!(),
    )
    .unwrap();

    assert_eq!(
        linked,
        LinkedExprIr::RepresentationWrap {
            value: ExprRefIr { expression: 19 },
            type_ref: LinkedTypeRef::AppliedNominal {
                base: LinkedNominalTypeRefBase::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::Dependency {
                            dependency_ref: "models".to_string(),
                        },
                        symbol_path: "api.OuterRepresentation".to_string(),
                        abi_expectation: Some("local-abi:models".to_string()),
                    },
                },
                arguments: vec![LinkedTypeRef::AppliedNominal {
                    base: LinkedNominalTypeRefBase::LocalType { type_index: 5 },
                    arguments: vec![LinkedTypeRef::Native {
                        name: "string".to_string(),
                        args: Vec::new(),
                    }],
                }],
            },
        }
    );
}

fn generic_type_file() -> artifact::FileIrUnit {
    let mut file = artifact::FileIrUnit::empty("models", "source");
    file.type_table.push(artifact::TypeDeclIr {
        name: "Box".to_string(),
        descriptor: artifact::TypeDescriptorIr::Record {
            fields: BTreeMap::from([(
                "value".to_string(),
                artifact::TypeRefIr::TypeParam {
                    name: "T".to_string(),
                },
            )]),
        },
        type_params: vec!["T".to_string()],
        implements: Vec::new(),
        source_span: None,
    });
    file.type_table.push(artifact::TypeDeclIr {
        name: "Holder".to_string(),
        descriptor: artifact::TypeDescriptorIr::Record {
            fields: BTreeMap::from([(
                "boxed".to_string(),
                artifact::TypeRefIr::AppliedNominal {
                    base: artifact::NominalTypeRefBaseIr::LocalType { type_index: 0 },
                    arguments: vec![artifact::TypeRefIr::builtin("string")],
                },
            )]),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file
}

#[test]
fn linked_file_conversion_preserves_applied_nominal_wrapper_and_arguments() {
    let linked = linked_file_unit_from_assembly_artifact(
        &generic_type_file(),
        &|_| unreachable!(),
        &|_| unreachable!(),
    )
    .unwrap();
    let LinkedTypeDescriptor::Record { fields } = &linked.types[1].descriptor else {
        panic!("holder must remain a record")
    };
    assert!(matches!(
        &fields["boxed"],
        LinkedTypeRef::AppliedNominal {
            base: LinkedNominalTypeRefBase::LocalType { type_index: 0 },
            arguments
        } if arguments == &vec![LinkedTypeRef::Native {
            name: "string".to_string(),
            args: Vec::new(),
        }]
    ));
}

#[test]
fn linked_file_conversion_rejects_applied_nominal_wrong_arity_before_linking() {
    let mut file = generic_type_file();
    let artifact::TypeDescriptorIr::Record { fields } = &mut file.type_table[1].descriptor else {
        unreachable!()
    };
    let artifact::TypeRefIr::AppliedNominal { arguments, .. } = fields.get_mut("boxed").unwrap()
    else {
        unreachable!()
    };
    arguments.push(artifact::TypeRefIr::builtin("number"));

    let error =
        linked_file_unit_from_assembly_artifact(&file, &|_| unreachable!(), &|_| unreachable!())
            .unwrap_err();
    assert!(error.to_string().contains("has arity 2, expected 1"));
}

#[test]
fn linked_file_conversion_preserves_encrypted_db_field_storage() {
    let mut artifact = artifact::FileIrUnit::empty("internal.credential", "source");
    artifact.declarations.db.insert(
        "Credential".to_string(),
        artifact::DbDeclarationIr {
            type_ref: artifact::TypeRefIr::builtin("Credential"),
            type_name: "Credential".to_string(),
            collection_name: "credential".to_string(),
            kind: artifact::DbObjectKindIr::Object,
            key: artifact::DbObjectKeyIr {
                name: "id".to_string(),
                ty: artifact::TypeRefIr::builtin("string"),
            },
            fields: vec![artifact::DbObjectFieldIr {
                name: "apiKey".to_string(),
                ty: artifact::TypeRefIr::builtin("string"),
                storage: artifact::DbFieldStorageIr::Encrypted,
            }],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );

    let linked = linked_file_unit_from_assembly_artifact(
        &artifact,
        &|target| anyhow::bail!("unexpected canonical call target {target:?}"),
        &|target| anyhow::bail!("unexpected canonical DB target {target:?}"),
    )
    .unwrap();
    assert_eq!(
        linked.declarations.db["Credential"].fields[0].storage,
        DbFieldStorageIr::Encrypted
    );
}

fn actor_file() -> artifact::FileIrUnit {
    let mut file = artifact::FileIrUnit::empty("actors", "source");
    let abi = artifact::ActorAbiInput {
        actor_name: "DocHub".to_string(),
        actor_id_type: artifact::TypeRefIr::builtin("string"),
        key_field: "id".to_string(),
        fields: vec![
            artifact::ActorFieldIr {
                name: "id".to_string(),
                ty: artifact::TypeRefIr::builtin("string"),
                encoding: artifact::ActorFieldEncodingIr::CanonicalValueV1,
            },
            artifact::ActorFieldIr {
                name: "nextSeq".to_string(),
                ty: artifact::TypeRefIr::builtin("number"),
                encoding: artifact::ActorFieldEncodingIr::CanonicalValueV1,
            },
        ],
        create: None,
        public_methods: Vec::new(),
        actor_runtime_abi_version: artifact::ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
    };
    file.actor_declarations.push(artifact::ActorDeclarationIr {
        actor_abi_identity: skiff_artifact_identity::actor_abi_identity(&abi).unwrap(),
        actor_implementation_identity: artifact::ActorImplementationIdentity::new(
            "actor-impl:test",
        ),
        abi,
        method_implementations: std::collections::BTreeMap::new(),
        create_implementation: None,
    });
    file
}

fn actor_file_with_method() -> artifact::FileIrUnit {
    let mut file = actor_file();
    let method_identity = artifact::ActorMethodIdentity::new("actor-method:read");
    file.executables.push(artifact::ExecutableIr {
        kind: artifact::ExecutableKind::ImplMethod,
        symbol: "actors.DocHub.read".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: artifact::TypeRefIr::builtin("number"),
        self_type: Some(artifact::TypeRefIr::builtin("number")),
        slots: artifact::SlotLayout::default(),
        may_suspend: false,
        body: artifact::ExecutableBody {
            expressions: vec![artifact::ExprIr::ActorSelfField {
                field: "nextSeq".to_string(),
                field_type: artifact::TypeRefIr::builtin("number"),
            }],
            ..artifact::ExecutableBody::default()
        },
        source_span: None,
    });
    file.actor_declarations[0]
        .abi
        .public_methods
        .push(artifact::ActorPublicMethodIr {
            method_identity: method_identity.clone(),
            name: "read".to_string(),
            parameters: Vec::new(),
            return_type: artifact::TypeRefIr::builtin("number"),
            may_suspend: false,
        });
    file.actor_declarations[0]
        .method_implementations
        .insert(method_identity, 0);
    file.actor_declarations[0].actor_abi_identity =
        skiff_artifact_identity::actor_abi_identity(&file.actor_declarations[0].abi).unwrap();
    file
}

#[test]
fn linked_file_conversion_preserves_actor_declaration_owner_and_encoding() {
    let artifact = actor_file();
    let linked = linked_file_unit_from_assembly_artifact(
        &artifact,
        &|target| anyhow::bail!("unexpected canonical call target {target:?}"),
        &|target| anyhow::bail!("unexpected canonical DB target {target:?}"),
    )
    .unwrap();
    let actor = &linked.actor_declarations[0];
    assert_eq!(actor.actor_type.module_path, "actors");
    assert_eq!(actor.actor_type.symbol, "DocHub");
    assert!(actor.implementation_owner.is_none());
    assert_eq!(actor.actor_name, "DocHub");
    assert_eq!(
        actor.fields[0].encoding,
        artifact::ActorFieldEncodingIr::CanonicalValueV1
    );
}

#[test]
fn linked_file_conversion_preserves_validated_actor_self_field() {
    let file = actor_file_with_method();
    let linked =
        linked_file_unit_from_assembly_artifact(&file, &|_| unreachable!(), &|_| unreachable!())
            .unwrap();
    assert!(matches!(
        &linked.executables[0].body.expressions[0],
        LinkedExprIr::ActorSelfField { field, field_type }
            if field == "nextSeq"
                && field_type == &linked_type_ref(&artifact::TypeRefIr::builtin("number"))
    ));
}

#[test]
fn linked_file_conversion_rejects_actor_self_field_outside_actor_method() {
    let mut file = actor_file();
    file.constants.push(artifact::ConstIr {
        name: "forged".to_string(),
        ty: artifact::TypeRefIr::builtin("number"),
        body: artifact::ExecutableBody {
            expressions: vec![artifact::ExprIr::ActorSelfField {
                field: "nextSeq".to_string(),
                field_type: artifact::TypeRefIr::builtin("number"),
            }],
            ..artifact::ExecutableBody::default()
        },
        source_span: None,
    });
    assert!(linked_file_unit_from_assembly_artifact(
        &file,
        &|_| unreachable!(),
        &|_| unreachable!(),
    )
    .unwrap_err()
    .to_string()
    .contains("outside an Actor method"));
}

#[test]
fn linked_file_conversion_rejects_actor_self_field_type_forgery() {
    let mut file = actor_file_with_method();
    file.executables[0].body.expressions[0] = artifact::ExprIr::ActorSelfField {
        field: "nextSeq".to_string(),
        field_type: artifact::TypeRefIr::builtin("string"),
    };
    assert!(linked_file_unit_from_assembly_artifact(
        &file,
        &|_| unreachable!(),
        &|_| unreachable!(),
    )
    .unwrap_err()
    .to_string()
    .contains("type does not match"));
}

#[test]
fn linked_file_conversion_rejects_duplicate_actor_owner() {
    let mut duplicate = actor_file();
    duplicate
        .actor_declarations
        .push(duplicate.actor_declarations[0].clone());
    assert!(linked_file_unit_from_assembly_artifact(
        &duplicate,
        &|_| unreachable!(),
        &|_| unreachable!(),
    )
    .unwrap_err()
    .to_string()
    .contains("duplicate actor declaration"));
}

#[test]
fn linked_call_preserves_actor_dispatch_identities_without_executable_address() {
    let call = artifact::CallIr {
        target: artifact::CallTargetIr::ActorMethod {
            actor: artifact::ServiceSymbolRef {
                module_path: "actors".to_string(),
                symbol: "DocHub".to_string(),
            },
            actor_abi_identity: artifact::ActorAbiIdentity::new("actor-abi:test"),
            actor_implementation_identity: artifact::ActorImplementationIdentity::new(
                "actor-impl:test",
            ),
            method_identity: artifact::ActorMethodIdentity::new("actor-method:submit"),
        },
        site: artifact::InstructionSourceSite::Synthetic {
            reason: artifact::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
        },
        args: Vec::new(),
        type_args: std::collections::BTreeMap::new(),
        metadata: std::collections::BTreeMap::new(),
    };
    let linked = linked_call(&call, &|_| unreachable!()).unwrap();
    let LinkedCallTarget::ActorMethod {
        actor,
        actor_abi_identity,
        actor_implementation_identity,
        method_identity,
    } = linked.target
    else {
        panic!("Actor call must remain a non-executable Actor target")
    };
    assert_eq!(actor.symbol, "DocHub");
    assert_eq!(actor_abi_identity.as_str(), "actor-abi:test");
    assert_eq!(actor_implementation_identity.as_str(), "actor-impl:test");
    assert_eq!(method_identity.as_str(), "actor-method:submit");
}

#[test]
fn linked_file_conversion_rejects_actor_method_entry_out_of_bounds() {
    let mut file = actor_file();
    let method_identity = artifact::ActorMethodIdentity::new("actor-method:submit");
    file.actor_declarations[0]
        .abi
        .public_methods
        .push(artifact::ActorPublicMethodIr {
            method_identity: method_identity.clone(),
            name: "submit".to_string(),
            parameters: Vec::new(),
            return_type: artifact::TypeRefIr::builtin("void"),
            may_suspend: false,
        });
    file.actor_declarations[0]
        .method_implementations
        .insert(method_identity, 0);
    file.actor_declarations[0].actor_abi_identity =
        skiff_artifact_identity::actor_abi_identity(&file.actor_declarations[0].abi).unwrap();

    assert!(linked_file_unit_from_assembly_artifact(
        &file,
        &|_| unreachable!(),
        &|_| unreachable!(),
    )
    .unwrap_err()
    .to_string()
    .contains("implementation index 0 is out of bounds"));
}

#[test]
fn linked_file_conversion_rejects_tampered_actor_abi_identity() {
    let mut artifact = actor_file();
    artifact.actor_declarations[0].actor_abi_identity = artifact::ActorAbiIdentity::new("tampered");
    assert!(linked_file_unit_from_assembly_artifact(
        &artifact,
        &|_| unreachable!(),
        &|_| unreachable!(),
    )
    .unwrap_err()
    .to_string()
    .contains("ABI identity does not match"));
}
mod timeout_execution;
