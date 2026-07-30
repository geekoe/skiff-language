use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use crate::canonical_fixture::{CanonicalFixtureError, HttpPhase};

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
    max_response_bytes: usize,
    timing: HttpTiming,
}

#[derive(Debug, Clone, Copy)]
struct HttpTiming {
    started: Instant,
    deadline: Instant,
}

impl HttpTiming {
    fn new(deadline: Instant) -> Self {
        Self {
            started: Instant::now(),
            deadline,
        }
    }
}

pub(super) fn request_url(
    url: &str,
    method: &str,
    host_override: Option<&str>,
    body: &[u8],
    deadline: Instant,
    max_response_bytes: usize,
) -> Result<ConnectedHttpResponse, CanonicalFixtureError> {
    request_url_with_resolver(
        url,
        method,
        host_override,
        body,
        deadline,
        max_response_bytes,
        |authority| {
            authority
                .to_socket_addrs()
                .map(|addresses| addresses.collect())
        },
    )
}

fn request_url_with_resolver<Resolver>(
    url: &str,
    method: &str,
    host_override: Option<&str>,
    body: &[u8],
    deadline: Instant,
    max_response_bytes: usize,
    resolver: Resolver,
) -> Result<ConnectedHttpResponse, CanonicalFixtureError>
where
    Resolver: FnOnce(String) -> std::io::Result<Vec<SocketAddr>> + Send + 'static,
{
    let timing = HttpTiming::new(deadline);
    let target = parse_http_target(url)?;
    let addresses = resolve(target.authority, url, timing, resolver)?;
    if addresses.is_empty() {
        return Err(http_error(
            HttpPhase::Resolve,
            url,
            timing,
            std::io::Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                "HTTP authority resolved to no addresses",
            ),
        ));
    }
    let mut last_error = None;
    for address in addresses {
        match connect(address, url, timing) {
            Ok(stream) => {
                let peer_addr = stream
                    .peer_addr()
                    .map_err(|source| http_error(HttpPhase::Connect, url, timing, source))?;
                let host = host_override.unwrap_or(target.authority);
                let response = exchange(
                    stream,
                    ExchangeRequest {
                        label: url,
                        method,
                        host,
                        path: &target.path,
                        body,
                        max_response_bytes,
                        timing,
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
    Err(last_error.expect("non-empty resolved address list attempted at least one connection"))
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
    request_peer_with_connector(
        peer_addr,
        authority,
        path,
        method,
        body,
        deadline,
        max_response_bytes,
        |address, timeout| TcpStream::connect_timeout(&address, timeout),
    )
}

fn request_peer_with_connector<Connector>(
    peer_addr: SocketAddr,
    authority: &str,
    path: &str,
    method: &str,
    body: &[u8],
    deadline: Instant,
    max_response_bytes: usize,
    connector: Connector,
) -> Result<HttpResponse, CanonicalFixtureError>
where
    Connector: FnOnce(SocketAddr, Duration) -> std::io::Result<TcpStream> + Send + 'static,
{
    if authority.is_empty() || !path.starts_with('/') {
        return Err(CanonicalFixtureError::InvalidInput(
            "explicit-peer HTTP request requires an authority and absolute path".to_string(),
        ));
    }
    let label = format!("http://{authority}{path}");
    let timing = HttpTiming::new(deadline);
    let stream = connect_with(peer_addr, &label, timing, connector)?;
    exchange(
        stream,
        ExchangeRequest {
            label: &label,
            method,
            host: authority,
            path,
            body,
            max_response_bytes,
            timing,
        },
    )
}

fn connect(
    peer_addr: SocketAddr,
    label: &str,
    timing: HttpTiming,
) -> Result<TcpStream, CanonicalFixtureError> {
    connect_with(peer_addr, label, timing, |address, timeout| {
        TcpStream::connect_timeout(&address, timeout)
    })
}

fn connect_with<Connector>(
    peer_addr: SocketAddr,
    label: &str,
    timing: HttpTiming,
    connector: Connector,
) -> Result<TcpStream, CanonicalFixtureError>
where
    Connector: FnOnce(SocketAddr, Duration) -> std::io::Result<TcpStream> + Send + 'static,
{
    run_phase_worker(HttpPhase::Connect, label, timing, move |timeout| {
        connector(peer_addr, timeout)
    })
}

fn resolve<Resolver>(
    authority: &str,
    label: &str,
    timing: HttpTiming,
    resolver: Resolver,
) -> Result<Vec<SocketAddr>, CanonicalFixtureError>
where
    Resolver: FnOnce(String) -> std::io::Result<Vec<SocketAddr>> + Send + 'static,
{
    let authority = authority.to_string();
    run_phase_worker(HttpPhase::Resolve, label, timing, move |_| {
        resolver(authority)
    })
}

fn run_phase_worker<Output, Work>(
    phase: HttpPhase,
    label: &str,
    timing: HttpTiming,
    work: Work,
) -> Result<Output, CanonicalFixtureError>
where
    Output: Send + 'static,
    Work: FnOnce(Duration) -> std::io::Result<Output> + Send + 'static,
{
    let operation_timeout = remaining(timing, phase, label)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = sender.send(work(operation_timeout));
    });
    let wait_timeout = remaining(timing, phase, label)?;
    match receiver.recv_timeout(wait_timeout) {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(source)) => Err(http_error(phase, label, timing, source)),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(http_error(
            phase,
            label,
            timing,
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("HTTP {phase} deadline elapsed"),
            ),
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(http_error(
            phase,
            label,
            timing,
            std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                format!("HTTP {phase} worker exited without a result"),
            ),
        )),
    }
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
        request.timing,
    )?;
    write_bytes(&mut stream, request.label, request.body, request.timing)?;
    let bytes = read_response(
        &mut stream,
        request.label,
        request.timing,
        request.max_response_bytes,
    )?;
    decode_response(request.label, bytes)
}

fn write_bytes(
    stream: &mut TcpStream,
    label: &str,
    bytes: &[u8],
    timing: HttpTiming,
) -> Result<(), CanonicalFixtureError> {
    let mut written = 0;
    while written < bytes.len() {
        stream
            .set_write_timeout(Some(remaining(timing, HttpPhase::Write, label)?))
            .map_err(|source| http_error(HttpPhase::Write, label, timing, source))?;
        match stream.write(&bytes[written..]) {
            Ok(0) => {
                return Err(http_error(
                    HttpPhase::Write,
                    label,
                    timing,
                    std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "failed to write HTTP request",
                    ),
                ));
            }
            Ok(count) => written += count,
            Err(source) if source.kind() == std::io::ErrorKind::Interrupted => {}
            Err(source) => {
                return Err(http_error(HttpPhase::Write, label, timing, source));
            }
        }
    }
    Ok(())
}

fn read_response(
    stream: &mut TcpStream,
    label: &str,
    timing: HttpTiming,
    max_response_bytes: usize,
) -> Result<Vec<u8>, CanonicalFixtureError> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        stream
            .set_read_timeout(Some(remaining(timing, HttpPhase::Read, label)?))
            .map_err(|source| http_error(HttpPhase::Read, label, timing, source))?;
        match stream.read(&mut chunk) {
            Ok(0) if bytes.is_empty() => {
                return Err(http_error(
                    HttpPhase::Read,
                    label,
                    timing,
                    std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "HTTP peer closed before sending a response",
                    ),
                ));
            }
            Ok(0) => return Ok(bytes),
            Ok(count) => {
                let next_size = bytes
                    .len()
                    .checked_add(count)
                    .ok_or_else(|| wire_error(label, "response size accounting overflowed"))?;
                if next_size > max_response_bytes {
                    return Err(wire_error(
                        label,
                        format!("response exceeds {max_response_bytes} bytes"),
                    ));
                }
                bytes.extend_from_slice(&chunk[..count]);
            }
            Err(source) if source.kind() == std::io::ErrorKind::Interrupted => {}
            Err(source) => return Err(http_error(HttpPhase::Read, label, timing, source)),
        }
    }
}

fn decode_response(label: &str, bytes: Vec<u8>) -> Result<HttpResponse, CanonicalFixtureError> {
    let response = String::from_utf8(bytes)
        .map_err(|source| wire_error(label, format!("response is not valid UTF-8: {source}")))?;
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| wire_error(label, "response has no header terminator"))?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let mut status_parts = status_line.split_whitespace();
    let version = status_parts.next().unwrap_or_default();
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return Err(wire_error(label, "response has an invalid HTTP version"));
    }
    let status = status_parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|status| (100..600).contains(status))
        .ok_or_else(|| wire_error(label, "response has an invalid HTTP status"))?;
    let mut content_length = None;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| wire_error(label, "response has an invalid HTTP header"))?;
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(wire_error(
                label,
                "response uses unsupported transfer-encoding",
            ));
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(wire_error(label, "response has duplicate content-length"));
            }
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| wire_error(label, "response has invalid content-length"))?,
            );
        }
    }
    if content_length.is_some_and(|length| length != body.len()) {
        return Err(wire_error(label, "response content-length mismatch"));
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

fn remaining(
    timing: HttpTiming,
    phase: HttpPhase,
    label: &str,
) -> Result<Duration, CanonicalFixtureError> {
    let remaining = timing.deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        Err(http_error(
            phase,
            label,
            timing,
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "HTTP request deadline elapsed",
            ),
        ))
    } else {
        Ok(remaining)
    }
}

fn http_error(
    phase: HttpPhase,
    label: &str,
    timing: HttpTiming,
    source: std::io::Error,
) -> CanonicalFixtureError {
    CanonicalFixtureError::Http {
        phase,
        target: label.to_string(),
        kind: source.kind(),
        raw_os_error: source.raw_os_error(),
        elapsed_ms: timing.started.elapsed().as_millis(),
        deadline_ms: timing
            .deadline
            .saturating_duration_since(timing.started)
            .as_millis(),
        source,
    }
}

fn wire_error(label: &str, message: impl Into<String>) -> CanonicalFixtureError {
    CanonicalFixtureError::Wire {
        context: format!("HTTP response from {label}"),
        message: message.into(),
    }
}

#[cfg(test)]
#[path = "tests/http.rs"]
mod tests;
