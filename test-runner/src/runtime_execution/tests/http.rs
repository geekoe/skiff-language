use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use super::*;

#[test]
fn hostname_activation_returns_peer_and_explicit_peer_preserves_host_without_dns() {
    let (activation_addr, activation_server) = spawn_server(|mut stream| {
        let request = read_request_head(&mut stream);
        write_response(&mut stream, b"activated");
        request
    });
    let activation_authority = format!("localhost:{}", activation_addr.port());
    let activation = request_url(
        &format!("http://{activation_authority}/__skiff/activate-assembly"),
        "POST",
        None,
        b"{}",
        Instant::now() + Duration::from_secs(1),
        4096,
    )
    .unwrap();
    assert_eq!(activation.response.body, "activated");
    assert_eq!(activation.peer_addr, activation_addr);
    assert_eq!(activation.authority, activation_authority);
    let activation_request = activation_server.join().unwrap();
    assert!(activation_request.contains(&format!("Host: {activation_authority}\r\n")));

    let (health_addr, health_server) = spawn_server(|mut stream| {
        let request = read_request_head(&mut stream);
        write_response(&mut stream, b"healthy");
        request
    });
    let unresolved_authority = "must-not-resolve.invalid:4321";
    let health = request_peer(
        health_addr,
        unresolved_authority,
        "/__router/health",
        "GET",
        &[],
        Instant::now() + Duration::from_secs(1),
        4096,
    )
    .unwrap();
    assert_eq!(health.body, "healthy");
    let health_request = health_server.join().unwrap();
    assert!(health_request.contains("GET /__router/health HTTP/1.1\r\n"));
    assert!(health_request.contains(&format!("Host: {unresolved_authority}\r\n")));
}

#[test]
fn expired_absolute_deadline_prevents_connect() {
    let started = Instant::now();
    let result = request_peer(
        "127.0.0.1:9".parse().unwrap(),
        "ignored.invalid:9",
        "/__router/health",
        "GET",
        &[],
        Instant::now(),
        4096,
    );

    assert_http_failure(
        result.unwrap_err(),
        HttpPhase::Connect,
        std::io::ErrorKind::TimedOut,
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn stalled_connect_uses_the_absolute_deadline_and_reports_connect_phase() {
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let started = Instant::now();
    let result = request_peer_with_connector(
        "127.0.0.1:9".parse().unwrap(),
        "stalled-connect.invalid:9",
        "/test",
        "GET",
        &[],
        started + Duration::from_millis(20),
        4096,
        move |_, _| {
            let _ = release_rx.recv();
            Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "connector released after caller deadline",
            ))
        },
    );
    release_tx.send(()).unwrap();

    assert_http_failure(
        result.unwrap_err(),
        HttpPhase::Connect,
        std::io::ErrorKind::TimedOut,
    );
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[test]
fn stalled_resolve_uses_the_same_absolute_deadline_and_reports_resolve_phase() {
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let started = Instant::now();
    let result = request_url_with_resolver(
        "http://stalled-resolve.invalid/test",
        "GET",
        None,
        &[],
        started + Duration::from_millis(20),
        4096,
        move |_| {
            let _ = release_rx.recv();
            Ok(Vec::new())
        },
    );
    release_tx.send(()).unwrap();

    assert_http_failure(
        result.unwrap_err(),
        HttpPhase::Resolve,
        std::io::ErrorKind::TimedOut,
    );
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[test]
fn absolute_deadline_bounds_read_and_server_joins() {
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (addr, server) = spawn_server(move |mut stream| {
        let _request = read_request_head(&mut stream);
        let _ = release_rx.recv();
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    });
    let started = Instant::now();
    let result = request_peer(
        addr,
        "read-deadline.invalid",
        "/__router/health",
        "GET",
        &[],
        started + Duration::from_millis(20),
        4096,
    );

    assert_http_failure(result.unwrap_err(), HttpPhase::Read, None);
    assert!(started.elapsed() < Duration::from_millis(500));
    release_tx.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn absolute_deadline_bounds_write_and_server_joins() {
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (addr, server) = spawn_server(move |_stream| {
        let _ = release_rx.recv();
    });
    let body = vec![b'x'; 32 * 1024 * 1024];
    let started = Instant::now();
    let result = request_peer(
        addr,
        "write-deadline.invalid",
        "/__router/health",
        "POST",
        &body,
        started + Duration::from_millis(20),
        4096,
    );

    assert_http_failure(result.unwrap_err(), HttpPhase::Write, None);
    assert!(started.elapsed() < Duration::from_millis(500));
    release_tx.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn response_size_limit_fails_closed_and_server_joins() {
    let (addr, server) = spawn_server(|mut stream| {
        let _request = read_request_head(&mut stream);
        write_response(&mut stream, &[b'x'; 128]);
    });
    let result = request_peer(
        addr,
        "size-limit.invalid",
        "/__router/health",
        "GET",
        &[],
        Instant::now() + Duration::from_secs(1),
        64,
    );

    let error = result.unwrap_err().to_string();
    assert!(error.contains("exceeds 64 bytes"), "{error}");
    server.join().unwrap();
}

#[test]
fn invalid_utf8_and_content_length_mismatch_fail_closed() {
    let (utf8_addr, utf8_server) = spawn_server(|mut stream| {
        let _request = read_request_head(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n\xff")
            .unwrap();
    });
    let utf8 = request_peer(
        utf8_addr,
        "utf8.invalid",
        "/__router/health",
        "GET",
        &[],
        Instant::now() + Duration::from_secs(1),
        4096,
    );
    assert!(utf8.unwrap_err().to_string().contains("not valid UTF-8"));
    utf8_server.join().unwrap();

    let (length_addr, length_server) = spawn_server(|mut stream| {
        let _request = read_request_head(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nshort")
            .unwrap();
    });
    let length = request_peer(
        length_addr,
        "length.invalid",
        "/__router/health",
        "GET",
        &[],
        Instant::now() + Duration::from_secs(1),
        4096,
    );
    assert!(length
        .unwrap_err()
        .to_string()
        .contains("content-length mismatch"));
    length_server.join().unwrap();
}

fn spawn_server<ResultType, Handler>(handler: Handler) -> (SocketAddr, JoinHandle<ResultType>)
where
    ResultType: Send + 'static,
    Handler: FnOnce(TcpStream) -> ResultType + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let join = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "test server accept timed out");
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("test server accept failed: {error}"),
            }
        };
        // macOS may inherit O_NONBLOCK from the listener onto the accepted
        // socket. The test handler intentionally uses blocking Read/Write with
        // bounded timeouts, so normalize the accepted side before the client
        // has necessarily finished sending its headers.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        handler(stream)
    });
    (address, join)
}

fn read_request_head(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0, "client closed before sending request headers");
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= 8192, "request headers exceeded test bound");
    }
    String::from_utf8(bytes).unwrap()
}

fn write_response(stream: &mut TcpStream, body: &[u8]) {
    let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
    stream.write_all(header.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
}

fn assert_http_failure(
    error: CanonicalFixtureError,
    expected_phase: HttpPhase,
    expected_kind: impl Into<Option<std::io::ErrorKind>>,
) {
    let rendered = error.to_string();
    let CanonicalFixtureError::Http {
        phase,
        kind,
        raw_os_error: _,
        elapsed_ms: _,
        deadline_ms: _,
        ..
    } = error
    else {
        panic!("expected typed HTTP failure, got {rendered}");
    };
    assert_eq!(phase, expected_phase);
    if let Some(expected_kind) = expected_kind.into() {
        assert_eq!(kind, expected_kind);
    } else {
        assert!(
            matches!(
                kind,
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ),
            "stalled socket returned unexpected ErrorKind {kind:?}"
        );
    }
    assert!(rendered.contains("elapsed="), "{rendered}");
    assert!(rendered.contains("deadline="), "{rendered}");
    assert!(rendered.contains("raw_errno="), "{rendered}");
}
