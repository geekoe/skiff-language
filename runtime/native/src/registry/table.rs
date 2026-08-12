use std::collections::BTreeSet;
#[cfg(all(test, debug_assertions))]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(debug_assertions)]
use std::sync::OnceLock;

use serde_json::Value;
use skiff_artifact_model::{
    native_signature_for_receiver_op, validate_supported_receiver_builtin_op,
    BuiltinReceiverCallableSemantics, NativeCallableSemantics, NativeSignatureDef,
    BUILTIN_RECEIVER_CALLABLE_SEMANTICS, STD_NATIVE_CALLABLE_SEMANTICS, STD_NATIVE_SIGNATURES,
};
use skiff_runtime_native_contract::{NativeRequiredContext, NativeSignatureRegistry};

use crate::dispatch::{runtime_shared_native_route_for_validation, RuntimeNativeRoute};
use crate::error::Result;
use crate::handlers::{
    array_empty, crypto_hmac_sha1_base64, crypto_random_token, crypto_sha256, crypto_uuid,
    crypto_uuid_simple, date_from_epoch_milliseconds, date_now, date_parse, date_require_parse,
    duration_milliseconds, duration_seconds, duration_to_milliseconds,
    json_codec_requires_runtime_dispatch, json_field_access_requires_runtime_dispatch, json_merge,
    map_empty, number_assert_safe_integer, number_is_integer, number_is_safe_integer, number_parse,
    string_encode_path, string_encode_query_component, string_is_ascii_digits, string_join,
    string_split, string_truncate_utf8_bytes,
};

pub(super) type RegistryValidationResult = std::result::Result<(), String>;

pub(super) type NativeHandler = fn(&[Value]) -> Result<Value>;

pub(super) struct NativeHandlerEntry {
    pub(super) binding_key: &'static str,
    pub(super) handler: NativeHandler,
}

#[cfg(debug_assertions)]
static BUILTIN_HANDLER_VALIDATION: OnceLock<RegistryValidationResult> = OnceLock::new();

#[cfg(all(test, debug_assertions))]
static BUILTIN_HANDLER_VALIDATION_COUNT: AtomicUsize = AtomicUsize::new(0);

pub(super) fn handler_entries() -> &'static [NativeHandlerEntry] {
    #[cfg(debug_assertions)]
    {
        let validation = BUILTIN_HANDLER_VALIDATION.get_or_init(|| {
            #[cfg(test)]
            BUILTIN_HANDLER_VALIDATION_COUNT.fetch_add(1, Ordering::Relaxed);
            validate_builtin_handlers()
        });
        if let Err(error) = validation {
            panic!("native handler registry table should validate: {error}");
        }
    }

    NATIVE_BINDINGS
}

#[cfg(all(test, debug_assertions))]
pub(super) fn builtin_handler_validation_count() -> usize {
    BUILTIN_HANDLER_VALIDATION_COUNT.load(Ordering::Relaxed)
}

pub(super) fn validate_builtin_handlers() -> RegistryValidationResult {
    validate_handler_entries(NATIVE_BINDINGS, REQUIRED_HANDLER_KEYS)?;
    validate_native_callable_semantics_registry(
        &STD_NATIVE_CALLABLE_SEMANTICS,
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

        let Some(canonical_semantics) = STD_NATIVE_CALLABLE_SEMANTICS
            .iter()
            .find(|entry| entry.binding_key == binding_key)
        else {
            return Err(format!(
                "native callable semantics entry {binding_key} is not in the exact audited registry"
            ));
        };
        if semantics != canonical_semantics {
            return Err(format!(
                "native callable semantics entry {binding_key} does not match the exact audited semantics"
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
        let canonical_registry_handler = NATIVE_BINDINGS
            .iter()
            .any(|entry| entry.binding_key == binding_key);
        let route =
            runtime_shared_native_route_for_validation(binding_key, canonical_registry_handler)
                .ok_or_else(|| {
                    format!("native callable semantics entry {binding_key} has no runtime route")
                })?;
        if !native_route_matches_required_context(binding_key, required_context, route) {
            return Err(format!(
                "native callable semantics entry {binding_key} has runtime parity mismatch: context {required_context:?}, route {route:?}"
            ));
        }
        let expected_handler_count = usize::from(matches!(
            route,
            RuntimeNativeRoute::NativeRegistry | RuntimeNativeRoute::Json
        ));
        if handler_count != expected_handler_count {
            return Err(format!(
                "native callable semantics entry {binding_key} expected {expected_handler_count} runtime registry handler(s), found {handler_count}"
            ));
        }
    }

    Ok(())
}

pub(super) fn native_route_matches_required_context(
    binding_key: &str,
    required_context: NativeRequiredContext,
    route: RuntimeNativeRoute,
) -> bool {
    if matches!(binding_key, "std.json.decode" | "std.json.encode") {
        return required_context == NativeRequiredContext::None
            && route == RuntimeNativeRoute::Json;
    }

    if matches!(
        binding_key,
        "std.http.request.headers"
            | "std.http.request.cookie"
            | "std.http.stream.start"
            | "std.http.stream.chunk"
            | "std.http.stream.end"
    ) {
        return required_context == NativeRequiredContext::None
            && route == RuntimeNativeRoute::Http;
    }

    if NativeRequiredContext::for_binding_key(binding_key) != Some(required_context) {
        return false;
    }

    match (required_context, route) {
        (NativeRequiredContext::Actor, RuntimeNativeRoute::Actor)
        | (NativeRequiredContext::Config, RuntimeNativeRoute::Config)
        | (NativeRequiredContext::Db, RuntimeNativeRoute::Db)
        | (NativeRequiredContext::File, RuntimeNativeRoute::File)
        | (NativeRequiredContext::HttpClient, RuntimeNativeRoute::Http)
        | (NativeRequiredContext::HttpResponseStream, RuntimeNativeRoute::Http)
        | (NativeRequiredContext::Websocket, RuntimeNativeRoute::Websocket)
        | (NativeRequiredContext::Telemetry, RuntimeNativeRoute::Telemetry)
        | (NativeRequiredContext::Resource, RuntimeNativeRoute::Resource)
        | (NativeRequiredContext::None, RuntimeNativeRoute::Bytes)
        | (NativeRequiredContext::None, RuntimeNativeRoute::Json)
        | (NativeRequiredContext::None, RuntimeNativeRoute::Builtin)
        | (NativeRequiredContext::None, RuntimeNativeRoute::ReceiverMethod)
        | (NativeRequiredContext::None, RuntimeNativeRoute::TaskControl)
        | (NativeRequiredContext::None, RuntimeNativeRoute::NativeRegistry)
        | (NativeRequiredContext::Time, RuntimeNativeRoute::Time) => true,
        // Date.now is synchronously backed by the registry, but still requires
        // the invocation layer to provide the audited Time context.
        (NativeRequiredContext::Time, RuntimeNativeRoute::NativeRegistry)
            if binding_key == "core.date.now" =>
        {
            true
        }
        _ => false,
    }
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
        validate_supported_receiver_builtin_op(&op).map_err(|error| {
            format!(
                "receiver callable semantics entry {} is not an exact supported runtime operation: {error}",
                op.canonical_key
            )
        })?;
        let canonical_semantics = BUILTIN_RECEIVER_CALLABLE_SEMANTICS
            .iter()
            .find(|entry| entry.op.canonical_key == op.canonical_key)
            .ok_or_else(|| {
                format!(
                    "receiver callable semantics entry {} is not in the exact audited registry",
                    op.canonical_key
                )
            })?;
        if semantics != canonical_semantics {
            return Err(format!(
                "receiver callable semantics entry {} does not match exact audited semantics",
                op.canonical_key
            ));
        }
        // Some receiver builtins are implemented directly by the evaluator and
        // intentionally have no NativeSignatureRegistry entry. Whenever an
        // audited receiver does map to a native signature, keep the semantics
        // descriptor pinned to that exact shared signature.
        if let Some(canonical_signature) = native_signature_for_receiver_op(op) {
            let mut matching_signatures = signatures
                .iter()
                .filter(|signature| signature.binding_key == canonical_signature.binding_key);
            let Some(signature) = matching_signatures.next() else {
                return Err(format!(
                    "receiver callable semantics entry {} is missing native signature {}",
                    op.canonical_key, canonical_signature.binding_key
                ));
            };
            if matching_signatures.next().is_some() {
                return Err(format!(
                    "receiver callable semantics entry {} does not have a unique native signature {}",
                    op.canonical_key, canonical_signature.binding_key
                ));
            }
            if signature != canonical_signature {
                return Err(format!(
                    "receiver callable semantics entry {} does not match the exact shared native signature {}",
                    op.canonical_key, canonical_signature.binding_key
                ));
            }
        }
        let handler_count = handler_entries
            .iter()
            .filter(|entry| entry.binding_key == op.canonical_key)
            .count();
        if handler_count != 0 {
            return Err(format!(
                "receiver callable semantics entry {} expected 0 runtime registry handler(s), found {handler_count}",
                op.canonical_key
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
    "std.json.get",
    "std.json.getString",
    "std.json.getNumber",
    "std.json.getBool",
    "std.json.getArray",
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
        binding_key: "std.json.get",
        handler: json_field_access_requires_runtime_dispatch,
    },
    NativeHandlerEntry {
        binding_key: "std.json.getString",
        handler: json_field_access_requires_runtime_dispatch,
    },
    NativeHandlerEntry {
        binding_key: "std.json.getNumber",
        handler: json_field_access_requires_runtime_dispatch,
    },
    NativeHandlerEntry {
        binding_key: "std.json.getBool",
        handler: json_field_access_requires_runtime_dispatch,
    },
    NativeHandlerEntry {
        binding_key: "std.json.getArray",
        handler: json_field_access_requires_runtime_dispatch,
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
