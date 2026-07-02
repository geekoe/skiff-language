use std::collections::BTreeSet;

use serde_json::Value;
use skiff_runtime_native_contract::NativeSignatureRegistry;

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
    validate_handler_entries(NATIVE_BINDINGS, REQUIRED_HANDLER_KEYS)
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
