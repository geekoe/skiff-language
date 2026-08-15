use serde_json::json;

use super::*;

fn string_node(value: &str) -> RecoverableNode {
    RecoverableNode::plain(
        RecoverableValueKind::String,
        RecoverableState::String(value.to_string()),
    )
}

fn local_concrete_node(concrete_type_identity: &str) -> RecoverableNode {
    RecoverableNode {
        value_kind: RecoverableValueKind::NominalObject,
        variant_identity: RecoverableVariantIdentity::None,
        code_identity: RecoverableCodeIdentity::LocalConcrete {
            owner: LocalConcreteOwner::Service,
            concrete_type_identity: concrete_type_identity.to_string(),
        },
        state: RecoverableState::NominalObject(NominalObjectState::DefaultFields {
            fields: vec![RecoverableField {
                field_identity: "value".to_string(),
                value: string_node("state"),
            }],
        }),
    }
}

fn nested_nominal_map_key(representation_count: usize) -> RecoverableMapKey {
    let mut key = RecoverableMapKey::String("leaf".to_string());
    for index in (0..representation_count).rev() {
        key = RecoverableMapKey::NominalRepresentation {
            representation_identity: format!("repr-{index}"),
            value: Box::new(key),
        };
    }
    key
}

#[test]
fn boundary_context_skips_empty_optional_fields() {
    let context = RuntimeRecoverableBoundaryContext::new(
        RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload,
        RuntimeRecoverableTrustBoundary::OwnerInternal,
        RuntimeRecoverableStorageLane::RecoverableEnvelope,
    );

    let json = serde_json::to_value(context).expect("context should serialize");

    assert_eq!(
        json,
        json!({
            "kind": "runtimeBinaryPayload",
            "trustBoundary": "ownerInternal",
            "storageLane": "recoverableEnvelope"
        })
    );
}

#[test]
fn expected_plan_preserves_unresolved_runtime_plan_explicitly() {
    let plan = RuntimeTypePlan {
        label: "anyInterface".to_string(),
        named_type_name: Some("pkg.Reader".to_string()),
        identity: Default::default(),
        node: RuntimeTypeNode::Unknown,
    };

    let recoverable =
        RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
            &plan,
        );

    assert_eq!(
        recoverable.identity,
        Some(RuntimeRecoverableTypeIdentityRef::RuntimeNamedType(
            RuntimeRecoverableNamedTypeRef::new("pkg.Reader")
        ))
    );
    assert_eq!(
        recoverable.node,
        RuntimeRecoverableExpectedTypeNode::Unresolved {
            diagnostic_label: "anyInterface".to_string()
        }
    );
}

#[test]
fn envelope_json_debug_roundtrip_preserves_nodes() {
    let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
        RecoverableValueKind::Array,
        RecoverableState::Array(vec![
            string_node("Ada"),
            RecoverableNode::plain(
                RecoverableValueKind::Number,
                RecoverableState::Number(
                    RecoverableNumber::try_from_f64(42.5).expect("finite number"),
                ),
            ),
        ]),
    ));

    let json = serde_json::to_string_pretty(&envelope).expect("json encode");
    let decoded: RecoverableEnvelope = serde_json::from_str(&json).expect("json decode");

    assert_eq!(decoded, envelope);
    decoded
        .validate(&RecoverableValidationLimits::default())
        .expect("debug JSON roundtrip should validate");
}

#[test]
fn canonical_binary_roundtrip_sorts_record_fields_and_map_entries() {
    let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
        RecoverableValueKind::Record,
        RecoverableState::Record(vec![
            RecoverableField {
                field_identity: "z".to_string(),
                value: string_node("last"),
            },
            RecoverableField {
                field_identity: "a".to_string(),
                value: RecoverableNode::plain(
                    RecoverableValueKind::Map,
                    RecoverableState::Map(vec![
                        (RecoverableMapKey::String("b".to_string()), string_node("2")),
                        (RecoverableMapKey::String("a".to_string()), string_node("1")),
                    ]),
                ),
            },
        ]),
    ));
    let limits = RecoverableValidationLimits::default();

    let first = envelope
        .to_canonical_bytes(&limits)
        .expect("canonical encode should succeed");
    let decoded =
        RecoverableEnvelope::from_canonical_bytes(&first, &limits).expect("canonical decode");
    let second = decoded
        .to_canonical_bytes(&limits)
        .expect("canonical re-encode should succeed");

    assert_eq!(first, second);
    let RecoverableState::Record(fields) = decoded.root.state else {
        panic!("expected record");
    };
    assert_eq!(fields[0].field_identity, "a");
    assert_eq!(fields[1].field_identity, "z");
}

#[test]
fn canonical_binary_rejects_non_canonical_bool_payload() {
    let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
        RecoverableValueKind::Bool,
        RecoverableState::Bool(true),
    ));
    let limits = RecoverableValidationLimits::default();
    let mut bytes = envelope
        .to_canonical_bytes(&limits)
        .expect("canonical bool encode should succeed");
    let bool_payload = bytes
        .last_mut()
        .expect("encoded bool should have payload byte");
    assert_eq!(*bool_payload, 1);
    *bool_payload = 2;

    let error = RecoverableEnvelope::from_canonical_bytes(&bytes, &limits)
        .expect_err("non-canonical bool payload must fail closed");

    assert_eq!(error.path(), "$.root");
    assert!(error
        .message()
        .contains("recoverable bool payload must be 0 or 1"));
}

#[test]
fn map_key_depth_limit_is_enforced_before_canonical_encode() {
    let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
        RecoverableValueKind::Map,
        RecoverableState::Map(vec![(nested_nominal_map_key(2), string_node("value"))]),
    ));
    let limits = RecoverableValidationLimits {
        max_nodes: 16,
        max_depth: 1,
        max_encoded_bytes: 4096,
    };

    let error = envelope
        .to_canonical_bytes(&limits)
        .expect_err("deep nominal map key must fail encode validation");

    assert_eq!(error.path(), "$.mapKey[0]");
    assert!(error.message().contains("recoverable depth exceeds 1"));
}

#[test]
fn nested_nominal_map_key_roundtrips_when_depth_allows_it() {
    let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
        RecoverableValueKind::Map,
        RecoverableState::Map(vec![(nested_nominal_map_key(2), string_node("value"))]),
    ));
    let limits = RecoverableValidationLimits {
        max_nodes: 16,
        max_depth: 3,
        max_encoded_bytes: 4096,
    };

    let bytes = envelope
        .to_canonical_bytes(&limits)
        .expect("allowed nominal map key depth should encode");
    let decoded = RecoverableEnvelope::from_canonical_bytes(&bytes, &limits)
        .expect("allowed nominal map key depth should decode");

    assert_eq!(decoded, envelope);
}

#[test]
fn canonical_decode_enforces_depth_before_constructing_tree() {
    let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
        RecoverableValueKind::Array,
        RecoverableState::Array(vec![RecoverableNode::plain(
            RecoverableValueKind::Array,
            RecoverableState::Array(vec![string_node("too-deep")]),
        )]),
    ));
    let bytes = envelope
        .to_canonical_bytes(&RecoverableValidationLimits::default())
        .expect("canonical nested array encode should succeed");
    let limits = RecoverableValidationLimits {
        max_nodes: 16,
        max_depth: 1,
        max_encoded_bytes: bytes.len(),
    };

    let error = RecoverableEnvelope::from_canonical_bytes(&bytes, &limits)
        .expect_err("decode must enforce max depth while reading");

    assert_eq!(error.path(), "$.root[0][0]");
    assert!(error.message().contains("recoverable depth exceeds 1"));
}

#[test]
fn canonical_decode_enforces_node_budget_before_container_allocation() {
    let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
        RecoverableValueKind::Array,
        RecoverableState::Array(vec![string_node("one"), string_node("two")]),
    ));
    let bytes = envelope
        .to_canonical_bytes(&RecoverableValidationLimits::default())
        .expect("canonical array encode should succeed");
    let limits = RecoverableValidationLimits {
        max_nodes: 2,
        max_depth: 512,
        max_encoded_bytes: bytes.len(),
    };

    let error = RecoverableEnvelope::from_canonical_bytes(&bytes, &limits)
        .expect_err("decode must enforce max node budget before Vec allocation");

    assert_eq!(error.path(), "$.root");
    assert!(error.message().contains("exceeding remaining node budget"));
}

#[test]
fn duplicate_field_identity_is_invalid() {
    let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
        RecoverableValueKind::Record,
        RecoverableState::Record(vec![
            RecoverableField {
                field_identity: "same".to_string(),
                value: string_node("1"),
            },
            RecoverableField {
                field_identity: "same".to_string(),
                value: string_node("2"),
            },
        ]),
    ));

    let error = envelope
        .validate(&RecoverableValidationLimits::default())
        .expect_err("duplicate fields must fail");

    assert!(error
        .message()
        .contains("duplicate recoverable field identity"));
}

#[test]
fn duplicate_canonical_map_key_is_invalid() {
    let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
        RecoverableValueKind::Map,
        RecoverableState::Map(vec![
            (
                RecoverableMapKey::String("same".to_string()),
                string_node("1"),
            ),
            (
                RecoverableMapKey::String("same".to_string()),
                string_node("2"),
            ),
        ]),
    ));

    let error = envelope
        .validate(&RecoverableValidationLimits::default())
        .expect_err("duplicate map keys must fail");

    assert!(error.message().contains("duplicate recoverable map key"));
}

#[test]
fn nested_local_concrete_refs_are_not_collected_as_artifact_refs() {
    let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
        RecoverableValueKind::Array,
        RecoverableState::Array(vec![
            local_concrete_node("pkg.User"),
            local_concrete_node("pkg.Org"),
        ]),
    ));

    let refs = envelope.collect_artifact_refs();

    assert!(refs.is_empty());
}

#[test]
fn interface_wrapper_has_no_code_identity_and_self_node_carries_local_concrete() {
    let envelope = RecoverableEnvelope::new(RecoverableNode {
        value_kind: RecoverableValueKind::InterfaceValue,
        variant_identity: RecoverableVariantIdentity::None,
        code_identity: RecoverableCodeIdentity::None,
        state: RecoverableState::InterfaceValue(InterfaceValueState::Local {
            self_node: Box::new(local_concrete_node("pkg.FileReader")),
        }),
    });

    envelope
        .validate(&RecoverableValidationLimits::default())
        .expect("interface wrapper with self_node code should validate");
    let refs = envelope.collect_artifact_refs();
    assert!(refs.is_empty());
}

#[test]
fn interface_wrapper_rejects_own_code_identity() {
    let envelope = RecoverableEnvelope::new(RecoverableNode {
        value_kind: RecoverableValueKind::InterfaceValue,
        variant_identity: RecoverableVariantIdentity::None,
        code_identity: RecoverableCodeIdentity::LocalConcrete {
            owner: LocalConcreteOwner::Service,
            concrete_type_identity: "pkg.FileReader".to_string(),
        },
        state: RecoverableState::InterfaceValue(InterfaceValueState::Local {
            self_node: Box::new(local_concrete_node("pkg.FileReader")),
        }),
    });

    let error = envelope
        .validate(&RecoverableValidationLimits::default())
        .expect_err("wrapper code identity must fail");

    assert!(error
        .message()
        .contains("InterfaceValue wrapper must not carry code identity"));
}

#[test]
fn interface_value_state_rejects_retired_remote_json_variant() {
    let error = serde_json::from_value::<InterfaceValueState>(serde_json::json!({
        "kind": "remote",
        "carrier": {
            "dependencyRef": "reader",
            "publicInstanceKey": "default",
            "operations": {
                "id": "remote:reader",
                "interfaceAbiId": "pkg.Reader",
                "slots": []
            }
        }
    }))
    .expect_err("retired remote interface state must fail typed deserialization");

    assert!(
        error.to_string().contains("unknown variant `remote`"),
        "unexpected error: {error}"
    );
}

#[test]
fn native_handle_adapter_identity_only_lives_in_code_identity() {
    let envelope = RecoverableEnvelope::new(RecoverableNode {
        value_kind: RecoverableValueKind::NativeHandle,
        variant_identity: RecoverableVariantIdentity::None,
        code_identity: RecoverableCodeIdentity::NativeAdapter {
            adapter_identity: "std.FileHandleAdapter".to_string(),
            adapter_schema_version: "1".to_string(),
            owner: NativeAdapterOwner::Artifact {
                artifact_identity: "svc/files".to_string(),
                build_id: "build-native".to_string(),
                package: None,
            },
            native_type_identity: "std.FileHandle".to_string(),
        },
        state: RecoverableState::NativeHandle(NativeHandleState {
            durable_state: Box::new(string_node("handle-state")),
        }),
    });

    envelope
        .validate(&RecoverableValidationLimits::default())
        .expect("native handle should validate");
    let refs = envelope.collect_artifact_refs();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].artifact_identity, "svc/files");
    assert_eq!(refs[0].node_path, "$.root");
}

#[test]
fn invalid_schema_and_limits_fail_closed() {
    let mut envelope = RecoverableEnvelope::new(string_node("Ada"));
    envelope.schema_version = "old".to_string();
    assert!(envelope
        .validate(&RecoverableValidationLimits::default())
        .expect_err("unknown schema should fail")
        .message()
        .contains("unsupported recoverable schema version"));

    let mut v1_bytes = RecoverableEnvelope::new(string_node("Ada"))
        .to_canonical_bytes(&RecoverableValidationLimits::default())
        .expect("v2 envelope should encode");
    let schema_offset = v1_bytes
        .windows(RECOVERABLE_ENVELOPE_SCHEMA_VERSION.len())
        .position(|window| window == RECOVERABLE_ENVELOPE_SCHEMA_VERSION.as_bytes())
        .expect("schema string should be encoded");
    *v1_bytes
        .get_mut(schema_offset + RECOVERABLE_ENVELOPE_SCHEMA_VERSION.len() - 1)
        .expect("schema version suffix should exist") = b'1';
    let error = RecoverableEnvelope::from_canonical_bytes(
        &v1_bytes,
        &RecoverableValidationLimits::default(),
    )
    .expect_err("v1 canonical bytes must fail closed");
    assert_eq!(error.path(), "$");
    assert!(error
        .message()
        .contains("unsupported recoverable schema version"));

    let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
        RecoverableValueKind::Array,
        RecoverableState::Array(vec![string_node("nested")]),
    ));
    let limits = RecoverableValidationLimits {
        max_nodes: 1,
        max_depth: 512,
        max_encoded_bytes: 1024,
    };
    assert!(envelope
        .validate(&limits)
        .expect_err("node count limit should fail")
        .message()
        .contains("node count exceeds"));
}

#[test]
fn local_concrete_restore_key_rejects_empty_stable_identity() {
    let empty_concrete = RecoverableNode::plain(
        RecoverableValueKind::NominalObject,
        RecoverableState::NominalObject(NominalObjectState::DefaultFields { fields: Vec::new() }),
    );
    let mut empty_concrete = empty_concrete;
    empty_concrete.code_identity = RecoverableCodeIdentity::LocalConcrete {
        owner: LocalConcreteOwner::Service,
        concrete_type_identity: String::new(),
    };

    let error = RecoverableEnvelope::new(empty_concrete)
        .validate(&RecoverableValidationLimits::default())
        .expect_err("empty LocalConcrete type identity must fail closed");
    assert_eq!(error.path(), "$");
    assert!(error.message().contains("non-empty concrete type identity"));

    let empty_package = RecoverableNode::plain(
        RecoverableValueKind::NominalObject,
        RecoverableState::NominalObject(NominalObjectState::DefaultFields { fields: Vec::new() }),
    );
    let mut empty_package = empty_package;
    empty_package.code_identity = RecoverableCodeIdentity::LocalConcrete {
        owner: LocalConcreteOwner::Package {
            package_id: " ".to_string(),
        },
        concrete_type_identity: "pkg.User".to_string(),
    };

    let error = RecoverableEnvelope::new(empty_package)
        .validate(&RecoverableValidationLimits::default())
        .expect_err("empty LocalConcrete package owner must fail closed");
    assert!(error.message().contains("non-empty package id"));
}

#[test]
fn native_adapter_artifact_owner_requires_exact_build_identity() {
    for (artifact_identity, build_id) in [("", "build-1"), ("svc/files", ""), ("", "")] {
        let node = RecoverableNode {
            value_kind: RecoverableValueKind::String,
            variant_identity: RecoverableVariantIdentity::None,
            code_identity: RecoverableCodeIdentity::NativeAdapter {
                adapter_identity: "adapter".to_string(),
                adapter_schema_version: "1".to_string(),
                owner: NativeAdapterOwner::Artifact {
                    artifact_identity: artifact_identity.to_string(),
                    build_id: build_id.to_string(),
                    package: None,
                },
                native_type_identity: "std.FileHandle".to_string(),
            },
            state: RecoverableState::String("state".to_string()),
        };

        let error = RecoverableEnvelope::new(node)
            .validate(&RecoverableValidationLimits::default())
            .expect_err("empty artifact/build identity must fail closed");
        assert!(
            error
                .message()
                .contains("exact artifact and build identities"),
            "{artifact_identity:?}/{build_id:?}: {error}"
        );
    }
}

#[test]
fn artifact_collector_keeps_distinct_builds_under_the_same_node_path() {
    let mut collector = RecoverableArtifactCollector::default();
    collector.insert_ref("svc/files", "build-a", &None, "$.root");
    collector.insert_ref("svc/files", "build-b", &None, "$.root");
    collector.insert_ref("svc/files", "build-b", &None, "$.root");

    let refs = collector.into_refs();

    assert_eq!(refs.len(), 2, "same path must not merge across builds");
    let build_ids = refs
        .iter()
        .map(|reference| reference.build_id.as_str())
        .collect::<Vec<_>>();
    assert!(build_ids.contains(&"build-a"));
    assert!(build_ids.contains(&"build-b"));
}
