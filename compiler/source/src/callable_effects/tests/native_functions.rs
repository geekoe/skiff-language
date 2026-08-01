use super::support::*;

#[test]
fn exact_context_free_native_uses_shared_callable_semantics() {
    let model = AnalysisFixture::new(
        r#"
            function digits(input: string) -> bool {
              return std.string.isAsciiDigits(input)
            }

            function truncate(input: string, maxBytes: number) -> string {
              return std.string.truncateUtf8Bytes(input, maxBytes)
            }

            function query(input: string) -> string {
              return std.string.encodeQueryComponent(input)
            }

            function path(input: string) -> string {
              return std.string.encodePath(input)
            }
        "#,
    )
    .analyze();

    for callable in ["digits", "truncate", "query", "path"] {
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
        let CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
            ..
        } = provenance(&model, callable)
        else {
            panic!("{callable} should retain exact native provenance");
        };
        assert_eq!(return_origins, &vec![ValueProvenance::Fresh], "{callable}");
        assert!(throw_origins.is_empty(), "{callable}");
        assert!(escape_lanes.is_empty(), "{callable}");
    }

    let native_keys = model
        .resolved_call_targets()
        .iter()
        .filter_map(|(_, target)| match target {
            ResolvedCallTarget::NativeFunction { binding_key } => Some(binding_key.as_str()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        native_keys,
        std::collections::BTreeSet::from([
            "std.string.encodePath",
            "std.string.encodeQueryComponent",
            "std.string.isAsciiDigits",
            "std.string.truncateUtf8Bytes",
        ])
    );
}

#[test]
fn date_from_epoch_milliseconds_wrapper_uses_exact_native_semantics() {
    let model = AnalysisFixture::new(
        r#"
            function fromEpoch(milliseconds: integer) -> Date {
              return Date.fromEpochMilliseconds(milliseconds)
            }
        "#,
    )
    .analyze();

    assert_eq!(effects(&model, "fromEpoch"), no_effects());
    let CallableProvenanceSummary::Analyzed {
        return_origins,
        throw_origins,
        escape_lanes,
        ..
    } = provenance(&model, "fromEpoch")
    else {
        panic!("Date constructor wrapper should retain exact native provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);
    assert!(throw_origins.is_empty());
    assert!(escape_lanes.is_empty());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "core.date.fromEpochMilliseconds"
        )
    }));
}

#[test]
fn map_empty_materialization_accumulator_uses_exact_native_semantics() {
    let model = AnalysisFixture::new(
        r#"
            function materializeCompletedResult() -> Map<string, Json> {
              const accumulator = Map.empty<string, Json>()
              return accumulator
            }
        "#,
    )
    .module("responses")
    .package("agine.ai/llm-api")
    .analyze();

    assert_eq!(
        effects_in(&model, "responses", "materializeCompletedResult"),
        no_effects()
    );
    let CallableProvenanceSummary::Analyzed {
        return_origins,
        throw_origins,
        escape_lanes,
        ..
    } = provenance_in(&model, "responses", "materializeCompletedResult")
    else {
        panic!("Map.empty accumulator should retain exact native provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);
    assert!(throw_origins.is_empty());
    assert!(escape_lanes.is_empty());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "core.map.empty"
        )
    }));
}

#[test]
fn json_decode_materialization_uses_exact_detached_semantics() {
    let model = AnalysisFixture::new(
        r#"
            type Event { id: string, values: Array<string> }

            function materializeCompletedResult(encoded: string) -> Event? {
              const decoded = catch<std.json.DecodeError>(
                std.json.decode<Event>(encoded)
              )
              if decoded.tag != "ok" {
                return null
              }
              return decoded.value
            }
        "#,
    )
    .module("responses")
    .package("agine.ai/llm-api")
    .analyze();

    assert_eq!(
        effects_in(&model, "responses", "materializeCompletedResult"),
        no_effects()
    );
    let CallableProvenanceSummary::Analyzed {
        return_origins,
        throw_origins,
        escape_lanes,
        ..
    } = provenance_in(&model, "responses", "materializeCompletedResult")
    else {
        panic!("std.json.decode should retain exact detached provenance");
    };
    assert!(
        return_origins.contains(&ValueProvenance::Fresh)
            && return_origins.contains(&ValueProvenance::Constant)
            && !return_origins.iter().any(is_caller_parameter_provenance),
        "unexpected return provenance: {return_origins:?}"
    );
    assert!(throw_origins.is_empty());
    assert!(escape_lanes.is_empty());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "std.json.decode"
        )
    }));
}

#[test]
fn json_merge_materialization_uses_exact_detached_semantics() {
    let model = AnalysisFixture::new(
        r#"
            function applyProviderOptions(base: Json, overlay: Json) -> Json {
              return std.json.merge(base, overlay)
            }
        "#,
    )
    .module("internal.aihub_service")
    .package("agine.ai/aihub")
    .analyze();

    assert_eq!(
        effects_in(&model, "internal.aihub_service", "applyProviderOptions"),
        no_effects()
    );
    assert!(matches!(
        provenance_in(
            &model,
            "internal.aihub_service",
            "applyProviderOptions"
        ),
        CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
            ..
        } if return_origins == &vec![ValueProvenance::Fresh]
            && throw_origins.is_empty()
            && escape_lanes.is_empty()
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "std.json.merge"
        )
    }));
}

#[test]
fn optional_date_parse_wrapper_uses_exact_native_semantics() {
    let model = AnalysisFixture::new(
        r#"
            function optionalInputDate(value: string?) -> Date? {
              if value == null {
                return null
              }
              return Date.parse(value)
            }

            function adminUpstreamSourceCreate(accessTokenExpiresAt: string?) -> Date? {
              return optionalInputDate(accessTokenExpiresAt)
            }
        "#,
    )
    .module("upstream_sources")
    .package("agine.ai/codex-relay")
    .analyze();

    for callable in ["optionalInputDate", "adminUpstreamSourceCreate"] {
        assert_eq!(
            effects_in(&model, "upstream_sources", callable),
            no_effects(),
            "{callable}"
        );
        let CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
            ..
        } = provenance_in(&model, "upstream_sources", callable)
        else {
            panic!("{callable} should retain exact native provenance");
        };
        assert!(
            return_origins.contains(&ValueProvenance::Fresh)
                && return_origins.contains(&ValueProvenance::Constant),
            "{callable}: {return_origins:?}"
        );
        assert!(throw_origins.is_empty(), "{callable}");
        assert!(escape_lanes.is_empty(), "{callable}");
    }

    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "core.date.parse"
        )
    }));
}

#[test]
fn bytes_from_base64_wrapper_uses_exact_native_semantics() {
    let model = AnalysisFixture::new(
        r#"
            function jwtPayload(value: string) -> bytes {
              return bytes.fromBase64(value)
            }
        "#,
    )
    .analyze();

    assert_eq!(effects(&model, "jwtPayload"), no_effects());
    let CallableProvenanceSummary::Analyzed {
        return_origins,
        throw_origins,
        escape_lanes,
        ..
    } = provenance(&model, "jwtPayload")
    else {
        panic!("Base64 decoder wrapper should retain exact native provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);
    assert!(throw_origins.is_empty());
    assert!(escape_lanes.is_empty());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "core.bytes.fromBase64"
        )
    }));
}

#[test]
fn bytes_from_hex_wrapper_uses_exact_native_semantics() {
    let model = AnalysisFixture::new(
        r#"
            function exactChunk(value: string) -> bytes {
              return bytes.fromHex(value)
            }
        "#,
    )
    .analyze();

    assert_eq!(effects(&model, "exactChunk"), no_effects());
    let CallableProvenanceSummary::Analyzed {
        return_origins,
        throw_origins,
        escape_lanes,
        ..
    } = provenance(&model, "exactChunk")
    else {
        panic!("hex decoder wrapper should retain exact native provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);
    assert!(throw_origins.is_empty());
    assert!(escape_lanes.is_empty());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "core.bytes.fromHex"
        )
    }));
}

#[test]
fn bytes_concat_openai_multipart_shape_uses_exact_native_semantics() {
    let model = AnalysisFixture::new(
        r#"
            type MultipartPart { body: bytes }

            function multipartBody(parts: Array<MultipartPart>, boundary: string) -> bytes {
              const chunks = Array.empty<bytes>()
              for part in parts {
                chunks.push(bytes.fromUtf8("--".concat(boundary).concat("\r\n")))
                chunks.push(part.body)
                chunks.push(bytes.fromUtf8("\r\n"))
              }
              chunks.push(bytes.fromUtf8("--".concat(boundary).concat("--\r\n")))
              return bytes.concat(chunks)
            }
        "#,
    )
    .analyze();

    assert_eq!(effects(&model, "multipartBody"), no_effects());
    let CallableProvenanceSummary::Analyzed {
        return_origins,
        throw_origins,
        escape_lanes,
        ..
    } = provenance(&model, "multipartBody")
    else {
        panic!("multipart bytes concatenation should retain exact native provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);
    assert!(throw_origins.is_empty());
    assert!(escape_lanes.is_empty());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "core.bytes.concat"
        )
    }));
}

#[test]
fn exact_http_request_natives_transfer_through_local_helpers() {
    let model = AnalysisFixture::new(
        r#"
            function cookieValue(request: std.http.HttpRequest) -> string? {
              return std.http.cookie(request, "session")
            }

            function headerValues(request: std.http.HttpRequest) -> Array<string> {
              return std.http.headers(request, "x-trace")
            }

            function handler(request: std.http.HttpRequest) -> std.http.HttpResponse {
              const values = headerValues(request)
              const session = cookieValue(request)
              return std.http.HttpResponse {
                status: 200,
                headers: Array.empty<std.http.HttpHeader>(),
                body: bytes.fromUtf8("ok"),
              }
            }
        "#,
    )
    .analyze();

    for (callable, expected_origins) in [
        ("cookieValue", vec![ValueProvenance::Fresh]),
        ("headerValues", vec![ValueProvenance::Fresh]),
        (
            "handler",
            vec![ValueProvenance::Fresh, ValueProvenance::Constant],
        ),
    ] {
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
        let CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
            ..
        } = provenance(&model, callable)
        else {
            panic!("{callable} should retain exact HTTP request native provenance");
        };
        assert_eq!(return_origins, &expected_origins, "{callable}");
        assert!(throw_origins.is_empty(), "{callable}");
        assert!(escape_lanes.is_empty(), "{callable}");
    }

    let native_keys = model
        .resolved_call_targets()
        .iter()
        .filter_map(|(_, target)| match target {
            ResolvedCallTarget::NativeFunction { binding_key } => Some(binding_key.as_str()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert!(native_keys.contains("std.http.request.headers"));
    assert!(native_keys.contains("std.http.request.cookie"));
}

#[test]
fn exact_http_client_stream_is_fresh_detached_and_suspending_through_raw_request() {
    let model = AnalysisFixture::new(r#"
            function rawRequest(input: std.http.HttpClientRequest) -> std.http.HttpClientRequest {
              return std.http.HttpClientRequest {
                method: input.method,
                url: input.url,
                headers: input.headers,
                body: input.body,
                timeoutMs: input.timeoutMs,
              }
            }

            function responses(input: std.http.HttpClientRequest) -> std.http.HttpClientStreamHandle {
              return std.http.stream(rawRequest(input))
            }
        "#).analyze();

    assert_eq!(
        effects(&model, "rawRequest"),
        CallableMayEffects {
            returns_caller_alias: true,
            ..no_effects()
        }
    );
    assert_eq!(effects(&model, "responses"), suspend_only_effects());
    assert!(matches!(
        provenance(&model, "responses"),
        CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
            ..
        } if return_origins == &vec![ValueProvenance::Fresh]
            && throw_origins.is_empty()
            && escape_lanes.is_empty()
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "std.http.client.stream"
        )
    }));
}

#[test]
fn exact_http_client_sse_is_fresh_detached_and_suspending_through_raw_request() {
    let model = AnalysisFixture::new(
        r#"
            function rawRequest(input: std.http.HttpClientRequest) -> std.http.HttpClientRequest {
              return std.http.HttpClientRequest {
                method: input.method,
                url: input.url,
                headers: input.headers,
                body: input.body,
                timeoutMs: input.timeoutMs,
              }
            }

            function responses(input: std.http.HttpClientRequest) -> Stream<std.http.HttpSseEvent> {
              return std.http.sse(rawRequest(input))
            }
        "#,
    )
    .analyze();

    assert_eq!(
        effects(&model, "rawRequest"),
        CallableMayEffects {
            returns_caller_alias: true,
            ..no_effects()
        }
    );
    assert_eq!(effects(&model, "responses"), suspend_only_effects());
    assert!(matches!(
        provenance(&model, "responses"),
        CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
            ..
        } if return_origins == &vec![ValueProvenance::Fresh]
            && throw_origins.is_empty()
            && escape_lanes.is_empty()
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "std.http.client.sse"
        )
    }));
}

#[test]
fn exact_http_response_stream_event_constructors_are_fresh_and_effect_free() {
    let model = AnalysisFixture::new(
        r#"
            function start(
              status: integer,
              headers: Array<std.http.HttpHeader>
            ) -> std.http.HttpResponseStreamEvent {
              return std.http.streamStart(status, headers)
            }

            function chunk(value: bytes) -> std.http.HttpResponseStreamEvent {
              return std.http.streamChunk(value)
            }

            function end() -> std.http.HttpResponseStreamEvent {
              return std.http.streamEnd()
            }

            function safeResponses(
              status: integer,
              headers: Array<std.http.HttpHeader>,
              value: bytes
            ) -> std.http.HttpResponseStreamEvent {
              const started = std.http.streamStart(status, headers)
              const chunked = std.http.streamChunk(value)
              return std.http.streamEnd()
            }
        "#,
    )
    .analyze();

    for callable in ["start", "chunk", "end", "safeResponses"] {
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed {
                return_origins,
                throw_origins,
                escape_lanes,
                ..
            } if return_origins == &vec![ValueProvenance::Fresh]
                && throw_origins.is_empty()
                && escape_lanes.is_empty()
        ));
    }

    let native_keys = model
        .resolved_call_targets()
        .iter()
        .filter_map(|(_, target)| match target {
            ResolvedCallTarget::NativeFunction { binding_key } => Some(binding_key.as_str()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        native_keys,
        std::collections::BTreeSet::from([
            "std.http.stream.chunk",
            "std.http.stream.end",
            "std.http.stream.start",
        ])
    );
}

#[test]
fn exact_http_response_stream_emit_escapes_and_suspends_only_for_caller_event() {
    let model = AnalysisFixture::new(
        r#"
            function emit(event: std.http.HttpResponseStreamEvent) -> void {
              std.http.emitResponseStream(event)
            }

            function emitFresh(value: bytes) -> void {
              std.http.emitResponseStream(std.http.streamChunk(value))
            }
        "#,
    )
    .analyze();

    assert_eq!(
        effects(&model, "emit"),
        CallableMayEffects {
            escapes_caller_value: true,
            may_suspend: true,
            ..no_effects()
        }
    );
    assert!(matches!(
        provenance(&model, "emit"),
        CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
            ..
        } if return_origins.is_empty()
            && throw_origins.is_empty()
            && escape_lanes == &vec![ValueEscapeLane::External]
    ));
    assert_eq!(
        effects(&model, "emitFresh"),
        CallableMayEffects {
            may_suspend: true,
            ..no_effects()
        }
    );
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "std.http.stream.emitResponse"
        )
    }));
}

#[test]
fn std_exact_native_matrix_uses_shared_callable_semantics() {
    let model = AnalysisFixture::new(
        r#"
            function dateNow() -> Date {
              return Date.now()
            }

            function durationMilliseconds() -> Duration {
              return Duration.milliseconds(1)
            }

            function durationSeconds() -> Duration {
              return Duration.seconds(1)
            }

            function safeInteger() -> integer {
              return std.number.assertSafeInteger(1)
            }

            function parseNumber(value: string) -> number? {
              return std.number.parse(value)
            }

            function hmac() -> string {
              return std.crypto.hmacSha1Base64("key", "text")
            }

            function sha256() -> string {
              return std.crypto.sha256("text")
            }

            function randomToken() -> string {
              return std.crypto.randomToken()
            }

            function uuid() -> string {
              return std.crypto.uuid()
            }

            function uuidSimple() -> string {
              return std.crypto.uuidSimple()
            }

            function sleep() -> void {
              return std.time.sleep(Duration.milliseconds(0))
            }
        "#,
    )
    .module("std.effect_test")
    .package(crate::shared::id::SKIFF_STD_PUBLICATION_ID)
    .analyze();

    for callable in [
        "dateNow",
        "durationMilliseconds",
        "durationSeconds",
        "safeInteger",
        "parseNumber",
        "hmac",
        "sha256",
        "randomToken",
        "uuid",
        "uuidSimple",
    ] {
        assert_eq!(
            effects_in(&model, "std.effect_test", callable),
            no_effects(),
            "{callable}"
        );
    }
    assert_eq!(
        effects_in(&model, "std.effect_test", "sleep"),
        suspend_only_effects()
    );

    let native_keys = model
        .resolved_call_targets()
        .iter()
        .filter_map(|(_, target)| match target {
            ResolvedCallTarget::NativeFunction { binding_key } => Some(binding_key.as_str()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        native_keys,
        std::collections::BTreeSet::from([
            "core.date.now",
            "core.duration.milliseconds",
            "core.duration.seconds",
            "core.number.parse",
            "core.number.assertSafeInteger",
            "std.crypto.hmacSha1Base64",
            "std.crypto.randomToken",
            "std.crypto.sha256",
            "std.crypto.uuid",
            "std.crypto.uuidSimple",
            "std.time.sleep",
        ])
    );
}

#[test]
fn exact_package_boundary_callables_transfer_canonical_effects_and_provenance() {
    let model = AnalysisFixture::new(
        r#"
            type Payload { value: string }

            function emptyArray() -> Array<string> {
              return Array.empty<string>()
            }

            function utf8() -> bytes {
              return bytes.fromUtf8("value")
            }

            function json() -> string {
              return std.json.encode(Payload { value: "ok" })
            }

            function decode(value: string) -> Payload {
              return std.json.decode<Payload>(value)
            }

            function join(items: Array<string>) -> string {
              return string.join(items, ",")
            }

            function split(value: string) -> Array<string> {
              return string.split(value, ",")
            }

            function arrayLength(items: Array<string>) -> number {
              return items.length()
            }

            function bytesLength(value: bytes) -> number {
              return value.length()
            }

            function floor(value: number) -> number {
              return value.floor()
            }

            function ceil(value: number) -> number {
              return value.ceil()
            }

            function round(value: number) -> number {
              return value.round()
            }

            function concat(value: string) -> string {
              return value.concat("!")
            }

            function endsWith(value: string) -> bool {
              return value.endsWith("!")
            }

            function lowercase(value: string) -> string {
              return value.lowercase()
            }

            function startsWith(value: string) -> bool {
              return value.startsWith("!")
            }

            function request(input: std.http.HttpClientRequest) -> std.http.HttpClientResponse {
              return std.http.request(input)
            }

            function push(items: Array<string>) -> void {
              return items.push("value")
            }
        "#,
    )
    .module("std.effect_test")
    .package(crate::shared::id::SKIFF_STD_PUBLICATION_ID)
    .analyze();

    for callable in [
        "emptyArray",
        "utf8",
        "json",
        "decode",
        "join",
        "split",
        "arrayLength",
        "bytesLength",
        "floor",
        "ceil",
        "round",
        "concat",
        "endsWith",
        "lowercase",
        "startsWith",
    ] {
        assert_eq!(
            effects_in(&model, "std.effect_test", callable),
            no_effects(),
            "{callable}"
        );
        assert!(
            matches!(
                provenance_in(&model, "std.effect_test", callable),
                CallableProvenanceSummary::Analyzed { .. }
            ),
            "{callable}"
        );
    }
    let CallableProvenanceSummary::Analyzed { return_origins, .. } =
        provenance_in(&model, "std.effect_test", "ceil")
    else {
        panic!("number.ceil must keep exact detached provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);

    assert_eq!(
        effects_in(&model, "std.effect_test", "request"),
        suspend_only_effects()
    );
    let CallableProvenanceSummary::Analyzed { return_origins, .. } =
        provenance_in(&model, "std.effect_test", "request")
    else {
        panic!("HTTP response must keep exact detached provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);

    assert_eq!(
        effects_in(&model, "std.effect_test", "push"),
        CallableMayEffects {
            writes_caller_reachable: true,
            ..no_effects()
        }
    );
    let CallableProvenanceSummary::Analyzed { return_origins, .. } =
        provenance_in(&model, "std.effect_test", "push")
    else {
        panic!("Array.push must keep exact constant-null provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Constant]);
}

#[test]
fn config_intrinsics_are_exact_detached_sources() {
    let model = AnalysisFixture::new(
        r#"
            type Config { name: string, optional: string?, present: bool }
            function load() -> Config {
              return Config {
                name: config.require<string>("name"),
                optional: config.optional<string>("optional"),
                present: config.has("present"),
              }
            }
        "#,
    )
    .analyze();
    assert_eq!(effects(&model, "load"), no_effects());
    assert!(matches!(
        provenance(&model, "load"),
        CallableProvenanceSummary::Analyzed { .. }
    ));
    let targets = model
        .resolved_call_targets()
        .iter()
        .filter_map(|(_, target)| match target {
            ResolvedCallTarget::ConfigIntrinsic { intrinsic } => Some(*intrinsic),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        targets,
        vec![
            crate::ConfigIntrinsic::Require,
            crate::ConfigIntrinsic::Optional,
            crate::ConfigIntrinsic::Has,
        ]
    );
    assert!(model
        .resolved_call_targets()
        .iter()
        .all(|(_, target)| !matches!(target, ResolvedCallTarget::Unknown { .. })));
}

#[test]
fn exact_file_creation_wrappers_are_fresh_and_only_suspend() {
    let model = AnalysisFixture::new(
        r#"
            function createBytes(content: bytes) -> std.file.ImmutableFile {
              return std.file.create(content, null)
            }

            function createStream(source: Stream<bytes>) -> std.file.ImmutableFile {
              return std.file.createFromStream(source, null)
            }
        "#,
    )
    .module("std.file_effect_test")
    .package(crate::shared::id::SKIFF_STD_PUBLICATION_ID)
    .analyze();

    for (callable, binding_key) in [
        ("createBytes", "std.file.create"),
        ("createStream", "std.file.createFromStream"),
    ] {
        assert_eq!(
            effects_in(&model, "std.file_effect_test", callable),
            suspend_only_effects(),
            "{callable}"
        );
        assert!(matches!(
            provenance_in(&model, "std.file_effect_test", callable),
            CallableProvenanceSummary::Analyzed {
                return_origins,
                throw_origins,
                escape_lanes,
                ..
            } if return_origins == &vec![ValueProvenance::Fresh]
                && throw_origins.is_empty()
                && escape_lanes.is_empty()
        ));
        assert!(model.resolved_call_targets().iter().any(|(_, target)| {
            matches!(
                target,
                ResolvedCallTarget::NativeFunction { binding_key: actual }
                    if actual == binding_key
            )
        }));
    }
}
