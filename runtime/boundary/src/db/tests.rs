use std::collections::BTreeMap;

use skiff_artifact_model::{InterfaceInstantiationRef, TypeRefIr};
use skiff_runtime_model::{
    recoverable::{
        RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
        RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableStorageLane,
        RuntimeRecoverableTrustBoundary,
    },
    request_heap::RequestHeap,
    runtime_value::RuntimeValue,
    type_plan::RuntimeTypeNode,
};

use crate::{
    db::{
        collection_item_plan_for_path, db_result_decode_plan_from_artifact_type_ref,
        db_storage_lane_from_artifact_type_ref, db_value_projection,
        db_write_projection_plan_from_artifact_type_ref,
        field_path_has_reserved_db_business_metadata, field_plan_for_path,
        is_reserved_db_business_metadata_name, normalize_db_field_path_text, DbBoundaryValuePlan,
        DbFieldPathPolicy, DbFieldPathPolicyError, DbValueProjection, MONGO_ID_FIELD,
    },
    plan::{BoundaryDirection, BoundaryUse},
    recoverable::RecoverableBoundaryCodec,
};

#[test]
fn db_type_path_descends_nullable_records() {
    let plan = DbBoundaryValuePlan::from_artifact_type_ref(nullable(record([
        ("createdAt", native("Date")),
        (
            "payload",
            nullable(record([
                ("recoverAt", native("Date")),
                (
                    "attempts",
                    TypeRefIr::Builtin {
                        name: "Array".to_string(),
                        args: vec![record([("at", native("Date"))])],
                    },
                ),
            ])),
        ),
    ])));

    let payload_recover = plan
        .write_projection_ref()
        .descend_path(["payload", "recoverAt"].into_iter())
        .expect("nested type should resolve");
    assert!(matches!(
        db_value_projection(payload_recover),
        DbValueProjection::Date
    ));

    let attempts = plan
        .write_projection_ref()
        .descend_path(["payload", "attempts"].into_iter())
        .expect("array field should resolve");
    let item = match db_value_projection(attempts) {
        DbValueProjection::Array(item) => item,
        other => panic!("expected array projection, got {other:?}"),
    };
    assert!(matches!(
        item.descend_path(["at"].into_iter())
            .map(db_value_projection),
        Some(DbValueProjection::Date)
    ));
}

#[test]
fn db_type_path_does_not_descend_into_recoverable_envelope_lane() {
    let plan = DbBoundaryValuePlan::from_artifact_type_ref(record([
        ("provider", any_interface()),
        ("label", native("string")),
    ]));

    assert_eq!(
        plan.storage_lane(),
        RuntimeRecoverableStorageLane::RecoverableEnvelope
    );
    assert!(matches!(
        db_value_projection(plan.write_projection_ref()),
        DbValueProjection::RecoverableEnvelope
    ));
    assert!(plan
        .write_projection_ref()
        .descend_path(["label"].into_iter())
        .is_none());
}

#[test]
fn db_field_type_helpers_route_key_and_declared_fields() {
    let key_plan = DbBoundaryValuePlan::from_artifact_type_ref(native("string"));
    let payload_plan =
        DbBoundaryValuePlan::from_artifact_type_ref(record([("recoverAt", native("Date"))]));

    let plan = field_plan_for_path("_id", "id", Some(&key_plan), |_| None)
        .expect("_id should use the key type");
    assert!(matches!(
        db_value_projection(plan),
        DbValueProjection::Scalar
    ));

    let plan = field_plan_for_path("payload.recoverAt", "id", Some(&key_plan), |top| {
        (top == "payload").then_some(&payload_plan)
    })
    .expect("declared field path should resolve");
    assert!(matches!(db_value_projection(plan), DbValueProjection::Date));

    let item = collection_item_plan_for_path("payload.recoverAt", "id", Some(&key_plan), |top| {
        (top == "payload").then_some(&payload_plan)
    })
    .expect("non-array field should return its own type");
    assert!(matches!(db_value_projection(item), DbValueProjection::Date));
}

#[test]
fn db_reserved_business_metadata_predicate_is_prefix_based() {
    assert!(is_reserved_db_business_metadata_name("__skiffLease"));
    assert!(field_path_has_reserved_db_business_metadata(
        "payload.__skiffType"
    ));
    assert!(!is_reserved_db_business_metadata_name("skiffType"));
    assert!(!field_path_has_reserved_db_business_metadata(
        "payload.public"
    ));
}

#[test]
fn db_field_path_policy_maps_key_and_accepts_internal_mongo_id() {
    let policy = DbFieldPathPolicy::new("id");

    let key = policy
        .resolve_business_field_path("id", "Thread", |_| false)
        .expect("business key should resolve even when it is not a value field");
    assert_eq!(key.business_path(), "id");
    assert_eq!(key.top_level(), "id");
    assert_eq!(key.mongo_path(), MONGO_ID_FIELD);

    let title = policy
        .resolve_business_field_path("title", "Thread", |top| top == "title")
        .expect("declared business field should resolve");
    assert_eq!(title.mongo_path(), "title");

    let mongo_id = policy
        .resolve_mongo_facing_field_path(MONGO_ID_FIELD, "Thread", |_| false)
        .expect("_id should stay accepted for mongo-facing paths");
    assert_eq!(mongo_id.mongo_path(), MONGO_ID_FIELD);
}

#[test]
fn db_field_path_policy_rejects_unsupported_reserved_and_undeclared_paths() {
    let policy = DbFieldPathPolicy::new("id");

    let error = policy
        .resolve_business_field_path("title.", "Thread", |top| top == "title")
        .expect_err("empty path segments should be rejected");
    assert!(matches!(
        error,
        DbFieldPathPolicyError::UnsupportedFieldPath { .. }
    ));

    let error = policy
        .resolve_business_field_path("title.__skiffType", "Thread", |top| top == "title")
        .expect_err("reserved metadata segments should be rejected");
    assert!(matches!(
        error,
        DbFieldPathPolicyError::ReservedBusinessMetadataPath { .. }
    ));

    let error = policy
        .resolve_business_field_path("missing.nested", "Thread", |top| top == "title")
        .expect_err("undeclared top-level fields should be rejected");
    assert!(matches!(
        error,
        DbFieldPathPolicyError::UndeclaredTopLevel { .. }
    ));
}

#[test]
fn db_field_path_policy_keeps_mutable_key_rejection() {
    let policy = DbFieldPathPolicy::new("id");

    for field in ["id", "id.part", "_id", "_id.part"] {
        let error = policy
            .resolve_mutable_business_field_path(field, "Thread", |_| true)
            .expect_err("key and mongo id paths should not be mutable");
        assert!(matches!(
            error,
            DbFieldPathPolicyError::MutableKeyPath { .. }
        ));
    }
}

#[test]
fn db_field_path_text_normalization_prefers_text_then_segments() {
    assert_eq!(
        normalize_db_field_path_text("title", ["ignored"]),
        "title".to_string()
    );
    assert_eq!(
        normalize_db_field_path_text("", ["payload", "createdAt"]),
        "payload.createdAt".to_string()
    );
    assert_eq!(
        normalize_db_field_path_text("   ", ["payload", "createdAt"]),
        "payload.createdAt".to_string()
    );
}

#[test]
fn db_artifact_type_ref_projection_builds_boundary_plans_without_program_context() {
    let ty = record([
        ("id", native("string")),
        ("createdAt", nullable(native("Date"))),
        (
            "tags",
            TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![native("string")],
            },
        ),
    ]);

    let write_plan = db_write_projection_plan_from_artifact_type_ref(&ty);
    assert_eq!(write_plan.use_case(), BoundaryUse::DbWriteProjection);
    assert_eq!(write_plan.direction(), BoundaryDirection::Project);
    assert!(matches!(
        write_plan.expected().node(),
        RuntimeTypeNode::Record { fields, .. } if fields.len() == 3
    ));

    let decode_plan = db_result_decode_plan_from_artifact_type_ref(&ty);
    assert_eq!(decode_plan.use_case(), BoundaryUse::DbResultDecode);
    assert_eq!(decode_plan.direction(), BoundaryDirection::Decode);

    let value_plan = DbBoundaryValuePlan::from_artifact_type_ref(ty);
    assert_eq!(
        value_plan.storage_lane(),
        RuntimeRecoverableStorageLane::SchemaProjectable
    );
    assert!(matches!(
        value_plan.write_projection_ref().projection(),
        DbValueProjection::Record(fields) if fields.field("createdAt").is_some()
    ));
    assert!(matches!(
        value_plan.result_decode_ref().projection(),
        DbValueProjection::Record(fields) if fields.field("createdAt").is_some()
    ));
}

#[test]
fn any_interface_artifact_type_ref_is_unknown_for_db_boundary_projection() {
    let ty = any_interface();

    let plan = db_result_decode_plan_from_artifact_type_ref(&ty);

    assert!(matches!(plan.expected().node(), RuntimeTypeNode::Unknown));
    assert_eq!(plan.expected().interface_identity(), Some("reader"));
    assert_eq!(
        db_storage_lane_from_artifact_type_ref(&ty),
        RuntimeRecoverableStorageLane::RecoverableEnvelope
    );
}

#[test]
fn db_lane_selects_schema_projectable_for_plain_nested_data() {
    let ty = record([
        ("label", native("string")),
        (
            "tags",
            TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![native("string")],
            },
        ),
    ]);

    assert_eq!(
        db_storage_lane_from_artifact_type_ref(&ty),
        RuntimeRecoverableStorageLane::SchemaProjectable
    );
}

#[test]
fn db_lane_selects_recoverable_envelope_for_behavior_or_nominal_nodes() {
    let nested_behavior = record([("provider", any_interface()), ("label", native("string"))]);

    for ty in [
        any_interface(),
        TypeRefIr::LocalType { type_index: 0 },
        nested_behavior,
    ] {
        assert_eq!(
            db_storage_lane_from_artifact_type_ref(&ty),
            RuntimeRecoverableStorageLane::RecoverableEnvelope
        );
    }
}

#[test]
fn recoverable_envelope_db_context_roundtrips_plain_value() {
    let heap = RequestHeap::default();
    let value = RuntimeValue::String("plain".to_string());
    let expected = RuntimeRecoverableExpectedTypePlan::unresolved("db envelope field");
    let context = recoverable_db_context();

    let bytes = RecoverableBoundaryCodec::encode(&value, &expected, &context, &heap)
        .expect("plain value should encode for DB envelope lane");
    let decoded =
        RecoverableBoundaryCodec::decode(&bytes, &expected, &context, &mut RequestHeap::default())
            .expect("plain value should decode for DB envelope lane");

    assert_eq!(decoded, value);
}

#[test]
fn db_artifact_anonymous_union_has_no_reconstructed_catch_identity() {
    let ty = TypeRefIr::Union {
        items: vec![native("string"), native("number")],
    };

    let plan = db_result_decode_plan_from_artifact_type_ref(&ty);

    assert!(plan.expected().catch_identity().is_none());
    let RuntimeTypeNode::Union(items) = plan.expected().node() else {
        panic!("expected union plan");
    };
    assert_eq!(items.len(), 2);
    assert!(matches!(items[0].node(), RuntimeTypeNode::String));
    assert!(items[0].catch_identity().is_none());
}

fn native(name: &str) -> TypeRefIr {
    TypeRefIr::Builtin {
        name: name.to_string(),
        args: Vec::new(),
    }
}

fn nullable(inner: TypeRefIr) -> TypeRefIr {
    TypeRefIr::Nullable {
        inner: Box::new(inner),
    }
}

fn record<const N: usize>(fields: [(&str, TypeRefIr); N]) -> TypeRefIr {
    TypeRefIr::Record {
        fields: BTreeMap::from(fields.map(|(name, ty)| (name.to_string(), ty))),
    }
}

fn any_interface() -> TypeRefIr {
    TypeRefIr::AnyInterface {
        interface: InterfaceInstantiationRef {
            interface_abi_id: "reader".to_string(),
            canonical_type_args: vec![native("string")],
        },
    }
}

fn recoverable_db_context() -> RuntimeRecoverableBoundaryContext {
    RuntimeRecoverableBoundaryContext::new(
        RuntimeRecoverableBoundaryKind::DbValue,
        RuntimeRecoverableTrustBoundary::OwnerInternal,
        RuntimeRecoverableStorageLane::RecoverableEnvelope,
    )
}
