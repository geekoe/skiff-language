use super::{TARGET_STD_HTTP_REQUEST, TARGET_STD_HTTP_SSE, TARGET_STD_HTTP_STREAM};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeRequiredContext {
    None,
    Actor,
    File,
    Time,
    HttpClient,
    HttpResponseStream,
    Websocket,
    Telemetry,
}

impl NativeRequiredContext {
    pub fn for_binding_key(binding_key: &str) -> Option<Self> {
        Some(match binding_key {
            "actor.put" | "actor.get" | "actor.find" | "actor.remove" => Self::Actor,
            "core.array.empty"
            | "core.map.empty"
            | "core.date.fromEpochMilliseconds"
            | "core.date.parse"
            | "core.date.requireParse"
            | "core.date.toEpochMilliseconds"
            | "core.date.toISOString"
            | "core.date.addMilliseconds"
            | "core.date.diffMilliseconds"
            | "core.date.compare"
            | "core.date.isBefore"
            | "core.date.isAfter"
            | "core.duration.milliseconds"
            | "core.duration.seconds"
            | "core.duration.toMilliseconds"
            | "core.number.parse"
            | "core.number.isInteger"
            | "core.number.isSafeInteger"
            | "core.number.assertSafeInteger"
            | "std.json.encode"
            | "std.json.decode"
            | "std.json.merge"
            | "std.string.join"
            | "std.string.split"
            | "std.string.isAsciiDigits"
            | "std.string.truncateUtf8Bytes"
            | "std.string.encodeQueryComponent"
            | "std.string.encodePath"
            | "std.crypto.hmacSha1Base64"
            | "std.crypto.sha256"
            | "std.crypto.randomToken"
            | "std.crypto.uuid"
            | "std.crypto.uuidSimple"
            | "core.bytes.fromBase64"
            | "core.bytes.fromHex"
            | "core.bytes.fromUtf8"
            | "core.bytes.concat"
            | "std.http.request.header"
            | "std.http.request.headers"
            | "std.http.request.query"
            | "std.http.request.cookie"
            | "std.http.request.decodeJson"
            | "std.http.request.requireMethod"
            | "std.http.response.json"
            | "std.http.response.jsonWithHeaders"
            | "std.http.response.error"
            | "std.http.response.noContent"
            | "std.http.response.methodNotAllowed"
            | "std.http.headers.forwardable"
            | "std.http.headers.sse"
            | "std.http.stream.start"
            | "std.http.stream.chunk"
            | "std.http.stream.end" => Self::None,
            "core.date.now" | "std.time.sleep" => Self::Time,
            TARGET_STD_HTTP_REQUEST | TARGET_STD_HTTP_STREAM | TARGET_STD_HTTP_SSE => {
                Self::HttpClient
            }
            "std.http.stream.emitResponse" => Self::HttpResponseStream,
            "std.file.create"
            | "std.file.createText"
            | "std.file.read"
            | "std.file.readText"
            | "std.file.info"
            | "std.file.delete"
            | "std.file.createFromStream" => Self::File,
            "std.telemetry.emit" => Self::Telemetry,
            "std.websocket.sendTextToConnection"
            | "std.websocket.sendBinaryToConnection"
            | "std.websocket.sendTextToBusinessIdentity"
            | "std.websocket.sendBinaryToBusinessIdentity" => Self::Websocket,
            _ => return None,
        })
    }
}
