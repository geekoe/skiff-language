use std::collections::BTreeSet;

use serde_json::Value;
use skiff_artifact_model::{
    native_signature_for_receiver_op, BuiltinReceiverCallableSemantics, NativeCallableSemantics,
    NativeSignatureDef, BUILTIN_RECEIVER_CALLABLE_SEMANTICS, STD_NATIVE_CALLABLE_SEMANTICS,
    STD_NATIVE_SIGNATURES,
};
use skiff_runtime_native_contract::{NativeRequiredContext, NativeSignatureRegistry};

use crate::dispatch::{runtime_shared_native_route_for_validation, RuntimeNativeRoute};
use crate::error::Result;
use crate::handlers::{
    array_empty, crypto_hmac_sha1_base64, crypto_random_token, crypto_sha256, crypto_uuid,
    crypto_uuid_simple, date_from_epoch_milliseconds, date_now, date_parse, date_require_parse,
    duration_milliseconds, duration_seconds, duration_to_milliseconds,
    json_codec_requires_runtime_dispatch, json_merge, map_empty, number_assert_safe_integer,
    number_is_integer, number_is_safe_integer, number_parse, string_encode_path,
    string_encode_query_component, string_is_ascii_digits, string_join, string_split,
    string_truncate_utf8_bytes,
};

pub(super) type RegistryValidationResult = std::result::Result<(), String>;

pub(super) type NativeHandler = fn(&[Value]) -> Result<Value>;

pub(super) struct NativeHandlerEntry {
    pub(super) binding_key: &'static str,
    pub(super) handler: NativeHandler,
}

pub(super) fn handler_entries() -> &'static [NativeHandlerEntry] {
    debug_assert!(
        validate_builtin_handlers().is_ok(),
        "native handler registry table should validate"
    );
    NATIVE_BINDINGS
}

pub(super) fn validate_builtin_handlers() -> RegistryValidationResult {
    validate_handler_entries(NATIVE_BINDINGS, REQUIRED_HANDLER_KEYS)?;
    validate_native_callable_semantics_registry(
        STD_NATIVE_CALLABLE_SEMANTICS,
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )?;
    validate_receiver_callable_semantics_registry(
        BUILTIN_RECEIVER_CALLABLE_SEMANTICS,
        STD_NATIVE_SIGNATURES,
        NATIVE_BINDINGS,
    )
}

pub(super) fn validate_handler_entries(
    entries: &[NativeHandlerEntry],
    required_handler_keys: &[&'static str],
) -> RegistryValidationResult {
    let signature_registry = NativeSignatureRegistry::builtins();
    let mut registered_keys = BTreeSet::new();

    for entry in entries {
        if signature_registry.signature(entry.binding_key).is_none() {
            return Err(format!(
                "native handler registry entry {} is not declared in NativeSignatureRegistry",
                entry.binding_key
            ));
        }

        if !registered_keys.insert(entry.binding_key) {
            return Err(format!(
                "native handler registry entry {} is registered more than once",
                entry.binding_key
            ));
        }
    }

    for required_key in required_handler_keys {
        if !registered_keys.contains(required_key) {
            return Err(format!(
                "native handler registry is missing required handler {required_key}"
            ));
        }
    }

    Ok(())
}

pub(super) fn validate_native_callable_semantics_registry(
    semantics_entries: &[NativeCallableSemantics],
    signatures: &[NativeSignatureDef],
    handler_entries: &[NativeHandlerEntry],
) -> RegistryValidationResult {
    let mut registered_keys = BTreeSet::new();

    for semantics in semantics_entries {
        let binding_key = semantics.binding_key;
        if !registered_keys.insert(binding_key) {
            return Err(format!(
                "native callable semantics entry {binding_key} is registered more than once"
            ));
        }

        let Some(canonical_signature) = STD_NATIVE_SIGNATURES
            .iter()
            .find(|signature| signature.binding_key == binding_key)
        else {
            return Err(format!(
                "native callable semantics entry {binding_key} has an unknown binding key"
            ));
        };
        let mut matching_signatures = signatures
            .iter()
            .filter(|signature| signature.binding_key == binding_key);
        let Some(signature) = matching_signatures.next() else {
            return Err(format!(
                "native callable semantics entry {binding_key} is missing from the native signatures"
            ));
        };
        if matching_signatures.next().is_some() {
            return Err(format!(
                "native callable semantics entry {binding_key} does not have a unique native signature"
            ));
        }
        if signature != canonical_signature {
            return Err(format!(
                "native callable semantics entry {binding_key} does not match the exact shared native signature"
            ));
        }

        let Some(required_context) = NativeRequiredContext::for_binding_key(binding_key) else {
            return Err(format!(
                "native callable semantics entry {binding_key} has no known required context"
            ));
        };
        let handler_count = handler_entries
            .iter()
            .filter(|entry| entry.binding_key == binding_key)
            .count();
        if required_context == NativeRequiredContext::None && handler_count == 0 {
            return Err(format!(
                "native callable semantics entry {binding_key} is missing a runtime handler"
            ));
        }
        let route = runtime_shared_native_route_for_validation(binding_key, handler_count > 0)
            .ok_or_else(|| {
                format!("native callable semantics entry {binding_key} has no runtime route")
            })?;
        let route_matches = match (binding_key, required_context, route) {
            ("core.date.now", NativeRequiredContext::Time, RuntimeNativeRoute::NativeRegistry)
            | ("std.time.sleep", NativeRequiredContext::Time, RuntimeNativeRoute::Time) => true,
            (_, NativeRequiredContext::None, RuntimeNativeRoute::NativeRegistry) => true,
            _ => false,
        };
        if !route_matches {
            return Err(format!(
                "native callable semantics entry {binding_key} has runtime parity mismatch: context {required_context:?}, route {route:?}"
            ));
        }
        let expected_handler_count = usize::from(route == RuntimeNativeRoute::NativeRegistry);
        if handler_count != expected_handler_count {
            return Err(format!(
                "native callable semantics entry {binding_key} expected {expected_handler_count} runtime registry handler(s), found {handler_count}"
            ));
        }
    }

    Ok(())
}

pub(super) fn validate_receiver_callable_semantics_registry(
    semantics_entries: &[BuiltinReceiverCallableSemantics],
    signatures: &[NativeSignatureDef],
    handler_entries: &[NativeHandlerEntry],
) -> RegistryValidationResult {
    let mut registered_keys = BTreeSet::new();
    for semantics in semantics_entries {
        let op = semantics.op;
        if !registered_keys.insert(op.canonical_key) {
            return Err(format!(
                "receiver callable semantics entry {} is registered more than once",
                op.canonical_key
            ));
        }
        let canonical_signature = native_signature_for_receiver_op(op).ok_or_else(|| {
            format!(
                "receiver callable semantics entry {} has no exact native signature",
                op.canonical_key
            )
        })?;
        let mut matching_signatures = signatures
            .iter()
            .filter(|signature| signature.binding_key == canonical_signature.binding_key);
        let signature = matching_signatures.next().ok_or_else(|| {
            format!(
                "receiver callable semantics entry {} is missing signature {}",
                op.canonical_key, canonical_signature.binding_key
            )
        })?;
        if matching_signatures.next().is_some() || signature != canonical_signature {
            return Err(format!(
                "receiver callable semantics entry {} does not match exact signature {}",
                op.canonical_key, canonical_signature.binding_key
            ));
        }
        let context = NativeRequiredContext::for_binding_key(canonical_signature.binding_key)
            .ok_or_else(|| {
                format!(
                    "receiver callable semantics entry {} has no known required context",
                    op.canonical_key
                )
            })?;
        let handler_count = handler_entries
            .iter()
            .filter(|entry| entry.binding_key == canonical_signature.binding_key)
            .count();
        let route = runtime_shared_native_route_for_validation(
            canonical_signature.binding_key,
            handler_count > 0,
        )
        .ok_or_else(|| {
            format!(
                "receiver callable semantics entry {} has no runtime route",
                op.canonical_key
            )
        })?;
        if context != NativeRequiredContext::None || route != RuntimeNativeRoute::ReceiverMethod {
            return Err(format!(
                "receiver callable semantics entry {} has runtime parity mismatch: binding {}, context {context:?}, route {route:?}",
                op.canonical_key, canonical_signature.binding_key
            ));
        }
    }
    Ok(())
}

impl NativeHandlerEntry {
    pub(super) fn matches(&self, binding_key: &str) -> bool {
        self.binding_key == binding_key
    }

    pub(super) fn dispatch(&self, args: &[Value]) -> Result<Value> {
        (self.handler)(args)
    }
}

pub(super) const REQUIRED_HANDLER_KEYS: &[&str] = &[
    "core.array.empty",
    "core.map.empty",
    "core.date.now",
    "core.date.fromEpochMilliseconds",
    "core.date.parse",
    "core.date.requireParse",
    "core.duration.milliseconds",
    "core.duration.seconds",
    "core.duration.toMilliseconds",
    "core.number.parse",
    "core.number.isInteger",
    "core.number.isSafeInteger",
    "core.number.assertSafeInteger",
    "std.json.encode",
    "std.json.decode",
    "std.json.merge",
    "std.string.join",
    "std.string.split",
    "std.string.isAsciiDigits",
    "std.string.truncateUtf8Bytes",
    "std.string.encodeQueryComponent",
    "std.string.encodePath",
    "std.crypto.hmacSha1Base64",
    "std.crypto.sha256",
    "std.crypto.randomToken",
    "std.crypto.uuid",
    "std.crypto.uuidSimple",
];

pub(super) const NATIVE_BINDINGS: &[NativeHandlerEntry] = &[
    NativeHandlerEntry {
        binding_key: "core.array.empty",
        handler: array_empty,
    },
    NativeHandlerEntry {
        binding_key: "core.map.empty",
        handler: map_empty,
    },
    NativeHandlerEntry {
        binding_key: "core.date.now",
        handler: date_now,
    },
    NativeHandlerEntry {
        binding_key: "core.date.fromEpochMilliseconds",
        handler: date_from_epoch_milliseconds,
    },
    NativeHandlerEntry {
        binding_key: "core.date.parse",
        handler: date_parse,
    },
    NativeHandlerEntry {
        binding_key: "core.date.requireParse",
        handler: date_require_parse,
    },
    NativeHandlerEntry {
        binding_key: "core.duration.milliseconds",
        handler: duration_milliseconds,
    },
    NativeHandlerEntry {
        binding_key: "core.duration.seconds",
        handler: duration_seconds,
    },
    NativeHandlerEntry {
        binding_key: "core.duration.toMilliseconds",
        handler: duration_to_milliseconds,
    },
    NativeHandlerEntry {
        binding_key: "core.number.parse",
        handler: number_parse,
    },
    NativeHandlerEntry {
        binding_key: "core.number.isInteger",
        handler: number_is_integer,
    },
    NativeHandlerEntry {
        binding_key: "core.number.isSafeInteger",
        handler: number_is_safe_integer,
    },
    NativeHandlerEntry {
        binding_key: "core.number.assertSafeInteger",
        handler: number_assert_safe_integer,
    },
    NativeHandlerEntry {
        binding_key: "std.json.encode",
        handler: json_codec_requires_runtime_dispatch,
    },
    NativeHandlerEntry {
        binding_key: "std.json.decode",
        handler: json_codec_requires_runtime_dispatch,
    },
    NativeHandlerEntry {
        binding_key: "std.json.merge",
        handler: json_merge,
    },
    NativeHandlerEntry {
        binding_key: "std.string.join",
        handler: string_join,
    },
    NativeHandlerEntry {
        binding_key: "std.string.split",
        handler: string_split,
    },
    NativeHandlerEntry {
        binding_key: "std.string.isAsciiDigits",
        handler: string_is_ascii_digits,
    },
    NativeHandlerEntry {
        binding_key: "std.string.truncateUtf8Bytes",
        handler: string_truncate_utf8_bytes,
    },
    NativeHandlerEntry {
        binding_key: "std.string.encodeQueryComponent",
        handler: string_encode_query_component,
    },
    NativeHandlerEntry {
        binding_key: "std.string.encodePath",
        handler: string_encode_path,
    },
    NativeHandlerEntry {
        binding_key: "std.crypto.hmacSha1Base64",
        handler: crypto_hmac_sha1_base64,
    },
    NativeHandlerEntry {
        binding_key: "std.crypto.sha256",
        handler: crypto_sha256,
    },
    NativeHandlerEntry {
        binding_key: "std.crypto.randomToken",
        handler: crypto_random_token,
    },
    NativeHandlerEntry {
        binding_key: "std.crypto.uuid",
        handler: crypto_uuid,
    },
    NativeHandlerEntry {
        binding_key: "std.crypto.uuidSimple",
        handler: crypto_uuid_simple,
    },
];
