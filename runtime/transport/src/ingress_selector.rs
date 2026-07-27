use skiff_artifact_model::{IngressProtocol, IngressSelector};
use url::Url;

use crate::protocol::RequestStartFrameHeader;

const MISSING_SELECTOR: &str =
    "request.start does not contain a complete canonical ingress selector";
const AMBIGUOUS_SELECTOR: &str =
    "request.start contains ambiguous canonical ingress selector inputs";

/// Projects gateway wire metadata to the only identity accepted by assembly ingress.
///
/// Legacy operation selectors, build ids, ABI ids, display paths, and gateway entry
/// identities are deliberately ignored here. If the wire cannot prove one canonical
/// ingress selector, admission fails before any runtime registry is consulted.
pub fn ingress_selector_from_start_frame(
    header: &RequestStartFrameHeader,
) -> Result<IngressSelector, String> {
    match &header.http_request {
        Some(request) => {
            let url = parse_route_url(&request.url, &["http", "https"])?;
            let path = canonical_path(&request.path)?;
            if path != url.path() {
                return Err(AMBIGUOUS_SELECTOR.to_string());
            }
            let method = request.method.trim();
            if method.is_empty() {
                return Err(MISSING_SELECTOR.to_string());
            }
            Ok(IngressSelector {
                protocol: IngressProtocol::Http,
                host: canonical_host(&url)?,
                method: Some(method.to_ascii_uppercase()),
                path: path.to_string(),
            })
        }
        None => Err(MISSING_SELECTOR.to_string()),
    }
}

fn parse_route_url(raw: &str, accepted_schemes: &[&str]) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| MISSING_SELECTOR.to_string())?;
    if !accepted_schemes.contains(&url.scheme())
        || !url.has_host()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(MISSING_SELECTOR.to_string());
    }
    Ok(url)
}

fn canonical_host(url: &Url) -> Result<String, String> {
    let host = url
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or_else(|| MISSING_SELECTOR.to_string())?;
    Ok(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

fn canonical_path(path: &str) -> Result<&str, String> {
    let path = path.trim();
    if path.is_empty() || !path.starts_with('/') || path.contains('#') || path.contains('?') {
        return Err(MISSING_SELECTOR.to_string());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use skiff_artifact_model::{IngressProtocol, IngressSelector};

    use super::ingress_selector_from_start_frame;
    use crate::protocol::{
        RequestStartFrameHeader, RuntimeCallerFrameHeader, RuntimeHttpRequestFrameHeader,
        RuntimeTraceContextFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
    };

    #[test]
    fn projects_http_wire_to_canonical_selector_without_legacy_identity() {
        let mut header = request_start_header();
        header.build_id = "wrong-build".to_string();
        header.operation_abi_id = Some("wrong-abi".to_string());
        header.selector = Some("wrong-display-selector".to_string());
        header.gateway_entry_identity = Some("wrong-gateway-entry".to_string());
        header.http_request = Some(RuntimeHttpRequestFrameHeader {
            method: "get".to_string(),
            url: "https://EXAMPLE.com:8443/items?q=1".to_string(),
            path: "/items".to_string(),
            query: Vec::new(),
            headers: Vec::new(),
        });

        assert_eq!(
            ingress_selector_from_start_frame(&header).expect("HTTP selector should project"),
            IngressSelector {
                protocol: IngressProtocol::Http,
                host: "example.com:8443".to_string(),
                method: Some("GET".to_string()),
                path: "/items".to_string(),
            }
        );
    }

    #[test]
    fn rejects_ambiguous_or_incomplete_wire_metadata() {
        let mut header = request_start_header();
        assert!(ingress_selector_from_start_frame(&header).is_err());

        header.http_request = Some(RuntimeHttpRequestFrameHeader {
            method: "GET".to_string(),
            url: "https://example.com/right".to_string(),
            path: "/wrong".to_string(),
            query: Vec::new(),
            headers: Vec::new(),
        });
        assert!(ingress_selector_from_start_frame(&header).is_err());
    }

    fn request_start_header() -> RequestStartFrameHeader {
        RequestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "request.start".to_string(),
            request_id: "request-1".to_string(),
            mode: "unary".to_string(),
            caller: RuntimeCallerFrameHeader {
                kind: "gateway".to_string(),
                target: "gateway".to_string(),
            },
            target: "legacy-target".to_string(),
            operation_abi_id: None,
            selector: None,
            service_id: None,
            version: None,
            build_id: "legacy-build".to_string(),
            service_protocol_identity: "legacy-protocol".to_string(),
            activation_identity: None,
            gateway_entry_identity: None,
            client_session: None,
            deadline: None,
            trace: RuntimeTraceContextFrameHeader {
                trace_id: "trace-1".to_string(),
                span_id: "span-1".to_string(),
                parent_span_id: None,
                sampled: None,
            },
            http_request: None,
            http_adapter: None,
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
        }
    }
}
