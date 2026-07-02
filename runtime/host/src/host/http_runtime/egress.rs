use std::net::{IpAddr, SocketAddr};

use reqwest::Url;

use crate::{
    capability_context::{HttpRuntimeOptions, TARGET_STD_HTTP_REQUEST},
    error::{Result, RuntimeError},
};

#[cfg(test)]
use crate::capability_context::HTTP_REQUEST_ADMIN_OVERRIDE_ENV;

const HTTP_REQUEST_OBVIOUS_LOCAL_HOSTS: &[&str] = &[
    "localhost",
    "localhost.",
    "localhost.localdomain",
    "ip6-localhost",
];

#[derive(Debug)]
pub(super) struct GuardedHttpTarget {
    pub(super) resolved_addrs: Option<Vec<SocketAddr>>,
}

#[cfg(test)]
pub(super) static HTTP_EGRESS_OVERRIDE_TEST_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> =
    std::sync::OnceLock::new();

#[cfg(test)]
pub(crate) async fn with_http_admin_unsafe_override_for_test<R>(
    allow_unsafe_targets: bool,
    f: impl std::future::Future<Output = R>,
) -> R {
    let lock = HTTP_EGRESS_OVERRIDE_TEST_LOCK.get_or_init(|| tokio::sync::Mutex::new(()));
    let _guard = lock.lock().await;

    let previous = std::env::var(HTTP_REQUEST_ADMIN_OVERRIDE_ENV).ok();
    if allow_unsafe_targets {
        std::env::set_var(HTTP_REQUEST_ADMIN_OVERRIDE_ENV, "true");
    } else {
        std::env::remove_var(HTTP_REQUEST_ADMIN_OVERRIDE_ENV);
    }

    let output = f.await;

    match previous {
        Some(value) => std::env::set_var(HTTP_REQUEST_ADMIN_OVERRIDE_ENV, value),
        None => std::env::remove_var(HTTP_REQUEST_ADMIN_OVERRIDE_ENV),
    }

    output
}

pub(super) async fn enforce_http_egress_guard(
    url: &Url,
    options: HttpRuntimeOptions,
    field: &'static str,
) -> Result<GuardedHttpTarget> {
    if options.allow_unsafe_targets() {
        return Ok(GuardedHttpTarget {
            resolved_addrs: None,
        });
    }

    let host = url.host_str().ok_or_else(|| {
        RuntimeError::http_error(
            "std.http.request.url must be an absolute URL with host".to_string(),
            None,
        )
    })?;

    if is_unsafe_host_name(host) {
        return Err(unsafe_host_error(field));
    }

    if let Some(ip) = parse_host_ip_literal(host) {
        if is_unsafe_ip(&ip) {
            return Err(unsafe_host_error(field));
        }
        return Ok(GuardedHttpTarget {
            resolved_addrs: None,
        });
    }

    let port = url.port_or_known_default().unwrap_or(443);
    let addrs = tokio::net::lookup_host((host, port)).await.map_err(|_| {
        RuntimeError::ProviderUnavailable {
            target: TARGET_STD_HTTP_REQUEST.to_string(),
            reason: "connection failed".to_string(),
        }
    })?;

    let mut resolved_addrs = Vec::new();
    for addr in addrs {
        if is_unsafe_ip(&addr.ip()) {
            return Err(unsafe_host_error(field));
        }
        resolved_addrs.push(addr);
    }
    if resolved_addrs.is_empty() {
        return Err(RuntimeError::ProviderUnavailable {
            target: TARGET_STD_HTTP_REQUEST.to_string(),
            reason: "connection failed".to_string(),
        });
    }

    Ok(GuardedHttpTarget {
        resolved_addrs: Some(resolved_addrs),
    })
}

fn is_unsafe_host_name(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if HTTP_REQUEST_OBVIOUS_LOCAL_HOSTS
        .iter()
        .any(|entry| host == *entry || host.ends_with(&format!(".{entry}")))
    {
        return true;
    }
    host.ends_with(".local") || host.starts_with("localhost") || host.starts_with("::1")
}

fn is_unsafe_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_private()
                || v4.is_link_local()
                || *ip == IpAddr::V4(std::net::Ipv4Addr::new(169, 254, 169, 254))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_unsafe_ip(&IpAddr::V4(v4));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || v6.is_unicast_link_local()
                || v6.is_unique_local()
        }
    }
}

fn parse_host_ip_literal(host: &str) -> Option<IpAddr> {
    host.parse::<IpAddr>().ok().or_else(|| {
        host.strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .and_then(|host| host.parse::<IpAddr>().ok())
    })
}

fn unsafe_host_error(field: &str) -> RuntimeError {
    RuntimeError::http_error(
        format!("std.http.request.{field} points to a blocked network target"),
        None,
    )
}
