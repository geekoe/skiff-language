use crate::{
    builtin_receiver_ops::{
        validate_supported_receiver_builtin_op, BuiltinReceiverOp, SUPPORTED_RECEIVER_BUILTIN_OPS,
    },
    CallableMayEffects, ValueProvenance,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeSignatureDef {
    pub target: &'static str,
    pub binding_key: &'static str,
    pub aliases: &'static [&'static str],
    pub type_param_count: usize,
    pub params: &'static [NativeSignatureTypeExpr],
    pub return_type: NativeSignatureTypeExpr,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSignatureTypeExpr {
    TypeParam(usize),
    Builtin(&'static str),
    Package {
        package_id: &'static str,
        public_path: &'static str,
    },
    Array(&'static NativeSignatureTypeExpr),
    Map(
        &'static NativeSignatureTypeExpr,
        &'static NativeSignatureTypeExpr,
    ),
    Nullable(&'static NativeSignatureTypeExpr),
    Stream(&'static NativeSignatureTypeExpr),
}

/// Audited callable semantics for an exact native binding.
///
/// This registry is intentionally sparse: absence means that source effect
/// analysis must fail closed. Signature shape, required context, and runtime
/// handler presence are validated by their respective consumers rather than
/// inferred as a safety guarantee.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeCallableSemantics {
    pub binding_key: &'static str,
    pub effects: CallableMayEffects,
    pub return_provenance: ValueProvenance,
}

const fn detached_scalar_native(binding_key: &'static str) -> NativeCallableSemantics {
    detached_native(binding_key, false)
}

const fn detached_native(binding_key: &'static str, may_suspend: bool) -> NativeCallableSemantics {
    NativeCallableSemantics {
        binding_key,
        effects: CallableMayEffects {
            writes_caller_reachable: false,
            returns_caller_alias: false,
            throws_caller_alias: false,
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_suspend,
        },
        return_provenance: ValueProvenance::Fresh,
    }
}

const fn escaping_suspending_native(binding_key: &'static str) -> NativeCallableSemantics {
    NativeCallableSemantics {
        binding_key,
        effects: CallableMayEffects {
            writes_caller_reachable: false,
            returns_caller_alias: false,
            throws_caller_alias: false,
            escapes_caller_value: true,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_suspend: true,
        },
        return_provenance: ValueProvenance::Fresh,
    }
}

pub const STD_NATIVE_CALLABLE_SEMANTICS: &[NativeCallableSemantics] = &[
    detached_native("std.actor.getOrCreate", true),
    detached_native("std.actor.replace", true),
    detached_native("std.actor.find", true),
    detached_native("std.actor.remove", true),
    detached_scalar_native("core.array.empty"),
    detached_scalar_native("core.map.empty"),
    detached_scalar_native("core.bytes.concat"),
    detached_scalar_native("core.bytes.fromBase64"),
    detached_scalar_native("core.bytes.fromHex"),
    detached_scalar_native("core.bytes.fromUtf8"),
    detached_scalar_native("core.date.fromEpochMilliseconds"),
    detached_scalar_native("core.date.now"),
    detached_scalar_native("core.date.parse"),
    detached_scalar_native("core.duration.milliseconds"),
    detached_scalar_native("core.duration.seconds"),
    detached_scalar_native("core.number.parse"),
    detached_scalar_native("core.number.assertSafeInteger"),
    detached_scalar_native("std.json.decode"),
    detached_scalar_native("std.json.encode"),
    detached_scalar_native("std.json.merge"),
    detached_scalar_native("std.string.join"),
    detached_scalar_native("std.string.split"),
    detached_scalar_native("std.string.isAsciiDigits"),
    detached_scalar_native("std.string.truncateUtf8Bytes"),
    detached_scalar_native("std.string.encodeQueryComponent"),
    detached_scalar_native("std.string.encodePath"),
    detached_scalar_native("std.crypto.hmacSha1Base64"),
    detached_scalar_native("std.crypto.sha256"),
    detached_scalar_native("std.crypto.randomToken"),
    detached_scalar_native("std.crypto.uuid"),
    detached_scalar_native("std.crypto.uuidSimple"),
    detached_native("std.http.client.request", true),
    detached_native("std.http.client.stream", true),
    detached_native("std.http.client.sse", true),
    detached_scalar_native("std.http.request.cookie"),
    detached_scalar_native("std.http.request.headers"),
    detached_scalar_native("std.http.stream.start"),
    detached_scalar_native("std.http.stream.chunk"),
    detached_scalar_native("std.http.stream.end"),
    escaping_suspending_native("std.http.stream.emitResponse"),
    detached_native("std.file.create", true),
    detached_native("std.file.createFromStream", true),
    detached_native("std.time.sleep", true),
    detached_scalar_native("std.websocket.sendTextToConnection"),
    detached_scalar_native("std.websocket.sendBinaryToConnection"),
    detached_scalar_native("std.websocket.sendTextToBusinessIdentity"),
    detached_scalar_native("std.websocket.sendBinaryToBusinessIdentity"),
    detached_native("std.websocket.requestJsonToConnection", true),
];

pub fn native_callable_semantics(binding_key: &str) -> Option<&'static NativeCallableSemantics> {
    STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|semantics| semantics.binding_key == binding_key)
}

const T0: NativeSignatureTypeExpr = NativeSignatureTypeExpr::TypeParam(0);
const T1: NativeSignatureTypeExpr = NativeSignatureTypeExpr::TypeParam(1);
const T2: NativeSignatureTypeExpr = NativeSignatureTypeExpr::TypeParam(2);
const STRING: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Builtin("string");
const BOOL: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Builtin("bool");
const NUMBER: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Builtin("number");
const INTEGER: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Builtin("integer");
const BYTES: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Builtin("bytes");
const DATE: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Builtin("Date");
const JSON: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Builtin("Json");
const JSON_OBJECT: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Builtin("JsonObject");
const VOID: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Builtin("void");
const DURATION: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Package {
    package_id: "skiff.run/std",
    public_path: "std.time.Duration",
};
const DATE_NULLABLE: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Nullable(&DATE);
const STRING_ARRAY: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Array(&STRING);
const BYTES_ARRAY: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Array(&BYTES);
const HTTP_HEADER_ARRAY: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Array(&HTTP_HEADER);
const STRING_NULLABLE: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Nullable(&STRING);
const HTTP_RESPONSE_NULLABLE: NativeSignatureTypeExpr =
    NativeSignatureTypeExpr::Nullable(&HTTP_RESPONSE);
const JSON_NULLABLE: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Nullable(&JSON);
const HTTP_HEADER: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Package {
    package_id: "skiff.run/std",
    public_path: "std.http.HttpHeader",
};
const HTTP_REQUEST: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Package {
    package_id: "skiff.run/std",
    public_path: "std.http.HttpRequest",
};
const HTTP_RESPONSE: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Package {
    package_id: "skiff.run/std",
    public_path: "std.http.HttpResponse",
};
const HTTP_CLIENT_REQUEST: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Package {
    package_id: "skiff.run/std",
    public_path: "std.http.HttpClientRequest",
};
const HTTP_CLIENT_RESPONSE: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Package {
    package_id: "skiff.run/std",
    public_path: "std.http.HttpClientResponse",
};
const HTTP_CLIENT_STREAM_HANDLE: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Package {
    package_id: "skiff.run/std",
    public_path: "std.http.HttpClientStreamHandle",
};
const HTTP_SSE_EVENT: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Package {
    package_id: "skiff.run/std",
    public_path: "std.http.HttpSseEvent",
};
const HTTP_RESPONSE_STREAM_EVENT: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Package {
    package_id: "skiff.run/std",
    public_path: "std.http.HttpResponseStreamEvent",
};
const HTTP_SSE_STREAM: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Stream(&HTTP_SSE_EVENT);
const BYTE_STREAM: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Stream(&BYTES);
const FILE_IMMUTABLE: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Package {
    package_id: "skiff.run/std",
    public_path: "std.file.ImmutableFile",
};
const FILE_CREATE_OPTIONS: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Package {
    package_id: "skiff.run/std",
    public_path: "std.file.CreateOptions",
};
const FILE_CREATE_OPTIONS_NULLABLE: NativeSignatureTypeExpr =
    NativeSignatureTypeExpr::Nullable(&FILE_CREATE_OPTIONS);
const FILE_INFO: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Package {
    package_id: "skiff.run/std",
    public_path: "std.file.FileInfo",
};
const RESOURCE_INFO: NativeSignatureTypeExpr = NativeSignatureTypeExpr::Package {
    package_id: "skiff.run/std",
    public_path: "std.resource.ResourceInfo",
};
pub const STD_NATIVE_SIGNATURES: &[NativeSignatureDef] = &[
    NativeSignatureDef {
        target: "std.actor.getOrCreate",
        binding_key: "std.actor.getOrCreate",
        aliases: &[],
        type_param_count: 3,
        params: &[T1, T2],
        return_type: T0,
    },
    NativeSignatureDef {
        target: "std.actor.replace",
        binding_key: "std.actor.replace",
        aliases: &[],
        type_param_count: 3,
        params: &[T1, T2],
        return_type: T0,
    },
    NativeSignatureDef {
        target: "std.actor.find",
        binding_key: "std.actor.find",
        aliases: &[],
        type_param_count: 2,
        params: &[T1],
        return_type: NativeSignatureTypeExpr::Nullable(&T0),
    },
    NativeSignatureDef {
        target: "std.actor.remove",
        binding_key: "std.actor.remove",
        aliases: &[],
        type_param_count: 2,
        params: &[T1],
        return_type: BOOL,
    },
    NativeSignatureDef {
        target: "Array.empty",
        binding_key: "core.array.empty",
        aliases: &[],
        type_param_count: 1,
        params: &[],
        return_type: NativeSignatureTypeExpr::Array(&T0),
    },
    NativeSignatureDef {
        target: "Map.empty",
        binding_key: "core.map.empty",
        aliases: &[],
        type_param_count: 2,
        params: &[],
        return_type: NativeSignatureTypeExpr::Map(&T0, &T1),
    },
    NativeSignatureDef {
        target: "Date.now",
        binding_key: "core.date.now",
        aliases: &[],
        type_param_count: 0,
        params: &[],
        return_type: DATE,
    },
    NativeSignatureDef {
        target: "Date.fromEpochMilliseconds",
        binding_key: "core.date.fromEpochMilliseconds",
        aliases: &[],
        type_param_count: 0,
        params: &[INTEGER],
        return_type: DATE,
    },
    NativeSignatureDef {
        target: "Date.parse",
        binding_key: "core.date.parse",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING],
        return_type: DATE_NULLABLE,
    },
    NativeSignatureDef {
        target: "Date.requireParse",
        binding_key: "core.date.requireParse",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING],
        return_type: DATE,
    },
    NativeSignatureDef {
        target: "Date.toEpochMilliseconds",
        binding_key: "core.date.toEpochMilliseconds",
        aliases: &[],
        type_param_count: 0,
        params: &[DATE],
        return_type: INTEGER,
    },
    NativeSignatureDef {
        target: "Date.toISOString",
        binding_key: "core.date.toISOString",
        aliases: &[],
        type_param_count: 0,
        params: &[DATE],
        return_type: STRING,
    },
    NativeSignatureDef {
        target: "Date.addMilliseconds",
        binding_key: "core.date.addMilliseconds",
        aliases: &[],
        type_param_count: 0,
        params: &[DATE, INTEGER],
        return_type: DATE,
    },
    NativeSignatureDef {
        target: "Date.diffMilliseconds",
        binding_key: "core.date.diffMilliseconds",
        aliases: &[],
        type_param_count: 0,
        params: &[DATE, DATE],
        return_type: INTEGER,
    },
    NativeSignatureDef {
        target: "Date.compare",
        binding_key: "core.date.compare",
        aliases: &[],
        type_param_count: 0,
        params: &[DATE, DATE],
        return_type: INTEGER,
    },
    NativeSignatureDef {
        target: "Date.isBefore",
        binding_key: "core.date.isBefore",
        aliases: &[],
        type_param_count: 0,
        params: &[DATE, DATE],
        return_type: BOOL,
    },
    NativeSignatureDef {
        target: "Date.isAfter",
        binding_key: "core.date.isAfter",
        aliases: &[],
        type_param_count: 0,
        params: &[DATE, DATE],
        return_type: BOOL,
    },
    NativeSignatureDef {
        target: "Duration.milliseconds",
        binding_key: "core.duration.milliseconds",
        aliases: &[],
        type_param_count: 0,
        params: &[INTEGER],
        return_type: DURATION,
    },
    NativeSignatureDef {
        target: "Duration.seconds",
        binding_key: "core.duration.seconds",
        aliases: &[],
        type_param_count: 0,
        params: &[INTEGER],
        return_type: DURATION,
    },
    NativeSignatureDef {
        target: "Duration.toMilliseconds",
        binding_key: "core.duration.toMilliseconds",
        aliases: &[],
        type_param_count: 0,
        params: &[DURATION],
        return_type: INTEGER,
    },
    NativeSignatureDef {
        target: "std.number.parse",
        binding_key: "core.number.parse",
        aliases: &["number.parse"],
        type_param_count: 0,
        params: &[STRING],
        return_type: NativeSignatureTypeExpr::Nullable(&NUMBER),
    },
    NativeSignatureDef {
        target: "std.number.isInteger",
        binding_key: "core.number.isInteger",
        aliases: &["number.isInteger"],
        type_param_count: 0,
        params: &[NUMBER],
        return_type: BOOL,
    },
    NativeSignatureDef {
        target: "std.number.isSafeInteger",
        binding_key: "core.number.isSafeInteger",
        aliases: &["number.isSafeInteger"],
        type_param_count: 0,
        params: &[NUMBER],
        return_type: BOOL,
    },
    NativeSignatureDef {
        target: "std.number.assertSafeInteger",
        binding_key: "core.number.assertSafeInteger",
        aliases: &["number.assertSafeInteger"],
        type_param_count: 0,
        params: &[NUMBER],
        return_type: INTEGER,
    },
    NativeSignatureDef {
        target: "std.json.encode",
        binding_key: "std.json.encode",
        aliases: &[],
        type_param_count: 1,
        params: &[T0],
        return_type: STRING,
    },
    NativeSignatureDef {
        target: "std.json.decode",
        binding_key: "std.json.decode",
        aliases: &[],
        type_param_count: 1,
        params: &[STRING],
        return_type: T0,
    },
    NativeSignatureDef {
        target: "std.json.merge",
        binding_key: "std.json.merge",
        aliases: &[],
        type_param_count: 0,
        params: &[JSON, JSON],
        return_type: JSON,
    },
    NativeSignatureDef {
        target: "std.string.join",
        binding_key: "std.string.join",
        aliases: &["string.join"],
        type_param_count: 0,
        params: &[STRING_ARRAY, STRING],
        return_type: STRING,
    },
    NativeSignatureDef {
        target: "std.string.split",
        binding_key: "std.string.split",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING, STRING],
        return_type: STRING_ARRAY,
    },
    NativeSignatureDef {
        target: "std.string.isAsciiDigits",
        binding_key: "std.string.isAsciiDigits",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING],
        return_type: BOOL,
    },
    NativeSignatureDef {
        target: "std.string.truncateUtf8Bytes",
        binding_key: "std.string.truncateUtf8Bytes",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING, NUMBER],
        return_type: STRING,
    },
    NativeSignatureDef {
        target: "std.string.encodeQueryComponent",
        binding_key: "std.string.encodeQueryComponent",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING],
        return_type: STRING,
    },
    NativeSignatureDef {
        target: "std.string.encodePath",
        binding_key: "std.string.encodePath",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING],
        return_type: STRING,
    },
    NativeSignatureDef {
        target: "std.crypto.hmacSha1Base64",
        binding_key: "std.crypto.hmacSha1Base64",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING, STRING],
        return_type: STRING,
    },
    NativeSignatureDef {
        target: "std.crypto.sha256",
        binding_key: "std.crypto.sha256",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING],
        return_type: STRING,
    },
    NativeSignatureDef {
        target: "std.crypto.randomToken",
        binding_key: "std.crypto.randomToken",
        aliases: &[],
        type_param_count: 0,
        params: &[],
        return_type: STRING,
    },
    NativeSignatureDef {
        target: "std.crypto.uuid",
        binding_key: "std.crypto.uuid",
        aliases: &[],
        type_param_count: 0,
        params: &[],
        return_type: STRING,
    },
    NativeSignatureDef {
        target: "std.crypto.uuidSimple",
        binding_key: "std.crypto.uuidSimple",
        aliases: &[],
        type_param_count: 0,
        params: &[],
        return_type: STRING,
    },
    NativeSignatureDef {
        target: "std.time.sleep",
        binding_key: "std.time.sleep",
        aliases: &[],
        type_param_count: 0,
        params: &[DURATION],
        return_type: VOID,
    },
    NativeSignatureDef {
        target: "std.bytes.fromBase64",
        binding_key: "core.bytes.fromBase64",
        aliases: &["bytes.fromBase64"],
        type_param_count: 0,
        params: &[STRING],
        return_type: BYTES,
    },
    NativeSignatureDef {
        target: "std.bytes.fromHex",
        binding_key: "core.bytes.fromHex",
        aliases: &["bytes.fromHex"],
        type_param_count: 0,
        params: &[STRING],
        return_type: BYTES,
    },
    NativeSignatureDef {
        target: "std.bytes.fromUtf8",
        binding_key: "core.bytes.fromUtf8",
        aliases: &["bytes.fromUtf8"],
        type_param_count: 0,
        params: &[STRING],
        return_type: BYTES,
    },
    NativeSignatureDef {
        target: "std.bytes.concat",
        binding_key: "core.bytes.concat",
        aliases: &["bytes.concat"],
        type_param_count: 0,
        params: &[BYTES_ARRAY],
        return_type: BYTES,
    },
    NativeSignatureDef {
        target: "std.http.request",
        binding_key: "std.http.client.request",
        aliases: &[],
        type_param_count: 0,
        params: &[HTTP_CLIENT_REQUEST],
        return_type: HTTP_CLIENT_RESPONSE,
    },
    NativeSignatureDef {
        target: "std.http.stream",
        binding_key: "std.http.client.stream",
        aliases: &[],
        type_param_count: 0,
        params: &[HTTP_CLIENT_REQUEST],
        return_type: HTTP_CLIENT_STREAM_HANDLE,
    },
    NativeSignatureDef {
        target: "std.http.sse",
        binding_key: "std.http.client.sse",
        aliases: &[],
        type_param_count: 0,
        params: &[HTTP_CLIENT_REQUEST],
        return_type: HTTP_SSE_STREAM,
    },
    NativeSignatureDef {
        target: "std.http.header",
        binding_key: "std.http.request.header",
        aliases: &[],
        type_param_count: 0,
        params: &[HTTP_REQUEST, STRING],
        return_type: STRING_NULLABLE,
    },
    NativeSignatureDef {
        target: "std.http.headers",
        binding_key: "std.http.request.headers",
        aliases: &[],
        type_param_count: 0,
        params: &[HTTP_REQUEST, STRING],
        return_type: STRING_ARRAY,
    },
    NativeSignatureDef {
        target: "std.http.query",
        binding_key: "std.http.request.query",
        aliases: &[],
        type_param_count: 0,
        params: &[HTTP_REQUEST, STRING],
        return_type: STRING_NULLABLE,
    },
    NativeSignatureDef {
        target: "std.http.cookie",
        binding_key: "std.http.request.cookie",
        aliases: &[],
        type_param_count: 0,
        params: &[HTTP_REQUEST, STRING],
        return_type: STRING_NULLABLE,
    },
    NativeSignatureDef {
        target: "std.http.json",
        binding_key: "std.http.response.json",
        aliases: &[],
        type_param_count: 1,
        params: &[INTEGER, T0],
        return_type: HTTP_RESPONSE,
    },
    NativeSignatureDef {
        target: "std.http.jsonWithHeaders",
        binding_key: "std.http.response.jsonWithHeaders",
        aliases: &[],
        type_param_count: 1,
        params: &[INTEGER, T0, HTTP_HEADER_ARRAY],
        return_type: HTTP_RESPONSE,
    },
    NativeSignatureDef {
        target: "std.http.errorResponse",
        binding_key: "std.http.response.error",
        aliases: &[],
        type_param_count: 0,
        params: &[INTEGER, STRING, STRING, JSON_NULLABLE],
        return_type: HTTP_RESPONSE,
    },
    NativeSignatureDef {
        target: "std.http.noContent",
        binding_key: "std.http.response.noContent",
        aliases: &[],
        type_param_count: 0,
        params: &[],
        return_type: HTTP_RESPONSE,
    },
    NativeSignatureDef {
        target: "std.http.methodNotAllowed",
        binding_key: "std.http.response.methodNotAllowed",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING],
        return_type: HTTP_RESPONSE,
    },
    NativeSignatureDef {
        target: "std.http.decodeJson",
        binding_key: "std.http.request.decodeJson",
        aliases: &[],
        type_param_count: 1,
        params: &[HTTP_REQUEST],
        return_type: T0,
    },
    NativeSignatureDef {
        target: "std.http.requireMethod",
        binding_key: "std.http.request.requireMethod",
        aliases: &[],
        type_param_count: 0,
        params: &[HTTP_REQUEST, STRING],
        return_type: HTTP_RESPONSE_NULLABLE,
    },
    NativeSignatureDef {
        target: "std.http.forwardableHeaders",
        binding_key: "std.http.headers.forwardable",
        aliases: &[],
        type_param_count: 0,
        params: &[HTTP_HEADER_ARRAY],
        return_type: HTTP_HEADER_ARRAY,
    },
    NativeSignatureDef {
        target: "std.http.sseHeaders",
        binding_key: "std.http.headers.sse",
        aliases: &[],
        type_param_count: 0,
        params: &[],
        return_type: HTTP_HEADER_ARRAY,
    },
    NativeSignatureDef {
        target: "std.http.streamStart",
        binding_key: "std.http.stream.start",
        aliases: &[],
        type_param_count: 0,
        params: &[INTEGER, HTTP_HEADER_ARRAY],
        return_type: HTTP_RESPONSE_STREAM_EVENT,
    },
    NativeSignatureDef {
        target: "std.http.streamChunk",
        binding_key: "std.http.stream.chunk",
        aliases: &[],
        type_param_count: 0,
        params: &[BYTES],
        return_type: HTTP_RESPONSE_STREAM_EVENT,
    },
    NativeSignatureDef {
        target: "std.http.streamEnd",
        binding_key: "std.http.stream.end",
        aliases: &[],
        type_param_count: 0,
        params: &[],
        return_type: HTTP_RESPONSE_STREAM_EVENT,
    },
    NativeSignatureDef {
        target: "std.http.emitResponseStream",
        binding_key: "std.http.stream.emitResponse",
        aliases: &[],
        type_param_count: 0,
        params: &[HTTP_RESPONSE_STREAM_EVENT],
        return_type: VOID,
    },
    NativeSignatureDef {
        target: "std.file.create",
        binding_key: "std.file.create",
        aliases: &[],
        type_param_count: 0,
        params: &[BYTES, FILE_CREATE_OPTIONS_NULLABLE],
        return_type: FILE_IMMUTABLE,
    },
    NativeSignatureDef {
        target: "std.file.createText",
        binding_key: "std.file.createText",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING, FILE_CREATE_OPTIONS_NULLABLE],
        return_type: FILE_IMMUTABLE,
    },
    NativeSignatureDef {
        target: "std.file.read",
        binding_key: "std.file.read",
        aliases: &[],
        type_param_count: 0,
        params: &[FILE_IMMUTABLE],
        return_type: BYTES,
    },
    NativeSignatureDef {
        target: "std.file.readText",
        binding_key: "std.file.readText",
        aliases: &[],
        type_param_count: 0,
        params: &[FILE_IMMUTABLE],
        return_type: STRING,
    },
    NativeSignatureDef {
        target: "std.file.info",
        binding_key: "std.file.info",
        aliases: &[],
        type_param_count: 0,
        params: &[FILE_IMMUTABLE],
        return_type: FILE_INFO,
    },
    NativeSignatureDef {
        target: "std.file.delete",
        binding_key: "std.file.delete",
        aliases: &[],
        type_param_count: 0,
        params: &[FILE_IMMUTABLE],
        return_type: VOID,
    },
    NativeSignatureDef {
        target: "std.file.createFromStream",
        binding_key: "std.file.createFromStream",
        aliases: &[],
        type_param_count: 0,
        params: &[BYTE_STREAM, FILE_CREATE_OPTIONS_NULLABLE],
        return_type: FILE_IMMUTABLE,
    },
    NativeSignatureDef {
        target: "std.resource.bytes",
        binding_key: "std.resource.bytes",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING],
        return_type: BYTES,
    },
    NativeSignatureDef {
        target: "std.resource.text",
        binding_key: "std.resource.text",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING],
        return_type: STRING,
    },
    NativeSignatureDef {
        target: "std.resource.json",
        binding_key: "std.resource.json",
        aliases: &[],
        type_param_count: 1,
        params: &[STRING],
        return_type: T0,
    },
    NativeSignatureDef {
        target: "std.resource.info",
        binding_key: "std.resource.info",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING],
        return_type: RESOURCE_INFO,
    },
    NativeSignatureDef {
        target: "std.resource.exists",
        binding_key: "std.resource.exists",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING],
        return_type: BOOL,
    },
    NativeSignatureDef {
        target: "std.telemetry.emit",
        binding_key: "std.telemetry.emit",
        aliases: &[],
        type_param_count: 0,
        params: &[
            STRING,
            STRING,
            NativeSignatureTypeExpr::Nullable(&JSON_OBJECT),
        ],
        return_type: VOID,
    },
    NativeSignatureDef {
        target: "std.websocket.sendTextToConnection",
        binding_key: "std.websocket.sendTextToConnection",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING, STRING],
        return_type: VOID,
    },
    NativeSignatureDef {
        target: "std.websocket.sendBinaryToConnection",
        binding_key: "std.websocket.sendBinaryToConnection",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING, BYTES],
        return_type: VOID,
    },
    NativeSignatureDef {
        target: "std.websocket.sendTextToBusinessIdentity",
        binding_key: "std.websocket.sendTextToBusinessIdentity",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING, STRING],
        return_type: VOID,
    },
    NativeSignatureDef {
        target: "std.websocket.sendBinaryToBusinessIdentity",
        binding_key: "std.websocket.sendBinaryToBusinessIdentity",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING, BYTES],
        return_type: VOID,
    },
    NativeSignatureDef {
        target: "std.websocket.requestJsonToConnection",
        binding_key: "std.websocket.requestJsonToConnection",
        aliases: &[],
        type_param_count: 2,
        params: &[STRING, STRING, T0],
        return_type: T1,
    },
];

pub fn is_runtime_receiver_native_binding_key(binding_key: &str) -> bool {
    STD_NATIVE_SIGNATURES
        .iter()
        .filter(|signature| signature.binding_key == binding_key)
        .any(|signature| {
            SUPPORTED_RECEIVER_BUILTIN_OPS
                .iter()
                .any(|spec| native_signature_target_matches_receiver_op(signature.target, spec.op))
        })
}

pub fn native_signature_for_receiver_op(
    op: BuiltinReceiverOp,
) -> Option<&'static NativeSignatureDef> {
    validate_supported_receiver_builtin_op(&op).ok()?;
    let mut matches = STD_NATIVE_SIGNATURES
        .iter()
        .filter(|signature| native_signature_target_matches_receiver_op(signature.target, op));
    let signature = matches.next()?;
    matches.next().is_none().then_some(signature)
}

fn native_signature_target_matches_receiver_op(target: &str, op: BuiltinReceiverOp) -> bool {
    let Some(method) = target
        .strip_prefix(op.receiver.as_str())
        .and_then(|suffix| suffix.strip_prefix('.'))
    else {
        return false;
    };
    method == op.method.as_str()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::{builtin_receiver_op_by_name, CallableMayEffects, ValueProvenance};

    use super::{
        is_runtime_receiver_native_binding_key, native_callable_semantics,
        native_signature_for_receiver_op, STD_NATIVE_CALLABLE_SEMANTICS, STD_NATIVE_SIGNATURES,
        STRING, T0, T1,
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
        assert!(request_semantics.effects.may_suspend);
        assert!(!request_semantics.effects.writes_caller_reachable);
        assert!(!request_semantics.effects.returns_caller_alias);
        assert!(!request_semantics.effects.throws_caller_alias);
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
                    .may_suspend,
                "{raw_send} must remain non-suspending"
            );
        }
    }

    #[test]
    fn native_callable_semantics_registry_is_sparse_exact_and_safe() {
        let expected = BTreeSet::from([
            "std.actor.find",
            "std.actor.getOrCreate",
            "std.actor.remove",
            "std.actor.replace",
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
            "std.string.join",
            "std.string.split",
            "std.string.encodePath",
            "std.string.encodeQueryComponent",
            "std.string.isAsciiDigits",
            "std.string.truncateUtf8Bytes",
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
        for semantics in STD_NATIVE_CALLABLE_SEMANTICS {
            assert!(STD_NATIVE_SIGNATURES
                .iter()
                .any(|signature| signature.binding_key == semantics.binding_key));
            let is_emit_response = semantics.binding_key == "std.http.stream.emitResponse";
            assert_eq!(semantics.effects.writes_caller_reachable, false);
            assert_eq!(semantics.effects.returns_caller_alias, false);
            assert_eq!(semantics.effects.throws_caller_alias, false);
            assert_eq!(semantics.effects.escapes_caller_value, is_emit_response);
            assert_eq!(semantics.effects.requires_same_heap_identity, false);
            assert_eq!(semantics.effects.invokes_unknown_target, false);
            assert_eq!(
                semantics.effects.may_suspend,
                is_emit_response
                    || matches!(
                        semantics.binding_key,
                        "std.actor.getOrCreate"
                            | "std.actor.replace"
                            | "std.actor.find"
                            | "std.actor.remove"
                            | "std.file.create"
                            | "std.file.createFromStream"
                            | "std.http.client.request"
                            | "std.http.client.sse"
                            | "std.http.client.stream"
                            | "std.time.sleep"
                            | "std.websocket.requestJsonToConnection"
                    )
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
                writes_caller_reachable: false,
                returns_caller_alias: false,
                throws_caller_alias: false,
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_suspend: false,
            }
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
                writes_caller_reachable: false,
                returns_caller_alias: false,
                throws_caller_alias: false,
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_suspend: false,
            }
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
                writes_caller_reachable: false,
                returns_caller_alias: false,
                throws_caller_alias: false,
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_suspend: false,
            }
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
                writes_caller_reachable: false,
                returns_caller_alias: false,
                throws_caller_alias: false,
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_suspend: false,
            }
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
                writes_caller_reachable: false,
                returns_caller_alias: false,
                throws_caller_alias: false,
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_suspend: false,
            }
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
                writes_caller_reachable: false,
                returns_caller_alias: false,
                throws_caller_alias: false,
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_suspend: false,
            }
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
                writes_caller_reachable: false,
                returns_caller_alias: false,
                throws_caller_alias: false,
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_suspend: false,
            }
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
                    writes_caller_reachable: false,
                    returns_caller_alias: false,
                    throws_caller_alias: false,
                    escapes_caller_value: false,
                    requires_same_heap_identity: false,
                    invokes_unknown_target: false,
                    may_suspend: false,
                }
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
                writes_caller_reachable: false,
                returns_caller_alias: false,
                throws_caller_alias: false,
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_suspend: true,
            }
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
                writes_caller_reachable: false,
                returns_caller_alias: false,
                throws_caller_alias: false,
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_suspend: true,
            }
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
                    writes_caller_reachable: false,
                    returns_caller_alias: false,
                    throws_caller_alias: false,
                    escapes_caller_value: false,
                    requires_same_heap_identity: false,
                    invokes_unknown_target: false,
                    may_suspend: false,
                }
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
                writes_caller_reachable: false,
                returns_caller_alias: false,
                throws_caller_alias: false,
                escapes_caller_value: true,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_suspend: true,
            }
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
}
