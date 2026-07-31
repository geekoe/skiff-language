use std::collections::BTreeSet;

use skiff_artifact_model::STD_NATIVE_SIGNATURES;

use skiff_artifact_model::{MetadataValue, NativeTarget};

use crate::{NativeDispatchTarget, NativeRequiredContext, NativeSignatureRegistry};

#[test]
fn native_signature_registry_resolves_every_declared_binding_key_to_contract_spec() {
    let registry = NativeSignatureRegistry::builtins();

    for signature in STD_NATIVE_SIGNATURES {
        let spec = registry
            .binding_spec(signature.binding_key)
            .unwrap_or_else(|| {
                panic!(
                    "native signature registry did not resolve binding key {}",
                    signature.binding_key
                )
            });
        assert_eq!(
            spec.key.as_str(),
            signature.binding_key,
            "native signature registry resolved key mismatch"
        );
        assert_eq!(
            spec.signature.target, signature.target,
            "native signature registry resolved binding key {} to {}",
            signature.binding_key, spec.signature.target
        );
    }
}

#[test]
fn native_signature_registry_binding_keys_are_unique() {
    let mut names = BTreeSet::new();

    for signature in STD_NATIVE_SIGNATURES {
        assert!(
            names.insert(signature.binding_key),
            "duplicate native signature binding key {}",
            signature.binding_key
        );
    }
}

#[test]
fn native_required_context_is_explicit_for_contextful_std_bindings() {
    let registry = NativeSignatureRegistry::builtins();
    let cases = [
        ("std.actor.get", NativeRequiredContext::Actor),
        ("std.file.create", NativeRequiredContext::File),
        ("std.file.createFromStream", NativeRequiredContext::File),
        ("core.date.now", NativeRequiredContext::Time),
        ("std.time.sleep", NativeRequiredContext::Time),
        ("std.http.client.request", NativeRequiredContext::HttpClient),
        (
            "std.http.stream.emitResponse",
            NativeRequiredContext::HttpResponseStream,
        ),
        (
            "std.websocket.sendTextToConnection",
            NativeRequiredContext::Websocket,
        ),
        (
            "std.websocket.requestJsonToConnection",
            NativeRequiredContext::Websocket,
        ),
        ("std.telemetry.emit", NativeRequiredContext::Telemetry),
        ("std.resource.bytes", NativeRequiredContext::Resource),
        ("std.resource.text", NativeRequiredContext::Resource),
        ("std.resource.json", NativeRequiredContext::Resource),
        ("std.resource.info", NativeRequiredContext::Resource),
        ("std.resource.exists", NativeRequiredContext::Resource),
        ("std.json.encode", NativeRequiredContext::None),
    ];

    for (binding_key, expected_context) in cases {
        let spec = registry
            .binding_spec(binding_key)
            .unwrap_or_else(|| panic!("{binding_key} should have a native binding spec"));
        assert_eq!(
            spec.required_context, expected_context,
            "{binding_key} required context"
        );
    }
    assert!(
        STD_NATIVE_SIGNATURES
            .iter()
            .filter_map(|signature| registry.binding_spec(signature.binding_key))
            .any(|spec| spec.required_context != NativeRequiredContext::None),
        "known native bindings must not all map to NativeRequiredContext::None"
    );
    assert!(
        registry.binding_spec("unknown.native").is_none(),
        "external or unknown native calls must not produce a known binding spec"
    );
}

#[test]
fn native_dispatch_target_rejects_reserved_std_without_binding_key() {
    let registry = NativeSignatureRegistry::builtins();
    let target = NativeTarget {
        namespace: "std.json".to_string(),
        symbol: "decode".to_string(),
        binding_key: None,
        metadata: Default::default(),
    };

    let result = registry.validate_native_dispatch_target(&target);

    assert!(matches!(
        result,
        NativeDispatchTarget::Invalid(message)
            if message.contains("std.json.decode")
                && message.contains("missing artifact bindingKey")
    ));
}

#[test]
fn native_dispatch_target_rejects_known_std_metadata() {
    let registry = NativeSignatureRegistry::builtins();
    let mut target = NativeTarget {
        namespace: "std.json".to_string(),
        symbol: "decode".to_string(),
        binding_key: Some("std.json.decode".to_string()),
        metadata: Default::default(),
    };
    target.metadata.insert(
        "mode".to_string(),
        MetadataValue::String("ignored".to_string()),
    );

    let result = registry.validate_native_dispatch_target(&target);

    assert!(matches!(
        result,
        NativeDispatchTarget::Invalid(message)
            if message.contains("std.json.decode")
                && message.contains("target metadata is not supported")
    ));
}
