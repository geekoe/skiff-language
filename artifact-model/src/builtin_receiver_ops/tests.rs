use std::collections::BTreeSet;

use super::*;

#[test]
fn callable_semantics_registry_is_sparse_exact_and_safe() {
    let expected = BTreeSet::from([
        "receiver:Array.length@1",
        "receiver:Array.push@1",
        "receiver:Date.addMilliseconds@1",
        "receiver:Date.compare@1",
        "receiver:Date.diffMilliseconds@1",
        "receiver:Date.isBefore@1",
        "receiver:Date.toEpochMilliseconds@1",
        "receiver:Duration.toMilliseconds@1",
        "receiver:JsonObject.delete@1",
        "receiver:JsonObject.get@1",
        "receiver:JsonObject.has@1",
        "receiver:JsonObject.set@1",
        "receiver:Map.get@1",
        "receiver:Map.has@1",
        "receiver:Map.set@1",
        "receiver:bytes.length@1",
        "receiver:bytes.toHex@1",
        "receiver:bytes.toUtf8String@1",
        "receiver:number.ceil@1",
        "receiver:number.floor@1",
        "receiver:number.round@1",
        "receiver:string.concat@1",
        "receiver:string.contains@1",
        "receiver:string.endsWith@1",
        "receiver:string.lowercase@1",
        "receiver:string.startsWith@1",
    ]);
    let actual = BUILTIN_RECEIVER_CALLABLE_SEMANTICS
        .iter()
        .map(|semantics| semantics.op.canonical_key)
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), BUILTIN_RECEIVER_CALLABLE_SEMANTICS.len());

    for semantics in BUILTIN_RECEIVER_CALLABLE_SEMANTICS {
        let op = SUPPORTED_RECEIVER_BUILTIN_OPS
            .iter()
            .find(|spec| spec.op == semantics.op)
            .map(|spec| spec.op)
            .expect("callable receiver semantics must name a supported exact op");
        assert_eq!(builtin_receiver_callable_semantics(op), Some(semantics));
        let mutates_receiver = matches!(
            semantics.op.canonical_key,
            "receiver:Array.push@1"
                | "receiver:Map.set@1"
                | "receiver:JsonObject.set@1"
                | "receiver:JsonObject.delete@1"
        );
        let aliases_receiver = matches!(
            semantics.op.canonical_key,
            "receiver:JsonObject.get@1" | "receiver:Map.get@1"
        );
        assert_eq!(
            semantics.effects,
            CallableMayEffects {
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_pending: false,
                pending_effect_categories: Vec::new(),
                inout_path_effects: Vec::new(),
            }
        );
        assert_eq!(
            semantics.return_provenance,
            if aliases_receiver {
                ValueProvenance::CallerParameter { index: 0 }
            } else if mutates_receiver {
                ValueProvenance::Constant
            } else {
                ValueProvenance::Fresh
            }
        );
    }

    let mutable_array = builtin_receiver_op_by_name("Array", "push")
        .expect("Array.push must remain a supported runtime receiver op");
    assert!(builtin_receiver_callable_semantics(mutable_array).is_some());
    let mutable_json_object = builtin_receiver_op_by_name("JsonObject", "set")
        .expect("JsonObject.set must remain a supported runtime receiver op");
    assert!(builtin_receiver_callable_semantics(mutable_json_object).is_some());
    let mutable_map = builtin_receiver_op_by_name("Map", "set")
        .expect("Map.set must remain a supported runtime receiver op");
    assert!(builtin_receiver_callable_semantics(mutable_map).is_some());
    let deleting_json_object = builtin_receiver_op_by_name("JsonObject", "delete")
        .expect("JsonObject.delete must remain a supported runtime receiver op");
    assert!(builtin_receiver_callable_semantics(deleting_json_object).is_some());

    let missing = builtin_receiver_op_by_name("string", "replaceAll").unwrap();
    assert_eq!(
        builtin_receiver_callable_semantics(missing),
        None,
        "{} must remain fail closed",
        missing.canonical_key
    );
}

#[test]
fn bytes_to_hex_callable_semantics_are_exact() {
    let exact = builtin_receiver_op_by_name("bytes", "toHex").expect("bytes.toHex op should exist");
    let semantics = builtin_receiver_callable_semantics(exact)
        .expect("bytes.toHex callable semantics should exist");
    assert_eq!(
        semantics.effects,
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: false,
            pending_effect_categories: Vec::new(),
            inout_path_effects: Vec::new(),
        }
    );
    assert_eq!(semantics.return_provenance, ValueProvenance::Fresh);

    for lookalike in [
        BuiltinReceiverOp {
            signature_version: 2,
            canonical_key: "receiver:bytes.toHex@2",
            ..exact
        },
        BuiltinReceiverOp {
            receiver: BuiltinReceiverRoot::StringText,
            canonical_key: "receiver:bytes.toHex@1",
            ..exact
        },
        BuiltinReceiverOp {
            method: BuiltinReceiverMethod::ToBase64,
            canonical_key: "receiver:bytes.toHex@1",
            ..exact
        },
    ] {
        assert_eq!(
            builtin_receiver_callable_semantics(lookalike),
            None,
            "{} must remain fail closed",
            lookalike.canonical_key
        );
    }
}

#[test]
fn date_and_duration_receiver_ops_publish_integer_return_types() {
    for (root, method) in [
        ("Date", "toEpochMilliseconds"),
        ("Date", "diffMilliseconds"),
        ("Date", "compare"),
        ("Duration", "toMilliseconds"),
    ] {
        let spec = builtin_receiver_op_spec_by_name(root, method)
            .expect("builtin receiver op spec should exist");
        assert_eq!(
            spec.public_return_type,
            BuiltinReceiverPublicReturnType::Fixed("integer"),
            "{root}.{method} should publish integer return type"
        );
    }
}

#[test]
fn number_ceil_callable_semantics_are_exact_and_detached() {
    let op = builtin_receiver_op_by_name("number", "ceil")
        .expect("number.ceil receiver op should exist");
    let semantics =
        builtin_receiver_callable_semantics(op).expect("number.ceil semantics should exist");

    assert_eq!(op.canonical_key, "receiver:number.ceil@1");
    assert_eq!(
        builtin_receiver_op_spec_by_name("number", "ceil")
            .expect("number.ceil spec should exist")
            .public_return_type,
        BuiltinReceiverPublicReturnType::Fixed("number")
    );
    assert_eq!(
        semantics.effects,
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: false,
            pending_effect_categories: Vec::new(),
            inout_path_effects: Vec::new(),
        }
    );
    assert_eq!(semantics.return_provenance, ValueProvenance::Fresh);

    for lookalike in [
        BuiltinReceiverOp {
            signature_version: 2,
            canonical_key: "receiver:number.ceil@2",
            ..op
        },
        BuiltinReceiverOp {
            canonical_key: "receiver:Number.ceil@1",
            ..op
        },
        BuiltinReceiverOp {
            method: BuiltinReceiverMethod::Floor,
            canonical_key: "receiver:number.ceil@1",
            ..op
        },
    ] {
        assert_eq!(
            builtin_receiver_callable_semantics(lookalike),
            None,
            "{} must remain fail closed",
            lookalike.canonical_key
        );
    }
}

#[test]
fn map_get_callable_semantics_are_exact_and_receiver_reachable() {
    let op = builtin_receiver_op_by_name("Map", "get").expect("Map.get receiver op should exist");
    let semantics =
        builtin_receiver_callable_semantics(op).expect("Map.get semantics should exist");

    assert_eq!(op.canonical_key, "receiver:Map.get@1");
    assert_eq!(
        builtin_receiver_op_spec_by_name("Map", "get")
            .expect("Map.get spec should exist")
            .public_return_type,
        BuiltinReceiverPublicReturnType::MapValue
    );
    assert_eq!(
        semantics.effects,
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: false,
            pending_effect_categories: Vec::new(),
            inout_path_effects: Vec::new(),
        }
    );
    assert_eq!(
        semantics.return_provenance,
        ValueProvenance::CallerParameter { index: 0 }
    );

    for lookalike in [
        BuiltinReceiverOp {
            signature_version: 2,
            canonical_key: "receiver:Map.get@2",
            ..op
        },
        BuiltinReceiverOp {
            canonical_key: "receiver:map.get@1",
            ..op
        },
        BuiltinReceiverOp {
            receiver: BuiltinReceiverRoot::JsonObject,
            canonical_key: "receiver:Map.get@1",
            ..op
        },
    ] {
        assert_eq!(
            builtin_receiver_callable_semantics(lookalike),
            None,
            "{} must remain fail closed",
            lookalike.canonical_key
        );
    }
}

#[test]
fn map_has_and_set_callable_semantics_are_exact() {
    let has = builtin_receiver_op_by_name("Map", "has").expect("Map.has op should exist");
    let has_semantics =
        builtin_receiver_callable_semantics(has).expect("Map.has semantics should exist");
    assert_eq!(
        has_semantics.effects,
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: false,
            pending_effect_categories: Vec::new(),
            inout_path_effects: Vec::new(),
        }
    );
    assert_eq!(has_semantics.return_provenance, ValueProvenance::Fresh);

    let set = builtin_receiver_op_by_name("Map", "set").expect("Map.set op should exist");
    let set_semantics =
        builtin_receiver_callable_semantics(set).expect("Map.set semantics should exist");
    assert_eq!(
        set_semantics.effects,
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: false,
            pending_effect_categories: Vec::new(),
            inout_path_effects: Vec::new(),
        }
    );
    assert_eq!(set_semantics.return_provenance, ValueProvenance::Constant);

    for lookalike in [
        BuiltinReceiverOp {
            signature_version: 2,
            canonical_key: "receiver:Map.has@2",
            ..has
        },
        BuiltinReceiverOp {
            receiver: BuiltinReceiverRoot::JsonObject,
            canonical_key: "receiver:Map.has@1",
            ..has
        },
        BuiltinReceiverOp {
            canonical_key: "receiver:Map.has@1",
            ..set
        },
    ] {
        assert_eq!(
            builtin_receiver_callable_semantics(lookalike),
            None,
            "{} must remain fail closed",
            lookalike.canonical_key
        );
    }
}

#[test]
fn string_replace_all_receiver_op_is_supported() {
    let spec = builtin_receiver_op_spec_by_name("string", "replaceAll")
        .expect("string.replaceAll receiver op should exist");

    assert_eq!(
        spec.public_return_type,
        BuiltinReceiverPublicReturnType::Fixed("string")
    );
    assert_eq!(spec.op.canonical_key, "receiver:string.replaceAll@1");
}
