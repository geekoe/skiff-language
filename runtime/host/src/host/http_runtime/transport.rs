use std::{
    collections::VecDeque,
    net::SocketAddr,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use reqwest::{
    header::{HeaderMap, HeaderValue},
    redirect::Policy,
    Client,
};

use super::{
    call_context::HttpCallContext,
    cancel::{check_cancel_signals, wait_for_cancel_signals},
    egress::enforce_http_egress_guard,
    input::parse_input,
    HTTP_REQUEST_TIMEOUT_REASON,
};
use crate::error::{Result, RuntimeError};

const HTTP_CLIENT_CACHE_MAX_ENTRIES: usize = 128;

static HTTP_CLIENT_CACHE: OnceLock<Mutex<HttpClientCache>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpClientCacheKey {
    allow_unsafe_targets: bool,
    route: HttpClientRouteKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HttpClientRouteKey {
    Direct {
        host: String,
        resolved_addrs: Option<Vec<SocketAddr>>,
    },
    Proxy {
        proxy_url: String,
    },
}

struct CachedHttpClient {
    key: HttpClientCacheKey,
    client: Client,
}

#[derive(Default)]
struct HttpClientCache {
    entries: VecDeque<CachedHttpClient>,
}

impl HttpClientCache {
    fn get(&mut self, key: &HttpClientCacheKey) -> Option<Client> {
        let index = self.entries.iter().position(|entry| &entry.key == key)?;
        let entry = self
            .entries
            .remove(index)
            .expect("cache entry index should exist");
        let client = entry.client.clone();
        self.entries.push_front(entry);
        Some(client)
    }

    fn insert(&mut self, key: HttpClientCacheKey, client: Client) {
        if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            self.entries.remove(index);
        }
        self.entries.push_front(CachedHttpClient { key, client });
        while self.entries.len() > HTTP_CLIENT_CACHE_MAX_ENTRIES {
            self.entries.pop_back();
        }
    }
}

pub(super) async fn send_request(context: &HttpCallContext<'_, '_>) -> Result<reqwest::Response> {
    check_cancel_signals(context.cancel_signals())?;
    let parsed = parse_input(context.input())?;
    let options = context.options();
    let proxy_url = options.egress_proxy().map(str::to_owned);
    let guarded_target = enforce_http_egress_guard(&parsed.url, options.clone(), "url").await?;

    let timeout_ms = effective_timeout_ms(parsed.timeout_ms, context.frame_deadline_ms());
    if timeout_ms == Some(0) {
        return Err(RuntimeError::ProviderUnavailable {
            target: context.target().to_string(),
            reason: HTTP_REQUEST_TIMEOUT_REASON.to_string(),
        });
    }

    let body_bytes = parsed.body.clone();
    let headers = parsed.headers;

    let client_key = http_client_cache_key(
        options.allow_unsafe_targets(),
        proxy_url.as_deref(),
        &parsed.url,
        guarded_target.resolved_addrs.as_deref(),
    )?;
    let client = cached_http_client_for(context.target(), client_key)?;

    let mut request_builder = client.request(parsed.method, parsed.url);
    let mut request_headers = HeaderMap::new();

    for (name, value) in headers {
        let header_name = name
            .parse::<reqwest::header::HeaderName>()
            .map_err(|error| {
                RuntimeError::http_error(format!("invalid header name {name}: {error}"), None)
            })?;
        let header_value = HeaderValue::from_str(&value).map_err(|error| {
            RuntimeError::http_error(format!("invalid header value for {name}: {error}"), None)
        })?;
        request_headers.append(header_name, header_value);
    }

    request_builder = request_builder.headers(request_headers).body(body_bytes);
    if let Some(timeout_ms) = timeout_ms {
        request_builder = request_builder.timeout(Duration::from_millis(timeout_ms));
    }

    if !context.cancel_signals().is_empty() {
        let mut response_future = request_builder.send();
        tokio::select! {
            response = &mut response_future => {
                response.map_err(|error| map_reqwest_error_for(context.target(), error))
            }
            _ = wait_for_cancel_signals(context.cancel_signals()) => Err(RuntimeError::cancelled()),
        }
    } else {
        request_builder
            .send()
            .await
            .map_err(|error| map_reqwest_error_for(context.target(), error))
    }
}

fn http_client_cache_key(
    allow_unsafe_targets: bool,
    proxy_url: Option<&str>,
    target_url: &reqwest::Url,
    guarded_target_addrs: Option<&[SocketAddr]>,
) -> Result<HttpClientCacheKey> {
    let route = if let Some(proxy_url) = proxy_url {
        HttpClientRouteKey::Proxy {
            proxy_url: proxy_url.to_string(),
        }
    } else {
        let host = target_url.host_str().ok_or_else(|| {
            RuntimeError::http_error(
                "std.http.request.url must be an absolute URL with host".to_string(),
                None,
            )
        })?;
        HttpClientRouteKey::Direct {
            host: host.to_string(),
            resolved_addrs: guarded_target_addrs.map(<[_]>::to_vec),
        }
    };

    Ok(HttpClientCacheKey {
        allow_unsafe_targets,
        route,
    })
}

fn cached_http_client_for(target: &str, key: HttpClientCacheKey) -> Result<Client> {
    let cache = HTTP_CLIENT_CACHE.get_or_init(|| Mutex::new(HttpClientCache::default()));
    if let Some(client) = cache
        .lock()
        .expect("HTTP client cache mutex poisoned")
        .get(&key)
    {
        return Ok(client);
    }

    let client = build_http_client(target, &key)?;

    let mut cache = cache.lock().expect("HTTP client cache mutex poisoned");
    if let Some(existing) = cache.get(&key) {
        return Ok(existing);
    }
    cache.insert(key, client.clone());
    Ok(client)
}

fn build_http_client(target: &str, key: &HttpClientCacheKey) -> Result<Client> {
    let mut client_builder = Client::builder().redirect(Policy::none()).no_proxy();
    match &key.route {
        HttpClientRouteKey::Direct {
            host,
            resolved_addrs,
        } => {
            if let Some(resolved_addrs) = resolved_addrs.as_deref() {
                client_builder = client_builder.resolve_to_addrs(host, resolved_addrs);
            }
        }
        HttpClientRouteKey::Proxy { proxy_url } => {
            client_builder =
                client_builder.proxy(reqwest::Proxy::all(proxy_url.as_str()).map_err(|error| {
                    RuntimeError::http_error(
                        format!("runtime config http.egress.proxy failed to configure: {error}"),
                        None,
                    )
                })?);
        }
    }

    client_builder
        .build()
        .map_err(|_| RuntimeError::ProviderUnavailable {
            target: target.to_string(),
            reason: "failed to build HTTP client".to_string(),
        })
}

pub(super) fn map_reqwest_error_for(target: &str, error: reqwest::Error) -> RuntimeError {
    if error.is_timeout() {
        RuntimeError::ProviderUnavailable {
            target: target.to_string(),
            reason: HTTP_REQUEST_TIMEOUT_REASON.to_string(),
        }
    } else if error.is_connect() {
        RuntimeError::ProviderUnavailable {
            target: target.to_string(),
            reason: "connection failed".to_string(),
        }
    } else {
        RuntimeError::ProviderUnavailable {
            target: target.to_string(),
            reason: "request failed".to_string(),
        }
    }
}

fn effective_timeout_ms(
    input_timeout_ms: Option<u64>,
    frame_deadline_ms: Option<u64>,
) -> Option<u64> {
    match (input_timeout_ms, frame_deadline_ms) {
        (Some(input_timeout_ms), Some(deadline_ms)) => Some(input_timeout_ms.min(deadline_ms)),
        (Some(input_timeout_ms), None) => Some(input_timeout_ms),
        (None, Some(deadline_ms)) => Some(deadline_ms),
        (None, None) => None,
    }
}
