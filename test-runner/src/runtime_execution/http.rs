use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};

use crate::canonical_fixture::CanonicalFixtureError;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct HttpResponse {
    pub(super) status: u16,
    pub(super) body: String,
}

#[derive(Debug)]
pub(super) struct ConnectedHttpResponse {
    pub(super) response: HttpResponse,
    pub(super) peer_addr: SocketAddr,
    pub(super) authority: String,
}

struct HttpTarget<'a> {
    authority: &'a str,
    path: String,
}

struct ExchangeRequest<'a> {
    label: &'a str,
    method: &'a str,
    host: &'a str,
    path: &'a str,
    body: &'a [u8],
    deadline: Instant,
    max_response_bytes: usize,
}

pub(super) fn request_url(
    url: &str,
    method: &str,
    host_override: Option<&str>,
    body: &[u8],
    deadline: Instant,
    max_response_bytes: usize,
) -> Result<ConnectedHttpResponse, CanonicalFixtureError> {
    let target = parse_http_target(url)?;
    let addresses = target
        .authority
        .to_socket_addrs()
        .map_err(|source| io_error(url, source))?;
    let mut last_error = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, remaining(deadline, url)?) {
            Ok(stream) => {
                let peer_addr = stream.peer_addr().map_err(|source| io_error(url, source))?;
                let host = host_override.unwrap_or(target.authority);
                let response = exchange(
                    stream,
                    ExchangeRequest {
                        label: url,
                        method,
                        host,
                        path: &target.path,
                        body,
                        deadline,
                        max_response_bytes,
                    },
                )?;
                return Ok(ConnectedHttpResponse {
                    response,
                    peer_addr,
                    authority: target.authority.to_string(),
                });
            }
            Err(source) => last_error = Some(source),
        }
    }
    Err(io_error(
        url,
        last_error.unwrap_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                "HTTP authority resolved to no addresses",
            )
        }),
    ))
}

pub(super) fn request_peer(
    peer_addr: SocketAddr,
    authority: &str,
    path: &str,
    method: &str,
    body: &[u8],
    deadline: Instant,
    max_response_bytes: usize,
) -> Result<HttpResponse, CanonicalFixtureError> {
    if authority.is_empty() || !path.starts_with('/') {
        return Err(CanonicalFixtureError::InvalidInput(
            "explicit-peer HTTP request requires an authority and absolute path".to_string(),
        ));
    }
    let label = format!("http://{authority}{path}");
    let stream = TcpStream::connect_timeout(&peer_addr, remaining(deadline, &label)?)
        .map_err(|source| io_error(&label, source))?;
    exchange(
        stream,
        ExchangeRequest {
            label: &label,
            method,
            host: authority,
            path,
            body,
            deadline,
            max_response_bytes,
        },
    )
}

fn exchange(
    mut stream: TcpStream,
    request: ExchangeRequest<'_>,
) -> Result<HttpResponse, CanonicalFixtureError> {
    let header = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        request.method,
        request.path,
        request.host,
        request.body.len()
    );
    write_bytes(
        &mut stream,
        request.label,
        header.as_bytes(),
        request.deadline,
    )?;
    write_bytes(&mut stream, request.label, request.body, request.deadline)?;
    let bytes = read_response(
        &mut stream,
        request.label,
        request.deadline,
        request.max_response_bytes,
    )?;
    decode_response(request.label, bytes)
}

fn write_bytes(
    stream: &mut TcpStream,
    label: &str,
    bytes: &[u8],
    deadline: Instant,
) -> Result<(), CanonicalFixtureError> {
    let mut written = 0;
    while written < bytes.len() {
        stream
            .set_write_timeout(Some(remaining(deadline, label)?))
            .map_err(|source| io_error(label, source))?;
        match stream.write(&bytes[written..]) {
            Ok(0) => {
                return Err(io_error(
                    label,
                    std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "failed to write HTTP request",
                    ),
                ));
            }
            Ok(count) => written += count,
            Err(source) if source.kind() == std::io::ErrorKind::Interrupted => {}
            Err(source) => return Err(io_error(label, source)),
        }
    }
    Ok(())
}

fn read_response(
    stream: &mut TcpStream,
    label: &str,
    deadline: Instant,
    max_response_bytes: usize,
) -> Result<Vec<u8>, CanonicalFixtureError> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        stream
            .set_read_timeout(Some(remaining(deadline, label)?))
            .map_err(|source| io_error(label, source))?;
        match stream.read(&mut chunk) {
            Ok(0) => return Ok(bytes),
            Ok(count) => {
                let next_size = bytes.len().checked_add(count).ok_or_else(|| {
                    CanonicalFixtureError::InvalidInput(format!(
                        "HTTP response from {label} exceeds size accounting"
                    ))
                })?;
                if next_size > max_response_bytes {
                    return Err(CanonicalFixtureError::InvalidInput(format!(
                        "HTTP response from {label} exceeds {max_response_bytes} bytes"
                    )));
                }
                bytes.extend_from_slice(&chunk[..count]);
            }
            Err(source) if source.kind() == std::io::ErrorKind::Interrupted => {}
            Err(source) => return Err(io_error(label, source)),
        }
    }
}

fn decode_response(label: &str, bytes: Vec<u8>) -> Result<HttpResponse, CanonicalFixtureError> {
    let response = String::from_utf8(bytes).map_err(|source| {
        CanonicalFixtureError::InvalidInput(format!(
            "HTTP response from {label} is not valid UTF-8: {source}"
        ))
    })?;
    let (head, body) = response.split_once("\r\n\r\n").ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(format!("invalid HTTP response from {label}"))
    })?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let mut status_parts = status_line.split_whitespace();
    let version = status_parts.next().unwrap_or_default();
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "invalid HTTP version from {label}"
        )));
    }
    let status = status_parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|status| (100..600).contains(status))
        .ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(format!("invalid HTTP status from {label}"))
        })?;
    let mut content_length = None;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(format!("invalid HTTP header from {label}"))
        })?;
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(CanonicalFixtureError::InvalidInput(format!(
                "unsupported transfer-encoding from {label}"
            )));
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(CanonicalFixtureError::InvalidInput(format!(
                    "duplicate content-length from {label}"
                )));
            }
            content_length = Some(value.trim().parse::<usize>().map_err(|_| {
                CanonicalFixtureError::InvalidInput(format!("invalid content-length from {label}"))
            })?);
        }
    }
    if content_length.is_some_and(|length| length != body.len()) {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "HTTP content-length mismatch from {label}"
        )));
    }
    Ok(HttpResponse {
        status,
        body: body.to_string(),
    })
}

fn parse_http_target(url: &str) -> Result<HttpTarget<'_>, CanonicalFixtureError> {
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(format!("HTTP fixture URL must use http://: {url}"))
    })?;
    let (authority, path) = rest
        .split_once('/')
        .map(|(authority, path)| (authority, format!("/{path}")))
        .unwrap_or((rest, "/".to_string()));
    if authority.is_empty() {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "HTTP fixture URL must include an authority: {url}"
        )));
    }
    Ok(HttpTarget { authority, path })
}

fn remaining(deadline: Instant, label: &str) -> Result<Duration, CanonicalFixtureError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        Err(io_error(
            label,
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "HTTP request deadline elapsed",
            ),
        ))
    } else {
        Ok(remaining)
    }
}

fn io_error(label: &str, source: std::io::Error) -> CanonicalFixtureError {
    CanonicalFixtureError::Io {
        path: label.to_string(),
        source,
    }
}

#[cfg(test)]
#[path = "tests/http.rs"]
mod tests;
