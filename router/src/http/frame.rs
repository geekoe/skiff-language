//! Typed `request.start` construction and response frame decode/validation
//! over the canonical `skiff-runtime-transport` codecs (C-model-request).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use serde_json::Value;
use skiff_runtime_request_contract::OpaqueServiceError;
use skiff_runtime_transport::protocol::{
    decode_response_error_frame, decode_typed_binary_frame, encode_binary_frame,
    ResponseChunkFrameHeader, ResponseEndFrameHeader, ResponseStartFrameHeader,
    RUNTIME_FRAME_SCHEMA_VERSION,
};
use skiff_runtime_transport::protocol::{
    BytecodeHttpRequestFrameHeader, BytecodeRequestCallerFrameHeader,
    BytecodeRequestDeadlineFrameHeader, BytecodeRequestIngressFrameHeader,
    BytecodeRequestIngressProtocol, BytecodeRequestNameValueFrameHeader,
    BytecodeRequestRoutingFrameHeader, BytecodeRequestStartFrameHeader,
    BytecodeRequestTraceFrameHeader,
};
use skiff_runtime_transport::TransportError;

use super::error::HttpError;
use super::ingress::HttpIngressBinding;
use super::selector::{HttpRequestMetadata, TestCaseCorrelation};

static REQUEST_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Builds the canonical HTTP `request.start` typed header for one ingress
/// (M4: the routing header carries the release-resolved build id; the
/// assembly identity/generation tuple fields are left absent).
pub fn build_request_start_header(
    binding: &HttpIngressBinding,
    request_id: String,
    timeout: Duration,
    metadata: &HttpRequestMetadata,
    test_correlation: Option<&TestCaseCorrelation>,
) -> Result<BytecodeRequestStartFrameHeader, HttpError> {
    let (timeout_ms, expires_at) = deadline_parts(timeout);
    let gateway_entry_identity = binding.gateway_entry_identity.clone();
    let method = binding
        .selector
        .method
        .as_deref()
        .unwrap_or_default()
        .to_ascii_uppercase();
    Ok(BytecodeRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id,
        mode: binding.mode.as_wire_str().to_string(),
        caller: BytecodeRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: BytecodeRequestRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: None,
            assembly_generation: None,
            deployment: binding.deployment.clone(),
            build_id: Some(binding.build_id.clone()),
            gateway_entry_identity,
            ingress: BytecodeRequestIngressFrameHeader {
                protocol: BytecodeRequestIngressProtocol::Http,
                method,
                path: binding.selector.path.clone(),
            },
        },
        client_session: None,
        deadline: Some(BytecodeRequestDeadlineFrameHeader {
            timeout_ms,
            expires_at,
        }),
        trace: BytecodeRequestTraceFrameHeader {
            trace_id: new_trace_id(),
            span_id: new_span_id(),
            parent_span_id: None,
            sampled: None,
        },
        http_request: BytecodeHttpRequestFrameHeader {
            method: metadata.method.clone(),
            url: metadata.url.clone(),
            path: metadata.path.clone(),
            query: metadata
                .query
                .iter()
                .map(|item| BytecodeRequestNameValueFrameHeader {
                    name: item.name.clone(),
                    value: item.value.clone(),
                })
                .collect(),
            headers: metadata
                .headers
                .iter()
                .map(|item| BytecodeRequestNameValueFrameHeader {
                    name: item.name.clone(),
                    value: item.value.clone(),
                })
                .collect(),
        },
        test_effects_enabled: test_correlation.is_some(),
        test_case_capability: test_correlation
            .map(|correlation| correlation.test_case_capability.clone()),
        test_case_parent_request_id: test_correlation.and_then(|correlation| {
            (!correlation.parent_request_id.is_empty())
                .then(|| correlation.parent_request_id.clone())
        }),
    })
}

/// Encodes a `request.start` binary frame with the raw opaque body payload.
pub fn encode_request_start_frame(
    header: &BytecodeRequestStartFrameHeader,
    payload: &[u8],
) -> Result<Vec<u8>, TransportError> {
    encode_binary_frame(header, payload)
}

/// One decoded response frame from the dispatcher's runtime peer.
#[derive(Debug, Clone, PartialEq)]
pub enum HttpResponseFrame {
    Start {
        http_response: skiff_runtime_transport::protocol::RuntimeHttpResponseFrameHeader,
    },
    Chunk {
        seq: u64,
        payload: Bytes,
    },
    End {
        payload: Bytes,
        http_response: Option<skiff_runtime_transport::protocol::RuntimeHttpResponseFrameHeader>,
    },
    ErrorControl {
        code: String,
        message: String,
        status: Option<u16>,
        details: Option<Value>,
    },
    ErrorFixedService(OpaqueServiceError),
}

/// Decodes one canonical response-family binary frame (fail closed on
/// unknown types, wrong payload presence and malformed fixed-service errors).
pub fn decode_response_frame(frame: &[u8]) -> Result<HttpResponseFrame, String> {
    let decoded = skiff_runtime_transport::protocol::decode_binary_frame(frame)
        .map_err(|error| error.to_string())?;
    let header = decoded.header;
    let _payload = decoded.payload_bytes;
    let frame_type = header
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "response frame is missing type".to_string())?;
    match frame_type {
        "response.start" => {
            let (decoded, payload): (ResponseStartFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(frame).map_err(|error| error.to_string())?;
            if !payload.is_empty() {
                return Err("response.start payload must be empty".to_string());
            }
            Ok(HttpResponseFrame::Start {
                http_response: decoded.http_response,
            })
        }
        "response.chunk" => {
            let (decoded, payload): (ResponseChunkFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(frame).map_err(|error| error.to_string())?;
            Ok(HttpResponseFrame::Chunk {
                seq: decoded.seq,
                payload: Bytes::from(payload),
            })
        }
        "response.end" => {
            let (decoded, payload): (ResponseEndFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(frame).map_err(|error| error.to_string())?;
            if decoded.payload_present == payload.is_empty() {
                return Err(
                    "response.end payloadPresent must match the response payload presence"
                        .to_string(),
                );
            }
            let http_response = match decoded.metadata {
                skiff_runtime_transport::protocol::ResponseEndFrameMetadata::None => None,
                skiff_runtime_transport::protocol::ResponseEndFrameMetadata::Http(http) => {
                    Some(http)
                }
            };
            Ok(HttpResponseFrame::End {
                payload: Bytes::from(payload),
                http_response,
            })
        }
        "response.error" => {
            let (_, validated) =
                decode_response_error_frame(frame).map_err(|error| error.to_string())?;
            match validated {
                skiff_runtime_transport::protocol::ValidatedResponseErrorFrame::Control(error) => {
                    Ok(HttpResponseFrame::ErrorControl {
                        code: error.code,
                        message: error.message,
                        status: error.status,
                        details: error.details,
                    })
                }
                skiff_runtime_transport::protocol::ValidatedResponseErrorFrame::FixedService(
                    error,
                ) => Ok(HttpResponseFrame::ErrorFixedService(error)),
            }
        }
        other => Err(format!("unexpected response frame type {other:?}")),
    }
}

/// Unary `response.end` must be in the HTTP phase (status/headers present).
pub fn unary_http_response(
    frame: HttpResponseFrame,
) -> Result<
    (
        u16,
        Vec<skiff_runtime_transport::protocol::RuntimeHttpNameValueFrameHeader>,
        Bytes,
    ),
    HttpError,
> {
    let HttpResponseFrame::End {
        payload,
        http_response,
    } = frame
    else {
        return Err(HttpError::platform(
            502,
            "InvalidRuntimeResponse",
            "unary dispatch did not complete with response.end",
            None,
        ));
    };
    let Some(http_response) = http_response else {
        return Err(HttpError::platform(
            502,
            "InvalidRuntimeResponse",
            "HTTP unary response must include status and headers",
            None,
        ));
    };
    Ok((http_response.status, http_response.headers, payload))
}

pub fn new_request_id() -> String {
    format!(
        "req-{}-{}-{}",
        now_nanos(),
        REQUEST_ID_COUNTER.fetch_add(1, Ordering::Relaxed),
        std::process::id()
    )
}

pub(crate) fn new_trace_id() -> String {
    format!("trace-{}", now_nanos())
}

pub(crate) fn new_span_id() -> String {
    format!(
        "span-{}-{}",
        now_nanos(),
        REQUEST_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Fresh canonical test-case correlation capability for one test dispatch
/// (TS `randomUUID()` parity; bounded canonical token).
pub(crate) fn new_test_case_capability() -> String {
    format!(
        "test-case-{}-{}",
        now_nanos(),
        REQUEST_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

pub(crate) fn deadline_parts(timeout: Duration) -> (u64, String) {
    let timeout_ms = timeout.as_millis().min(u64::MAX as u128) as u64;
    let expires_at = SystemTime::now() + timeout;
    (timeout_ms, format_iso8601(expires_at))
}

fn format_iso8601(instant: SystemTime) -> String {
    let duration = instant.duration_since(UNIX_EPOCH).unwrap_or_default();
    let millis = duration.as_millis() as u64;
    let days = millis / 86_400_000;
    let millis_of_day = millis % 86_400_000;
    let (year, month, day) = civil_from_days(days as i64);
    let hour = millis_of_day / 3_600_000;
    let minute = (millis_of_day % 3_600_000) / 60_000;
    let second = (millis_of_day % 60_000) / 1000;
    let millisecond = millis_of_day % 1000;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millisecond:03}Z")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}
