//! Trusted HTTP selector headers and origin-form request metadata
//! (TS `serviceDeploymentSelection.ts` / `bind.ts` parity).

use std::fmt;

use hyper::header::HeaderValue;
use hyper::http::uri::Authority;
use hyper::{HeaderMap, Method, Uri};

use super::error::HttpError;

pub const SERVICE_HEADER: &str = "x-skiff-service";
pub const VERSION_HEADER: &str = "x-skiff-version";
pub const RELEASE_HEADER: &str = "x-skiff-release";
pub const TEST_CASE_CAPABILITY_HEADER: &str = "x-skiff-test-case-capability";
pub const TEST_CASE_PARENT_REQUEST_ID_HEADER: &str = "x-skiff-test-case-parent-request-id";

/// Trusted service-scoped deployment coordinate (release-mode selector).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDeploymentSelector {
    pub service_id: String,
    pub contract_version: String,
}

/// Test-dispatch correlation headers (self-ingress test isolation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestCaseCorrelation {
    pub test_case_capability: String,
    pub parent_request_id: String,
}

/// HTTP request metadata projected into the `request.start` frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequestMetadata {
    pub method: String,
    pub url: String,
    pub path: String,
    pub query: Vec<HttpNameValue>,
    pub headers: Vec<HttpNameValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpNameValue {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestTarget {
    pub host: String,
    pub path: String,
    pub url: String,
    pub query: Vec<HttpNameValue>,
}

pub fn parse_service_deployment_selector(
    headers: &HeaderMap,
) -> Result<ServiceDeploymentSelector, HttpError> {
    let service_id = read_required_selector_header(
        headers,
        SERVICE_HEADER,
        "ServiceSelectorRequired",
        "ServiceSelectorInvalid",
    )?;
    let contract_version = parse_version_selector(headers)?;
    Ok(ServiceDeploymentSelector {
        service_id,
        contract_version,
    })
}

fn read_required_selector_header(
    headers: &HeaderMap,
    name: &str,
    missing_code: &str,
    invalid_code: &str,
) -> Result<String, HttpError> {
    let Some(raw) = single_header_value(headers, name, invalid_code)? else {
        return Err(HttpError::platform(
            400,
            missing_code,
            format!("{name} is required"),
            None,
        ));
    };
    let value = raw.trim();
    if value.is_empty()
        || value != raw
        || value
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(HttpError::platform(
            400,
            invalid_code,
            format!("{name} must be a non-empty canonical token"),
            None,
        ));
    }
    Ok(value.to_string())
}

/// Reads `X-Skiff-Version` with the `X-Skiff-Release` alias and conflict rule.
pub fn parse_version_selector(headers: &HeaderMap) -> Result<String, HttpError> {
    let version = single_header_value(headers, VERSION_HEADER, "InvalidVersionHeader")?;
    let release = single_header_value(headers, RELEASE_HEADER, "InvalidVersionHeader")?;
    let normalize = |value: String| {
        let trimmed = value.trim();
        if trimmed != value || trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    };
    let version = version.and_then(normalize);
    let release = release.and_then(normalize);
    if let (Some(version), Some(release)) = (&version, &release) {
        if version != release {
            return Err(HttpError::platform(
                400,
                "InvalidVersionHeader",
                "X-Skiff-Version conflicts with X-Skiff-Release",
                None,
            ));
        }
    }
    version.or(release).ok_or_else(|| {
        HttpError::platform(
            400,
            "VersionSelectorRequired",
            "X-Skiff-Version is required",
            None,
        )
    })
}

pub fn parse_test_case_correlation(
    headers: &HeaderMap,
) -> Result<Option<TestCaseCorrelation>, HttpError> {
    let capability = header_values(headers, TEST_CASE_CAPABILITY_HEADER);
    let parent = header_values(headers, TEST_CASE_PARENT_REQUEST_ID_HEADER);
    if capability.is_empty() && parent.is_empty() {
        return Ok(None);
    }
    if capability.len() != 1
        || parent.len() != 1
        || !is_test_case_token(&capability[0])
        || !is_test_case_token(&parent[0])
    {
        return Err(HttpError::platform(
            400,
            "InvalidTestCaseCorrelation",
            "test case capability and parent request headers must be a singular valid pair",
            None,
        ));
    }
    Ok(Some(TestCaseCorrelation {
        test_case_capability: capability[0].clone(),
        parent_request_id: parent[0].clone(),
    }))
}

/// Whether any test-case correlation header is present (used to reject
/// automatic CORS preflight regardless of token validity, TS parity).
pub fn has_test_case_correlation_headers(headers: &HeaderMap) -> bool {
    headers.contains_key(TEST_CASE_CAPABILITY_HEADER)
        || headers.contains_key(TEST_CASE_PARENT_REQUEST_ID_HEADER)
}

fn is_test_case_token(value: &str) -> bool {
    (1..=256).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'))
}

/// Parses the canonical origin-form request target (TS
/// `readOriginFormUrlForGatewayMetadata` parity). Returns the canonical host,
/// the path (percent-encoded as received), the full URL and query pairs.
pub fn parse_request_target(headers: &HeaderMap, uri: &Uri) -> Result<RequestTarget, HttpError> {
    let host = read_host(headers)?;
    let raw_target = uri
        .path_and_query()
        .map(|part| part.as_str())
        .unwrap_or("/");
    if !raw_target.starts_with('/') || raw_target.starts_with("//") {
        return Err(HttpError::platform(
            400,
            "RequestUrlInvalid",
            "request target must be canonical origin-form",
            None,
        ));
    }
    let path = uri.path();
    let url = format!("http://{host}{raw_target}");
    Ok(RequestTarget {
        host,
        path: path.to_string(),
        url,
        query: read_query(uri.query()),
    })
}

pub fn build_http_request_metadata(
    method: &Method,
    target: &RequestTarget,
    headers: &HeaderMap,
) -> HttpRequestMetadata {
    let method = method.as_str().to_ascii_uppercase();
    HttpRequestMetadata {
        method,
        url: target.url.clone(),
        path: target.path.clone(),
        query: target.query.clone(),
        headers: read_headers(headers),
    }
}

fn read_host(headers: &HeaderMap) -> Result<String, HttpError> {
    let Some(raw) = single_header_value(headers, "host", "RequestHostInvalid")? else {
        return Err(HttpError::platform(
            400,
            "RequestHostRequired",
            "request Host must be singular and present",
            None,
        ));
    };
    if raw.is_empty() || raw.contains('/') || raw.contains('@') {
        return Err(HttpError::platform(
            400,
            "RequestHostInvalid",
            "request Host is invalid",
            None,
        ));
    }
    let authority = raw.parse::<Authority>().map_err(|_| {
        HttpError::platform(400, "RequestHostInvalid", "request Host is invalid", None)
    })?;
    Ok(authority.to_string())
}

fn read_query(raw: Option<&str>) -> Vec<HttpNameValue> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let mut query = Vec::new();
    for segment in raw.split('&') {
        if segment.is_empty() {
            continue;
        }
        let (name, value) = match segment.split_once('=') {
            Some((name, value)) => (name, value),
            None => (segment, ""),
        };
        query.push(HttpNameValue {
            name: percent_decode(name),
            value: percent_decode(value),
        });
    }
    query
}

fn read_headers(headers: &HeaderMap) -> Vec<HttpNameValue> {
    let mut out = Vec::new();
    for name in headers.keys() {
        let lower = name.as_str().to_ascii_lowercase();
        if lower == TEST_CASE_CAPABILITY_HEADER || lower == TEST_CASE_PARENT_REQUEST_ID_HEADER {
            continue;
        }
        for value in headers.get_all(name) {
            out.push(HttpNameValue {
                name: lower.clone(),
                value: value.to_str().unwrap_or_default().to_string(),
            });
        }
    }
    out
}

fn single_header_value(
    headers: &HeaderMap,
    name: &str,
    invalid_code: &str,
) -> Result<Option<String>, HttpError> {
    let values = header_values(headers, name);
    if values.len() > 1 || values.iter().any(|value| value.contains(',')) {
        return Err(HttpError::platform(
            400,
            invalid_code,
            format!("{name} must be singular"),
            None,
        ));
    }
    Ok(values.into_iter().next())
}

fn header_values<'a>(headers: &'a HeaderMap, name: &str) -> Vec<String> {
    headers
        .get_all(name)
        .iter()
        .filter_map(|value: &'a HeaderValue| value.to_str().ok().map(str::to_string))
        .collect()
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let high = hex_value(bytes[index + 1]);
                let low = hex_value(bytes[index + 2]);
                if let (Some(high), Some(low)) = (high, low) {
                    decoded.push(high << 4 | low);
                    index += 3;
                } else {
                    decoded.push(b'%');
                    index += 1;
                }
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

impl fmt::Display for ServiceDeploymentSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}@{}", self.service_id, self.contract_version)
    }
}
