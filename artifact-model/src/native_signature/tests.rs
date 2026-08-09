use std::collections::BTreeSet;

use crate::{builtin_receiver_op_by_name, CallableMayEffects, PendingEffectCategory, ValueProvenance};

use super::{
    is_runtime_receiver_native_binding_key, native_callable_semantics,
    native_signature_for_receiver_op, STD_NATIVE_CALLABLE_SEMANTICS, STD_NATIVE_SIGNATURES, STRING,
    T0, T1,
};

#[test]
fn websocket_request_signature_and_suspension_are_exact() {
    let request = STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == "std.websocket.requestJsonToConnection")
        .expect("WebSocket request native signature must be registered");
    assert_eq!(request.target, "std.websocket.requestJsonToConnection");
    assert!(request.aliases.is_empty());
    assert_eq!(request.type_param_count, 2);
    assert_eq!(request.params, &[STRING, STRING, T0]);
    assert_eq!(request.return_type, T1);

    let request_semantics = native_callable_semantics("std.websocket.requestJsonToConnection")
        .expect("WebSocket request callable semantics must be registered");
    assert!(request_semantics.effects.may_pending);
    assert_eq!(
        request_semantics.effects.pending_effect_categories,
        vec![PendingEffectCategory::NativeCall]
    );
    assert!(!request_semantics.effects.escapes_caller_value);
    assert!(!request_semantics.effects.requires_same_heap_identity);
    assert!(!request_semantics.effects.invokes_unknown_target);

    for raw_send in [
        "std.websocket.sendTextToConnection",
        "std.websocket.sendBinaryToConnection",
        "std.websocket.sendTextToBusinessIdentity",
        "std.websocket.sendBinaryToBusinessIdentity",
    ] {
        assert!(
            !native_callable_semantics(raw_send)
                .expect("raw WebSocket send semantics must remain registered")
                .effects
                .may_pending,
            "{raw_send} must remain non-pending"
        );
        assert!(
            native_callable_semantics(raw_send)
                .expect("raw WebSocket send semantics must remain registered")
                .effects
                .pending_effect_categories
                .is_empty(),
            "{raw_send} must have no pending categories"
        );
    }
}

#[test]
fn native_callable_semantics_registry_is_sparse_exact_and_safe() {
    let expected = BTreeSet::from([
        "std.actor.get",
        "core.array.empty",
        "core.map.empty",
        "core.bytes.concat",
        "core.bytes.fromBase64",
        "core.bytes.fromHex",
        "core.bytes.fromUtf8",
        "core.date.fromEpochMilliseconds",
        "core.date.now",
        "core.date.parse",
        "core.duration.milliseconds",
        "core.duration.seconds",
        "core.number.parse",
        "core.number.assertSafeInteger",
        "std.crypto.hmacSha1Base64",
        "std.crypto.randomToken",
        "std.crypto.sha256",
        "std.crypto.uuid",
        "std.crypto.uuidSimple",
        "std.file.create",
        "std.file.createFromStream",
        "std.http.client.request",
        "std.http.client.sse",
        "std.http.client.stream",
        "std.http.request.cookie",
        "std.http.request.headers",
        "std.http.stream.chunk",
        "std.http.stream.end",
        "std.http.stream.emitResponse",
        "std.http.stream.start",
        "std.json.encode",
        "std.json.decode",
        "std.json.merge",
        "std.json.get",
        "std.json.getString",
        "std.json.getNumber",
        "std.json.getBool",
        "std.json.getArray",
        "std.string.join",
        "std.string.split",
        "std.string.encodePath",
        "std.string.encodeQueryComponent",
        "std.string.isAsciiDigits",
        "std.string.truncateUtf8Bytes",
        "std.task.cancel",
        "std.task.status",
        "std.time.sleep",
        "std.websocket.sendBinaryToBusinessIdentity",
        "std.websocket.sendBinaryToConnection",
        "std.websocket.sendTextToBusinessIdentity",
        "std.websocket.sendTextToConnection",
        "std.websocket.requestJsonToConnection",
    ]);
    let actual = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .map(|semantics| semantics.binding_key)
        .collect::<BTreeSet<_>>();

    assert_eq!(actual, expected);
    assert_eq!(actual.len(), STD_NATIVE_CALLABLE_SEMANTICS.len());
    for semantics in STD_NATIVE_CALLABLE_SEMANTICS.iter() {
        assert!(STD_NATIVE_SIGNATURES
            .iter()
            .any(|signature| signature.binding_key == semantics.binding_key));
        let is_emit_response = semantics.binding_key == "std.http.stream.emitResponse";
        assert_eq!(semantics.effects.escapes_caller_value, is_emit_response);
        assert_eq!(semantics.effects.requires_same_heap_identity, false);
        assert_eq!(semantics.effects.invokes_unknown_target, false);
        let is_pending = is_emit_response
            || matches!(
                semantics.binding_key,
                "std.actor.get"
                    | "std.file.create"
                    | "std.file.createFromStream"
                    | "std.http.client.request"
                    | "std.http.client.sse"
                    | "std.http.client.stream"
                    | "std.task.cancel"
                    | "std.task.status"
                    | "std.time.sleep"
                    | "std.websocket.requestJsonToConnection"
            );
        assert_eq!(semantics.effects.may_pending, is_pending);
        assert_eq!(
            semantics.effects.pending_effect_categories,
            if is_pending {
                vec![PendingEffectCategory::NativeCall]
            } else {
                Vec::new()
            }
        );
        assert_eq!(semantics.return_provenance, ValueProvenance::Fresh);
        assert_eq!(
            native_callable_semantics(semantics.binding_key),
            Some(semantics)
        );
    }

    for missing in [
        "core.date.fromEpoch",
        "core.date.fromEpochMilliseconds.custom",
        "std.file.readText",
        "std.http.request.header",
        "std.http.request.query",
        "std.http.response.json",
        "std.http.stream.start.extra",
        "std.http.stream.chunked",
        "std.http.stream.ending",
        "custom.native",
        "std.json.merged",
    ] {
        assert_eq!(native_callable_semantics(missing), None, "{missing}");
    }
}

#[test]
fn json_merge_semantics_are_exact_fresh_and_detached() {
    let semantics = native_callable_semantics("std.json.merge")
        .expect("audited std.json.merge semantics should be registered");
    assert_eq!(
        semantics.effects,
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: false,
            pending_effect_categories: if false { vec![PendingEffectCategory::NativeCall] } else { Vec::new() },
            inout_path_effects: Vec::new(),        }
    );
    assert_eq!(semantics.return_provenance, ValueProvenance::Fresh);

    for lookalike in [
        "json.merge",
        "std.json.merged",
        "std.json.merge.custom",
        "platform.json.merge",
    ] {
        assert_eq!(native_callable_semantics(lookalike), None, "{lookalike}");
    }
}

#[test]
fn date_from_epoch_milliseconds_semantics_match_exact_signature() {
    let semantics = native_callable_semantics("core.date.fromEpochMilliseconds")
        .expect("audited Date constructor should have exact semantics");
    assert_eq!(
        semantics.effects,
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: false,
            pending_effect_categories: if false { vec![PendingEffectCategory::NativeCall] } else { Vec::new() },
            inout_path_effects: Vec::new(),        }
    );
    assert_eq!(semantics.return_provenance, ValueProvenance::Fresh);

    let signature = STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == semantics.binding_key)
        .expect("audited Date constructor should have a native signature");
    assert_eq!(signature.params, &[super::INTEGER]);
    assert_eq!(signature.return_type, super::DATE);
}

#[test]
fn map_empty_semantics_match_exact_generic_signature() {
    let semantics = native_callable_semantics("core.map.empty")
        .expect("audited Map.empty constructor should have exact semantics");
    assert_eq!(
        semantics.effects,
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: false,
            pending_effect_categories: if false { vec![PendingEffectCategory::NativeCall] } else { Vec::new() },
            inout_path_effects: Vec::new(),        }
    );
    assert_eq!(semantics.return_provenance, ValueProvenance::Fresh);

    let signature = STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == semantics.binding_key)
        .expect("audited Map.empty constructor should have a native signature");
    assert_eq!(signature.target, "Map.empty");
    assert!(signature.aliases.is_empty());
    assert_eq!(signature.type_param_count, 2);
    assert!(signature.params.is_empty());
    assert_eq!(
        signature.return_type,
        super::NativeSignatureTypeExpr::Map(&super::T0, &super::T1)
    );

    for near_miss in [
        "core.map.empty.custom",
        "Map.empty",
        "std.map.empty",
        "core.map.empt",
    ] {
        assert_eq!(
            native_callable_semantics(near_miss),
            None,
            "{near_miss} must not inherit exact callable semantics"
        );
    }
}

#[test]
fn date_parse_semantics_match_exact_signature() {
    let semantics = native_callable_semantics("core.date.parse")
        .expect("audited Date parser should have exact semantics");
    assert_eq!(
        semantics.effects,
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: false,
            pending_effect_categories: if false { vec![PendingEffectCategory::NativeCall] } else { Vec::new() },
            inout_path_effects: Vec::new(),        }
    );
    assert_eq!(semantics.return_provenance, ValueProvenance::Fresh);

    let signature = STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == semantics.binding_key)
        .expect("audited Date parser should have a native signature");
    assert_eq!(signature.target, "Date.parse");
    assert!(signature.aliases.is_empty());
    assert_eq!(signature.params, &[super::STRING]);
    assert_eq!(signature.return_type, super::DATE_NULLABLE);

    for near_miss in [
        "core.date.parse.custom",
        "Date.parse",
        "std.date.parse",
        "core.date.requireParse",
    ] {
        assert_eq!(
            native_callable_semantics(near_miss),
            None,
            "{near_miss} must not inherit exact callable semantics"
        );
    }
}

#[test]
fn bytes_from_base64_semantics_match_exact_signature() {
    let semantics = native_callable_semantics("core.bytes.fromBase64")
        .expect("audited Base64 decoder should have exact semantics");
    assert_eq!(
        semantics.effects,
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: false,
            pending_effect_categories: if false { vec![PendingEffectCategory::NativeCall] } else { Vec::new() },
            inout_path_effects: Vec::new(),        }
    );
    assert_eq!(semantics.return_provenance, ValueProvenance::Fresh);

    let signature = STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == semantics.binding_key)
        .expect("audited Base64 decoder should have a native signature");
    assert_eq!(signature.target, "std.bytes.fromBase64");
    assert_eq!(signature.aliases, &["bytes.fromBase64"]);
    assert_eq!(signature.params, &[super::STRING]);
    assert_eq!(signature.return_type, super::BYTES);

    for near_miss in [
        "core.bytes.fromBase64.custom",
        "std.bytes.fromBase64",
        "bytes.fromBase64",
        "core.bytes.fromBase64Url",
    ] {
        assert_eq!(
            native_callable_semantics(near_miss),
            None,
            "{near_miss} must not inherit exact callable semantics"
        );
    }
}

#[test]
fn bytes_from_hex_semantics_match_exact_signature() {
    let semantics = native_callable_semantics("core.bytes.fromHex")
        .expect("audited hex decoder should have exact semantics");
    assert_eq!(
        semantics.effects,
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: false,
            pending_effect_categories: if false { vec![PendingEffectCategory::NativeCall] } else { Vec::new() },
            inout_path_effects: Vec::new(),        }
    );
    assert_eq!(semantics.return_provenance, ValueProvenance::Fresh);

    let signature = STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == semantics.binding_key)
        .expect("audited hex decoder should have a native signature");
    assert_eq!(signature.target, "std.bytes.fromHex");
    assert_eq!(signature.aliases, &["bytes.fromHex"]);
    assert_eq!(signature.params, &[super::STRING]);
    assert_eq!(signature.return_type, super::BYTES);

    for near_miss in [
        "core.bytes.fromHex.custom",
        "std.bytes.fromHex",
        "bytes.fromHex",
        "core.bytes.fromHEX",
    ] {
        assert_eq!(
            native_callable_semantics(near_miss),
            None,
            "{near_miss} must not inherit exact callable semantics"
        );
    }
}

#[test]
fn bytes_concat_semantics_match_exact_signature() {
    let semantics = native_callable_semantics("core.bytes.concat")
        .expect("audited bytes concatenation should have exact semantics");
    assert_eq!(
        semantics.effects,
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: false,
            pending_effect_categories: if false { vec![PendingEffectCategory::NativeCall] } else { Vec::new() },
            inout_path_effects: Vec::new(),        }
    );
    assert_eq!(semantics.return_provenance, ValueProvenance::Fresh);

    let signature = STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == semantics.binding_key)
        .expect("audited bytes concatenation should have a native signature");
    assert_eq!(signature.target, "std.bytes.concat");
    assert_eq!(signature.aliases, &["bytes.concat"]);
    assert_eq!(signature.params, &[super::BYTES_ARRAY]);
    assert_eq!(signature.return_type, super::BYTES);

    for near_miss in [
        "core.bytes.concat.custom",
        "std.bytes.concat",
        "bytes.concat",
        "core.array.concat",
    ] {
        assert_eq!(
            native_callable_semantics(near_miss),
            None,
            "{near_miss} must not inherit exact callable semantics"
        );
    }
}

#[test]
fn http_request_native_semantics_match_exact_signatures() {
    for (binding_key, return_type) in [
        (
            "std.http.request.headers",
            super::NativeSignatureTypeExpr::Array(&super::STRING),
        ),
        ("std.http.request.cookie", super::STRING_NULLABLE),
    ] {
        let semantics = native_callable_semantics(binding_key)
            .expect("audited HTTP request binding should have exact semantics");
        assert_eq!(
            semantics.effects,
            CallableMayEffects {
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_pending: false,
            pending_effect_categories: if false { vec![PendingEffectCategory::NativeCall] } else { Vec::new() },
            inout_path_effects: Vec::new(),            }
        );
        assert_eq!(semantics.return_provenance, ValueProvenance::Fresh);

        let signature = STD_NATIVE_SIGNATURES
            .iter()
            .find(|signature| signature.binding_key == binding_key)
            .expect("audited HTTP request binding should have a native signature");
        assert_eq!(signature.params, &[super::HTTP_REQUEST, super::STRING]);
        assert_eq!(signature.return_type, return_type);
    }
}

#[test]
fn http_client_stream_semantics_match_exact_signature_and_remain_canonical() {
    let semantics = native_callable_semantics("std.http.client.stream")
        .expect("audited HTTP client stream should have exact semantics");
    assert_eq!(
        semantics.effects,
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: true,
            pending_effect_categories: if true { vec![PendingEffectCategory::NativeCall] } else { Vec::new() },
            inout_path_effects: Vec::new(),        }
    );
    assert_eq!(semantics.return_provenance, ValueProvenance::Fresh);

    let signature = STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == semantics.binding_key)
        .expect("audited HTTP client stream should have a native signature");
    assert_eq!(signature.target, "std.http.stream");
    assert!(signature.aliases.is_empty());
    assert_eq!(signature.type_param_count, 0);
    assert_eq!(signature.params, &[super::HTTP_CLIENT_REQUEST]);
    assert_eq!(signature.return_type, super::HTTP_CLIENT_STREAM_HANDLE);

    for near_miss in [
        "std.http.stream",
        "std.http.client.stream.extra",
        "std.http.client.streams",
    ] {
        assert_eq!(
            native_callable_semantics(near_miss),
            None,
            "{near_miss} must not inherit HTTP client stream semantics"
        );
    }
}

#[test]
fn http_client_sse_semantics_match_exact_signature_and_remain_canonical() {
    let semantics = native_callable_semantics("std.http.client.sse")
        .expect("audited HTTP client SSE should have exact semantics");
    assert_eq!(
        semantics.effects,
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: true,
            pending_effect_categories: if true { vec![PendingEffectCategory::NativeCall] } else { Vec::new() },
            inout_path_effects: Vec::new(),        }
    );
    assert_eq!(semantics.return_provenance, ValueProvenance::Fresh);

    let signature = STD_NATIVE_SIGNATURES
        .iter()
        .find(|signature| signature.binding_key == semantics.binding_key)
        .expect("audited HTTP client SSE should have a native signature");
    assert_eq!(signature.target, "std.http.sse");
    assert!(signature.aliases.is_empty());
    assert_eq!(signature.type_param_count, 0);
    assert_eq!(signature.params, &[super::HTTP_CLIENT_REQUEST]);
    assert_eq!(signature.return_type, super::HTTP_SSE_STREAM);

    for near_miss in [
        "std.http.sse",
        "std.http.client.sse.extra",
        "std.http.client.sses",
    ] {
        assert_eq!(
            native_callable_semantics(near_miss),
            None,
            "{near_miss} must not inherit HTTP client SSE semantics"
        );
    }
}

#[test]
fn http_response_stream_event_constructor_semantics_match_exact_signatures() {
    let cases = [
        (
            "std.http.stream.start",
            "std.http.streamStart",
            &[super::INTEGER, super::HTTP_HEADER_ARRAY][..],
        ),
        (
            "std.http.stream.chunk",
            "std.http.streamChunk",
            &[super::BYTES][..],
        ),
        ("std.http.stream.end", "std.http.streamEnd", &[][..]),
    ];

    for (binding_key, target, params) in cases {
        let semantics = native_callable_semantics(binding_key)
            .expect("audited HTTP stream event constructor should have exact semantics");
        assert_eq!(
            semantics.effects,
            CallableMayEffects {
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_pending: false,
            pending_effect_categories: if false { vec![PendingEffectCategory::NativeCall] } else { Vec::new() },
            inout_path_effects: Vec::new(),            }
        );
        assert_eq!(semantics.return_provenance, ValueProvenance::Fresh);

        let matching = STD_NATIVE_SIGNATURES
            .iter()
            .filter(|signature| signature.binding_key == binding_key)
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1, "{binding_key} signature must be unique");
        let signature = matching[0];
        assert_eq!(signature.target, target);
        assert!(signature.aliases.is_empty());
        assert_eq!(signature.type_param_count, 0);
        assert_eq!(signature.params, params);
        assert_eq!(signature.return_type, super::HTTP_RESPONSE_STREAM_EVENT);
    }

    for lookalike in [
        "std.http.stream",
        "std.http.stream.starts",
        "std.http.stream.start.extra",
        "std.http.stream.chunked",
        "std.http.stream.end.extra",
    ] {
        assert_eq!(
            native_callable_semantics(lookalike),
            None,
            "{lookalike} must not inherit constructor semantics"
        );
    }
}

#[test]
fn http_response_stream_emit_semantics_match_exact_signature() {
    let semantics = native_callable_semantics("std.http.stream.emitResponse")
        .expect("audited HTTP response stream emitter should have exact semantics");
    assert_eq!(
        semantics.effects,
        CallableMayEffects {
            escapes_caller_value: true,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: true,
            pending_effect_categories: if true { vec![PendingEffectCategory::NativeCall] } else { Vec::new() },
            inout_path_effects: Vec::new(),        }
    );
    assert_eq!(semantics.return_provenance, ValueProvenance::Fresh);

    let matching = STD_NATIVE_SIGNATURES
        .iter()
        .filter(|signature| signature.binding_key == semantics.binding_key)
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1, "emitResponse signature must be unique");
    let signature = matching[0];
    assert_eq!(signature.target, "std.http.emitResponseStream");
    assert!(signature.aliases.is_empty());
    assert_eq!(signature.type_param_count, 0);
    assert_eq!(signature.params, &[super::HTTP_RESPONSE_STREAM_EVENT]);
    assert_eq!(signature.return_type, super::VOID);

    for lookalike in [
        "std.http.emitResponseStream",
        "std.http.stream.emitResponses",
        "std.http.stream.emitResponse.extra",
        "std.http.stream.start",
    ] {
        assert_ne!(
            native_callable_semantics(lookalike),
            Some(semantics),
            "{lookalike} must not inherit response emitter semantics"
        );
    }
}

#[test]
fn runtime_receiver_native_binding_keys_are_derived_from_receiver_registry() {
    assert!(is_runtime_receiver_native_binding_key(
        "core.date.toEpochMilliseconds"
    ));
    assert!(is_runtime_receiver_native_binding_key(
        "core.duration.toMilliseconds"
    ));
    assert!(!is_runtime_receiver_native_binding_key("core.date.now"));
    assert!(!is_runtime_receiver_native_binding_key("std.time.sleep"));
}

#[test]
fn std_package_types_are_not_encoded_as_builtins() {
    fn visit(expr: &super::NativeSignatureTypeExpr, package_paths: &mut Vec<&'static str>) {
        match expr {
            super::NativeSignatureTypeExpr::TypeParam(_) => {}
            super::NativeSignatureTypeExpr::Builtin(name) => {
                assert!(
                    !name.contains('.'),
                    "package public path {name} must not masquerade as a builtin"
                );
            }
            super::NativeSignatureTypeExpr::Package {
                package_id,
                public_path,
            } => {
                assert_eq!(*package_id, "skiff.run/std");
                package_paths.push(public_path);
            }
            super::NativeSignatureTypeExpr::Array(item)
            | super::NativeSignatureTypeExpr::Nullable(item)
            | super::NativeSignatureTypeExpr::Stream(item) => visit(item, package_paths),
            super::NativeSignatureTypeExpr::Map(key, value) => {
                visit(key, package_paths);
                visit(value, package_paths);
            }
        }
    }

    let mut package_paths = Vec::new();
    for signature in super::STD_NATIVE_SIGNATURES {
        for expr in signature
            .params
            .iter()
            .chain(std::iter::once(&signature.return_type))
        {
            visit(expr, &mut package_paths);
        }
    }
    for expected in [
        "std.time.Duration",
        "std.file.ImmutableFile",
        "std.file.CreateOptions",
        "std.file.FileInfo",
        "std.http.HttpRequest",
        "std.http.HttpResponse",
        "std.resource.ResourceInfo",
    ] {
        assert!(
            package_paths.contains(&expected),
            "missing structured package type in native signature {expected}"
        );
    }
}

#[test]
fn audited_receiver_identities_map_to_exact_native_signatures() {
    for (root, method, binding_key) in [
        ("Date", "addMilliseconds", "core.date.addMilliseconds"),
        ("Date", "compare", "core.date.compare"),
        ("Date", "diffMilliseconds", "core.date.diffMilliseconds"),
        ("Date", "isBefore", "core.date.isBefore"),
        (
            "Date",
            "toEpochMilliseconds",
            "core.date.toEpochMilliseconds",
        ),
        ("Duration", "toMilliseconds", "core.duration.toMilliseconds"),
    ] {
        let op = builtin_receiver_op_by_name(root, method)
            .expect("audited receiver op should be supported");
        assert_eq!(
            native_signature_for_receiver_op(op).map(|signature| signature.binding_key),
            Some(binding_key)
        );
    }
}
