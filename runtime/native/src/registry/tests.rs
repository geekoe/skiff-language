use serde_json::json;
use skiff_artifact_model::{
    BuiltinReceiverCallableSemantics, NativeCallableSemantics, NativeSignatureTypeExpr,
    BUILTIN_RECEIVER_CALLABLE_SEMANTICS, STD_NATIVE_CALLABLE_SEMANTICS, STD_NATIVE_SIGNATURES,
};
use skiff_runtime_model::service_error::PlatformBuiltinErrorIdentity;
use skiff_runtime_native_contract::NativeRequiredContext;

use crate::{
    dispatch::{runtime_shared_native_route_for_validation, RuntimeNativeRoute},
    handlers::{array_empty, map_empty},
};

use super::{
    table::{
        native_route_matches_required_context, validate_handler_entries,
        validate_native_callable_semantics_registry, validate_receiver_callable_semantics_registry,
        NativeHandlerEntry, NATIVE_BINDINGS,
    },
    NativeRegistry,
};

fn detached_native_semantics(binding_key: &'static str) -> NativeCallableSemantics {
    let mut semantics = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|semantics| semantics.binding_key == "std.string.truncateUtf8Bytes")
        .expect("detached non-suspending native semantics should exist")
        .clone();
    semantics.binding_key = binding_key;
    semantics
}

fn assert_decode_target_projection(
    error: crate::error::RuntimeError,
    target: &str,
    code: &str,
    expected_identity: PlatformBuiltinErrorIdentity,
    message_fragment: &str,
) {
    let payload = error
        .ordinary_payload()
        .expect("decode target error remains ordinary");
    assert_eq!(payload.code, code, "unexpected error: {error}");
    assert_eq!(
        payload
            .details
            .as_ref()
            .and_then(|details| details["target"].as_str()),
        Some(target),
        "unexpected error: {error}"
    );
    assert!(
        error.to_string().contains(message_fragment),
        "unexpected error: {error}"
    );
    assert_eq!(
        error
            .ordinary_catch_projection()
            .map(|(identity, _)| identity),
        Some(expected_identity.catch_identity())
    );
}

#[test]
fn native_handler_registry_builtin_table_validates_against_signature_contract() {
    NativeRegistry::validate_builtin_handlers()
        .expect("native handler registry table should validate");
}

#[test]
fn native_callable_semantics_registry_validates_exact_builtin_matrix() {
    validate_native_callable_semantics_registry(
        STD_NATIVE_CALLABLE_SEMANTICS,
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect("native callable semantics registry should validate");
}

#[test]
fn date_from_epoch_milliseconds_semantics_has_exact_runtime_handler() {
    let binding_key = "core.date.fromEpochMilliseconds";
    let semantics = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|entry| entry.binding_key == binding_key)
        .expect("audited Date constructor semantics should be registered");

    validate_native_callable_semantics_registry(
        std::slice::from_ref(semantics),
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect("Date constructor semantics should match its canonical runtime route");
    assert_eq!(
        NATIVE_BINDINGS
            .iter()
            .filter(|entry| entry.binding_key == binding_key)
            .count(),
        1,
        "Date constructor should have exactly one native registry handler"
    );
}

#[test]
fn map_empty_semantics_matches_exact_generic_signature_and_runtime_handler() {
    let binding_key = "core.map.empty";
    let semantics = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|entry| entry.binding_key == binding_key)
        .expect("audited Map.empty semantics should be registered");

    validate_native_callable_semantics_registry(
        std::slice::from_ref(semantics),
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect("Map.empty semantics should match its canonical runtime route");
    assert_eq!(
        NATIVE_BINDINGS
            .iter()
            .filter(|entry| entry.binding_key == binding_key)
            .count(),
        1,
    );
    assert_eq!(
        semantics.return_provenance,
        skiff_artifact_model::ValueProvenance::Fresh
    );
    assert_eq!(
        semantics.effects,
        skiff_artifact_model::CallableMayEffects {
            writes_caller_reachable: false,
            returns_caller_alias: false,
            throws_caller_alias: false,
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_suspend: false,
        }
    );
}

#[test]
fn map_empty_semantics_rejects_malformed_native_signatures_and_lookalikes() {
    let semantics = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|entry| entry.binding_key == "core.map.empty")
        .cloned()
        .expect("Map.empty callable semantics should be registered");
    let canonical = *STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == "core.map.empty")
        .expect("Map.empty native signature should exist");

    let cases = [
        {
            let mut signature = canonical;
            signature.target = "std.map.empty";
            ("identity", signature)
        },
        {
            let mut signature = canonical;
            signature.type_param_count = 1;
            ("generic arity", signature)
        },
        {
            let mut signature = canonical;
            signature.params = &[NativeSignatureTypeExpr::TypeParam(0)];
            ("value arity", signature)
        },
        {
            let mut signature = canonical;
            signature.return_type =
                NativeSignatureTypeExpr::Array(&NativeSignatureTypeExpr::TypeParam(0));
            ("return", signature)
        },
    ];

    for (case, signature) in cases {
        let error = validate_native_callable_semantics_registry(
            std::slice::from_ref(&semantics),
            &[signature],
            NATIVE_BINDINGS,
        )
        .expect_err("malformed Map.empty native signature should fail closed");
        assert!(
            error.contains("core.map.empty")
                && error.contains("does not match the exact shared native signature"),
            "{case}: unexpected error: {error}"
        );
    }

    let mut lookalike = semantics;
    lookalike.binding_key = "std.map.empty";
    let error = validate_native_callable_semantics_registry(
        &[lookalike],
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect_err("non-canonical Map.empty lookalike should fail closed");
    assert!(
        error.contains("std.map.empty") && error.contains("not in the exact audited registry"),
        "unexpected error: {error}"
    );
}

#[test]
fn json_decode_semantics_matches_exact_generic_signature_and_json_route() {
    let binding_key = "std.json.decode";
    let semantics = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|entry| entry.binding_key == binding_key)
        .expect("audited std.json.decode semantics should be registered");

    validate_native_callable_semantics_registry(
        std::slice::from_ref(semantics),
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect("std.json.decode semantics should match its canonical Json route");
    assert_eq!(
        runtime_shared_native_route_for_validation(binding_key, true),
        Some(RuntimeNativeRoute::Json)
    );
    assert_eq!(
        semantics.return_provenance,
        skiff_artifact_model::ValueProvenance::Fresh
    );
    assert_eq!(
        semantics.effects,
        skiff_artifact_model::CallableMayEffects {
            writes_caller_reachable: false,
            returns_caller_alias: false,
            throws_caller_alias: false,
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_suspend: false,
        }
    );
}

#[test]
fn json_decode_semantics_rejects_malformed_signatures_routes_and_lookalikes() {
    let semantics = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|entry| entry.binding_key == "std.json.decode")
        .cloned()
        .expect("std.json.decode callable semantics should be registered");
    let canonical = *STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == "std.json.decode")
        .expect("std.json.decode native signature should exist");

    let cases = [
        {
            let mut signature = canonical;
            signature.target = "platform.json.decode";
            ("identity", signature)
        },
        {
            let mut signature = canonical;
            signature.type_param_count = 0;
            ("generic arity", signature)
        },
        {
            let mut signature = canonical;
            signature.params = &[];
            ("value arity", signature)
        },
        {
            let mut signature = canonical;
            signature.params = &[NativeSignatureTypeExpr::TypeParam(0)];
            ("argument", signature)
        },
        {
            let mut signature = canonical;
            signature.return_type = NativeSignatureTypeExpr::Builtin("Json");
            ("return", signature)
        },
    ];

    for (case, signature) in cases {
        let error = validate_native_callable_semantics_registry(
            std::slice::from_ref(&semantics),
            &[signature],
            NATIVE_BINDINGS,
        )
        .expect_err("malformed std.json.decode signature should fail closed");
        assert!(
            error.contains("std.json.decode")
                && error.contains("does not match the exact shared native signature"),
            "{case}: unexpected error: {error}"
        );
    }

    assert!(!native_route_matches_required_context(
        "std.json.decode",
        NativeRequiredContext::None,
        RuntimeNativeRoute::NativeRegistry,
    ));

    let mut lookalike = semantics;
    lookalike.binding_key = "platform.json.decode";
    let error = validate_native_callable_semantics_registry(
        &[lookalike],
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect_err("non-canonical json.decode lookalike should fail closed");
    assert!(
        error.contains("platform.json.decode")
            && error.contains("not in the exact audited registry"),
        "unexpected error: {error}"
    );
}

#[test]
fn json_merge_semantics_match_only_the_exact_non_generic_json_signature() {
    let semantics = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|entry| entry.binding_key == "std.json.merge")
        .cloned()
        .expect("std.json.merge callable semantics should be registered");
    let canonical = *STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == "std.json.merge")
        .expect("std.json.merge native signature should exist");

    validate_native_callable_semantics_registry(
        std::slice::from_ref(&semantics),
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect("std.json.merge semantics should match its exact native registry route");
    assert_eq!(
        semantics.return_provenance,
        skiff_artifact_model::ValueProvenance::Fresh
    );
    assert_eq!(
        semantics.effects,
        skiff_artifact_model::CallableMayEffects {
            writes_caller_reachable: false,
            returns_caller_alias: false,
            throws_caller_alias: false,
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_suspend: false,
        }
    );

    let cases = [
        {
            let mut signature = canonical;
            signature.target = "platform.json.merge";
            ("identity", signature)
        },
        {
            let mut signature = canonical;
            signature.type_param_count = 1;
            ("generic arity", signature)
        },
        {
            let mut signature = canonical;
            signature.params = &[NativeSignatureTypeExpr::Builtin("Json")];
            ("value arity", signature)
        },
        {
            let mut signature = canonical;
            signature.params = &[
                NativeSignatureTypeExpr::Builtin("JsonObject"),
                NativeSignatureTypeExpr::Builtin("Json"),
            ];
            ("parameter", signature)
        },
        {
            let mut signature = canonical;
            signature.return_type = NativeSignatureTypeExpr::Builtin("JsonObject");
            ("return", signature)
        },
    ];

    for (case, signature) in cases {
        let error = validate_native_callable_semantics_registry(
            std::slice::from_ref(&semantics),
            &[signature],
            NATIVE_BINDINGS,
        )
        .expect_err("malformed std.json.merge signature should fail closed");
        assert!(
            error.contains("std.json.merge")
                && error.contains("does not match the exact shared native signature"),
            "{case}: unexpected error: {error}"
        );
    }

    let mut lookalike = semantics;
    lookalike.binding_key = "platform.json.merge";
    let error = validate_native_callable_semantics_registry(
        &[lookalike],
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect_err("non-canonical std.json.merge lookalike should fail closed");
    assert!(
        error.contains("platform.json.merge")
            && error.contains("not in the exact audited registry"),
        "unexpected error: {error}"
    );
}

#[test]
fn date_parse_semantics_matches_exact_native_signature_and_runtime_handler() {
    let binding_key = "core.date.parse";
    let semantics = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|entry| entry.binding_key == binding_key)
        .expect("audited Date.parse semantics should be registered");

    validate_native_callable_semantics_registry(
        std::slice::from_ref(semantics),
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect("Date.parse semantics should match its canonical runtime route");
    assert_eq!(
        NATIVE_BINDINGS
            .iter()
            .filter(|entry| entry.binding_key == binding_key)
            .count(),
        1,
    );
    assert_eq!(
        semantics.return_provenance,
        skiff_artifact_model::ValueProvenance::Fresh
    );
    assert_eq!(
        semantics.effects,
        skiff_artifact_model::CallableMayEffects {
            writes_caller_reachable: false,
            returns_caller_alias: false,
            throws_caller_alias: false,
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_suspend: false,
        }
    );
}

#[test]
fn date_parse_semantics_rejects_malformed_native_signatures() {
    let semantics = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|entry| entry.binding_key == "core.date.parse")
        .cloned()
        .expect("Date.parse callable semantics should be registered");
    let canonical = *STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == "core.date.parse")
        .expect("Date.parse native signature should exist");

    let cases = [
        {
            let mut signature = canonical;
            signature.target = "std.date.parse";
            ("identity", signature)
        },
        {
            let mut signature = canonical;
            signature.params = &[];
            ("arity", signature)
        },
        {
            let mut signature = canonical;
            signature.params = &[NativeSignatureTypeExpr::Builtin("Date")];
            ("argument", signature)
        },
        {
            let mut signature = canonical;
            signature.return_type = NativeSignatureTypeExpr::Builtin("Date");
            ("return", signature)
        },
    ];

    for (case, signature) in cases {
        let error = validate_native_callable_semantics_registry(
            std::slice::from_ref(&semantics),
            &[signature],
            NATIVE_BINDINGS,
        )
        .expect_err("malformed Date.parse native signature should fail closed");
        assert!(
            error.contains("core.date.parse")
                && error.contains("does not match the exact shared native signature"),
            "{case}: unexpected error: {error}"
        );
    }
}

#[test]
fn date_parse_semantics_rejects_noncanonical_lookalike() {
    let mut semantics = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|entry| entry.binding_key == "core.date.parse")
        .cloned()
        .expect("Date.parse callable semantics should be registered");
    semantics.binding_key = "std.date.parse";
    let error = validate_native_callable_semantics_registry(
        &[semantics],
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect_err("non-canonical Date.parse lookalike should fail closed");
    assert!(
        error.contains("std.date.parse") && error.contains("not in the exact audited registry"),
        "unexpected error: {error}"
    );
}

#[test]
fn native_callable_semantics_registry_validates_exact_receiver_matrix() {
    validate_receiver_callable_semantics_registry(
        BUILTIN_RECEIVER_CALLABLE_SEMANTICS,
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect("receiver callable semantics registry should validate");
}

fn date_compare_receiver_semantics() -> BuiltinReceiverCallableSemantics {
    BUILTIN_RECEIVER_CALLABLE_SEMANTICS
        .iter()
        .find(|entry| entry.op.canonical_key == "receiver:Date.compare@1")
        .cloned()
        .expect("Date.compare callable semantics should be registered")
}

#[test]
fn date_compare_receiver_semantics_matches_exact_native_signature() {
    let semantics = date_compare_receiver_semantics();
    validate_receiver_callable_semantics_registry(
        std::slice::from_ref(&semantics),
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect("Date.compare semantics should match the canonical native signature");
    assert_eq!(
        semantics.return_provenance,
        skiff_artifact_model::ValueProvenance::Fresh
    );
    assert_eq!(
        semantics.effects,
        skiff_artifact_model::CallableMayEffects {
            writes_caller_reachable: false,
            returns_caller_alias: false,
            throws_caller_alias: false,
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_suspend: false,
        }
    );
}

#[test]
fn date_compare_receiver_semantics_rejects_malformed_native_signatures() {
    let semantics = date_compare_receiver_semantics();
    let canonical = *STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == "core.date.compare")
        .expect("Date.compare native signature should exist");

    let cases = [
        {
            let mut signature = canonical;
            signature.target = "Duration.compare";
            ("receiver", signature)
        },
        {
            let mut signature = canonical;
            signature.params = &[NativeSignatureTypeExpr::Builtin("Date")];
            ("arity", signature)
        },
        {
            let mut signature = canonical;
            signature.params = &[
                NativeSignatureTypeExpr::Builtin("Date"),
                NativeSignatureTypeExpr::Builtin("string"),
            ];
            ("argument", signature)
        },
        {
            let mut signature = canonical;
            signature.return_type = NativeSignatureTypeExpr::Builtin("number");
            ("return", signature)
        },
    ];

    for (case, signature) in cases {
        let error = validate_receiver_callable_semantics_registry(
            std::slice::from_ref(&semantics),
            &[signature],
            NATIVE_BINDINGS,
        )
        .expect_err("malformed Date.compare native signature should fail closed");
        assert!(
            error.contains("receiver:Date.compare@1")
                && error.contains("does not match the exact shared native signature"),
            "{case}: unexpected error: {error}"
        );
    }
}

#[test]
fn date_compare_receiver_semantics_rejects_noncanonical_lookalike() {
    let mut descriptor = date_compare_receiver_semantics();
    descriptor.op.canonical_key = "receiver:std.time.Date.compare@1";
    let error = validate_receiver_callable_semantics_registry(
        &[descriptor],
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect_err("non-canonical Date.compare lookalike should fail closed");
    assert!(
        error.contains("receiver:std.time.Date.compare@1")
            && error.contains("not an exact supported runtime operation"),
        "unexpected error: {error}"
    );
}

fn date_add_milliseconds_receiver_semantics() -> BuiltinReceiverCallableSemantics {
    BUILTIN_RECEIVER_CALLABLE_SEMANTICS
        .iter()
        .find(|entry| entry.op.canonical_key == "receiver:Date.addMilliseconds@1")
        .cloned()
        .expect("Date.addMilliseconds callable semantics should be registered")
}

#[test]
fn date_add_milliseconds_receiver_semantics_matches_exact_native_signature() {
    let semantics = date_add_milliseconds_receiver_semantics();
    validate_receiver_callable_semantics_registry(
        std::slice::from_ref(&semantics),
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect("Date.addMilliseconds semantics should match the canonical native signature");
    assert_eq!(
        semantics.return_provenance,
        skiff_artifact_model::ValueProvenance::Fresh
    );
    assert_eq!(
        semantics.effects,
        skiff_artifact_model::CallableMayEffects {
            writes_caller_reachable: false,
            returns_caller_alias: false,
            throws_caller_alias: false,
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_suspend: false,
        }
    );
}

#[test]
fn date_add_milliseconds_receiver_semantics_rejects_malformed_native_signatures() {
    let semantics = date_add_milliseconds_receiver_semantics();
    let canonical = *STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == "core.date.addMilliseconds")
        .expect("Date.addMilliseconds native signature should exist");

    let cases = [
        {
            let mut signature = canonical;
            signature.target = "Duration.addMilliseconds";
            ("receiver", signature)
        },
        {
            let mut signature = canonical;
            signature.params = &[NativeSignatureTypeExpr::Builtin("Date")];
            ("arity", signature)
        },
        {
            let mut signature = canonical;
            signature.params = &[
                NativeSignatureTypeExpr::Builtin("Date"),
                NativeSignatureTypeExpr::Builtin("string"),
            ];
            ("argument", signature)
        },
        {
            let mut signature = canonical;
            signature.return_type = NativeSignatureTypeExpr::Builtin("integer");
            ("return", signature)
        },
    ];

    for (case, signature) in cases {
        let error = validate_receiver_callable_semantics_registry(
            std::slice::from_ref(&semantics),
            &[signature],
            NATIVE_BINDINGS,
        )
        .expect_err("malformed Date.addMilliseconds native signature should fail closed");
        assert!(
            error.contains("receiver:Date.addMilliseconds@1")
                && error.contains("does not match the exact shared native signature"),
            "{case}: unexpected error: {error}"
        );
    }
}

#[test]
fn date_add_milliseconds_receiver_semantics_rejects_noncanonical_lookalike() {
    let mut descriptor = date_add_milliseconds_receiver_semantics();
    descriptor.op.canonical_key = "receiver:std.time.Date.addMilliseconds@1";
    let error = validate_receiver_callable_semantics_registry(
        &[descriptor],
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect_err("non-canonical Date.addMilliseconds lookalike should fail closed");
    assert!(
        error.contains("receiver:std.time.Date.addMilliseconds@1")
            && error.contains("not an exact supported runtime operation"),
        "unexpected error: {error}"
    );
}

fn date_diff_milliseconds_receiver_semantics() -> BuiltinReceiverCallableSemantics {
    BUILTIN_RECEIVER_CALLABLE_SEMANTICS
        .iter()
        .find(|entry| entry.op.canonical_key == "receiver:Date.diffMilliseconds@1")
        .cloned()
        .expect("Date.diffMilliseconds callable semantics should be registered")
}

#[test]
fn date_diff_milliseconds_receiver_semantics_matches_exact_native_signature() {
    let semantics = date_diff_milliseconds_receiver_semantics();
    validate_receiver_callable_semantics_registry(
        std::slice::from_ref(&semantics),
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect("Date.diffMilliseconds semantics should match the canonical native signature");
    assert_eq!(
        semantics.return_provenance,
        skiff_artifact_model::ValueProvenance::Fresh
    );
    assert_eq!(
        semantics.effects,
        skiff_artifact_model::CallableMayEffects {
            writes_caller_reachable: false,
            returns_caller_alias: false,
            throws_caller_alias: false,
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_suspend: false,
        }
    );
}

#[test]
fn date_diff_milliseconds_receiver_semantics_rejects_malformed_native_signatures() {
    let semantics = date_diff_milliseconds_receiver_semantics();
    let canonical = *STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == "core.date.diffMilliseconds")
        .expect("Date.diffMilliseconds native signature should exist");

    let cases = [
        {
            let mut signature = canonical;
            signature.target = "Duration.diffMilliseconds";
            ("receiver", signature)
        },
        {
            let mut signature = canonical;
            signature.params = &[NativeSignatureTypeExpr::Builtin("Date")];
            ("arity", signature)
        },
        {
            let mut signature = canonical;
            signature.params = &[
                NativeSignatureTypeExpr::Builtin("Date"),
                NativeSignatureTypeExpr::Builtin("string"),
            ];
            ("argument", signature)
        },
        {
            let mut signature = canonical;
            signature.return_type = NativeSignatureTypeExpr::Builtin("number");
            ("return", signature)
        },
    ];

    for (case, signature) in cases {
        let error = validate_receiver_callable_semantics_registry(
            std::slice::from_ref(&semantics),
            &[signature],
            NATIVE_BINDINGS,
        )
        .expect_err("malformed Date.diffMilliseconds native signature should fail closed");
        assert!(
            error.contains("receiver:Date.diffMilliseconds@1")
                && error.contains("does not match the exact shared native signature"),
            "{case}: unexpected error: {error}"
        );
    }
}

#[test]
fn date_diff_milliseconds_receiver_semantics_rejects_noncanonical_lookalike() {
    let mut descriptor = date_diff_milliseconds_receiver_semantics();
    descriptor.op.canonical_key = "receiver:std.time.Date.diffMilliseconds@1";
    let error = validate_receiver_callable_semantics_registry(
        &[descriptor],
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect_err("non-canonical Date.diffMilliseconds lookalike should fail closed");
    assert!(
        error.contains("receiver:std.time.Date.diffMilliseconds@1")
            && error.contains("not an exact supported runtime operation"),
        "unexpected error: {error}"
    );
}

#[test]
fn native_callable_semantics_registry_rejects_duplicate_receiver_descriptor() {
    let descriptor: BuiltinReceiverCallableSemantics =
        BUILTIN_RECEIVER_CALLABLE_SEMANTICS[0].clone();
    let canonical_key = descriptor.op.canonical_key;
    let error = validate_receiver_callable_semantics_registry(
        &[descriptor.clone(), descriptor],
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect_err("duplicate receiver semantics should fail validation");
    assert!(
        error.contains(canonical_key) && error.contains("more than once"),
        "unexpected error: {error}"
    );
}

#[test]
fn native_callable_semantics_registry_rejects_forged_receiver_identity() {
    let mut descriptor = BUILTIN_RECEIVER_CALLABLE_SEMANTICS[0].clone();
    descriptor.op.canonical_key = "receiver:Date.isAfter@1";
    let error = validate_receiver_callable_semantics_registry(
        &[descriptor],
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect_err("forged receiver identity should fail validation");
    assert!(
        error.contains("receiver:Date.isAfter@1")
            && error.contains("not an exact supported runtime operation"),
        "unexpected error: {error}"
    );
}

#[test]
fn native_callable_semantics_registry_rejects_unknown_descriptor() {
    let error = validate_native_callable_semantics_registry(
        &[detached_native_semantics("std.unknown.safe")],
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect_err("unknown semantics binding should fail validation");

    assert!(
        error.contains("std.unknown.safe") && error.contains("not in the exact audited registry"),
        "unexpected error: {error}"
    );
}

#[test]
fn native_callable_semantics_registry_rejects_duplicate_descriptor() {
    let descriptor = detached_native_semantics("std.string.truncateUtf8Bytes");
    let error = validate_native_callable_semantics_registry(
        &[descriptor.clone(), descriptor],
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect_err("duplicate semantics binding should fail validation");

    assert!(
        error.contains("std.string.truncateUtf8Bytes") && error.contains("more than once"),
        "unexpected error: {error}"
    );
}

#[test]
fn native_callable_semantics_registry_rejects_missing_signature() {
    let signatures = STD_NATIVE_SIGNATURES
        .iter()
        .copied()
        .filter(|signature| signature.binding_key != "std.string.truncateUtf8Bytes")
        .collect::<Vec<_>>();
    let error = validate_native_callable_semantics_registry(
        &[detached_native_semantics("std.string.truncateUtf8Bytes")],
        &signatures,
        NATIVE_BINDINGS,
    )
    .expect_err("descriptor without matching signature should fail validation");

    assert!(
        error.contains("std.string.truncateUtf8Bytes")
            && error.contains("missing from the native signatures"),
        "unexpected error: {error}"
    );
}

#[test]
fn native_callable_semantics_registry_rejects_signature_mismatch() {
    let mut signature = *STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == "std.string.truncateUtf8Bytes")
        .expect("truncate signature should exist");
    signature.return_type = NativeSignatureTypeExpr::Builtin("number");
    let error = validate_native_callable_semantics_registry(
        &[detached_native_semantics("std.string.truncateUtf8Bytes")],
        &[signature],
        NATIVE_BINDINGS,
    )
    .expect_err("mismatched exact signature should fail validation");

    assert!(
        error.contains("std.string.truncateUtf8Bytes")
            && error.contains("does not match the exact shared native signature"),
        "unexpected error: {error}"
    );
}

#[test]
fn native_callable_semantics_registry_accepts_exact_websocket_route_matrix() {
    for binding_key in [
        "std.websocket.sendTextToConnection",
        "std.websocket.sendBinaryToConnection",
        "std.websocket.sendTextToBusinessIdentity",
        "std.websocket.sendBinaryToBusinessIdentity",
    ] {
        let semantics = STD_NATIVE_CALLABLE_SEMANTICS
            .iter()
            .find(|semantics| semantics.binding_key == binding_key)
            .unwrap_or_else(|| panic!("{binding_key} should have audited callable semantics"));
        let handler_count = NATIVE_BINDINGS
            .iter()
            .filter(|entry| entry.binding_key == binding_key)
            .count();

        assert_eq!(
            NativeRequiredContext::for_binding_key(binding_key),
            Some(NativeRequiredContext::Websocket),
            "{binding_key} required context"
        );
        assert_eq!(handler_count, 0, "{binding_key} registry handler count");
        assert_eq!(
            runtime_shared_native_route_for_validation(binding_key, handler_count > 0),
            Some(RuntimeNativeRoute::Websocket),
            "{binding_key} runtime route"
        );
        validate_native_callable_semantics_registry(
            std::slice::from_ref(semantics),
            STD_NATIVE_SIGNATURES,
            NATIVE_BINDINGS,
        )
        .unwrap_or_else(|error| panic!("{binding_key} should validate: {error}"));
    }
}

#[test]
fn native_callable_semantics_registry_accepts_http_suspend_detachment_routes() {
    for binding_key in [
        "std.http.client.request",
        "std.http.client.stream",
        "std.http.client.sse",
    ] {
        let semantics = STD_NATIVE_CALLABLE_SEMANTICS
            .iter()
            .find(|semantics| semantics.binding_key == binding_key)
            .cloned()
            .unwrap_or_else(|| panic!("{binding_key} must have exact callable semantics"));
        assert!(semantics.effects.may_suspend);
        assert!(!semantics.effects.writes_caller_reachable);
        assert!(!semantics.effects.returns_caller_alias);
        assert!(!semantics.effects.throws_caller_alias);
        assert!(!semantics.effects.escapes_caller_value);
        assert!(!semantics.effects.requires_same_heap_identity);
        assert!(!semantics.effects.invokes_unknown_target);
        let handler_count = NATIVE_BINDINGS
            .iter()
            .filter(|entry| entry.binding_key == semantics.binding_key)
            .count();

        assert_eq!(
            NativeRequiredContext::for_binding_key(semantics.binding_key),
            Some(NativeRequiredContext::HttpClient)
        );
        assert_eq!(
            runtime_shared_native_route_for_validation(semantics.binding_key, handler_count > 0),
            Some(RuntimeNativeRoute::Http)
        );
        validate_native_callable_semantics_registry(
            &[semantics],
            STD_NATIVE_SIGNATURES,
            NATIVE_BINDINGS,
        )
        .unwrap_or_else(|error| {
            panic!("detached, suspending {binding_key} should match its runtime route: {error}")
        });
    }
}

#[test]
fn native_callable_semantics_registry_accepts_exact_response_stream_emit_route() {
    let binding_key = "std.http.stream.emitResponse";
    let semantics = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|semantics| semantics.binding_key == binding_key)
        .cloned()
        .expect("response stream emitter must have exact callable semantics");
    assert_eq!(
        semantics.effects,
        skiff_artifact_model::CallableMayEffects {
            writes_caller_reachable: false,
            returns_caller_alias: false,
            throws_caller_alias: false,
            escapes_caller_value: true,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_suspend: true,
        }
    );
    assert_eq!(
        NativeRequiredContext::for_binding_key(binding_key),
        Some(NativeRequiredContext::HttpResponseStream)
    );
    assert_eq!(
        runtime_shared_native_route_for_validation(binding_key, false),
        Some(RuntimeNativeRoute::Http)
    );
    validate_native_callable_semantics_registry(
        &[semantics],
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect("response stream emitter semantics must match its route and context");
}

#[test]
fn file_create_semantics_match_exact_signatures_and_file_routes() {
    for binding_key in ["std.file.create", "std.file.createFromStream"] {
        let semantics = STD_NATIVE_CALLABLE_SEMANTICS
            .iter()
            .find(|semantics| semantics.binding_key == binding_key)
            .cloned()
            .unwrap_or_else(|| panic!("{binding_key} must have exact callable semantics"));

        assert_eq!(
            semantics.return_provenance,
            skiff_artifact_model::ValueProvenance::Fresh
        );
        assert_eq!(
            semantics.effects,
            skiff_artifact_model::CallableMayEffects {
                writes_caller_reachable: false,
                returns_caller_alias: false,
                throws_caller_alias: false,
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_suspend: true,
            }
        );
        assert_eq!(
            NativeRequiredContext::for_binding_key(binding_key),
            Some(NativeRequiredContext::File)
        );
        assert_eq!(
            runtime_shared_native_route_for_validation(binding_key, false),
            Some(RuntimeNativeRoute::File)
        );
        validate_native_callable_semantics_registry(
            &[semantics],
            STD_NATIVE_SIGNATURES,
            NATIVE_BINDINGS,
        )
        .unwrap_or_else(|error| {
            panic!("{binding_key} should match its exact File capability route: {error}")
        });
    }
}

#[test]
fn file_create_semantics_reject_malformed_signatures_and_lookalikes() {
    for binding_key in ["std.file.create", "std.file.createFromStream"] {
        let semantics = STD_NATIVE_CALLABLE_SEMANTICS
            .iter()
            .find(|semantics| semantics.binding_key == binding_key)
            .cloned()
            .unwrap_or_else(|| panic!("{binding_key} must have exact callable semantics"));
        let canonical = *STD_NATIVE_SIGNATURES
            .iter()
            .find(|signature| signature.binding_key == binding_key)
            .unwrap_or_else(|| panic!("{binding_key} must have an exact signature"));

        for signature in [
            {
                let mut signature = canonical;
                signature.params = &[];
                signature
            },
            {
                let mut signature = canonical;
                signature.params = &[NativeSignatureTypeExpr::Builtin("string")];
                signature
            },
            {
                let mut signature = canonical;
                signature.return_type = NativeSignatureTypeExpr::Builtin("string");
                signature
            },
        ] {
            let error = validate_native_callable_semantics_registry(
                std::slice::from_ref(&semantics),
                &[signature],
                NATIVE_BINDINGS,
            )
            .expect_err("malformed file creation signature must fail closed");
            assert!(
                error.contains(binding_key)
                    && error.contains("does not match the exact shared native signature"),
                "unexpected error: {error}"
            );
        }

        let mut lookalike = semantics;
        lookalike.binding_key = if binding_key == "std.file.create" {
            "std.file.create.extra"
        } else {
            "std.file.createFromStreams"
        };
        let error = validate_native_callable_semantics_registry(
            &[lookalike],
            STD_NATIVE_SIGNATURES,
            NATIVE_BINDINGS,
        )
        .expect_err("non-canonical file creation lookalike must fail closed");
        assert!(
            error.contains("not in the exact audited registry"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn native_callable_semantics_registry_accepts_only_exact_context_free_http_request_helpers() {
    for binding_key in [
        "std.http.request.headers",
        "std.http.request.cookie",
        "std.http.stream.start",
        "std.http.stream.chunk",
        "std.http.stream.end",
    ] {
        assert!(native_route_matches_required_context(
            binding_key,
            NativeRequiredContext::None,
            RuntimeNativeRoute::Http,
        ));
    }

    for binding_key in [
        "std.http.request.header",
        "std.http.request.query",
        "std.http.request.decodeJson",
        "std.http.request.headers.extra",
        "std.http.request.cookie.extra",
        "std.http.stream.starts",
        "std.http.stream.start.extra",
        "std.http.stream.chunked",
        "std.http.stream.end.extra",
        "std.http.stream.emitResponse",
    ] {
        assert!(
            !native_route_matches_required_context(
                binding_key,
                NativeRequiredContext::None,
                RuntimeNativeRoute::Http,
            ),
            "{binding_key} must not acquire a route-wide or prefix exception"
        );
    }
}

#[test]
fn native_callable_semantics_registry_rejects_forged_http_helper_context_or_route() {
    for binding_key in [
        "std.http.request.headers",
        "std.http.request.cookie",
        "std.http.stream.start",
        "std.http.stream.chunk",
        "std.http.stream.end",
    ] {
        assert!(!native_route_matches_required_context(
            binding_key,
            NativeRequiredContext::HttpClient,
            RuntimeNativeRoute::Http,
        ));
        assert!(!native_route_matches_required_context(
            binding_key,
            NativeRequiredContext::None,
            RuntimeNativeRoute::Json,
        ));
    }
}

#[test]
fn native_callable_semantics_registry_keeps_contextful_http_route_matrix_strict() {
    for (binding_key, required_context) in [
        ("std.http.client.request", NativeRequiredContext::HttpClient),
        ("std.http.client.stream", NativeRequiredContext::HttpClient),
        ("std.http.client.sse", NativeRequiredContext::HttpClient),
        (
            "std.http.stream.emitResponse",
            NativeRequiredContext::HttpResponseStream,
        ),
    ] {
        assert!(native_route_matches_required_context(
            binding_key,
            required_context,
            RuntimeNativeRoute::Http,
        ));
        assert!(!native_route_matches_required_context(
            binding_key,
            NativeRequiredContext::None,
            RuntimeNativeRoute::Http,
        ));
    }
}

#[test]
fn native_callable_semantics_registry_rejects_http_registry_handler() {
    let semantics = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|semantics| semantics.binding_key == "std.http.client.request")
        .cloned()
        .expect("HTTP request must have exact callable semantics");
    let handler_entries = &[NativeHandlerEntry {
        binding_key: semantics.binding_key,
        handler: array_empty,
    }];
    let error = validate_native_callable_semantics_registry(
        &[semantics],
        STD_NATIVE_SIGNATURES,
        handler_entries,
    )
    .expect_err("HTTP client calls must not acquire a JSON registry handler");

    assert!(
        error.contains("std.http.client.request")
            && error.contains("expected 0 runtime registry handler(s), found 1"),
        "unexpected error: {error}"
    );
}

#[test]
fn http_client_stream_semantics_reject_malformed_signature_and_noncanonical_lookalike() {
    let semantics = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|semantics| semantics.binding_key == "std.http.client.stream")
        .cloned()
        .expect("HTTP client stream must have exact callable semantics");
    let canonical = STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == semantics.binding_key)
        .cloned()
        .expect("HTTP client stream must have an exact signature");

    for (case, signature) in [
        {
            let mut signature = canonical;
            signature.params = &[];
            ("missing request", signature)
        },
        {
            let mut signature = canonical;
            signature.params = &[skiff_artifact_model::NativeSignatureTypeExpr::Builtin(
                "string",
            )];
            ("wrong request", signature)
        },
        {
            let mut signature = canonical;
            signature.return_type =
                skiff_artifact_model::NativeSignatureTypeExpr::Builtin("string");
            ("wrong return", signature)
        },
    ] {
        let error = validate_native_callable_semantics_registry(
            std::slice::from_ref(&semantics),
            &[signature],
            NATIVE_BINDINGS,
        )
        .expect_err("malformed HTTP client stream signature must fail closed");
        assert!(
            error.contains("std.http.client.stream")
                && error.contains("does not match the exact shared native signature"),
            "{case}: unexpected error: {error}"
        );
    }

    let mut lookalike = semantics;
    lookalike.binding_key = "std.http.client.stream.extra";
    let error = validate_native_callable_semantics_registry(
        &[lookalike],
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect_err("non-canonical HTTP client stream lookalike must fail closed");
    assert!(
        error.contains("std.http.client.stream.extra")
            && error.contains("not in the exact audited registry"),
        "unexpected error: {error}"
    );
}

#[test]
fn http_client_sse_semantics_reject_malformed_signature_context_route_and_lookalike() {
    let semantics = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|semantics| semantics.binding_key == "std.http.client.sse")
        .cloned()
        .expect("HTTP client SSE must have exact callable semantics");
    let canonical = *STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == semantics.binding_key)
        .expect("HTTP client SSE must have an exact signature");

    for (case, signature) in [
        {
            let mut signature = canonical;
            signature.params = &[];
            ("missing request", signature)
        },
        {
            let mut signature = canonical;
            signature.params = &[NativeSignatureTypeExpr::Builtin("string")];
            ("wrong request", signature)
        },
        {
            let mut signature = canonical;
            signature.return_type = NativeSignatureTypeExpr::Builtin("string");
            ("wrong return", signature)
        },
    ] {
        let error = validate_native_callable_semantics_registry(
            std::slice::from_ref(&semantics),
            &[signature],
            NATIVE_BINDINGS,
        )
        .expect_err("malformed HTTP client SSE signature must fail closed");
        assert!(
            error.contains("std.http.client.sse")
                && error.contains("does not match the exact shared native signature"),
            "{case}: unexpected error: {error}"
        );
    }

    assert!(native_route_matches_required_context(
        semantics.binding_key,
        NativeRequiredContext::HttpClient,
        RuntimeNativeRoute::Http,
    ));
    assert!(!native_route_matches_required_context(
        semantics.binding_key,
        NativeRequiredContext::None,
        RuntimeNativeRoute::Http,
    ));
    assert!(!native_route_matches_required_context(
        semantics.binding_key,
        NativeRequiredContext::HttpClient,
        RuntimeNativeRoute::Json,
    ));

    let mut lookalike = semantics;
    lookalike.binding_key = "std.http.client.sse.extra";
    let error = validate_native_callable_semantics_registry(
        &[lookalike],
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect_err("non-canonical HTTP client SSE lookalike must fail closed");
    assert!(
        error.contains("std.http.client.sse.extra")
            && error.contains("not in the exact audited registry"),
        "unexpected error: {error}"
    );
}

#[test]
fn http_stream_event_constructor_semantics_require_exact_runtime_contracts() {
    for binding_key in [
        "std.http.stream.start",
        "std.http.stream.chunk",
        "std.http.stream.end",
    ] {
        let semantics = STD_NATIVE_CALLABLE_SEMANTICS
            .iter()
            .find(|semantics| semantics.binding_key == binding_key)
            .cloned()
            .unwrap_or_else(|| panic!("{binding_key} must have exact callable semantics"));
        let canonical = *STD_NATIVE_SIGNATURES
            .iter()
            .find(|signature| signature.binding_key == binding_key)
            .unwrap_or_else(|| panic!("{binding_key} must have an exact signature"));

        assert_eq!(
            NativeRequiredContext::for_binding_key(binding_key),
            Some(NativeRequiredContext::None)
        );
        assert_eq!(
            runtime_shared_native_route_for_validation(binding_key, false),
            Some(RuntimeNativeRoute::Http)
        );
        validate_native_callable_semantics_registry(
            std::slice::from_ref(&semantics),
            STD_NATIVE_SIGNATURES,
            NATIVE_BINDINGS,
        )
        .unwrap_or_else(|error| panic!("{binding_key} should validate: {error}"));

        for (case, signature) in [
            {
                let mut signature = canonical;
                signature.params = &[];
                ("arity", signature)
            },
            {
                let mut signature = canonical;
                signature.return_type = NativeSignatureTypeExpr::Builtin("string");
                ("return", signature)
            },
        ] {
            if signature == canonical {
                continue;
            }
            let error = validate_native_callable_semantics_registry(
                std::slice::from_ref(&semantics),
                &[signature],
                NATIVE_BINDINGS,
            )
            .expect_err("malformed constructor signature must fail closed");
            assert!(
                error.contains(binding_key)
                    && error.contains("does not match the exact shared native signature"),
                "{case}: unexpected error: {error}"
            );
        }
    }

    for lookalike in [
        "std.http.stream.start.extra",
        "std.http.stream.chunked",
        "std.http.stream.end.extra",
    ] {
        let error = validate_native_callable_semantics_registry(
            &[detached_native_semantics(lookalike)],
            STD_NATIVE_SIGNATURES,
            NATIVE_BINDINGS,
        )
        .expect_err("non-canonical constructor lookalike must fail closed");
        assert!(
            error.contains(lookalike) && error.contains("not in the exact audited registry"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn response_stream_emit_semantics_reject_malformed_signature_and_lookalikes() {
    let binding_key = "std.http.stream.emitResponse";
    let semantics = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|semantics| semantics.binding_key == binding_key)
        .cloned()
        .expect("response stream emitter must have exact callable semantics");
    let canonical = *STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == binding_key)
        .expect("response stream emitter must have an exact signature");

    for signature in [
        {
            let mut signature = canonical;
            signature.params = &[];
            signature
        },
        {
            let mut signature = canonical;
            signature.params = &[NativeSignatureTypeExpr::Builtin("Json")];
            signature
        },
        {
            let mut signature = canonical;
            signature.return_type = NativeSignatureTypeExpr::Builtin("string");
            signature
        },
    ] {
        let error = validate_native_callable_semantics_registry(
            std::slice::from_ref(&semantics),
            &[signature],
            NATIVE_BINDINGS,
        )
        .expect_err("malformed response stream emitter signature must fail closed");
        assert!(
            error.contains(binding_key)
                && error.contains("does not match the exact shared native signature"),
            "unexpected error: {error}"
        );
    }

    for lookalike in [
        "std.http.emitResponseStream",
        "std.http.stream.emitResponses",
        "std.http.stream.emitResponse.extra",
    ] {
        let mut lookalike_semantics = semantics.clone();
        lookalike_semantics.binding_key = lookalike;
        let error = validate_native_callable_semantics_registry(
            &[lookalike_semantics],
            STD_NATIVE_SIGNATURES,
            NATIVE_BINDINGS,
        )
        .expect_err("non-canonical response stream emitter lookalike must fail closed");
        assert!(
            error.contains(lookalike) && error.contains("not in the exact audited registry"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn native_callable_semantics_registry_rejects_websocket_registry_handler() {
    let binding_key = "std.websocket.sendTextToConnection";
    let semantics = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|semantics| semantics.binding_key == binding_key)
        .expect("websocket send should have audited callable semantics");
    let handler_entries = &[NativeHandlerEntry {
        binding_key,
        handler: array_empty,
    }];
    let error = validate_native_callable_semantics_registry(
        std::slice::from_ref(semantics),
        STD_NATIVE_SIGNATURES,
        handler_entries,
    )
    .expect_err("websocket route must not use a context-free registry handler");

    assert!(
        error.contains(binding_key)
            && error.contains("expected 0 runtime registry handler(s), found 1"),
        "unexpected error: {error}"
    );
}

#[test]
fn native_callable_semantics_registry_rejects_unaudited_file_capability_descriptor() {
    let error = validate_native_callable_semantics_registry(
        &[detached_native_semantics("std.file.readText")],
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
    .expect_err("unaudited file descriptor should fail validation");

    assert!(
        error.contains("std.file.readText") && error.contains("not in the exact audited registry"),
        "unexpected error: {error}"
    );
}

#[test]
fn native_callable_semantics_registry_rejects_missing_handler() {
    let error = validate_native_callable_semantics_registry(
        &[detached_native_semantics("std.string.truncateUtf8Bytes")],
        STD_NATIVE_SIGNATURES,
        &[],
    )
    .expect_err("descriptor without runtime handler should fail validation");

    assert!(
        error.contains("std.string.truncateUtf8Bytes")
            && error.contains("expected 1 runtime registry handler(s), found 0"),
        "unexpected error: {error}"
    );
}

#[test]
fn receiver_callable_semantics_registry_rejects_registry_handler() {
    let semantics = BUILTIN_RECEIVER_CALLABLE_SEMANTICS[0].clone();
    let handlers = &[NativeHandlerEntry {
        binding_key: semantics.op.canonical_key,
        handler: array_empty,
    }];
    let error = validate_receiver_callable_semantics_registry(
        &[semantics],
        STD_NATIVE_SIGNATURES,
        handlers,
    )
    .expect_err("receiver methods must remain on receiver dispatch");

    assert!(
        error.contains("expected 0 runtime registry handler(s), found 1"),
        "unexpected error: {error}"
    );
}

#[test]
fn native_callable_semantics_registry_keeps_unaudited_non_websocket_capabilities_unregistered() {
    for binding_key in [
        "std.file.readText",
        "std.telemetry.emit",
        "std.resource.text",
    ] {
        assert!(STD_NATIVE_CALLABLE_SEMANTICS
            .iter()
            .all(|semantics| semantics.binding_key != binding_key));
    }
}

#[test]
fn native_handler_registry_rejects_unknown_binding_key() {
    let entries = &[NativeHandlerEntry {
        binding_key: "std.unknown.handler",
        handler: array_empty,
    }];

    let error = validate_handler_entries(entries, &[])
        .expect_err("unknown binding key should fail validation");

    assert!(
        error.contains("std.unknown.handler") && error.contains("NativeSignatureRegistry"),
        "unexpected error: {error}"
    );
}

#[test]
fn native_handler_registry_rejects_duplicate_binding_key() {
    let entries = &[
        NativeHandlerEntry {
            binding_key: "core.array.empty",
            handler: array_empty,
        },
        NativeHandlerEntry {
            binding_key: "core.array.empty",
            handler: array_empty,
        },
    ];

    let error = validate_handler_entries(entries, &[])
        .expect_err("duplicate binding key should fail validation");

    assert!(
        error.contains("core.array.empty") && error.contains("more than once"),
        "unexpected error: {error}"
    );
}

#[test]
fn native_handler_registry_rejects_missing_required_handler() {
    let entries = &[NativeHandlerEntry {
        binding_key: "core.array.empty",
        handler: array_empty,
    }];

    let error = validate_handler_entries(entries, &["core.array.empty", "core.map.empty"])
        .expect_err("missing required handler should fail validation");

    assert!(
        error.contains("core.map.empty") && error.contains("missing required handler"),
        "unexpected error: {error}"
    );
}

#[test]
fn ext_llm_error_helpers_are_not_runtime_natives() {
    for target in [
        "llm.failDecode",
        "llm.failProviderUnavailable",
        "llm.failProtocol",
        "llm.parseSseJson",
    ] {
        assert!(
            NATIVE_BINDINGS
                .iter()
                .all(|binding| binding.binding_key != target),
            "{target} should not be a runtime native"
        );
    }
}

#[test]
fn native_registry_has_no_ext_targets() {
    assert!(
        NATIVE_BINDINGS
            .iter()
            .all(|binding| !binding.binding_key.starts_with("ext.")),
        "removed ext package functions must not be runtime natives"
    );
}

#[test]
fn native_registry_exposes_current_std_crypto_and_date_targets() {
    let registry = NativeRegistry;

    for target in [
        "core.date.now",
        "core.date.fromEpochMilliseconds",
        "core.date.parse",
        "core.date.requireParse",
        "core.duration.milliseconds",
        "core.duration.seconds",
        "core.duration.toMilliseconds",
        "std.crypto.hmacSha1Base64",
        "std.crypto.sha256",
        "std.crypto.randomToken",
        "std.crypto.uuid",
        "std.crypto.uuidSimple",
    ] {
        assert!(
            registry.is_registered(target),
            "{target} should be registered"
        );
    }

    for target in [
        "Date.now",
        "Date.fromEpochMilliseconds",
        "Date.parse",
        "Date.requireParse",
        "std.time.nowEpochMilliseconds",
        "std.time.nowEpochSeconds",
        "std.time.nowUnixSeconds",
        "std.time.nowHttpDate",
        "std.time.sleep",
        "std.collection.Array.empty",
        "std.array.empty",
        "std.collection.Map.empty",
        "std.map.empty",
        "std.collection.Set.empty",
        "Set.empty",
        "std.set.empty",
        "platform.crypto.randomToken",
        "platform.crypto.sha256",
        "platform.time.nowMs",
        "platform.id.uuid",
        "platform.json.parse",
        "platform.json.stringify",
    ] {
        assert!(
            !registry.is_registered(target),
            "{target} should not be registered"
        );
    }
}

#[test]
fn native_registry_exposes_only_json_codecs_for_std_json() {
    let registry = NativeRegistry;

    assert!(registry.is_registered("std.json.encode"));
    assert!(registry.is_registered("std.json.decode"));
    assert!(registry.is_registered("std.json.merge"));

    for target in [
        "std.json.parse",
        "std.json.stringify",
        "std.json.from",
        "std.json.get",
        "std.json.at",
        "std.json.asString",
        "std.json.asNumber",
        "std.json.asBool",
        "std.json.asArray",
    ] {
        assert!(
            !registry.is_registered(target),
            "{target} should not be registered"
        );
    }
}

#[test]
fn native_registry_exposes_current_std_string_targets() {
    let registry = NativeRegistry;

    for target in [
        "std.string.join",
        "std.string.split",
        "std.string.isAsciiDigits",
        "std.string.truncateUtf8Bytes",
        "std.string.encodeQueryComponent",
        "std.string.encodePath",
    ] {
        assert!(
            registry.is_registered(target),
            "{target} should be registered"
        );
    }
}

#[test]
fn std_bytes_targets_are_not_json_registry_natives() {
    let registry = NativeRegistry;

    for target in [
        "core.bytes.fromBase64",
        "std.bytes.fromBase64",
        "bytes.fromBase64",
        "core.bytes.fromHex",
        "std.bytes.fromHex",
        "bytes.fromHex",
        "core.bytes.fromUtf8",
        "std.bytes.fromUtf8",
        "bytes.fromUtf8",
        "core.bytes.concat",
        "std.bytes.concat",
        "bytes.concat",
    ] {
        assert!(
            !registry.is_registered(target),
            "{target} should be handled by RuntimeProgram dispatch, not the JSON native registry"
        );
        assert!(
            registry
                .dispatch(target, &[json!({ "__skiffBytesBase64": "YQ==" })])
                .expect("unregistered bytes native should not error")
                .is_none(),
            "{target} should not accept JSON bytes shims"
        );
    }
}

#[test]
fn std_crypto_native_targets_dispatch() {
    let registry = NativeRegistry;

    let hash = registry
        .dispatch("std.crypto.sha256", &[json!("hello")])
        .expect("sha256 should dispatch")
        .expect("sha256 should be registered");
    assert_eq!(
        hash,
        json!("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
    );

    let token = registry
        .dispatch("std.crypto.randomToken", &[])
        .expect("randomToken should dispatch")
        .expect("randomToken should be registered");
    let token = token.as_str().expect("randomToken should return a string");
    assert_eq!(token.len(), 64);
    assert!(token
        .chars()
        .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase()));

    let uuid = registry
        .dispatch("std.crypto.uuid", &[])
        .expect("uuid should dispatch")
        .expect("uuid should be registered");
    uuid::Uuid::parse_str(uuid.as_str().expect("uuid should return a string"))
        .expect("uuid should return a valid UUID");

    let uuid_simple = registry
        .dispatch("std.crypto.uuidSimple", &[])
        .expect("uuidSimple should dispatch")
        .expect("uuidSimple should be registered");
    let uuid_simple = uuid_simple
        .as_str()
        .expect("uuidSimple should return a string");
    assert_eq!(uuid_simple.len(), 32);
    assert!(uuid_simple
        .chars()
        .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase()));
    assert!(!uuid_simple.contains('-'));
    uuid::Uuid::parse_str(uuid_simple).expect("uuidSimple should return a valid UUID");
}

#[test]
fn date_native_targets_dispatch() {
    let registry = NativeRegistry;

    let now = registry
        .dispatch("core.date.now", &[])
        .expect("Date.now should dispatch")
        .expect("Date.now should be registered");
    assert!(
        now.as_str().is_some_and(|value| value.ends_with('Z')),
        "Date.now should return an RFC3339 string"
    );

    let epoch = registry
        .dispatch("core.date.fromEpochMilliseconds", &[json!(0)])
        .expect("Date.fromEpochMilliseconds should dispatch")
        .expect("Date.fromEpochMilliseconds should be registered");
    assert_eq!(epoch, json!("1970-01-01T00:00:00.000Z"));

    let parsed = registry
        .dispatch("core.date.parse", &[json!("2026-06-04T23:12:03.456+08:00")])
        .expect("Date.parse should dispatch")
        .expect("Date.parse should be registered");
    assert_eq!(parsed, json!("2026-06-04T15:12:03.456Z"));

    let invalid = registry
        .dispatch("core.date.parse", &[json!("not a date")])
        .expect("Date.parse should handle invalid strings")
        .expect("Date.parse should be registered");
    assert_eq!(invalid, json!(null));

    let leap_second = registry
        .dispatch("core.date.parse", &[json!("2016-12-31T23:59:60Z")])
        .expect("Date.parse should reject leap seconds without error")
        .expect("Date.parse should be registered");
    assert_eq!(leap_second, json!(null));

    let required = registry
        .dispatch("core.date.requireParse", &[json!("1970-01-01T00:00:00Z")])
        .expect("Date.requireParse should dispatch")
        .expect("Date.requireParse should be registered");
    assert_eq!(required, json!("1970-01-01T00:00:00.000Z"));

    let error = registry
        .dispatch("core.date.requireParse", &[json!("not a date")])
        .expect_err("Date.requireParse should error on invalid strings");
    assert_decode_target_projection(
        error,
        "Date.requireParse",
        "std.time.DecodeError",
        PlatformBuiltinErrorIdentity::TimeDecode,
        "requires RFC3339 Date",
    );
}

#[test]
fn map_empty_runtime_values_are_independent() {
    let mut first = map_empty(&[]).expect("first Map.empty should succeed");
    let second = map_empty(&[]).expect("second Map.empty should succeed");

    first
        .as_object_mut()
        .expect("Map.empty should return an object")
        .insert("event".to_string(), json!("completed"));

    assert_eq!(first, json!({ "event": "completed" }));
    assert_eq!(second, json!({}));
}

#[test]
fn duration_native_targets_dispatch_erased_milliseconds() {
    let registry = NativeRegistry;

    let millis = registry
        .dispatch("core.duration.milliseconds", &[json!(2_000)])
        .expect("Duration.milliseconds should dispatch")
        .expect("Duration.milliseconds should be registered");
    assert_eq!(millis, json!(2_000));

    let seconds = registry
        .dispatch("core.duration.seconds", &[json!(2)])
        .expect("Duration.seconds should dispatch")
        .expect("Duration.seconds should be registered");
    assert_eq!(seconds, json!(2_000));

    let projected = registry
        .dispatch("core.duration.toMilliseconds", &[seconds])
        .expect("Duration.toMilliseconds should dispatch")
        .expect("Duration.toMilliseconds should be registered");
    assert_eq!(projected, json!(2_000));

    let error = registry
        .dispatch("core.duration.seconds", &[json!(9_007_199_254_741_i64)])
        .expect_err("Duration.seconds overflow must fail safe-integer validation");
    assert_decode_target_projection(
        error,
        "Duration.seconds",
        "std.time.DecodeError",
        PlatformBuiltinErrorIdentity::TimeDecode,
        "safe integer",
    );
}

#[test]
fn std_number_safe_integer_natives_dispatch() {
    let registry = NativeRegistry;

    let value = registry
        .dispatch("core.number.isInteger", &[json!(2.5)])
        .expect("isInteger should dispatch")
        .expect("isInteger should be registered");
    assert_eq!(value, json!(false));

    assert!(
        !registry.is_registered("number.isInteger"),
        "public number.isInteger alias should not be a native registry binding key"
    );

    let value = registry
        .dispatch(
            "core.number.isSafeInteger",
            &[json!(9_007_199_254_740_991.0)],
        )
        .expect("isSafeInteger should dispatch")
        .expect("isSafeInteger should be registered");
    assert_eq!(value, json!(true));

    let value = registry
        .dispatch(
            "core.number.isSafeInteger",
            &[json!(9_007_199_254_740_992.0)],
        )
        .expect("isSafeInteger should dispatch")
        .expect("isSafeInteger should be registered");
    assert_eq!(value, json!(false));

    let value = registry
        .dispatch("core.number.assertSafeInteger", &[json!(2.0)])
        .expect("assertSafeInteger should dispatch")
        .expect("assertSafeInteger should be registered");
    assert_eq!(value.to_string(), "2");

    let error = registry
        .dispatch("core.number.assertSafeInteger", &[json!(2.5)])
        .expect_err("fractional values must fail");
    assert_decode_target_projection(
        error,
        "number.assertSafeInteger",
        "std.number.DecodeError",
        PlatformBuiltinErrorIdentity::NumberDecode,
        "safe integer",
    );
}

#[test]
fn std_json_merge_overlays_object_fields() {
    let registry = NativeRegistry;

    // Overlay overrides same-named keys, keeps base-only keys, appends overlay-only
    // keys. Insertion order: base keys keep position, overlay-only keys are appended.
    let merged = registry
        .dispatch(
            "std.json.merge",
            &[
                json!({ "name": "Ada", "keep": 1 }),
                json!({ "name": "Lovelace", "added": true }),
            ],
        )
        .expect("std.json.merge should dispatch")
        .expect("std.json.merge should be registered");
    assert_eq!(
        merged,
        json!({ "name": "Lovelace", "keep": 1, "added": true })
    );
    assert_eq!(
        merged.to_string(),
        r#"{"name":"Lovelace","keep":1,"added":true}"#,
        "merge should preserve base insertion order with overlay-only keys appended"
    );

    // Overlay null value is kept as null (it does not delete the key).
    let with_null = registry
        .dispatch(
            "std.json.merge",
            &[json!({ "a": 1, "b": 2 }), json!({ "b": null })],
        )
        .expect("std.json.merge should dispatch")
        .expect("std.json.merge should be registered");
    assert_eq!(with_null, json!({ "a": 1, "b": null }));

    // Empty overlay leaves base unchanged.
    let empty_overlay = registry
        .dispatch("std.json.merge", &[json!({ "a": 1 }), json!({})])
        .expect("std.json.merge should dispatch")
        .expect("std.json.merge should be registered");
    assert_eq!(empty_overlay, json!({ "a": 1 }));
}

#[test]
fn std_json_merge_handles_non_object_arguments() {
    let registry = NativeRegistry;

    // Null overlay returns base unchanged.
    let null_overlay = registry
        .dispatch("std.json.merge", &[json!({ "a": 1 }), json!(null)])
        .expect("std.json.merge should dispatch")
        .expect("std.json.merge should be registered");
    assert_eq!(null_overlay, json!({ "a": 1 }));

    // Non-null, non-object overlay replaces base entirely.
    let scalar_overlay = registry
        .dispatch("std.json.merge", &[json!({ "a": 1 }), json!(42)])
        .expect("std.json.merge should dispatch")
        .expect("std.json.merge should be registered");
    assert_eq!(scalar_overlay, json!(42));

    let array_overlay = registry
        .dispatch("std.json.merge", &[json!({ "a": 1 }), json!([1, 2])])
        .expect("std.json.merge should dispatch")
        .expect("std.json.merge should be registered");
    assert_eq!(array_overlay, json!([1, 2]));

    // Non-object base with object overlay: overlay replaces base.
    let non_object_base = registry
        .dispatch("std.json.merge", &[json!(7), json!({ "a": 1 })])
        .expect("std.json.merge should dispatch")
        .expect("std.json.merge should be registered");
    assert_eq!(non_object_base, json!({ "a": 1 }));
}

#[test]
fn std_json_merge_detaches_nested_values_from_both_inputs() {
    let registry = NativeRegistry;
    let mut base = json!({
        "baseOnly": { "items": [1] },
        "overridden": { "source": "base" }
    });
    let mut overlay = json!({
        "overridden": { "source": "overlay", "items": [2] },
        "overlayOnly": { "items": [3] }
    });
    let mut merged = registry
        .dispatch("std.json.merge", &[base.clone(), overlay.clone()])
        .expect("std.json.merge should dispatch")
        .expect("std.json.merge should be registered");

    base["baseOnly"]["items"][0] = json!(10);
    overlay["overridden"]["items"][0] = json!(20);
    overlay["overlayOnly"]["items"][0] = json!(30);
    assert_eq!(
        merged,
        json!({
            "baseOnly": { "items": [1] },
            "overridden": { "source": "overlay", "items": [2] },
            "overlayOnly": { "items": [3] }
        })
    );

    merged["baseOnly"]["items"][0] = json!(100);
    merged["overridden"]["items"][0] = json!(200);
    merged["overlayOnly"]["items"][0] = json!(300);
    assert_eq!(base["baseOnly"]["items"], json!([10]));
    assert_eq!(overlay["overridden"]["items"], json!([20]));
    assert_eq!(overlay["overlayOnly"]["items"], json!([30]));
}

#[test]
fn std_string_truncate_utf8_bytes_dispatches() {
    let registry = NativeRegistry;

    for (value, max_bytes, expected) in [
        ("abcdef", 3, "abc"),
        ("hello", 5, "hello"),
        ("hello", 99, "hello"),
        ("你好", 4, "你"),
        ("你好", 2, ""),
        ("a🙂b", 4, "a"),
        ("a🙂b", 5, "a🙂"),
        ("abc", 0, ""),
        ("abc", -1, ""),
    ] {
        let actual = registry
            .dispatch(
                "std.string.truncateUtf8Bytes",
                &[json!(value), json!(max_bytes)],
            )
            .expect("truncateUtf8Bytes should dispatch")
            .expect("truncateUtf8Bytes should be registered");
        assert_eq!(actual, json!(expected));
    }
}

#[test]
fn std_string_truncate_utf8_bytes_rejects_invalid_limit() {
    let registry = NativeRegistry;

    for invalid in [json!(2.5), json!("2"), json!(null)] {
        let error = registry
            .dispatch("std.string.truncateUtf8Bytes", &[json!("abcdef"), invalid])
            .expect_err("invalid maxBytes should fail");
        assert!(
            error.to_string().contains("truncateUtf8Bytes maxBytes"),
            "unexpected error: {error}"
        );
    }
}
