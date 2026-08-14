use std::{
    collections::BTreeSet,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const IO_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestObservation {
    pub ordinal: u64,
    pub method: String,
    pub path: String,
    pub response_head_sent: bool,
    pub chunks_sent: usize,
}

/// Hermetic upstream for VCP-5.  Its blocking threads are the remote-server
/// side of the socket: the runtime under test remains on the single Tokio
/// worker used by the canary tests.
pub struct Phase5TcpServer {
    address: SocketAddr,
    shared: Arc<Shared>,
    accept_thread: Option<thread::JoinHandle<()>>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    stopping: AtomicBool,
    next_ordinal: AtomicU64,
}

#[derive(Default)]
struct State {
    released: BTreeSet<String>,
    observations: Vec<RequestObservation>,
}

impl Phase5TcpServer {
    pub fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind Phase 5 upstream");
        let address = listener
            .local_addr()
            .expect("read Phase 5 upstream address");
        listener
            .set_nonblocking(true)
            .expect("set Phase 5 upstream nonblocking accept");
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            stopping: AtomicBool::new(false),
            next_ordinal: AtomicU64::new(0),
        });
        let server_shared = Arc::clone(&shared);
        let accept_thread = thread::Builder::new()
            .name("phase-5-proof-upstream".to_string())
            .spawn(move || accept_connections(listener, server_shared))
            .expect("spawn Phase 5 upstream");
        Self {
            address,
            shared,
            accept_thread: Some(accept_thread),
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }

    pub fn release(&self, path: &str) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.released.insert(path.to_string());
        self.shared.changed.notify_all();
    }

    pub fn wait_for_path(&self, path: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        loop {
            if state.observations.iter().any(|entry| entry.path == path) {
                return true;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            let (next, result) = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|error| error.into_inner());
            state = next;
            if result.timed_out() {
                return state.observations.iter().any(|entry| entry.path == path);
            }
        }
    }

    /// Waits without parking the Tokio worker that owns the runtime under
    /// test. The Condvar variant above remains useful to the server's own
    /// blocking-thread self-test; Phase 5 runtime tests use this async view.
    pub async fn wait_for_path_async(&self, path: &str, timeout: Duration) -> bool {
        self.wait_for_observation_async(timeout, |entry| entry.path == path)
            .await
    }

    pub async fn wait_for_response_head_async(&self, path: &str, timeout: Duration) -> bool {
        self.wait_for_observation_async(timeout, |entry| {
            entry.path == path && entry.response_head_sent
        })
        .await
    }

    async fn wait_for_observation_async(
        &self,
        timeout: Duration,
        predicate: impl Fn(&RequestObservation) -> bool,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.snapshot().iter().any(&predicate) {
                return true;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            tokio::time::sleep(remaining.min(Duration::from_millis(1))).await;
        }
    }

    pub fn snapshot(&self) -> Vec<RequestObservation> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .observations
            .clone()
    }
}

impl Drop for Phase5TcpServer {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::Release);
        self.shared.changed.notify_all();
        let _ = TcpStream::connect(self.address);
        if let Some(handle) = self.accept_thread.take() {
            handle.join().expect("join Phase 5 upstream");
        }
    }
}

fn accept_connections(listener: TcpListener, shared: Arc<Shared>) {
    let mut connections = Vec::new();
    while !shared.stopping.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                let connection_shared = Arc::clone(&shared);
                connections.push(thread::spawn(move || {
                    serve_connection(stream, &connection_shared)
                }));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("Phase 5 upstream accept failed: {error}"),
        }
    }
    shared.changed.notify_all();
    for connection in connections {
        connection.join().expect("join Phase 5 upstream connection");
    }
}

fn serve_connection(mut stream: TcpStream, shared: &Shared) {
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .expect("set upstream read timeout");
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .expect("set upstream write timeout");
    let (method, path) = read_request(&mut stream);
    let ordinal = shared.next_ordinal.fetch_add(1, Ordering::Relaxed);
    record(
        shared,
        RequestObservation {
            ordinal,
            method,
            path: path.clone(),
            response_head_sent: false,
            chunks_sent: 0,
        },
    );

    if path == "/request" {
        wait_for_release(shared, &path);
        write_fixed(&mut stream, 200, b"UNARY");
        mark_head(shared, ordinal);
        return;
    }

    let chunks: &[&[u8]] = match path.as_str() {
        "/stream/left" => &[b"LEFT-1", b"LEFT-2"],
        "/stream/right" => &[b"RIGHT-1", b"RIGHT-2"],
        "/stream/drop-left" => &[b"DROP-LEFT-1", b"DROP-LEFT-LATE"],
        "/stream/drop-right" => &[b"DROP-RIGHT-1", b"DROP-RIGHT-2"],
        _ => {
            write_fixed(&mut stream, 404, b"UNKNOWN");
            mark_head(shared, ordinal);
            return;
        }
    };
    write_chunked_head(&mut stream, 200);
    mark_head(shared, ordinal);
    wait_for_release(shared, &path);
    for chunk in chunks {
        write_chunk(&mut stream, chunk);
        mark_chunk(shared, ordinal);
    }
    stream.write_all(b"0\r\n\r\n").expect("write chunked end");
    stream.flush().expect("flush chunked end");
}

fn read_request(stream: &mut TcpStream) -> (String, String) {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut buffer).expect("read Phase 5 HTTP request");
        assert!(read > 0, "Phase 5 upstream peer closed before request head");
        bytes.extend_from_slice(&buffer[..read]);
        assert!(
            bytes.len() <= 64 * 1024,
            "Phase 5 request head exceeded 64 KiB"
        );
    }
    let head = String::from_utf8(bytes).expect("Phase 5 HTTP request head is UTF-8");
    let line = head.lines().next().expect("Phase 5 HTTP request line");
    let mut fields = line.split_whitespace();
    let method = fields.next().expect("Phase 5 HTTP method").to_string();
    let path = fields.next().expect("Phase 5 HTTP path").to_string();
    assert_eq!(fields.next(), Some("HTTP/1.1"));
    (method, path)
}

fn wait_for_release(shared: &Shared, path: &str) {
    let mut state = shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    while !state.released.contains(path) && !shared.stopping.load(Ordering::Acquire) {
        state = shared
            .changed
            .wait(state)
            .unwrap_or_else(|error| error.into_inner());
    }
}

fn record(shared: &Shared, observation: RequestObservation) {
    shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .observations
        .push(observation);
    shared.changed.notify_all();
}

fn mark_head(shared: &Shared, ordinal: u64) {
    let mut state = shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    state
        .observations
        .iter_mut()
        .find(|entry| entry.ordinal == ordinal)
        .expect("recorded request ordinal")
        .response_head_sent = true;
    shared.changed.notify_all();
}

fn mark_chunk(shared: &Shared, ordinal: u64) {
    let mut state = shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    state
        .observations
        .iter_mut()
        .find(|entry| entry.ordinal == ordinal)
        .expect("recorded request ordinal")
        .chunks_sent += 1;
    shared.changed.notify_all();
}

fn write_fixed(stream: &mut TcpStream, status: u16, body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 {status} Fixture\r\ncontent-length: {}\r\nconnection: close\r\nx-phase5-upstream: fixed\r\n\r\n",
        body.len()
    )
    .expect("write fixed response head");
    stream.write_all(body).expect("write fixed response body");
    stream.flush().expect("flush fixed response");
}

fn write_chunked_head(stream: &mut TcpStream, status: u16) {
    write!(
        stream,
        "HTTP/1.1 {status} Fixture\r\ntransfer-encoding: chunked\r\nconnection: close\r\nx-phase5-upstream: stream\r\n\r\n"
    )
    .expect("write chunked response head");
    stream.flush().expect("flush chunked response head");
}

fn write_chunk(stream: &mut TcpStream, chunk: &[u8]) {
    write!(stream, "{:x}\r\n", chunk.len()).expect("write chunk length");
    stream.write_all(chunk).expect("write chunk body");
    stream.write_all(b"\r\n").expect("write chunk delimiter");
    stream.flush().expect("flush chunk");
}

#[test]
fn deterministic_tcp_server_gates_unary_and_distinguishes_streams() {
    let server = Phase5TcpServer::start();
    let address = server.address;
    let unary = thread::spawn(move || simple_get(address, "/request"));
    assert!(server.wait_for_path("/request", Duration::from_secs(1)));
    assert!(
        !unary.is_finished(),
        "unary response crossed the closed gate"
    );
    server.release("/request");
    assert!(String::from_utf8(unary.join().expect("join unary client"))
        .expect("unary response UTF-8")
        .ends_with("UNARY"));

    let address = server.address;
    let left = thread::spawn(move || simple_get(address, "/stream/left"));
    let address = server.address;
    let right = thread::spawn(move || simple_get(address, "/stream/right"));
    assert!(server.wait_for_path("/stream/left", Duration::from_secs(1)));
    assert!(server.wait_for_path("/stream/right", Duration::from_secs(1)));
    server.release("/stream/right");
    server.release("/stream/left");
    let left = String::from_utf8(left.join().expect("join left client")).expect("left UTF-8");
    let right = String::from_utf8(right.join().expect("join right client")).expect("right UTF-8");
    assert!(left.contains("LEFT-1") && left.contains("LEFT-2"));
    assert!(right.contains("RIGHT-1") && right.contains("RIGHT-2"));
    assert!(!left.contains("RIGHT") && !right.contains("LEFT"));
}

fn simple_get(address: SocketAddr, path: &str) -> Vec<u8> {
    let mut stream = TcpStream::connect(address).expect("connect Phase 5 upstream");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nhost: {address}\r\nconnection: close\r\n\r\n"
    )
    .expect("write test request");
    stream.flush().expect("flush test request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("read test response");
    response
}
