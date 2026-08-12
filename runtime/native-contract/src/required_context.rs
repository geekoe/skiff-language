use super::{TARGET_STD_HTTP_REQUEST, TARGET_STD_HTTP_SSE, TARGET_STD_HTTP_STREAM};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeRequiredContext {
    None,
    Actor,
    Config,
    Db,
    File,
    Time,
    HttpClient,
    HttpResponseStream,
    Websocket,
    Telemetry,
    Resource,
}

impl NativeRequiredContext {
    pub fn for_binding_key(binding_key: &str) -> Option<Self> {
        Some(match binding_key {
            "std.actor.get" => Self::Actor,
            "core.array.empty"
            | "core.array.push"
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
            | "std.json.get"
            | "std.json.getString"
            | "std.json.getNumber"
            | "std.json.getBool"
            | "std.json.getArray"
            | "std.string.join"
            | "std.string.concat"
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
            | "core.bytes.toUtf8String"
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
            "std.task.status" | "std.task.cancel" => Self::None,
            TARGET_STD_HTTP_REQUEST | TARGET_STD_HTTP_STREAM | TARGET_STD_HTTP_SSE => {
                Self::HttpClient
            }
            "std.http.stream.emitResponse" => Self::HttpResponseStream,
            "std.resource.bytes"
            | "std.resource.text"
            | "std.resource.json"
            | "std.resource.info"
            | "std.resource.exists" => Self::Resource,
            "std.file.create"
            | "std.file.createText"
            | "std.file.read"
            | "std.file.readText"
            | "std.file.info"
            | "std.file.delete"
            | "std.file.createFromStream" => Self::File,
            "std.telemetry.emit" => Self::Telemetry,
            "std.config.require"
            | "std.config.optional"
            | "std.config.has" => Self::Config,
            "std.db.operation" => Self::Db,
            "std.websocket.sendTextToConnection"
            | "std.websocket.sendBinaryToConnection"
            | "std.websocket.sendTextToBusinessIdentity"
            | "std.websocket.sendBinaryToBusinessIdentity"
            | "std.websocket.requestJsonToConnection" => Self::Websocket,
            _ => return None,
        })
    }
}
