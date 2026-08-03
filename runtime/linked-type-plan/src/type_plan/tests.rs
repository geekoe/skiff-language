use super::*;

pub(crate) fn test_runtime_package(
    slot: usize,
    package_id: &str,
    files: Vec<Arc<LinkedFileUnit>>,
) -> Arc<RuntimeExecutionPackage> {
    let file_refs = files
        .iter()
        .map(|file| {
            serde_json::json!({
                "fileIrIdentity": file.file_ir_identity,
                "modulePath": file.module_path,
                "sourceAstHash": file.source_ast_hash,
            })
        })
        .collect::<Vec<_>>();
    let artifact = serde_json::from_value(serde_json::json!({
        "schemaVersion": "skiff-package-artifact-v9",
        "packageId": package_id,
        "packageVersion": "1.0.0",
        "packageBuildId": format!("test-build:{slot}:{package_id}"),
        "files": file_refs,
        "staticResources": [],
        "packageLocalAbi": {
            "localAbiIdentity": format!("test-abi:{slot}:{package_id}"),
            "publicSymbols": {}
        },
        "packageSchemaIndex": {
            "packageId": package_id,
            "packageSchemaIndexIdentity": format!("test-schema:{slot}:{package_id}")
        },
        "packageSchemaTypeRecords": {},
        "implementationLinks": {},
        "callableLinks": {},
        "packageRequirements": [],
        "contractRequirements": [],
        "serviceRequirements": [],
        "runtimeRequirements": {
            "config": []
        },
        "callableSemanticFacts": {},
        "boundaryProjections": {},
        "serviceCallRefs": []
    }))
    .expect("test package artifact");
    Arc::new(
        RuntimeExecutionPackage::try_new(
            skiff_runtime_linked_program::PackageCodeSlotIndex::new(slot),
            Arc::new(artifact),
            files,
            Default::default(),
        )
        .expect("test runtime package context"),
    )
}

#[cfg(test)]
mod recoverable_expected_plan_tests {
    use std::{collections::BTreeMap, sync::Arc};

    use skiff_runtime_linked_program::{LinkedInterfaceInstantiationRef, RuntimeExecutionPackage};

    use super::super::*;

    fn empty_ctx<'a>(
        service_files: &'a [Arc<LinkedFileUnit>],
        packages: &'a [Arc<RuntimeExecutionPackage>],
        link_overlay: &'a LinkOverlay,
        types: &'a RuntimeTypeContext,
        addr: &'a ExecutableAddr,
    ) -> PlanContext<'a> {
        PlanContext::from_type_view(
            ProgramTypeView::new(service_files, packages, link_overlay, types),
            addr,
        )
    }

    fn string_type() -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: "string".to_string(),
            args: Vec::new(),
        }
    }

    #[test]
    fn native_builtin_plan_rejects_unknown_builtin_instead_of_using_opaque_fallback() {
        let error = native_builtin_plan("example.Unknown")
            .expect_err("unknown native signature builtin must fail closed");

        assert!(error
            .to_string()
            .contains("native signature references unknown builtin type example.Unknown"));
    }

    #[test]
    fn linked_recoverable_expected_plan_preserves_nested_any_interface() {
        let service_files = Vec::new();
        let packages = Vec::new();
        let link_overlay = LinkOverlay::default();
        let types = RuntimeTypeContext::default();
        let addr = ExecutableAddr::service(0, 0);
        let ctx = empty_ctx(&service_files, &packages, &link_overlay, &types, &addr);
        let interface = LinkedInterfaceInstantiationRef {
            interface_abi_id: "pkg.ToolProvider".to_string(),
            canonical_type_args: Vec::new(),
        };
        let ty = LinkedTypeRef::Record {
            fields: BTreeMap::from([(
                "provider".to_string(),
                LinkedTypeRef::AnyInterface {
                    interface: interface.clone(),
                },
            )]),
        };

        let expected = RuntimeRecoverableExpectedTypePlan::from_linked(&ty, &ctx)
            .expect("recoverable expected plan should build");

        let RuntimeRecoverableExpectedTypeNode::Record { fields, .. } = expected.node else {
            panic!("expected record node");
        };
        let RuntimeRecoverableExpectedTypeNode::AnyInterface { expected } = &fields[0].ty.node
        else {
            panic!("nested any interface must not collapse to unresolved/unknown");
        };
        assert_eq!(expected.interface_identity, "pkg.ToolProvider");
        assert_eq!(
            expected.method_projection_identity,
            "interface:pkg.ToolProvider"
        );
    }

    #[test]
    fn generic_interface_projection_identity_includes_canonical_args() {
        let interface = LinkedInterfaceInstantiationRef {
            interface_abi_id: "pkg.Provider".to_string(),
            canonical_type_args: vec![string_type()],
        };

        let projection = recoverable_interface_projection_identity(&interface);

        assert_ne!(projection, "interface:pkg.Provider");
        assert!(projection.starts_with("interface:{"));
        assert!(projection.contains("canonicalTypeArgs"));
    }

    #[test]
    fn linked_type_key_sorting_does_not_normalize_numbers() {
        let value = serde_json::json!({
            "z": 2,
            "a": serde_json::Number::from_f64(1.0).expect("number"),
        });

        assert_eq!(sorted_json_string(value), r#"{"a":1.0,"z":2}"#);
    }
}

#[cfg(test)]
mod applied_nominal_type_plan_tests {
    use std::sync::Arc;

    use skiff_runtime_linked_program::{
        FileAddr, LinkedNamedUnionBranch, LinkedNominalTypeRefBase, TypeDeclIr,
    };

    use super::super::*;

    fn builtin(name: &str) -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: name.to_string(),
            args: Vec::new(),
        }
    }

    fn type_param(name: &str) -> LinkedTypeRef {
        LinkedTypeRef::TypeParam {
            name: name.to_string(),
        }
    }

    fn addr(type_index: usize) -> TypeAddr {
        addr_in(0, type_index)
    }

    fn addr_in(package_slot: usize, type_index: usize) -> TypeAddr {
        TypeAddr {
            unit: UnitAddr::Package(package_slot),
            file: FileAddr::LoadedFileIndex(0),
            type_index,
        }
    }

    fn applied(type_index: usize, arguments: Vec<LinkedTypeRef>) -> LinkedTypeRef {
        LinkedTypeRef::AppliedNominal {
            base: LinkedNominalTypeRefBase::Address {
                addr: addr(type_index),
            },
            arguments,
        }
    }

    fn declaration(
        name: &str,
        type_params: &[&str],
        descriptor: LinkedTypeDescriptor,
    ) -> TypeDeclIr {
        TypeDeclIr {
            name: name.to_string(),
            descriptor,
            type_params: type_params
                .iter()
                .map(|parameter| (*parameter).to_string())
                .collect(),
            implements: Vec::new(),
            source_span: None,
        }
    }

    fn type_context() -> RuntimeTypeContext {
        let mut types = RuntimeTypeContext::default();
        types.descriptors.insert(
            addr(0),
            declaration(
                "Box",
                &["T"],
                LinkedTypeDescriptor::Record {
                    fields: BTreeMap::from([("value".to_string(), type_param("T"))]),
                },
            ),
        );
        types.descriptors.insert(
            addr(1),
            declaration(
                "Outer",
                &["T"],
                LinkedTypeDescriptor::Record {
                    fields: BTreeMap::from([(
                        "inner".to_string(),
                        applied(0, vec![type_param("T")]),
                    )]),
                },
            ),
        );
        types.descriptors.insert(
            addr(2),
            declaration(
                "Wrapped",
                &["T"],
                LinkedTypeDescriptor::Representation {
                    representation: type_param("T"),
                },
            ),
        );
        types.descriptors.insert(
            addr(3),
            declaration(
                "Choice",
                &["T"],
                LinkedTypeDescriptor::Union {
                    branches: vec![
                        LinkedNamedUnionBranch::ConcreteNominal {
                            nominal_type: applied(0, vec![type_param("T")]),
                        },
                        LinkedNamedUnionBranch::SyntheticDiscriminator {
                            payload_type: type_param("T"),
                            discriminator_field: "kind".to_string(),
                            discriminator_value: "value".to_string(),
                        },
                        LinkedNamedUnionBranch::Literal {
                            value: LiteralIr::String {
                                value: "none".to_string(),
                            },
                        },
                    ],
                },
            ),
        );
        types.descriptors.insert(
            addr(4),
            declaration(
                "BoxAlias",
                &[],
                LinkedTypeDescriptor::Alias {
                    target: applied(0, vec![builtin("string")]),
                },
            ),
        );
        types.descriptors.insert(
            addr(5),
            declaration("NotAValue", &["T"], LinkedTypeDescriptor::Interface),
        );
        types.descriptors.insert(
            addr_in(1, 0),
            declaration(
                "Wrapped",
                &["T"],
                LinkedTypeDescriptor::Representation {
                    representation: type_param("T"),
                },
            ),
        );
        types
    }

    fn with_context<T>(types: &RuntimeTypeContext, test: impl FnOnce(&PlanContext<'_>) -> T) -> T {
        let service_files: Vec<Arc<LinkedFileUnit>> = Vec::new();
        let packages: Vec<Arc<RuntimeExecutionPackage>> = Vec::new();
        let overlay = LinkOverlay::default();
        let current = ExecutableAddr::package(0, 0, 0);
        let context = PlanContext::from_type_view(
            ProgramTypeView::new(&service_files, &packages, &overlay, types),
            &current,
        );
        test(&context)
    }

    #[test]
    fn applied_nominal_arguments_produce_distinct_instantiated_record_facts() {
        let types = type_context();
        with_context(&types, |ctx| {
            let string_plan =
                RuntimeTypePlan::from_linked(&applied(0, vec![builtin("string")]), ctx).unwrap();
            let number_plan =
                RuntimeTypePlan::from_linked(&applied(0, vec![builtin("number")]), ctx).unwrap();

            assert_ne!(string_plan.label, number_plan.label);
            let RuntimeTypeNode::Record {
                fields: string_fields,
                ..
            } = string_plan.node
            else {
                panic!("Box<string> must instantiate as a record")
            };
            let RuntimeTypeNode::Record {
                fields: number_fields,
                ..
            } = number_plan.node
            else {
                panic!("Box<number> must instantiate as a record")
            };
            assert!(matches!(string_fields[0].ty.node, RuntimeTypeNode::String));
            assert!(matches!(number_fields[0].ty.node, RuntimeTypeNode::Number));
        });
    }

    #[test]
    fn nested_applied_nominal_recursively_substitutes_arguments() {
        let types = type_context();
        with_context(&types, |ctx| {
            let plan =
                RuntimeTypePlan::from_linked(&applied(1, vec![builtin("string")]), ctx).unwrap();
            let RuntimeTypeNode::Record { fields, .. } = plan.node else {
                panic!("Outer<string> must instantiate as a record")
            };
            let RuntimeTypeNode::Record {
                fields: inner_fields,
                ..
            } = &fields[0].ty.node
            else {
                panic!("nested Box<string> must remain a record plan")
            };
            assert!(matches!(inner_fields[0].ty.node, RuntimeTypeNode::String));
        });
    }

    #[test]
    fn generic_representation_and_named_union_keep_applied_owner_context() {
        let types = type_context();
        with_context(&types, |ctx| {
            let representation =
                RuntimeTypePlan::from_linked(&applied(2, vec![builtin("string")]), ctx).unwrap();
            let RuntimeTypeNode::Representation { type_name, payload } = representation.node else {
                panic!("generic representation must remain a representation")
            };
            assert_eq!(type_name, representation.label);
            assert!(matches!(payload.node, RuntimeTypeNode::String));

            let string_union =
                RuntimeTypePlan::from_linked(&applied(3, vec![builtin("string")]), ctx).unwrap();
            let number_union =
                RuntimeTypePlan::from_linked(&applied(3, vec![builtin("number")]), ctx).unwrap();
            assert_ne!(string_union.label, number_union.label);

            let RuntimeTypeNode::Union(string_branches) = &string_union.node else {
                panic!("generic named union must remain a union")
            };
            let RuntimeTypeNode::Union(number_branches) = &number_union.node else {
                panic!("generic named union must remain a union")
            };
            assert_eq!(string_branches.len(), 3);
            assert_eq!(number_branches.len(), 3);
            assert!(string_branches
                .iter()
                .all(|branch| branch.label.starts_with(&string_union.label)));
            assert!(number_branches
                .iter()
                .all(|branch| branch.label.starts_with(&number_union.label)));
            assert!(string_branches
                .iter()
                .zip(number_branches)
                .all(|(string_branch, number_branch)| string_branch.label != number_branch.label));
            assert!(string_branches[0].label.contains("concreteNominal"));
            assert!(string_branches[1]
                .label
                .contains("syntheticDiscriminator:kind=value"));
            assert!(string_branches[2].label.contains("literal"));
        });
    }

    #[test]
    fn representation_targets_produce_plans_from_exact_owner_and_argument_keys() {
        let types = type_context();
        with_context(&types, |ctx| {
            let local_string = applied(2, vec![builtin("string")]);
            let local_number = applied(2, vec![builtin("number")]);
            let external_string = LinkedTypeRef::AppliedNominal {
                base: LinkedNominalTypeRefBase::Address {
                    addr: addr_in(1, 0),
                },
                arguments: vec![builtin("string")],
            };

            let local_string_key = linked_type_ref_runtime_key(&local_string);
            let local_number_key = linked_type_ref_runtime_key(&local_number);
            let external_string_key = linked_type_ref_runtime_key(&external_string);
            assert_ne!(local_string_key, local_number_key);
            assert_ne!(local_string_key, external_string_key);

            let local_string_plan = RuntimeTypePlan::from_linked(&local_string, ctx).unwrap();
            let local_number_plan = RuntimeTypePlan::from_linked(&local_number, ctx).unwrap();
            let external_string_plan = RuntimeTypePlan::from_linked(&external_string, ctx).unwrap();
            for plan in [
                &local_string_plan,
                &local_number_plan,
                &external_string_plan,
            ] {
                assert!(matches!(plan.node, RuntimeTypeNode::Representation { .. }));
            }
            assert_ne!(local_string_plan.label, local_number_plan.label);
            assert_ne!(local_string_plan.label, external_string_plan.label);
            assert!(local_string_plan.label.contains(&local_string_key));
            assert!(local_number_plan.label.contains(&local_number_key));
            assert!(external_string_plan.label.contains(&external_string_key));
        });
    }

    #[test]
    fn plain_alias_expands_but_applied_admission_fails_closed() {
        let types = type_context();
        with_context(&types, |ctx| {
            let alias =
                RuntimeTypePlan::from_linked(&LinkedTypeRef::Address { addr: addr(4) }, ctx)
                    .unwrap();
            let RuntimeTypeNode::Alias(target) = alias.node else {
                panic!("plain alias must remain an alias plan")
            };
            let RuntimeTypeNode::Record { fields, .. } = target.node else {
                panic!("alias target applied nominal must expand to its record plan")
            };
            assert!(matches!(fields[0].ty.node, RuntimeTypeNode::String));

            for (ty, expected) in [
                (
                    LinkedTypeRef::AppliedNominal {
                        base: LinkedNominalTypeRefBase::Address { addr: addr(0) },
                        arguments: Vec::new(),
                    },
                    "non-empty",
                ),
                (
                    applied(0, vec![builtin("string"), builtin("number")]),
                    "arity 2, expected 1",
                ),
                (applied(5, vec![builtin("string")]), "targets interface"),
                (
                    applied(0, vec![type_param("Missing")]),
                    "unbound type parameter Missing",
                ),
                (
                    LinkedTypeRef::AppliedNominal {
                        base: LinkedNominalTypeRefBase::LocalType { type_index: 0 },
                        arguments: vec![builtin("string")],
                    },
                    "was not linked to an exact address",
                ),
                (applied(99, vec![builtin("string")]), "is not interned"),
                (
                    LinkedTypeRef::AppliedNominal {
                        base: LinkedNominalTypeRefBase::PackageSchema {
                            package_id: "example.models".to_string(),
                            stable_schema_key: "Box".to_string(),
                            package_schema_type_id: skiff_artifact_model::PackageSchemaTypeId::new(
                                "schema:box",
                            ),
                        },
                        arguments: vec![builtin("string")],
                    },
                    "applied PackageSchema is not admitted",
                ),
            ] {
                let error = RuntimeTypePlan::from_linked(&ty, ctx)
                    .err()
                    .expect("invalid applied nominal must fail closed");
                assert!(error.to_string().contains(expected), "{error}");
            }
        });
    }
}

/// Phase 0 differential baseline: `from_linked` vs the legacy JSON descriptor
/// bridge. Requires `--features test-support` (the legacy trait is gated);
/// semantically different inputs are pinned as expected differences.
#[cfg(all(test, feature = "test-support"))]
mod differential_legacy_json_baseline_tests {
    use std::{collections::BTreeMap, sync::Arc};

    use serde_json::{json, Value};

    use skiff_runtime_boundary::type_descriptor::RuntimeTypePlanDescriptorExt;
    use skiff_runtime_linked_program::{
        ExecutableAddr, FileAddr, LinkOverlay, LinkedFileUnit, LinkedTypeDescriptor, LinkedTypeRef,
        LiteralIr, RuntimeExecutionPackage, RuntimeTypeContext, TypeAddr, TypeDeclIr, UnitAddr,
    };

    use super::super::*;

    fn native(name: &str) -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: name.to_string(),
            args: Vec::new(),
        }
    }

    fn generic(name: &str, args: Vec<LinkedTypeRef>) -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: name.to_string(),
            args,
        }
    }

    fn builtin_descriptor(name: &str, args: Vec<Value>) -> Value {
        json!({ "kind": "builtin", "name": name, "args": args })
    }

    fn string_descriptor() -> Value {
        builtin_descriptor("string", Vec::new())
    }

    fn with_ctx<T>(
        types: Option<&RuntimeTypeContext>,
        substitutions: Option<&BTreeMap<String, LinkedTypeRef>>,
        test: impl FnOnce(&PlanContext<'_>) -> T,
    ) -> T {
        let owned_types = RuntimeTypeContext::default();
        let types = types.unwrap_or(&owned_types);
        let service_files: Vec<Arc<LinkedFileUnit>> = Vec::new();
        let packages: Vec<Arc<RuntimeExecutionPackage>> = Vec::new();
        let overlay = LinkOverlay::default();
        let current = ExecutableAddr::service(0, 0);
        let view = ProgramTypeView::new(&service_files, &packages, &overlay, types);
        let context = match substitutions {
            Some(substitutions) => {
                PlanContext::with_substitutions_from_type_view(view, &current, substitutions)
            }
            None => PlanContext::from_type_view(view, &current),
        };
        test(&context)
    }

    fn addr(type_index: usize) -> TypeAddr {
        TypeAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            type_index,
        }
    }

    fn declaration(name: &str, descriptor: LinkedTypeDescriptor) -> TypeDeclIr {
        TypeDeclIr {
            name: name.to_string(),
            descriptor,
            ..Default::default()
        }
    }

    fn type_context() -> RuntimeTypeContext {
        let mut types = RuntimeTypeContext::default();
        let representation = LinkedTypeDescriptor::Representation {
            representation: native("string"),
        };
        let alias = LinkedTypeDescriptor::Alias {
            target: native("string"),
        };
        types
            .descriptors
            .insert(addr(0), declaration("Wrapped", representation));
        types
            .descriptors
            .insert(addr(1), declaration("PlainAlias", alias));
        types
    }

    fn assert_plan_eq(actual: &RuntimeTypePlan, expected: &RuntimeTypePlan, context: &str) {
        assert_eq!(actual.label, expected.label, "{context}: label");
        assert_eq!(
            actual.named_type_name, expected.named_type_name,
            "{context}: named_type_name"
        );
        assert_eq!(actual.identity, expected.identity, "{context}: identity");
        assert_node_eq(&actual.node, &expected.node, context);
    }

    fn assert_node_eq(actual: &RuntimeTypeNode, expected: &RuntimeTypeNode, context: &str) {
        match (actual, expected) {
            (RuntimeTypeNode::Alias(a), RuntimeTypeNode::Alias(b))
            | (RuntimeTypeNode::Nullable(a), RuntimeTypeNode::Nullable(b))
            | (RuntimeTypeNode::Stream(a), RuntimeTypeNode::Stream(b))
            | (RuntimeTypeNode::Array(a), RuntimeTypeNode::Array(b)) => {
                assert_plan_eq(a, b, context)
            }
            (RuntimeTypeNode::Union(a), RuntimeTypeNode::Union(b)) => {
                assert_eq!(a.len(), b.len(), "{context}: union len");
                for (index, (a, b)) in a.iter().zip(b).enumerate() {
                    assert_plan_eq(a, b, &format!("{context}: union[{index}]"));
                }
            }
            (RuntimeTypeNode::LiteralString(a), RuntimeTypeNode::LiteralString(b)) => {
                assert_eq!(a, b, "{context}: literal")
            }
            (
                RuntimeTypeNode::Representation {
                    type_name: a_name,
                    payload: a_payload,
                },
                RuntimeTypeNode::Representation {
                    type_name: b_name,
                    payload: b_payload,
                },
            ) => {
                assert_eq!(a_name, b_name, "{context}: representation name");
                assert_plan_eq(a_payload, b_payload, context);
            }
            (
                RuntimeTypeNode::Map {
                    key: a_key,
                    value: a_value,
                },
                RuntimeTypeNode::Map {
                    key: b_key,
                    value: b_value,
                },
            ) => {
                assert_plan_eq(a_key, b_key, context);
                assert_plan_eq(a_value, b_value, context);
            }
            (
                RuntimeTypeNode::Record {
                    fields: a_fields,
                    boundary_record_kind: a_kind,
                },
                RuntimeTypeNode::Record {
                    fields: b_fields,
                    boundary_record_kind: b_kind,
                },
            ) => {
                assert_eq!(a_kind, b_kind, "{context}: record kind");
                assert_eq!(
                    a_fields.len(),
                    b_fields.len(),
                    "{context}: record fields len"
                );
                for (index, (a, b)) in a_fields.iter().zip(b_fields).enumerate() {
                    assert_eq!(a.name, b.name, "{context}: record[{index}].name");
                    assert_eq!(
                        a.required, b.required,
                        "{context}: record[{index}].required"
                    );
                    assert_eq!(
                        a.identity, b.identity,
                        "{context}: record[{index}].identity"
                    );
                    assert_plan_eq(&a.ty, &b.ty, &format!("{context}: record[{index}].ty"));
                }
            }
            // Remaining variants are payload-free leaves and Unknown; compare
            // discriminant (Debug output is not a stable semantic contract).
            _ => assert_eq!(
                std::mem::discriminant(actual),
                std::mem::discriminant(expected),
                "{context}: node"
            ),
        }
    }

    #[test]
    fn builtin_directory_matches_legacy_json_descriptors() {
        with_ctx(None, None, |ctx| {
            let mut cases: Vec<(LinkedTypeRef, Value)> = Vec::new();
            for name in
                "Json JsonObject bytes Date string bool boolean integer number null void".split(' ')
            {
                cases.push((native(name), builtin_descriptor(name, Vec::new())));
            }
            for name in "DbInsertManyResult DbUpdateManyResult DbDeleteManyResult".split(' ') {
                cases.push((
                    generic(name, Vec::new()),
                    builtin_descriptor(name, Vec::new()),
                ));
            }
            cases.push((
                generic("DbUpsertResult", vec![native("string")]),
                builtin_descriptor("DbUpsertResult", vec![string_descriptor()]),
            ));
            let number = builtin_descriptor("number", Vec::new());
            cases.push((
                generic("Array", vec![native("string")]),
                builtin_descriptor("Array", vec![string_descriptor()]),
            ));
            cases.push((
                generic("Map", vec![native("string"), native("number")]),
                builtin_descriptor("Map", vec![string_descriptor(), number]),
            ));
            cases.push((
                generic("Stream", vec![native("bytes")]),
                builtin_descriptor("Stream", vec![builtin_descriptor("bytes", Vec::new())]),
            ));
            for name in "std.http.HttpClientRequest std.http.HttpClientResponse std.http.HttpClientStreamHandle example.Unknown".split(' ') {
                cases.push((native(name), builtin_descriptor(name, Vec::new())));
            }
            for (index, (linked_ty, descriptor)) in cases.into_iter().enumerate() {
                let linked = RuntimeTypePlan::from_linked(&linked_ty, ctx)
                    .expect("linked builtin should build");
                let legacy = RuntimeTypePlan::from_descriptor(&descriptor)
                    .expect("legacy builtin should build");
                assert_plan_eq(&linked, &legacy, &format!("builtin case[{index}]"));
            }
            let unknown = RuntimeTypePlan::from_linked(&native("example.Unknown"), ctx)
                .expect("unknown builtin should not error");
            assert!(matches!(unknown.node, RuntimeTypeNode::Unknown));
        });
    }

    #[test]
    fn inline_record_union_nullable_and_literal_match_legacy_json_descriptors() {
        with_ctx(None, None, |ctx| {
            let nullable = |name: &str| LinkedTypeRef::Nullable {
                inner: Box::new(native(name)),
            };
            let literal = |value: &str| LinkedTypeRef::Literal {
                value: LiteralIr::String {
                    value: value.to_string(),
                },
            };
            let cases = [
                (
                    LinkedTypeRef::Record {
                        fields: BTreeMap::from([
                            ("age".to_string(), nullable("integer")),
                            ("name".to_string(), native("string")),
                            ("tags".to_string(), generic("Array", vec![native("string")])),
                        ]),
                    },
                    json!({"kind":"record","fields":{"age":{"kind":"nullable","inner":builtin_descriptor("integer",Vec::new())},"name":string_descriptor(),"tags":builtin_descriptor("Array",vec![string_descriptor()])}}),
                    "record",
                ),
                (
                    LinkedTypeRef::Union {
                        items: vec![native("string"), nullable("number"), literal("none")],
                    },
                    json!({"kind":"union","items":[string_descriptor(),{"kind":"nullable","inner":builtin_descriptor("number",Vec::new())},{"kind":"literal","value":{"kind":"string","value":"none"}}]}),
                    "union",
                ),
                (
                    nullable("string"),
                    json!({"kind":"nullable","inner":string_descriptor()}),
                    "nullable",
                ),
                (
                    literal("ok"),
                    json!({"kind":"literal","value":{"kind":"string","value":"ok"}}),
                    "literal",
                ),
            ];
            for (linked_ty, descriptor, label) in cases {
                let linked = RuntimeTypePlan::from_linked(&linked_ty, ctx)
                    .expect("linked structural shape should build");
                let legacy = RuntimeTypePlan::from_descriptor(&descriptor)
                    .expect("legacy structural shape should build");
                assert_plan_eq(&linked, &legacy, label);
            }
        });
    }

    #[test]
    fn address_resolved_descriptors_match_legacy_nodes_but_owner_context_is_outer_path_only() {
        let types = type_context();
        with_ctx(Some(&types), None, |ctx| {
            let representation = json!({"kind":"representation","name":"Wrapped","representation":string_descriptor()});
            let alias = json!({"kind":"alias","target":string_descriptor()});
            let cases = [
                (addr(0), representation, "Wrapped", "representation"),
                (addr(1), alias, "PlainAlias", "alias"),
            ];
            for (target_addr, descriptor, linked_label, legacy_label) in cases {
                let linked = RuntimeTypePlan::from_linked(
                    &LinkedTypeRef::Address { addr: target_addr },
                    ctx,
                )
                .expect("address should build");
                let legacy =
                    RuntimeTypePlan::from_descriptor(&descriptor).expect("legacy should build");
                assert_node_eq(&linked.node, &legacy.node, "address node");
                // 预期差异：owner context 由外层 JSON 路径应用（linked 侧
                // apply_nominal_owner_context），from_descriptor 桥本身不应用，
                // 因此 label/named_type_name 不同而 node 相同。
                assert_eq!(linked.label, linked_label);
                assert_eq!(linked.named_type_name, Some(linked_label.to_string()));
                assert_eq!(legacy.label, legacy_label);
                assert_eq!(legacy.named_type_name, None);
            }
        });
    }

    #[test]
    fn type_param_substitution_resolves_bound_ref_but_legacy_bridge_has_no_substitution_pass() {
        let substitutions = BTreeMap::from([("T".to_string(), native("string"))]);
        with_ctx(None, Some(&substitutions), |ctx| {
            let type_param = |name: &str| LinkedTypeRef::TypeParam {
                name: name.to_string(),
            };
            let linked = RuntimeTypePlan::from_linked(&type_param("T"), ctx)
                .expect("bound type parameter should resolve");
            let legacy =
                RuntimeTypePlan::from_descriptor(&string_descriptor()).expect("legacy string");
            assert_plan_eq(&linked, &legacy, "TypeParam<T> with T=string");

            // 预期差异：substitution 由外层 JSON 路径完成；裸 typeParam
            // descriptor 直接交给 from_descriptor 不会替换，落到 Unknown。
            let raw_legacy = RuntimeTypePlan::from_descriptor(&json!({
                "kind": "typeParam",
                "name": "T",
            }))
            .expect("raw typeParam descriptor should not error");
            assert!(matches!(raw_legacy.node, RuntimeTypeNode::Unknown));
            assert_eq!(raw_legacy.label, "typeParam");

            // 预期差异：linked 侧未绑定的 type param 直接报错（fail closed），
            // legacy 桥则保留为 Unknown。
            let unbound = RuntimeTypePlan::from_linked(&type_param("Missing"), ctx);
            assert!(unbound.is_err());
        });
    }

    #[test]
    fn depth_32_cap_truncates_linked_walk_but_legacy_bridge_recurses_uncapped() {
        fn innermost(node: &RuntimeTypeNode) -> &RuntimeTypeNode {
            match node {
                RuntimeTypeNode::Array(inner) => innermost(&inner.node),
                other => other,
            }
        }
        with_ctx(None, None, |ctx| {
            let mut at_cap_ref = native("string");
            let mut at_cap_descriptor = string_descriptor();
            for _ in 0..16 {
                at_cap_ref = generic("Array", vec![at_cap_ref]);
                at_cap_descriptor = builtin_descriptor("Array", vec![at_cap_descriptor]);
            }
            // 16 层 Array：最内层 depth = 2*16 = 32，未超过 cap，两路径完整一致。
            let at_cap =
                RuntimeTypePlan::from_linked(&at_cap_ref, ctx).expect("depth-32 plan should build");
            let at_cap_legacy = RuntimeTypePlan::from_descriptor(&at_cap_descriptor)
                .expect("legacy depth-32 plan should build");
            assert_plan_eq(&at_cap, &at_cap_legacy, "depth 32 array");
            assert!(matches!(innermost(&at_cap.node), RuntimeTypeNode::String));

            let mut over_cap_ref = native("string");
            let mut over_cap_descriptor = string_descriptor();
            for _ in 0..17 {
                over_cap_ref = generic("Array", vec![over_cap_ref]);
                over_cap_descriptor = builtin_descriptor("Array", vec![over_cap_descriptor]);
            }
            // 17 层 Array：最内层 depth = 34 > 32，from_linked 截断为 Unknown。
            let over_cap = RuntimeTypePlan::from_linked(&over_cap_ref, ctx)
                .expect("over-cap plan should build");
            assert!(matches!(
                innermost(&over_cap.node),
                RuntimeTypeNode::Unknown
            ));

            // 预期差异：legacy from_descriptor 自身没有 depth cap（cap 在外层
            // JSON walk resolve_program_descriptor_refs），同一输入不截断。
            let over_cap_legacy = RuntimeTypePlan::from_descriptor(&over_cap_descriptor)
                .expect("legacy over-cap plan should build");
            assert!(matches!(
                innermost(&over_cap_legacy.node),
                RuntimeTypeNode::String
            ));
        });
    }

    #[test]
    fn recoverable_expected_structural_shapes_match_legacy_shape_only_bridge() {
        with_ctx(None, None, |ctx| {
            let number = builtin_descriptor("number", Vec::new());
            let record_ref = LinkedTypeRef::Record {
                fields: BTreeMap::from([(
                    "name".to_string(),
                    LinkedTypeRef::Nullable {
                        inner: Box::new(native("string")),
                    },
                )]),
            };
            let record_descriptor = json!({"kind":"record","fields":{"name":{"kind":"nullable","inner":string_descriptor()}}});
            let cases = [
                (native("string"), string_descriptor()),
                (
                    generic("Array", vec![native("string")]),
                    builtin_descriptor("Array", vec![string_descriptor()]),
                ),
                (
                    generic("Map", vec![native("string"), native("number")]),
                    builtin_descriptor("Map", vec![string_descriptor(), number]),
                ),
                (record_ref, record_descriptor),
            ];
            for (index, (linked_ty, descriptor)) in cases.into_iter().enumerate() {
                let linked_expected =
                    RuntimeRecoverableExpectedTypePlan::from_linked(&linked_ty, ctx)
                        .expect("recoverable expected should build");
                let legacy_runtime = RuntimeTypePlan::from_descriptor(&descriptor)
                    .expect("legacy runtime plan should build");
                let legacy_expected = RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(&legacy_runtime);
                assert_eq!(
                    linked_expected, legacy_expected,
                    "recoverable case[{index}] should match legacy shape-only bridge"
                );
            }
        });
    }
}

#[cfg(test)]
mod builtin_catalog_tests {
    use super::super::*;
    use skiff_runtime_model::type_plan::RuntimeBuiltinShape;

    #[test]
    fn shape_of_name_resolves_bare_full_and_alias_spellings() {
        for (name, expected) in [
            ("Array", RuntimeBuiltinShape::Array),
            ("std.collection.Array", RuntimeBuiltinShape::Array),
            ("Stream", RuntimeBuiltinShape::Stream),
            ("std.stream.Stream", RuntimeBuiltinShape::Stream),
            ("Map", RuntimeBuiltinShape::Map),
            ("std.collection.Map", RuntimeBuiltinShape::Map),
            ("Json", RuntimeBuiltinShape::Json),
            ("JsonObject", RuntimeBuiltinShape::JsonObject),
            ("Date", RuntimeBuiltinShape::Date),
            ("string", RuntimeBuiltinShape::String),
            ("integer", RuntimeBuiltinShape::Integer),
            ("number", RuntimeBuiltinShape::Number),
            ("bool", RuntimeBuiltinShape::Bool),
            ("boolean", RuntimeBuiltinShape::Bool),
            ("bytes", RuntimeBuiltinShape::Bytes),
            ("null", RuntimeBuiltinShape::Null),
            ("void", RuntimeBuiltinShape::Null),
            ("std.http.Json", RuntimeBuiltinShape::Json),
            (
                "DbInsertManyResult",
                RuntimeBuiltinShape::DbInsertManyResult,
            ),
            (
                "DbUpdateManyResult",
                RuntimeBuiltinShape::DbUpdateManyResult,
            ),
            (
                "DbDeleteManyResult",
                RuntimeBuiltinShape::DbDeleteManyResult,
            ),
            ("DbUpsertResult", RuntimeBuiltinShape::DbUpsertResult),
        ] {
            assert_eq!(RuntimeBuiltinShape::of_name(name), Some(expected), "{name}");
        }
        for name in ["example.Unknown", "", "MyRecord"] {
            assert_eq!(RuntimeBuiltinShape::of_name(name), None, "{name}");
        }
    }

    #[test]
    fn leaf_node_maps_only_leaf_shapes() {
        let cases: [(&str, fn(&RuntimeTypeNode) -> bool); 10] = [
            ("Json", |n: &RuntimeTypeNode| {
                matches!(n, RuntimeTypeNode::Json)
            }),
            ("JsonObject", |n: &RuntimeTypeNode| {
                matches!(n, RuntimeTypeNode::JsonObject)
            }),
            ("Date", |n: &RuntimeTypeNode| {
                matches!(n, RuntimeTypeNode::Date)
            }),
            ("string", |n: &RuntimeTypeNode| {
                matches!(n, RuntimeTypeNode::String)
            }),
            ("integer", |n: &RuntimeTypeNode| {
                matches!(n, RuntimeTypeNode::Integer)
            }),
            ("number", |n: &RuntimeTypeNode| {
                matches!(n, RuntimeTypeNode::Number)
            }),
            ("bool", |n: &RuntimeTypeNode| {
                matches!(n, RuntimeTypeNode::Bool)
            }),
            ("bytes", |n: &RuntimeTypeNode| {
                matches!(n, RuntimeTypeNode::Bytes)
            }),
            ("null", |n: &RuntimeTypeNode| {
                matches!(n, RuntimeTypeNode::Null)
            }),
            ("void", |n: &RuntimeTypeNode| {
                matches!(n, RuntimeTypeNode::Null)
            }),
        ];
        for (name, is_expected) in cases {
            let node = RuntimeBuiltinShape::of_name(name).and_then(RuntimeBuiltinShape::leaf_node);
            assert!(node.as_ref().is_some_and(is_expected), "{name}");
        }
        for name in [
            "Array",
            "Stream",
            "Map",
            "DbInsertManyResult",
            "DbUpdateManyResult",
            "DbDeleteManyResult",
            "DbUpsertResult",
        ] {
            assert!(
                RuntimeBuiltinShape::of_name(name)
                    .and_then(RuntimeBuiltinShape::leaf_node)
                    .is_none(),
                "{name}"
            );
        }
    }
}

#[cfg(test)]
mod plan_input_forms_tests {
    use super::super::linked::{from_artifact_type_ref_in_program_ref, from_linked_ref};
    use super::super::*;

    fn with_empty_ctx<T>(test: impl FnOnce(&PlanContext<'_>) -> T) -> T {
        let service_files: Vec<Arc<LinkedFileUnit>> = Vec::new();
        let packages: Vec<Arc<RuntimeExecutionPackage>> = Vec::new();
        let overlay = LinkOverlay::default();
        let types = RuntimeTypeContext::default();
        let current = ExecutableAddr::service(0, 0);
        let ctx = PlanContext::from_type_view(
            ProgramTypeView::new(&service_files, &packages, &overlay, &types),
            &current,
        );
        test(&ctx)
    }

    fn node_key(node: &RuntimeTypeNode) -> String {
        match node {
            RuntimeTypeNode::Array(inner) => format!("array({})", node_key(&inner.node)),
            RuntimeTypeNode::Map { key, value } => {
                format!("map({}, {})", node_key(&key.node), node_key(&value.node))
            }
            RuntimeTypeNode::Stream(inner) => format!("stream({})", node_key(&inner.node)),
            RuntimeTypeNode::Record { fields, .. } => format!(
                "record({})",
                fields
                    .iter()
                    .map(|field| format!("{}={}", field.name, node_key(&field.ty.node)))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            RuntimeTypeNode::Union(items) => format!(
                "union({})",
                items
                    .iter()
                    .map(|plan| node_key(&plan.node))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            RuntimeTypeNode::Nullable(inner) => format!("nullable({})", node_key(&inner.node)),
            RuntimeTypeNode::Alias(inner) => format!("alias({})", node_key(&inner.node)),
            RuntimeTypeNode::Representation { .. } => "representation".to_string(),
            RuntimeTypeNode::LiteralString(_) => "literal".to_string(),
            RuntimeTypeNode::Json => "json".to_string(),
            RuntimeTypeNode::JsonObject => "jsonObject".to_string(),
            RuntimeTypeNode::Bytes => "bytes".to_string(),
            RuntimeTypeNode::Date => "date".to_string(),
            RuntimeTypeNode::String => "string".to_string(),
            RuntimeTypeNode::Bool => "bool".to_string(),
            RuntimeTypeNode::Integer => "integer".to_string(),
            RuntimeTypeNode::Number => "number".to_string(),
            RuntimeTypeNode::Null => "null".to_string(),
            RuntimeTypeNode::Unknown => "unknown".to_string(),
        }
    }

    fn artifact_builtin(
        name: &str,
        args: Vec<skiff_artifact_model::TypeRefIr>,
    ) -> skiff_artifact_model::TypeRefIr {
        skiff_artifact_model::TypeRefIr::Builtin {
            name: name.to_string(),
            args,
        }
    }

    fn linked_builtin(name: &str, args: Vec<LinkedTypeRef>) -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: name.to_string(),
            args,
        }
    }

    fn linked_leaf(name: &str) -> LinkedTypeRef {
        linked_builtin(name, Vec::new())
    }

    fn artifact_leaf(name: &str) -> skiff_artifact_model::TypeRefIr {
        artifact_builtin(name, Vec::new())
    }

    #[test]
    fn db_result_shapes_match_across_three_input_forms() {
        with_empty_ctx(|ctx| {
            for name in [
                "DbInsertManyResult",
                "DbUpdateManyResult",
                "DbDeleteManyResult",
                "DbUpsertResult",
            ] {
                let artifact = artifact_builtin(name, vec![artifact_leaf("string")]);
                let linked = linked_builtin(name, vec![linked_leaf("string")]);
                let artifact_key = RuntimeTypePlan::from_artifact_type_ref(&artifact).unwrap();
                let artifact_in_program_key =
                    from_artifact_type_ref_in_program_ref(&artifact, ctx).unwrap();
                let linked_key = from_linked_ref(&linked, ctx).unwrap();
                assert_eq!(
                    node_key(&artifact_key.node),
                    node_key(&linked_key.node),
                    "{name}"
                );
                assert_eq!(
                    node_key(&artifact_in_program_key.node),
                    node_key(&linked_key.node),
                    "{name}"
                );
            }
        });
    }

    #[test]
    fn structural_builtin_shapes_match_across_three_input_forms() {
        with_empty_ctx(|ctx| {
            for (name, artifact_args, linked_args) in [
                (
                    "Array",
                    vec![artifact_leaf("string")],
                    vec![linked_leaf("string")],
                ),
                (
                    "Stream",
                    vec![artifact_leaf("string")],
                    vec![linked_leaf("string")],
                ),
                (
                    "Map",
                    vec![artifact_leaf("string"), artifact_leaf("integer")],
                    vec![linked_leaf("string"), linked_leaf("integer")],
                ),
            ] {
                let artifact = artifact_builtin(name, artifact_args);
                let linked = linked_builtin(name, linked_args);
                let artifact_key = RuntimeTypePlan::from_artifact_type_ref(&artifact).unwrap();
                let artifact_in_program_key =
                    from_artifact_type_ref_in_program_ref(&artifact, ctx).unwrap();
                let linked_key = from_linked_ref(&linked, ctx).unwrap();
                assert_eq!(
                    node_key(&artifact_key.node),
                    node_key(&linked_key.node),
                    "{name}"
                );
                assert_eq!(
                    node_key(&artifact_in_program_key.node),
                    node_key(&linked_key.node),
                    "{name}"
                );
            }
        });
    }

    #[test]
    fn full_spelling_container_matching_preserves_historical_entry_difference() {
        // Historical difference locked by the three entries:
        // linked matches Array/Map only by the exact spelling, artifact entries
        // match through `bare_type_name`. The unified view keeps both rules.
        with_empty_ctx(|ctx| {
            let artifact = artifact_builtin("std.collection.Array", vec![artifact_leaf("string")]);
            let linked = linked_builtin("std.collection.Array", vec![linked_leaf("string")]);
            assert_eq!(
                node_key(
                    &RuntimeTypePlan::from_artifact_type_ref(&artifact)
                        .unwrap()
                        .node
                ),
                "array(string)"
            );
            assert_eq!(
                node_key(&from_linked_ref(&linked, ctx).unwrap().node),
                "unknown"
            );
        });
    }
}
