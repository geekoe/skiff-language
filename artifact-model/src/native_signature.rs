use crate::builtin_receiver_ops::{BuiltinReceiverOp, SUPPORTED_RECEIVER_BUILTIN_OPS};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeSignatureDef {
    pub target: &'static str,
    pub binding_key: &'static str,
    pub aliases: &'static [&'static str],
    pub type_param_count: usize,
    pub params: &'static [NativeTypeExprDef],
    pub return_type: NativeTypeExprDef,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeTypeExprDef {
    TypeParam(usize),
    Builtin(&'static str),
    Array(&'static NativeTypeExprDef),
    Map(&'static NativeTypeExprDef, &'static NativeTypeExprDef),
    Nullable(&'static NativeTypeExprDef),
    Stream(&'static NativeTypeExprDef),
    ActorRef(&'static NativeTypeExprDef),
}

const T0: NativeTypeExprDef = NativeTypeExprDef::TypeParam(0);
const T1: NativeTypeExprDef = NativeTypeExprDef::TypeParam(1);
const STRING: NativeTypeExprDef = NativeTypeExprDef::Builtin("string");
const BOOL: NativeTypeExprDef = NativeTypeExprDef::Builtin("bool");
const NUMBER: NativeTypeExprDef = NativeTypeExprDef::Builtin("number");
const INTEGER: NativeTypeExprDef = NativeTypeExprDef::Builtin("integer");
const BYTES: NativeTypeExprDef = NativeTypeExprDef::Builtin("bytes");
const DATE: NativeTypeExprDef = NativeTypeExprDef::Builtin("Date");
const DURATION: NativeTypeExprDef = NativeTypeExprDef::Builtin("Duration");
const JSON: NativeTypeExprDef = NativeTypeExprDef::Builtin("Json");
const JSON_OBJECT: NativeTypeExprDef = NativeTypeExprDef::Builtin("JsonObject");
const VOID: NativeTypeExprDef = NativeTypeExprDef::Builtin("void");
const DATE_NULLABLE: NativeTypeExprDef = NativeTypeExprDef::Nullable(&DATE);
const STRING_ARRAY: NativeTypeExprDef = NativeTypeExprDef::Array(&STRING);
const BYTES_ARRAY: NativeTypeExprDef = NativeTypeExprDef::Array(&BYTES);
const HTTP_HEADER_ARRAY: NativeTypeExprDef = NativeTypeExprDef::Array(&HTTP_HEADER);
const STRING_NULLABLE: NativeTypeExprDef = NativeTypeExprDef::Nullable(&STRING);
const HTTP_RESPONSE_NULLABLE: NativeTypeExprDef = NativeTypeExprDef::Nullable(&HTTP_RESPONSE);
const JSON_NULLABLE: NativeTypeExprDef = NativeTypeExprDef::Nullable(&JSON);
const HTTP_HEADER: NativeTypeExprDef = NativeTypeExprDef::Builtin("std.http.HttpHeader");
const HTTP_REQUEST: NativeTypeExprDef = NativeTypeExprDef::Builtin("std.http.HttpRequest");
const HTTP_RESPONSE: NativeTypeExprDef = NativeTypeExprDef::Builtin("std.http.HttpResponse");
const HTTP_CLIENT_REQUEST: NativeTypeExprDef =
    NativeTypeExprDef::Builtin("std.http.HttpClientRequest");
const HTTP_CLIENT_RESPONSE: NativeTypeExprDef =
    NativeTypeExprDef::Builtin("std.http.HttpClientResponse");
const HTTP_CLIENT_STREAM_HANDLE: NativeTypeExprDef =
    NativeTypeExprDef::Builtin("std.http.HttpClientStreamHandle");
const HTTP_SSE_EVENT: NativeTypeExprDef = NativeTypeExprDef::Builtin("std.http.HttpSseEvent");
const HTTP_RESPONSE_STREAM_EVENT: NativeTypeExprDef =
    NativeTypeExprDef::Builtin("std.http.HttpResponseStreamEvent");
const HTTP_SSE_STREAM: NativeTypeExprDef = NativeTypeExprDef::Stream(&HTTP_SSE_EVENT);
const BYTE_STREAM: NativeTypeExprDef = NativeTypeExprDef::Stream(&BYTES);
const FILE_IMMUTABLE: NativeTypeExprDef = NativeTypeExprDef::Builtin("std.file.ImmutableFile");
const FILE_CREATE_OPTIONS: NativeTypeExprDef = NativeTypeExprDef::Builtin("std.file.CreateOptions");
const FILE_CREATE_OPTIONS_NULLABLE: NativeTypeExprDef =
    NativeTypeExprDef::Nullable(&FILE_CREATE_OPTIONS);
const FILE_INFO: NativeTypeExprDef = NativeTypeExprDef::Builtin("std.file.FileInfo");
const ACTOR_REF_T0: NativeTypeExprDef = NativeTypeExprDef::ActorRef(&T0);
const ACTOR_REF_T0_NULLABLE: NativeTypeExprDef = NativeTypeExprDef::Nullable(&ACTOR_REF_T0);

pub const STD_NATIVE_SIGNATURES: &[NativeSignatureDef] = &[
    NativeSignatureDef {
        target: "std.actor.put",
        binding_key: "actor.put",
        aliases: &[],
        type_param_count: 2,
        params: &[T1, T0],
        return_type: ACTOR_REF_T0,
    },
    NativeSignatureDef {
        target: "std.actor.get",
        binding_key: "actor.get",
        aliases: &[],
        type_param_count: 2,
        params: &[T1],
        return_type: ACTOR_REF_T0,
    },
    NativeSignatureDef {
        target: "std.actor.find",
        binding_key: "actor.find",
        aliases: &[],
        type_param_count: 2,
        params: &[T1],
        return_type: ACTOR_REF_T0_NULLABLE,
    },
    NativeSignatureDef {
        target: "std.actor.remove",
        binding_key: "actor.remove",
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
        return_type: NativeTypeExprDef::Array(&T0),
    },
    NativeSignatureDef {
        target: "Map.empty",
        binding_key: "core.map.empty",
        aliases: &[],
        type_param_count: 2,
        params: &[],
        return_type: NativeTypeExprDef::Map(&T0, &T1),
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
        return_type: NativeTypeExprDef::Nullable(&NUMBER),
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
        target: "std.telemetry.emit",
        binding_key: "std.telemetry.emit",
        aliases: &[],
        type_param_count: 0,
        params: &[STRING, STRING, NativeTypeExprDef::Nullable(&JSON_OBJECT)],
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
    use super::is_runtime_receiver_native_binding_key;

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
}
