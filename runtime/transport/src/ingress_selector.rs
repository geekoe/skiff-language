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

fn canonical_path(path: &str) -> Result<&str, String> {
    let path = path.trim();
    if path.is_empty() || !path.starts_with('/') || path.contains('#') || path.contains('?') {
        return Err(MISSING_SELECTOR.to_string());
    }
    Ok(path)
}

#[cfg(test)]
mod tests;
