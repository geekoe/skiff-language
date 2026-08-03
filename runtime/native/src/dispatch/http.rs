use serde_json::Value;
use skiff_runtime_boundary::{contract::RuntimeBoundaryContract, plan::BoundaryUse};
use skiff_runtime_capability_context::NativeCapabilityContexts;

use super::http_helpers::{
    cookie_value, ensure_json_content_type, first_name_value, forwardable_headers,
    http_method_not_allowed_wire, http_response_wire, json_headers, name_values,
    optional_string_value, sse_headers, NameMatch,
};
use super::{
    ensure_native_capability_context, native_capability_route_mismatch,
    prepared::run_prepared_native_call, unsupported_native_target, PreparedExternalNativeOperation,
    PreparedNativeCall, RuntimeNativeInvocation,
};
use crate::error::{Result, RuntimeError};
use crate::{
    call_helpers::runtime_string_arg,
    capability::{NativeHttpClientCapability, NativeHttpResponseStreamCapability},
    runtime_value_facade::{
        bytes_payload, RequestHeap, RuntimeTypeNode, RuntimeTypePlan, RuntimeValue,
    },
};
use skiff_runtime_native_contract::{
    NativeRequiredContext, TARGET_STD_HTTP_REQUEST, TARGET_STD_HTTP_SSE, TARGET_STD_HTTP_STREAM,
};

pub(super) const HTTP_REQUEST_HEADER_KEY: &str = "std.http.request.header";
const HTTP_REQUEST_HEADERS_KEY: &str = "std.http.request.headers";
const HTTP_REQUEST_QUERY_KEY: &str = "std.http.request.query";
const HTTP_REQUEST_COOKIE_KEY: &str = "std.http.request.cookie";
const HTTP_REQUEST_DECODE_JSON_KEY: &str = "std.http.request.decodeJson";
const HTTP_REQUEST_REQUIRE_METHOD_KEY: &str = "std.http.request.requireMethod";
const HTTP_RESPONSE_JSON_KEY: &str = "std.http.response.json";
const HTTP_RESPONSE_JSON_WITH_HEADERS_KEY: &str = "std.http.response.jsonWithHeaders";
const HTTP_RESPONSE_ERROR_KEY: &str = "std.http.response.error";
const HTTP_RESPONSE_NO_CONTENT_KEY: &str = "std.http.response.noContent";
const HTTP_RESPONSE_METHOD_NOT_ALLOWED_KEY: &str = "std.http.response.methodNotAllowed";
const HTTP_HEADERS_FORWARDABLE_KEY: &str = "std.http.headers.forwardable";
const HTTP_HEADERS_SSE_KEY: &str = "std.http.headers.sse";
pub(super) const HTTP_STREAM_START_KEY: &str = "std.http.stream.start";
pub(super) const HTTP_STREAM_CHUNK_KEY: &str = "std.http.stream.chunk";
pub(super) const HTTP_STREAM_END_KEY: &str = "std.http.stream.end";
const HTTP_STREAM_EMIT_RESPONSE_KEY: &str = "std.http.stream.emitResponse";

pub(super) struct HttpNativeDispatch;

impl HttpNativeDispatch {
    pub(super) fn new() -> Self {
        Self
    }

    pub(super) fn matches(target: &str) -> bool {
        matches!(
            target,
            TARGET_STD_HTTP_REQUEST
                | TARGET_STD_HTTP_STREAM
                | TARGET_STD_HTTP_SSE
                | HTTP_REQUEST_HEADER_KEY
                | HTTP_REQUEST_HEADERS_KEY
                | HTTP_REQUEST_QUERY_KEY
                | HTTP_REQUEST_COOKIE_KEY
                | HTTP_REQUEST_DECODE_JSON_KEY
                | HTTP_REQUEST_REQUIRE_METHOD_KEY
                | HTTP_RESPONSE_JSON_KEY
                | HTTP_RESPONSE_JSON_WITH_HEADERS_KEY
                | HTTP_RESPONSE_ERROR_KEY
                | HTTP_RESPONSE_NO_CONTENT_KEY
                | HTTP_RESPONSE_METHOD_NOT_ALLOWED_KEY
                | HTTP_HEADERS_FORWARDABLE_KEY
                | HTTP_HEADERS_SSE_KEY
                | HTTP_STREAM_START_KEY
                | HTTP_STREAM_CHUNK_KEY
                | HTTP_STREAM_END_KEY
                | HTTP_STREAM_EMIT_RESPONSE_KEY
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare<
        'a,
        ActorContext,
        FileContext,
        TimeContext,
        HttpClientContext,
        HttpResponseStreamContext,
        WebsocketContext,
        TelemetryContext,
        ResourceContext,
    >(
        &self,
        native_capability_context: NativeCapabilityContexts<
            ActorContext,
            FileContext,
            TimeContext,
            HttpClientContext,
            HttpResponseStreamContext,
            WebsocketContext,
            TelemetryContext,
            ResourceContext,
        >,
        invocation: RuntimeNativeInvocation,
        diagnostic_target: String,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<PreparedNativeCall<'a>>
    where
        HttpClientContext: NativeHttpClientCapability + Send + 'a,
        HttpResponseStreamContext: NativeHttpResponseStreamCapability + Send + 'a,
    {
        let binding_key = invocation.binding_key().to_string();
        if matches!(
            binding_key.as_str(),
            HTTP_REQUEST_HEADER_KEY
                | HTTP_REQUEST_HEADERS_KEY
                | HTTP_REQUEST_QUERY_KEY
                | HTTP_REQUEST_COOKIE_KEY
                | HTTP_REQUEST_REQUIRE_METHOD_KEY
        ) {
            ensure_http_helper_none_capability_context(&binding_key, &native_capability_context)?;
            let value = self.dispatch_http_request_helper(
                &invocation,
                &binding_key,
                &diagnostic_target,
                args,
                heap,
            )?;
            return Ok(PreparedNativeCall::Ready(value));
        }
        if matches!(
            binding_key.as_str(),
            HTTP_RESPONSE_JSON_KEY
                | HTTP_RESPONSE_JSON_WITH_HEADERS_KEY
                | HTTP_RESPONSE_ERROR_KEY
                | HTTP_RESPONSE_NO_CONTENT_KEY
                | HTTP_RESPONSE_METHOD_NOT_ALLOWED_KEY
        ) {
            ensure_http_helper_none_capability_context(&binding_key, &native_capability_context)?;
            let value = self.dispatch_http_json_helper(
                &invocation,
                &binding_key,
                &diagnostic_target,
                args,
                heap,
            )?;
            return Ok(PreparedNativeCall::Ready(value));
        }
        if binding_key == HTTP_REQUEST_DECODE_JSON_KEY {
            ensure_http_helper_none_capability_context(&binding_key, &native_capability_context)?;
            let value =
                self.dispatch_http_decode_json_helper(&invocation, &diagnostic_target, args, heap)?;
            return Ok(PreparedNativeCall::Ready(value));
        }
        if matches!(
            binding_key.as_str(),
            HTTP_HEADERS_FORWARDABLE_KEY | HTTP_HEADERS_SSE_KEY
        ) {
            ensure_http_helper_none_capability_context(&binding_key, &native_capability_context)?;
            let value = self.dispatch_http_header_list_helper(
                &invocation,
                &binding_key,
                &diagnostic_target,
                args,
                heap,
            )?;
            return Ok(PreparedNativeCall::Ready(value));
        }
        if matches!(
            binding_key.as_str(),
            HTTP_STREAM_START_KEY | HTTP_STREAM_CHUNK_KEY | HTTP_STREAM_END_KEY
        ) {
            ensure_http_helper_none_capability_context(&binding_key, &native_capability_context)?;
            let value = self.dispatch_http_stream_event_helper(
                &invocation,
                &binding_key,
                &diagnostic_target,
                args,
                heap,
            )?;
            return Ok(PreparedNativeCall::Ready(value));
        }
        if binding_key == HTTP_STREAM_EMIT_RESPONSE_KEY {
            let response_stream_context = match native_capability_context {
                NativeCapabilityContexts::HttpResponseStream(response_stream_context) => {
                    response_stream_context
                }
                other => {
                    return Err(native_capability_route_mismatch(
                        &binding_key,
                        NativeRequiredContext::HttpResponseStream,
                        other.required_context(),
                    ));
                }
            };
            let item_type = response_stream_context.response_item_type(&diagnostic_target)?;
            let event = args.first().ok_or_else(|| {
                RuntimeError::Decode(format!("{diagnostic_target} requires event"))
            })?;
            let event = RuntimeBoundaryContract::default()
                .codec_for_expected(
                    &item_type,
                    BoundaryUse::TypedJson,
                    format!("{diagnostic_target} event"),
                )
                .to_wire_json(event, heap)?;
            let wait_target = diagnostic_target.clone();
            return Ok(PreparedNativeCall::ExternalWait(
                external_http_wire_operation(invocation, diagnostic_target, false, async move {
                    response_stream_context
                        .send_response_event(&wait_target, event)
                        .await?;
                    Ok(Value::Null)
                }),
            ));
        }
        let arg_plan = invocation.arg_plans()?.first().cloned();
        let return_plan = invocation.return_plan()?.clone();
        let effect_context = match native_capability_context {
            NativeCapabilityContexts::HttpClient(http_client_context) => http_client_context,
            other => {
                return Err(native_capability_route_mismatch(
                    &binding_key,
                    NativeRequiredContext::HttpClient,
                    other.required_context(),
                ));
            }
        };
        if let Some(value) = effect_context.dispatch_test_http_effect_invocation_double(
            &binding_key,
            args.first(),
            arg_plan.as_ref(),
            Some(&return_plan),
            heap,
        ) {
            return value.map(PreparedNativeCall::Ready);
        }

        let input = args.first().ok_or_else(|| {
            RuntimeError::Decode(format!("{diagnostic_target} requires one argument"))
        })?;
        let input = invocation.native_boundary()?.to_wire_arg(
            0,
            input,
            &format!("{diagnostic_target} input"),
            heap,
        )?;
        let internal_handle = binding_key == TARGET_STD_HTTP_STREAM;
        let wait = match binding_key.as_str() {
            TARGET_STD_HTTP_REQUEST => {
                Box::pin(async move { effect_context.dispatch_http_request(&input).await })
                    as std::pin::Pin<
                        Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>,
                    >
            }
            TARGET_STD_HTTP_STREAM => {
                let item_type =
                    http_stream_body_item_plan(&diagnostic_target, &return_plan)?.clone();
                Box::pin(async move {
                    effect_context
                        .dispatch_http_stream(&input, Some(&item_type))
                        .await
                })
            }
            TARGET_STD_HTTP_SSE => {
                let item_type =
                    stream_item_plan_from_return_plan(&diagnostic_target, &return_plan)?.clone();
                Box::pin(async move {
                    effect_context
                        .dispatch_http_sse(&input, Some(&item_type))
                        .await
                })
            }
            _ => return Err(unsupported_native_target(&binding_key)),
        };
        Ok(PreparedNativeCall::ExternalWait(
            external_http_wire_operation(invocation, diagnostic_target, internal_handle, wait),
        ))
    }

    #[allow(dead_code, clippy::too_many_arguments)]
    pub(super) async fn dispatch<
        ActorContext,
        FileContext,
        TimeContext,
        HttpClientContext,
        HttpResponseStreamContext,
        WebsocketContext,
        TelemetryContext,
        ResourceContext,
    >(
        &self,
        native_capability_context: NativeCapabilityContexts<
            ActorContext,
            FileContext,
            TimeContext,
            HttpClientContext,
            HttpResponseStreamContext,
            WebsocketContext,
            TelemetryContext,
            ResourceContext,
        >,
        invocation: RuntimeNativeInvocation,
        diagnostic_target: String,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue>
    where
        HttpClientContext: NativeHttpClientCapability + Send,
        HttpResponseStreamContext: NativeHttpResponseStreamCapability + Send,
    {
        let prepared = self.prepare(
            native_capability_context,
            invocation,
            diagnostic_target,
            args,
            heap,
        )?;
        run_prepared_native_call(prepared, heap).await
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_http_request_helper(
        &self,
        invocation: &RuntimeNativeInvocation,
        binding_key: &str,
        diagnostic_target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let request = args
            .first()
            .ok_or_else(|| RuntimeError::Decode(format!("{diagnostic_target} requires request")))?;
        let request = invocation.native_boundary()?.to_wire_arg(
            0,
            request,
            &format!("{diagnostic_target} request"),
            heap,
        )?;
        let name = args
            .get(1)
            .ok_or_else(|| RuntimeError::Decode(format!("{diagnostic_target} requires name")))?;
        let name = runtime_string_arg(name, &format!("{diagnostic_target} name"))?;
        let value = match binding_key {
            HTTP_REQUEST_HEADER_KEY => optional_string_value(first_name_value(
                &request,
                "headers",
                name,
                NameMatch::AsciiCaseInsensitive,
            )),
            HTTP_REQUEST_HEADERS_KEY => Value::Array(
                name_values(&request, "headers", name, NameMatch::AsciiCaseInsensitive)
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
            HTTP_REQUEST_QUERY_KEY => {
                optional_string_value(first_name_value(&request, "query", name, NameMatch::Exact))
            }
            HTTP_REQUEST_COOKIE_KEY => optional_string_value(cookie_value(
                &name_values(
                    &request,
                    "headers",
                    "cookie",
                    NameMatch::AsciiCaseInsensitive,
                ),
                name,
            )),
            HTTP_REQUEST_REQUIRE_METHOD_KEY => {
                let actual = request
                    .as_object()
                    .and_then(|object| object.get("method"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        RuntimeError::Decode(format!(
                            "{diagnostic_target} request.method must be a string"
                        ))
                    })?;
                if actual.eq_ignore_ascii_case(name) {
                    Value::Null
                } else {
                    http_method_not_allowed_wire(name)
                }
            }
            _ => return Err(unsupported_native_target(binding_key)),
        };
        invocation.native_boundary()?.from_wire_return(
            &value,
            &format!("{diagnostic_target} response"),
            heap,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_http_json_helper(
        &self,
        invocation: &RuntimeNativeInvocation,
        binding_key: &str,
        diagnostic_target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let native_boundary = invocation.native_boundary()?;
        if binding_key == HTTP_RESPONSE_NO_CONTENT_KEY {
            let response = http_response_wire(204, Value::Array(vec![]), Vec::new());
            return native_boundary.from_wire_return(
                &response,
                &format!("{diagnostic_target} response"),
                heap,
            );
        }
        if binding_key == HTTP_RESPONSE_METHOD_NOT_ALLOWED_KEY {
            let allow = args.first().ok_or_else(|| {
                RuntimeError::Decode(format!("{diagnostic_target} requires allow"))
            })?;
            let allow = runtime_string_arg(allow, &format!("{diagnostic_target} allow"))?;
            let response = http_method_not_allowed_wire(allow);
            return native_boundary.from_wire_return(
                &response,
                &format!("{diagnostic_target} response"),
                heap,
            );
        }
        if binding_key == HTTP_RESPONSE_ERROR_KEY {
            let status = http_status_arg(args.first(), diagnostic_target)?;
            let code = args.get(1).ok_or_else(|| {
                RuntimeError::Decode(format!("{diagnostic_target} requires code"))
            })?;
            let code = runtime_string_arg(code, &format!("{diagnostic_target} code"))?;
            let message = args.get(2).ok_or_else(|| {
                RuntimeError::Decode(format!("{diagnostic_target} requires message"))
            })?;
            let message = runtime_string_arg(message, &format!("{diagnostic_target} message"))?;
            let detail = args.get(3).ok_or_else(|| {
                RuntimeError::Decode(format!("{diagnostic_target} requires detail"))
            })?;
            let detail = native_boundary.to_wire_arg(
                3,
                detail,
                &format!("{diagnostic_target} detail"),
                heap,
            )?;
            let body = serde_json::to_vec(&serde_json::json!({
                "code": code,
                "message": message,
                "detail": detail,
            }))
            .map_err(|error| {
                RuntimeError::Decode(format!("{diagnostic_target} body encode failed: {error}"))
            })?;
            let response = http_response_wire(status, json_headers(), body);
            return native_boundary.from_wire_return(
                &response,
                &format!("{diagnostic_target} response"),
                heap,
            );
        }
        let status = http_status_arg(args.first(), diagnostic_target)?;
        let value = args
            .get(1)
            .ok_or_else(|| RuntimeError::Decode(format!("{diagnostic_target} requires value")))?;
        let encoded = RuntimeBoundaryContract::default()
            .codec_for_expected(
                invocation.arg_plan(1)?,
                BoundaryUse::HttpResponse,
                format!("{diagnostic_target} body"),
            )
            .encode_json_text_value(value, heap)?;
        let headers = if binding_key == HTTP_RESPONSE_JSON_WITH_HEADERS_KEY {
            let headers = args.get(2).ok_or_else(|| {
                RuntimeError::Decode(format!("{diagnostic_target} requires headers"))
            })?;
            let mut headers = native_boundary.to_wire_arg(
                2,
                headers,
                &format!("{diagnostic_target} headers"),
                heap,
            )?;
            ensure_json_content_type(&mut headers);
            headers
        } else {
            json_headers()
        };
        let response = http_response_wire(status, headers, encoded.into_bytes());
        native_boundary.from_wire_return(&response, &format!("{diagnostic_target} response"), heap)
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_http_header_list_helper(
        &self,
        invocation: &RuntimeNativeInvocation,
        binding_key: &str,
        diagnostic_target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let value = match binding_key {
            HTTP_HEADERS_FORWARDABLE_KEY => {
                let headers = args.first().ok_or_else(|| {
                    RuntimeError::Decode(format!("{diagnostic_target} requires headers"))
                })?;
                let headers = invocation.native_boundary()?.to_wire_arg(
                    0,
                    headers,
                    &format!("{diagnostic_target} headers"),
                    heap,
                )?;
                forwardable_headers(headers.as_array().ok_or_else(|| {
                    RuntimeError::Decode(format!("{diagnostic_target} headers must be an array"))
                })?)
            }
            HTTP_HEADERS_SSE_KEY => sse_headers(),
            _ => return Err(unsupported_native_target(binding_key)),
        };
        invocation.native_boundary()?.from_wire_return(
            &value,
            &format!("{diagnostic_target} response"),
            heap,
        )
    }

    fn dispatch_http_decode_json_helper(
        &self,
        invocation: &RuntimeNativeInvocation,
        diagnostic_target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let request = args
            .first()
            .ok_or_else(|| RuntimeError::Decode(format!("{diagnostic_target} requires request")))?;
        let request = invocation.native_boundary()?.to_wire_arg(
            0,
            request,
            &format!("{diagnostic_target} request"),
            heap,
        )?;
        let body = request
            .as_object()
            .and_then(|object| object.get("body"))
            .and_then(bytes_payload)
            .ok_or_else(|| {
                RuntimeError::Decode(format!("{diagnostic_target} request.body must be bytes"))
            })?;
        let body = String::from_utf8(body).map_err(|error| {
            RuntimeError::Decode(format!("{diagnostic_target} body is not UTF-8: {error}"))
        })?;
        Ok(RuntimeBoundaryContract::default()
            .codec_for_expected(
                invocation.return_plan()?,
                BoundaryUse::HttpRequest,
                format!("{diagnostic_target} response"),
            )
            .decode_json_text(&body, heap)?)
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_http_stream_event_helper(
        &self,
        invocation: &RuntimeNativeInvocation,
        binding_key: &str,
        diagnostic_target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let native_boundary = invocation.native_boundary()?;
        let value = match binding_key {
            HTTP_STREAM_START_KEY => {
                let status = http_status_arg(args.first(), diagnostic_target)?;
                let headers = args.get(1).ok_or_else(|| {
                    RuntimeError::Decode(format!("{diagnostic_target} requires headers"))
                })?;
                let headers = native_boundary.to_wire_arg(
                    1,
                    headers,
                    &format!("{diagnostic_target} headers"),
                    heap,
                )?;
                serde_json::json!({
                    "tag": "start",
                    "status": status,
                    "headers": headers,
                })
            }
            HTTP_STREAM_CHUNK_KEY => {
                let chunk = args.first().ok_or_else(|| {
                    RuntimeError::Decode(format!("{diagnostic_target} requires value"))
                })?;
                let chunk = native_boundary.to_wire_arg(
                    0,
                    chunk,
                    &format!("{diagnostic_target} value"),
                    heap,
                )?;
                serde_json::json!({
                    "tag": "chunk",
                    "value": chunk,
                })
            }
            HTTP_STREAM_END_KEY => serde_json::json!({ "tag": "end" }),
            _ => return Err(unsupported_native_target(binding_key)),
        };
        native_boundary.from_wire_return(&value, &format!("{diagnostic_target} response"), heap)
    }
}

fn external_http_wire_operation<'a>(
    invocation: RuntimeNativeInvocation,
    diagnostic_target: String,
    internal_handle: bool,
    wait: impl std::future::Future<Output = Result<Value>> + Send + 'a,
) -> PreparedExternalNativeOperation<'a> {
    PreparedExternalNativeOperation::new(wait, move |value, heap| {
        if internal_handle {
            invocation
                .native_boundary()?
                .from_wire_internal_handle_return(
                    &value,
                    &format!("{diagnostic_target} response"),
                    heap,
                )
        } else {
            invocation.native_boundary()?.from_wire_return(
                &value,
                &format!("{diagnostic_target} response"),
                heap,
            )
        }
    })
}

fn stream_item_plan_from_return_plan<'a>(
    target: &str,
    return_plan: &'a RuntimeTypePlan,
) -> Result<&'a RuntimeTypePlan> {
    match return_plan.node() {
        RuntimeTypeNode::Stream(item_type) => Ok(item_type),
        _ => Err(RuntimeError::InvalidArtifact(format!(
            "{target} return type is not a Stream"
        ))),
    }
}

fn http_stream_body_item_plan<'a>(
    target: &str,
    return_plan: &'a RuntimeTypePlan,
) -> Result<&'a RuntimeTypePlan> {
    match return_plan.node() {
        RuntimeTypeNode::Record { fields, .. } => {
            let body = fields
                .iter()
                .find(|field| field.name == "body")
                .ok_or_else(|| {
                    RuntimeError::InvalidArtifact(format!("{target} return type is missing body"))
                })?;
            stream_item_plan_from_return_plan(target, &body.ty)
        }
        _ => Err(RuntimeError::InvalidArtifact(format!(
            "{target} return type is not a stream handle record"
        ))),
    }
}

pub(super) fn http_status_arg(value: Option<&RuntimeValue>, target: &str) -> Result<u16> {
    let value = value.ok_or_else(|| RuntimeError::Decode(format!("{target} requires status")))?;
    let RuntimeValue::Number(value) = value else {
        return Err(RuntimeError::Decode(format!(
            "{target} status must be an integer"
        )));
    };
    if value.fract() != 0.0 || !(100.0..=599.0).contains(value) {
        return Err(RuntimeError::Decode(format!(
            "{target} status must be an integer between 100 and 599"
        )));
    }
    Ok(*value as u16)
}

pub(super) fn ensure_http_helper_none_capability_context<
    ActorContext,
    FileContext,
    TimeContext,
    HttpClientContext,
    HttpResponseStreamContext,
    WebsocketContext,
    TelemetryContext,
    ResourceContext,
>(
    binding_key: &str,
    native_capability_context: &NativeCapabilityContexts<
        ActorContext,
        FileContext,
        TimeContext,
        HttpClientContext,
        HttpResponseStreamContext,
        WebsocketContext,
        TelemetryContext,
        ResourceContext,
    >,
) -> Result<()> {
    ensure_native_capability_context(
        binding_key,
        NativeRequiredContext::None,
        native_capability_context.required_context(),
    )
}
