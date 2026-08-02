//! CORS preflight / automatic headers and service-managed detection
//! (TS `httpCors.ts` parity).

const CORS_ALLOWED_METHODS: [&str; 7] =
    ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];

const DEFAULT_CORS_ALLOWED_HEADERS: [&str; 9] = [
    "accept",
    "authorization",
    "content-type",
    "x-requested-with",
    "x-skiff-service",
    "x-skiff-version",
    "x-skiff-release",
    "x-skiff-trace-id",
    "x-skiff-user-admin",
];

pub fn is_preflight_request(
    method: &str,
    has_origin: bool,
    has_access_control_request_method: bool,
) -> bool {
    method.eq_ignore_ascii_case("OPTIONS") && has_origin && has_access_control_request_method
}

/// Automatic CORS headers for non-service-managed responses.
pub fn automatic_cors_headers(origin: &str) -> Vec<(String, String)> {
    vec![
        (
            "access-control-allow-origin".to_string(),
            origin.to_string(),
        ),
        (
            "access-control-allow-credentials".to_string(),
            "true".to_string(),
        ),
        ("vary".to_string(), "Origin".to_string()),
    ]
}

/// Automatic 204 preflight headers (`writeAutomaticCorsPreflightResponse`).
pub fn automatic_preflight_headers(
    origin: &str,
    requested_headers: Option<&str>,
) -> Vec<(String, String)> {
    let mut headers = automatic_cors_headers(origin);
    headers.push((
        "access-control-allow-methods".to_string(),
        CORS_ALLOWED_METHODS.join(", "),
    ));
    headers.push((
        "access-control-allow-headers".to_string(),
        allowed_preflight_headers(requested_headers),
    ));
    headers.push(("access-control-max-age".to_string(), "600".to_string()));
    headers.push((
        "vary".to_string(),
        "Access-Control-Request-Method, Access-Control-Request-Headers".to_string(),
    ));
    headers
}

fn allowed_preflight_headers(requested: Option<&str>) -> String {
    let Some(requested) = requested else {
        return DEFAULT_CORS_ALLOWED_HEADERS.join(", ");
    };
    let requested = requested.trim();
    if requested.is_empty() {
        return DEFAULT_CORS_ALLOWED_HEADERS.join(", ");
    }
    let mut seen = Vec::<String>::new();
    for raw in requested.split(',') {
        let header = raw.trim().to_ascii_lowercase();
        if header.is_empty() || !is_valid_header_name(&header) || seen.contains(&header) {
            continue;
        }
        seen.push(header);
    }
    if seen.is_empty() {
        DEFAULT_CORS_ALLOWED_HEADERS.join(", ")
    } else {
        seen.join(", ")
    }
}

pub fn is_cors_response_header(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("access-control-")
}

fn is_valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+.^_`|~-".contains(&byte))
}
