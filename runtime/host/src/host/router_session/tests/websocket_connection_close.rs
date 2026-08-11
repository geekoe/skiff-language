//! Close notification behavior against the compiled
//! `package-service-websocket-smoke` fixture: a `websocketConnect`
//! request.start is accepted, then a `websocketConnectionClosed`
//! request.start (the router's client-teardown notification) reaches the
//! connect entry's close handler, which records a `ConnectionCloseRecord`
//! through the service db capability. The close frame is one-way: the
//! handler executes to completion without any response frame.

use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use serde_json::{json, Value};
use skiff_artifact_model::WebSocketEntryId;
use skiff_compiler::CompilerPlatformSources;
use skiff_runtime_capability_context::{
    DbCapabilityContext, DbCapabilityContextApi, DbCapabilityError, DbCapabilityFactory,
    DbCapabilityFuture, DbCapabilityLeaseHandle, DbCapabilityLeaseHold, DbCapabilityResult,
    DbCapabilitySource, DbCapabilityStore, DbCapabilityStoreApi, DbDocument, DbKey,
    DbOneSelector, DbOrderEntry, DbPageResult, DbProviderBuildInput, DbProviderFactory,
    DbProviderSource, DbQuery, DbRecoverableRuntimeContext, DbRuntimeChange, DbWriteResult,
    FieldPath, FileCapabilityRecord, ServiceDbChange, ServiceDbFindOptions,
};
use skiff_runtime_model::request_heap::RequestHeap;
use skiff_runtime_model::runtime_value::RuntimeValue;
use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::{
    protocol::{encode_binary_frame, RUNTIME_FRAME_SCHEMA_VERSION},
    runtime_assembly_request::{
        decode_runtime_assembly_websocket_connect_response_end_frame,
        RuntimeAssemblyRequestCallerFrameHeader, RuntimeAssemblyRequestTraceFrameHeader,
        RuntimeAssemblyWebSocketConnectIngressFrameHeader,
        RuntimeAssemblyWebSocketConnectIngressProtocol,
        RuntimeAssemblyWebSocketConnectRequestFrameHeader,
        RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
        RuntimeAssemblyWebSocketConnectRoutingFrameHeader,
        RuntimeAssemblyWebSocketConnectionClosedIngressFrameHeader,
        RuntimeAssemblyWebSocketConnectionClosedRequestFrameHeader,
        RuntimeAssemblyWebSocketConnectionClosedRequestStartFrameHeader,
        RuntimeAssemblyWebSocketConnectionClosedRoutingFrameHeader,
    },
};
use skiff_test_runner::{
    canonical_package::compile_package_project_for_test, canonical_std_seed::seed_canonical_std,
    canonical_store::CanonicalBaseAssembly, test_discovery::discover_test_service_cases,
    test_service_fixture::assemble_test_service_fixture_for_run,
};
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::{host::RuntimeHost, loader::assembly_admission::ActiveAssemblyRoute};

const CONNECTION_ID: &str = "smoke-close-connection-1";

/// Recording in-memory service db: transactions and writes succeed and are
/// captured; every other operation fails closed (the smoke close handler
/// only writes one record).
#[derive(Clone, Default)]
struct RecordingDbProvider {
    writes: Arc<Mutex<Vec<(String, Value)>>>,
}

impl DbProviderFactory for RecordingDbProvider {
    fn build(&self, _input: DbProviderBuildInput) -> DbCapabilityResult<DbCapabilitySource> {
        Ok(DbCapabilitySource::new(Some(RecordingDbFactory {
            writes: Arc::clone(&self.writes),
        })))
    }
}

#[derive(Clone)]
struct RecordingDbFactory {
    writes: Arc<Mutex<Vec<(String, Value)>>>,
}

impl DbCapabilityFactory for RecordingDbFactory {
    fn context_for_request(&self, _owner: String, _request_id: String) -> DbCapabilityContext {
        DbCapabilityContext::new(RecordingDbContext {
            writes: Arc::clone(&self.writes),
        })
    }
}

#[derive(Clone)]
struct RecordingDbContext {
    writes: Arc<Mutex<Vec<(String, Value)>>>,
}

impl DbCapabilityContextApi for RecordingDbContext {
    fn require_store(
        &self,
        _target: &str,
        _unavailable_reason: &str,
    ) -> DbCapabilityResult<DbCapabilityStore> {
        Ok(DbCapabilityStore::new(RecordingDbStore {
            writes: Arc::clone(&self.writes),
        }))
    }
}

#[derive(Clone)]
struct RecordingDbStore {
    writes: Arc<Mutex<Vec<(String, Value)>>>,
}

impl RecordingDbStore {
    fn unexpected<'a, T>(&'a self, operation: &'static str) -> DbCapabilityFuture<'a, T>
    where
        T: Send + 'a,
    {
        Box::pin(async move {
            Err(DbCapabilityError::decode(format!(
                "unexpected DB operation {operation}"
            )))
        })
    }

    fn record(&self, type_name: &str, value: Value) -> DbCapabilityResult<()> {
        self.writes.lock().unwrap().push((type_name.to_string(), value));
        Ok(())
    }
}

impl DbCapabilityStoreApi for RecordingDbStore {
    fn begin_transaction(&self) -> DbCapabilityFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn commit_transaction(&self) -> DbCapabilityFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn abort_transaction(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }

    fn find_one_by_key<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        self.unexpected("find_one_by_key")
    }

    fn find_one_by_key_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _projection: Option<Vec<FieldPath>>,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        self.unexpected("find_one_by_key_runtime")
    }

    fn find_one_by_query<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _order: Vec<DbOrderEntry>,
        _projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        self.unexpected("find_one_by_query")
    }

    fn find_one_by_query_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _order: Vec<DbOrderEntry>,
        _projection: Option<Vec<FieldPath>>,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        self.unexpected("find_one_by_query_runtime")
    }

    fn find_many_page<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _options: ServiceDbFindOptions,
        _projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, DbPageResult> {
        self.unexpected("find_many_page")
    }

    fn find_many_page_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _options: ServiceDbFindOptions,
        _projection: Option<Vec<FieldPath>>,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Vec<RuntimeValue>> {
        self.unexpected("find_many_page_runtime")
    }

    fn create<'a>(
        &'a self,
        type_name: &'a str,
        value: DbDocument,
    ) -> DbCapabilityFuture<'a, DbDocument> {
        let recorded = self.record(type_name, value.clone().into());
        Box::pin(async move {
            recorded?;
            Ok(value)
        })
    }

    fn create_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _value: &'a RuntimeValue,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, RuntimeValue> {
        self.unexpected("create_runtime")
    }

    fn insert_many_result<'a>(
        &'a self,
        _type_name: &'a str,
        _values: Vec<DbDocument>,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        self.unexpected("insert_many_result")
    }

    fn update_one<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
        _change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        self.unexpected("update_one")
    }

    fn update_one_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
        _change: DbRuntimeChange,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        self.unexpected("update_one_runtime")
    }

    fn update_many<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        self.unexpected("update_many")
    }

    fn upsert_by_key<'a>(
        &'a self,
        type_name: &'a str,
        _key: DbKey,
        insert: DbDocument,
        _change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        let recorded = self.record(type_name, insert.clone().into());
        Box::pin(async move {
            recorded?;
            Ok(DbWriteResult::new(json!({ "upserted": 1 })))
        })
    }

    fn replace_one<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
        _value: DbDocument,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        self.unexpected("replace_one")
    }

    fn replace_one_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
        _value: &'a RuntimeValue,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        self.unexpected("replace_one_runtime")
    }

    fn delete_one<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
    ) -> DbCapabilityFuture<'a, bool> {
        self.unexpected("delete_one")
    }

    fn delete_many<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        self.unexpected("delete_many")
    }

    fn count<'a>(&'a self, _type_name: &'a str, _query: DbQuery) -> DbCapabilityFuture<'a, u64> {
        self.unexpected("count")
    }

    fn exists_by_key<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
    ) -> DbCapabilityFuture<'a, bool> {
        self.unexpected("exists_by_key")
    }

    fn exists_by_query<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
    ) -> DbCapabilityFuture<'a, bool> {
        self.unexpected("exists_by_query")
    }

    fn claim_lease<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _slot: &'a str,
    ) -> DbCapabilityFuture<'a, Option<DbCapabilityLeaseHandle>> {
        self.unexpected("claim_lease")
    }

    fn renew_lease<'a>(
        &'a self,
        _hold: &'a DbCapabilityLeaseHold,
    ) -> DbCapabilityFuture<'a, bool> {
        self.unexpected("renew_lease")
    }

    fn release_lease<'a>(&'a self, _hold: &'a DbCapabilityLeaseHold) -> DbCapabilityFuture<'a, ()> {
        self.unexpected("release_lease")
    }

    fn read_lease<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _slot: &'a str,
    ) -> DbCapabilityFuture<'a, Option<Value>> {
        self.unexpected("read_lease")
    }

    fn lease_lost(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async { false })
    }

    fn insert_skiff_file_record<'a>(
        &'a self,
        _record: FileCapabilityRecord,
    ) -> DbCapabilityFuture<'a, ()> {
        self.unexpected("insert_skiff_file_record")
    }

    fn find_skiff_file_by_id<'a>(
        &'a self,
        _id: &'a str,
    ) -> DbCapabilityFuture<'a, Option<FileCapabilityRecord>> {
        self.unexpected("find_skiff_file_by_id")
    }

    fn delete_skiff_file_by_id<'a>(&'a self, _id: &'a str) -> DbCapabilityFuture<'a, u64> {
        self.unexpected("delete_skiff_file_by_id")
    }
}

struct CompiledSmokeFixture {
    assembly: Arc<skiff_artifact_model::RuntimeAssembly>,
    artifact_root: PathBuf,
    _temp: TempFixture,
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/host must live below the Skiff root")
        .to_path_buf()
}

fn compile_smoke_fixture() -> CompiledSmokeFixture {
    let temp = TempFixture::new("host-websocket-close-smoke");
    let source_artifacts = temp.child("source-artifacts");
    let runtime_artifacts = temp.child("runtime-artifacts");
    let fixture_root = repository_root().join("test-runner/fixtures/package-service-websocket-smoke");
    let platform = CompilerPlatformSources::new(&repository_root()).expect("repository platform sources");
    seed_canonical_std(&platform, &source_artifacts).expect("canonical std seed");
    let project = compile_package_project_for_test(&platform, &fixture_root, &source_artifacts)
        .expect("websocket smoke test service production package");
    let cases = discover_test_service_cases(&fixture_root, &fixture_root, false)
        .expect("websocket smoke test discovery");
    assert_eq!(cases.len(), 1);
    let test_fixture = assemble_test_service_fixture_for_run(
        &project,
        &cases,
        CanonicalBaseAssembly::default(),
        "host-websocket-close-smoke",
        "test",
    )
    .expect("websocket smoke test-service assembly");
    test_fixture
        .publish(&source_artifacts, &runtime_artifacts)
        .expect("websocket smoke runtime records");
    let assembly = Arc::new(test_fixture.records.assembly.clone());
    CompiledSmokeFixture {
        assembly,
        artifact_root: runtime_artifacts,
        _temp: temp,
    }
}

fn compile_smoke_fixture_with_stack() -> CompiledSmokeFixture {
    thread::Builder::new()
        .name("host-websocket-close-smoke-fixture".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(compile_smoke_fixture)
        .expect("websocket close smoke fixture compiler thread")
        .join()
        .expect("websocket close smoke fixture compiler thread should not panic")
}

async fn smoke_host() -> (RuntimeHost, ActiveAssemblyRoute, Arc<Mutex<Vec<(String, Value)>>>) {
    let fixture = compile_smoke_fixture_with_stack();
    let resolver = skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver::open(
        &fixture.artifact_root,
    )
    .expect("websocket smoke filesystem resolver");
    let writes = Arc::new(Mutex::new(Vec::new()));
    let host = RuntimeHost::new(crate::host::RuntimeConfig {
        db_provider: DbProviderSource::new(RecordingDbProvider {
            writes: Arc::clone(&writes),
        }),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-websocket-close-smoke".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-websocket-close-smoke-test-home"),
        profile: "skiff-test".to_string(),
        bytecode_only: false,
        http_response_max_bytes: 1024,
        http_egress_proxy: None,
    })
    .expect("websocket close smoke runtime host should build");
    host.assembly_admission
        .admit_with_profile(
            Arc::clone(&fixture.assembly),
            &resolver,
            Some(&skiff_artifact_model::AssemblyActivationServiceDb {
                mongo_url: "mongodb://127.0.0.1:27017".to_string(),
            }),
            Some("skiff-test"),
        )
        .await
        .expect("websocket smoke assembly should admit");
    let key = fixture
        .assembly
        .gateway_ingress
        .iter()
        .find(|binding| binding.selector.path == "/socket" && binding.selector.method.is_none())
        .expect("websocket smoke physical selector")
        .service_ingress_key();
    let physical = host
        .lookup_active_assembly_request_route(&key)
        .expect("websocket smoke physical route");
    (host, physical, writes)
}

fn websocket_entry_id(route: &ActiveAssemblyRoute) -> WebSocketEntryId {
    skiff_artifact_identity::websocket_entry_id(
        &route.entry().owner().service_id,
        route.gateway_entry_key(),
    )
    .unwrap()
}

fn connect_header(
    physical: &ActiveAssemblyRoute,
    request_id: &str,
) -> RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
    RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.to_string(),
        mode: "unary".to_string(),
        caller: RuntimeAssemblyRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: RuntimeAssemblyWebSocketConnectRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: None,
            assembly_generation: None,
            deployment: physical.deployment().clone(),
            build_id: Some(
                physical
                    .deployment()
                    .deployment_artifact_identity
                    .as_str()
                    .to_string(),
            ),
            gateway_entry_identity: physical.gateway_entry_identity().clone(),
            ingress: RuntimeAssemblyWebSocketConnectIngressFrameHeader {
                protocol: RuntimeAssemblyWebSocketConnectIngressProtocol::WebSocket,
                method: (),
                path: physical.selector().path.clone(),
            },
        },
        client_session: None,
        deadline: None,
        trace: RuntimeAssemblyRequestTraceFrameHeader {
            trace_id: format!("trace-{request_id}"),
            span_id: format!("span-{request_id}"),
            parent_span_id: None,
            sampled: None,
        },
        websocket_connect: RuntimeAssemblyWebSocketConnectRequestFrameHeader {
            connection_id: CONNECTION_ID.to_string(),
            url: "ws://127.0.0.1:4000/socket".to_string(),
            query: Vec::new(),
            headers: Vec::new(),
            cookies: Vec::new(),
            version: None,
            websocket_entry_id: websocket_entry_id(physical),
            gateway_entry_identity: physical.gateway_entry_identity().clone(),
        },
        test_effects_enabled: false,
    }
}

fn connection_closed_header(
    physical: &ActiveAssemblyRoute,
    request_id: &str,
) -> RuntimeAssemblyWebSocketConnectionClosedRequestStartFrameHeader {
    RuntimeAssemblyWebSocketConnectionClosedRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.to_string(),
        mode: "unary".to_string(),
        caller: RuntimeAssemblyRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: RuntimeAssemblyWebSocketConnectionClosedRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: None,
            assembly_generation: None,
            deployment: physical.deployment().clone(),
            build_id: Some(
                physical
                    .deployment()
                    .deployment_artifact_identity
                    .as_str()
                    .to_string(),
            ),
            gateway_entry_identity: physical.gateway_entry_identity().clone(),
            ingress: RuntimeAssemblyWebSocketConnectionClosedIngressFrameHeader {
                protocol: RuntimeAssemblyWebSocketConnectIngressProtocol::WebSocket,
                path: physical.selector().path.clone(),
                entry_kind: "connectionClosed".to_string(),
            },
        },
        client_session: None,
        deadline: None,
        trace: RuntimeAssemblyRequestTraceFrameHeader {
            trace_id: format!("trace-{request_id}"),
            span_id: format!("span-{request_id}"),
            parent_span_id: None,
            sampled: None,
        },
        websocket_connection_closed: RuntimeAssemblyWebSocketConnectionClosedRequestFrameHeader {
            connection_id: CONNECTION_ID.to_string(),
            websocket_entry_id: websocket_entry_id(physical),
            gateway_entry_identity: physical.gateway_entry_identity().clone(),
            business_identity: None,
            close_code: None,
            close_reason: None,
        },
        test_effects_enabled: false,
    }
}

async fn dispatch(
    host: &RuntimeHost,
    frame: &[u8],
    sender: &mpsc::UnboundedSender<RouterWriterMessage>,
) -> crate::error::Result<()> {
    let mut control = None;
    let mut fingerprint = None;
    super::dispatch_router_binary_frame(host, frame, sender, &mut control, &mut fingerprint).await
}

#[tokio::test]
async fn websocket_close_smoke_handler_records_connection_closed_state() {
    let (host, physical, writes) = smoke_host().await;

    // Client connects: the connect handler accepts with the connectionId as
    // business identity.
    let connect = encode_binary_frame(
        &connect_header(&physical, "ws-close-smoke-connect"),
        &[],
    )
    .unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    dispatch(&host, &connect, &sender).await.unwrap();
    let RouterWriterMessage::Binary(frame) = timeout(Duration::from_secs(10), receiver.recv())
        .await
        .expect("websocketConnect response timeout")
        .expect("websocketConnect response channel")
    else {
        panic!("websocketConnect response must use binary wire")
    };
    let response = decode_runtime_assembly_websocket_connect_response_end_frame(&frame)
        .expect("typed websocketConnect response.end");
    assert!(matches!(
        response.websocket_connect,
        skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyWebSocketConnectResponseFrameHeader::Accept { .. }
    ));
    assert_eq!(host.request_supervisor.active_count().await, 0);

    // Client tears down: the router's one-way connectionClosed frame reaches
    // the close handler, which writes the ConnectionCloseRecord.
    let closed = encode_binary_frame(
        &connection_closed_header(&physical, "ws-close-smoke-close"),
        &[],
    )
    .unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    dispatch(&host, &closed, &sender).await.unwrap();

    // The close handler is a notification: it completes with no response
    // frame at all.
    timeout(Duration::from_millis(300), receiver.recv())
        .await
        .expect_err("connectionClosed must not produce a response frame");
    assert_eq!(host.request_supervisor.active_count().await, 0);

    // The close handler observed the connectionId scalar and recorded the
    // teardown into persistent service state. The store receives the
    // db-object target key as type name, so match on the recorded document.
    let writes = writes.lock().unwrap().clone();
    let record = writes
        .iter()
        .find(|(_, document)| {
            document.get("connectionId").and_then(Value::as_str) == Some(CONNECTION_ID)
        })
        .unwrap_or_else(|| {
            panic!("close handler must write a connection close record; wrote: {writes:?}")
        });
    assert_eq!(
        record.1,
        json!({
            "connectionId": CONNECTION_ID,
            "closedAt": "closed",
        })
    );
    assert_eq!(
        host.connection_requests.pending_count(),
        0,
        "close notification must not leak connection bookkeeping"
    );
}

#[tokio::test]
async fn websocket_close_without_matching_route_fails_closed() {
    // A close frame routed to a foreign gateway identity is refused before
    // the close handler runs: the wire admission writes an ordinary
    // response.error and no state is recorded.
    let (host, physical, writes) = smoke_host().await;
    let mut header = connection_closed_header(&physical, "ws-close-smoke-foreign");
    header.routing.gateway_entry_identity = skiff_artifact_model::GatewayEntryIdentity::parse(
        format!("skiff-gateway-entry-v2:sha256:{}", "f".repeat(64)),
    )
    .unwrap();
    header.websocket_connection_closed.gateway_entry_identity =
        header.routing.gateway_entry_identity.clone();
    let frame = encode_binary_frame(&header, &[]).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    dispatch(&host, &frame, &sender).await.unwrap();
    let RouterWriterMessage::Binary(frame) = timeout(Duration::from_secs(2), receiver.recv())
        .await
        .expect("close admission rejection timeout")
        .expect("close admission rejection channel")
    else {
        panic!("close admission rejection must use binary wire")
    };
    let (typed, _): (skiff_runtime_transport::protocol::TypedEnvelope, Vec<u8>) =
        skiff_runtime_transport::protocol::decode_typed_binary_frame(&frame)
            .expect("close admission rejection frame");
    assert_eq!(typed.envelope_type, "response.error");
    assert!(writes.lock().unwrap().is_empty());
    assert_eq!(host.request_supervisor.active_count().await, 0);
}

struct TempFixture {
    root: PathBuf,
}

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

impl TempFixture {
    fn new(label: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "skiff-runtime-host-{label}-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("websocket close smoke temp root");
        Self { root }
    }

    fn child(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
