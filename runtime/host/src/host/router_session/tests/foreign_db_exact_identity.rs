use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use skiff_artifact_model::{AssemblyActivationServiceDb, ServiceIngressKey};
use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_capability_context::{
    DbCapabilityContext, DbCapabilityContextApi, DbCapabilityError, DbCapabilityFactory,
    DbCapabilityFuture, DbCapabilityLeaseHandle, DbCapabilityLeaseHold, DbCapabilityResult,
    DbCapabilitySource, DbCapabilityStore, DbCapabilityStoreApi, DbDocument, DbKey, DbOneSelector,
    DbOrderEntry, DbPageResult, DbProviderBuildInput, DbProviderFactory, DbProviderSource, DbQuery,
    DbRecoverableRuntimeContext, DbRuntimeChange, DbWriteResult, FieldPath, FileCapabilityRecord,
    ServiceDbChange, ServiceDbFindOptions,
};
use skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver;
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};
use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::{
    protocol::{
        decode_typed_binary_frame, encode_binary_frame, ResponseEndFrameHeader,
        ResponseErrorFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
    },
    runtime_assembly_request::{
        RuntimeAssemblyHttpRequestFrameHeader, RuntimeAssemblyRequestCallerFrameHeader,
        RuntimeAssemblyRequestIngressFrameHeader, RuntimeAssemblyRequestIngressProtocol,
        RuntimeAssemblyRequestRoutingFrameHeader, RuntimeAssemblyRequestStartFrameHeader,
        RuntimeAssemblyRequestTraceFrameHeader,
    },
};
use skiff_test_runner::{
    canonical_package::compile_package_project_for_test, canonical_std_seed::seed_canonical_std,
    canonical_store::CanonicalBaseAssembly, test_discovery::discover_test_service_cases,
    test_service_fixture::assemble_test_service_fixture_for_run,
};
use tokio::sync::mpsc;

use crate::host::{RuntimeConfig, RuntimeHost};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
struct DbEvent {
    operation: &'static str,
    collection: String,
    type_name: String,
}

#[derive(Clone, Default)]
struct ExactDbProvider {
    inputs: Arc<Mutex<Vec<DbProviderBuildInput>>>,
    events: Arc<Mutex<Vec<DbEvent>>>,
    lookups: Arc<Mutex<Vec<String>>>,
}

impl DbProviderFactory for ExactDbProvider {
    fn build(&self, input: DbProviderBuildInput) -> DbCapabilityResult<DbCapabilitySource> {
        let targets = input
            .runtime_program_db
            .iter()
            .map(|entry| {
                (
                    entry.target.lookup_key().to_string(),
                    (
                        entry.metadata.collection_name.clone(),
                        entry.metadata.type_name.clone(),
                    ),
                )
            })
            .collect();
        self.inputs.lock().unwrap().push(input);
        Ok(DbCapabilitySource::new(Some(ExactDbFactory {
            store: ExactDbStore {
                targets,
                events: Arc::clone(&self.events),
            },
            lookups: Arc::clone(&self.lookups),
        })))
    }
}

#[derive(Clone)]
struct ExactDbFactory {
    store: ExactDbStore,
    lookups: Arc<Mutex<Vec<String>>>,
}

impl DbCapabilityFactory for ExactDbFactory {
    fn context_for_request(&self, _owner: String, _request_id: String) -> DbCapabilityContext {
        DbCapabilityContext::new(ExactDbContext {
            store: self.store.clone(),
            lookups: Arc::clone(&self.lookups),
        })
    }
}

#[derive(Clone)]
struct ExactDbContext {
    store: ExactDbStore,
    lookups: Arc<Mutex<Vec<String>>>,
}

impl DbCapabilityContextApi for ExactDbContext {
    fn require_store(
        &self,
        target: &str,
        _unavailable_reason: &str,
    ) -> DbCapabilityResult<DbCapabilityStore> {
        self.lookups.lock().unwrap().push(target.to_string());
        Ok(DbCapabilityStore::new(self.store.clone()))
    }
}

#[derive(Clone)]
struct ExactDbStore {
    targets: BTreeMap<String, (String, String)>,
    events: Arc<Mutex<Vec<DbEvent>>>,
}

impl ExactDbStore {
    fn record(&self, operation: &'static str, target_key: &str) -> DbCapabilityResult<()> {
        let (collection, type_name) = self.targets.get(target_key).ok_or_else(|| {
            DbCapabilityError::decode(format!("unknown exact DB target key {target_key}"))
        })?;
        self.events.lock().unwrap().push(DbEvent {
            operation,
            collection: collection.clone(),
            type_name: type_name.clone(),
        });
        Ok(())
    }

    fn unexpected<'a, T>(&'a self, operation: &'static str) -> DbCapabilityFuture<'a, T>
    where
        T: Send + 'a,
    {
        self.events.lock().unwrap().push(DbEvent {
            operation,
            collection: "<unexpected>".to_string(),
            type_name: "<unexpected>".to_string(),
        });
        Box::pin(async move {
            Err(DbCapabilityError::decode(format!(
                "unexpected DB operation {operation}"
            )))
        })
    }
}

impl DbCapabilityStoreApi for ExactDbStore {
    fn begin_transaction(&self) -> DbCapabilityFuture<'_, ()> {
        self.unexpected("begin_transaction")
    }

    fn commit_transaction(&self) -> DbCapabilityFuture<'_, ()> {
        self.unexpected("commit_transaction")
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
        _type_name: &'a str,
        _value: DbDocument,
    ) -> DbCapabilityFuture<'a, DbDocument> {
        self.unexpected("create")
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
        _type_name: &'a str,
        _key: DbKey,
        _insert: DbDocument,
        _change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        self.unexpected("upsert_by_key")
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
        target_key: &'a str,
        _selector: DbOneSelector,
    ) -> DbCapabilityFuture<'a, bool> {
        let recorded = self.record("write.delete", target_key);
        Box::pin(async move {
            recorded?;
            Ok(true)
        })
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
        target_key: &'a str,
        _key: DbKey,
    ) -> DbCapabilityFuture<'a, bool> {
        let recorded = self.record("read.exists", target_key);
        Box::pin(async move {
            recorded?;
            Ok(true)
        })
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

    fn renew_lease<'a>(&'a self, _hold: &'a DbCapabilityLeaseHold) -> DbCapabilityFuture<'a, bool> {
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
    ) -> DbCapabilityFuture<'a, Option<serde_json::Value>> {
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

#[test]
fn shared_test_assembly_keeps_case_routes_doubles_and_service_databases_isolated() {
    std::thread::Builder::new()
        .name("shared-test-assembly-isolation".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("shared test assembly runtime")
                .block_on(shared_test_assembly_isolation())
        })
        .expect("shared test assembly thread")
        .join()
        .expect("shared test assembly thread should not panic");
}

async fn shared_test_assembly_isolation() {
    let fixture = TempFixture::new("foreign-db-exact-identity");
    let source_artifacts = fixture.child("source-artifacts");
    let runtime_artifacts = fixture.child("runtime-artifacts");
    let first = fixture.child("first");
    let second = fixture.child("second");
    let tests = fixture.child("tests");
    let platform = repository_platform_sources();
    CanonicalArtifactStore::create(&source_artifacts).unwrap();
    seed_canonical_std(&platform, &source_artifacts).unwrap();
    write_provider(&first, "example.com/first-sessions", "first-db");
    write_provider(&second, "example.com/second-sessions", "second-db");
    publish_package(&platform, &first, &source_artifacts);
    publish_package(&platform, &second, &source_artifacts);
    write_test_service(&tests);

    let project = compile_package_project_for_test(&platform, &tests, &source_artifacts).unwrap();
    let cases = discover_test_service_cases(&tests, &tests, false).unwrap();
    assert_eq!(cases.len(), 2);
    let test_fixture = assemble_test_service_fixture_for_run(
        &project,
        &cases,
        CanonicalBaseAssembly::default(),
        "p3x-foreign-db",
    )
    .unwrap();
    test_fixture
        .publish(&source_artifacts, &runtime_artifacts)
        .unwrap();

    let store = CanonicalArtifactStore::open(&runtime_artifacts).unwrap();
    let roots = test_fixture
        .cases
        .iter()
        .map(|case| case.entrypoint.deployment.clone())
        .collect::<Vec<_>>();
    let shared_assembly = test_fixture.records.assembly.clone();
    let assembly_ref = skiff_artifact_identity::runtime_assembly_ref(&shared_assembly).unwrap();
    store.write_runtime_assembly(&shared_assembly).unwrap();
    store.read_runtime_assembly(&assembly_ref).unwrap();
    assert_eq!(shared_assembly.roots, roots);
    assert_eq!(shared_assembly.gateway_ingress.len(), 2);
    let provider = ExactDbProvider::default();
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: DbProviderSource::new(provider.clone()),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-p3x-foreign-db".to_string(),
        runtime_home: fixture.child("runtime-home"),
        environment: "skiff-test".to_string(),
        http_response_max_bytes: 1024,
        http_egress_proxy: None,
    })
    .unwrap();
    let resolver = FilesystemRuntimeAssemblyContentResolver::open(&runtime_artifacts).unwrap();
    let (config_snapshot, config_resolver) = crate::loader::config_snapshot::snapshot_for_assembly(
        "p3x-foreign-db",
        &shared_assembly,
        &resolver,
    );
    host.assembly_admission
        .recover_committed(
            "p3x-foreign-db",
            1,
            &assembly_ref,
            &config_snapshot,
            &resolver,
            &config_resolver,
            Some(&AssemblyActivationServiceDb {
                mongo_url: "mongodb://p3x.invalid".to_string(),
            }),
        )
        .await
        .unwrap();

    let routes = test_fixture
        .cases
        .iter()
        .map(|case| {
            host.lookup_active_assembly_request_route(&ServiceIngressKey {
                deployment: case.entrypoint.deployment.clone(),
                selector: case.entrypoint.selector.clone(),
            })
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(routes.len(), 2);
    assert_eq!(routes[0].assembly_identity(), routes[1].assembly_identity());
    assert_eq!(routes[0].generation(), routes[1].generation());
    assert_ne!(routes[0].deployment(), routes[1].deployment());

    for (index, route) in routes.iter().enumerate() {
        let header = test_case_header(
            route,
            &format!("p3x-shared-case-{index}"),
            &format!("test-case-capability-p3x-{index}"),
        );
        dispatch_test_case(&host, &provider, &header).await;
    }

    let inputs = provider.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 2);
    assert_eq!(
        inputs
            .iter()
            .map(|input| input.service_id.as_str())
            .collect::<BTreeSet<_>>(),
        roots
            .iter()
            .map(|deployment| deployment.service_id.as_str())
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(
        inputs
            .iter()
            .map(|input| input.environment.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["p3x-foreign-db"])
    );
    for input in inputs.iter() {
        let mut foreign = input
            .runtime_program_db
            .iter()
            .filter(|entry| entry.metadata.type_name == "Session")
            .collect::<Vec<_>>();
        foreign.sort_by(|left, right| {
            left.metadata
                .collection_name
                .cmp(&right.metadata.collection_name)
        });
        assert_eq!(foreign.len(), 2);
        assert_eq!(
            foreign
                .iter()
                .map(|entry| entry.metadata.collection_name.as_str())
                .collect::<Vec<_>>(),
            ["first_sessions", "second_sessions"]
        );
        assert_ne!(
            foreign[0].target.target_id.package_artifact_ref,
            foreign[1].target.target_id.package_artifact_ref
        );
        assert_ne!(
            foreign[0].target.target_id.file_ir_ref,
            foreign[1].target.target_id.file_ir_ref
        );
        assert_eq!(foreign[0].target.target_id.type_index, 0);
        assert_eq!(foreign[1].target.target_id.type_index, 0);
        assert_ne!(
            foreign[0].target.lookup_key(),
            foreign[1].target.lookup_key()
        );
    }

    let mut events = provider.events.lock().unwrap().clone();
    events.sort_by(|left, right| {
        (&left.collection, left.operation).cmp(&(&right.collection, right.operation))
    });
    assert_eq!(
        events,
        vec![
            DbEvent {
                operation: "read.exists",
                collection: "first_sessions".to_string(),
                type_name: "Session".to_string(),
            },
            DbEvent {
                operation: "read.exists",
                collection: "first_sessions".to_string(),
                type_name: "Session".to_string(),
            },
            DbEvent {
                operation: "write.delete",
                collection: "first_sessions".to_string(),
                type_name: "Session".to_string(),
            },
            DbEvent {
                operation: "write.delete",
                collection: "first_sessions".to_string(),
                type_name: "Session".to_string(),
            },
            DbEvent {
                operation: "read.exists",
                collection: "second_sessions".to_string(),
                type_name: "Session".to_string(),
            },
            DbEvent {
                operation: "read.exists",
                collection: "second_sessions".to_string(),
                type_name: "Session".to_string(),
            },
            DbEvent {
                operation: "write.delete",
                collection: "second_sessions".to_string(),
                type_name: "Session".to_string(),
            },
            DbEvent {
                operation: "write.delete",
                collection: "second_sessions".to_string(),
                type_name: "Session".to_string(),
            },
        ]
    );

    let mut crossed = test_case_header(
        &routes[1],
        "p3x-crossed-entrypoint",
        "test-case-capability-p3x-crossed",
    );
    crossed.routing.deployment = routes[0].deployment().clone();
    let frame = encode_binary_frame(&crossed, b"null").unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel::<RouterWriterMessage>();
    super::dispatch_router_binary_frame_with_http_response_max(&host, &frame, &sender, 1024)
        .await
        .unwrap();
    let RouterWriterMessage::Binary(frame) = receiver.recv().await.unwrap() else {
        panic!("crossed deployment and gateway entry must return a binary error")
    };
    let (error, _): (ResponseErrorFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&frame).unwrap();
    assert_eq!(error.request_id(), crossed.request_id);
    assert!(receiver.try_recv().is_err());
}

fn test_case_header(
    route: &crate::loader::assembly_admission::ActiveAssemblyRoute,
    request_id: &str,
    capability: &str,
) -> RuntimeAssemblyRequestStartFrameHeader {
    let selector = route.selector();
    let method = selector.method.clone().unwrap();
    RuntimeAssemblyRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.to_string(),
        mode: "unary".to_string(),
        caller: RuntimeAssemblyRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: RuntimeAssemblyRequestRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: route.assembly_identity().clone(),
            assembly_generation: route.generation(),
            deployment: route.deployment().clone(),
            gateway_entry_identity: route.gateway_entry_identity().clone(),
            ingress: RuntimeAssemblyRequestIngressFrameHeader {
                protocol: RuntimeAssemblyRequestIngressProtocol::Http,
                method: method.clone(),
                path: selector.path.clone(),
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
        http_request: RuntimeAssemblyHttpRequestFrameHeader {
            method,
            url: format!("http://localhost{}", selector.path),
            path: selector.path.clone(),
            query: Vec::new(),
            headers: Vec::new(),
        },
        test_effects_enabled: true,
        test_case_capability: Some(capability.to_string()),
    }
}

async fn dispatch_test_case(
    host: &RuntimeHost,
    provider: &ExactDbProvider,
    header: &RuntimeAssemblyRequestStartFrameHeader,
) {
    let frame = encode_binary_frame(header, b"null").unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel::<RouterWriterMessage>();
    super::dispatch_router_binary_frame_with_http_response_max(host, &frame, &sender, 1024)
        .await
        .unwrap();
    let terminal = receiver.recv().await.expect("one test-service terminal");
    match terminal {
        RouterWriterMessage::Binary(frame) => {
            if let Ok((end, payload)) = decode_typed_binary_frame::<ResponseEndFrameHeader>(&frame)
            {
                assert_eq!(end.request_id, header.request_id);
                assert_eq!(payload, b"null");
            } else {
                let (error, payload) =
                    decode_typed_binary_frame::<ResponseErrorFrameHeader>(&frame).unwrap();
                panic!(
                    "compiled foreign DB test failed: {error:?} payload={} inputs={:#?} lookups={:#?} events={:#?}",
                    String::from_utf8_lossy(&payload),
                    provider.inputs.lock().unwrap(),
                    provider.lookups.lock().unwrap(),
                    provider.events.lock().unwrap(),
                );
            }
        }
        other => panic!("unexpected test-service terminal {other:?}"),
    }
    assert!(receiver.try_recv().is_err());
}

fn write_provider(root: &Path, package_id: &str, _state_key: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("package.yml"),
        format!("id: {package_id}\nversion: 1.0.0\n"),
    )
    .unwrap();
    fs::write(root.join("api.yml"), "{}\n").unwrap();
    fs::write(
        root.join("model.skiff"),
        r#"type Session {
  id: string,
  value: string
}

db object Session {
  name "sessions"
  primary key(id)
}
"#,
    )
    .unwrap();
}

fn write_test_service(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("package.yml"),
        r#"id: example.com/foreign-db-tests
version: 1.0.0
packages:
  - id: example.com/first-sessions
    version: 1.0.0
    alias: first
    topLevelAlias: firstImpl
    collection_name_mapping:
      sessions: first_sessions
  - id: example.com/second-sessions
    version: 1.0.0
    alias: second
    topLevelAlias: secondImpl
    collection_name_mapping:
      sessions: second_sessions
"#,
    )
    .unwrap();
    fs::write(root.join("api.yml"), "{}\n").unwrap();
    fs::write(
        root.join("service.yml"),
        "id: example.com/foreign-db-tests\nkind: test\n",
    )
    .unwrap();
    fs::write(root.join("config.skiff-test.yml"), "{}\n").unwrap();
    fs::write(
        root.join("main.test.skiff"),
        r#"import std
import firstImpl
import secondImpl

function sharedDoubleRequest() -> std.http.HttpClientRequest {
  return std.http.HttpClientRequest {
    method: "GET",
    url: "https://case-isolation.test/shared",
    headers: Array.empty<std.http.HttpHeader>(),
    body: null,
    timeoutMs: null,
  }
}

test "case one keeps foreign DB identity and inline double local" effects {
  std/http.request {
    expect: {
      method: "GET",
      url: "https://case-isolation.test/shared",
    },
    respond: std.http.HttpClientResponse {
      status: 200,
      headers: Array.empty<std.http.HttpHeader>(),
      body: bytes.fromUtf8("case-one"),
    },
  }
} {
  const firstExists = db exists firstImpl/model.Session("first")
  const firstDeleted = db delete firstImpl/model.Session("first")
  const secondExists = db exists secondImpl/model.Session("second")
  const secondDeleted = db delete secondImpl/model.Session("second")
  const doubled = std.http.request(sharedDoubleRequest())
  assert firstExists
  assert firstDeleted
  assert secondExists
  assert secondDeleted
  assert doubled.body.toUtf8String() == "case-one"
}

test "case two gets a fresh heap and inline double sequence" effects {
  std/http.request {
    expect: {
      method: "GET",
      url: "https://case-isolation.test/shared",
    },
    respond: std.http.HttpClientResponse {
      status: 200,
      headers: Array.empty<std.http.HttpHeader>(),
      body: bytes.fromUtf8("case-two"),
    },
  }
} {
  const firstExists = db exists firstImpl/model.Session("first")
  const firstDeleted = db delete firstImpl/model.Session("first")
  const secondExists = db exists secondImpl/model.Session("second")
  const secondDeleted = db delete secondImpl/model.Session("second")
  const doubled = std.http.request(sharedDoubleRequest())
  assert firstExists
  assert firstDeleted
  assert secondExists
  assert secondDeleted
  assert doubled.body.toUtf8String() == "case-two"
}
"#,
    )
    .unwrap();
}

fn publish_package(platform: &CompilerPlatformSources, root: &Path, artifact_root: &Path) {
    build_authoring_object(
        platform,
        AuthoringObject::Package,
        root,
        artifact_root,
        "dev",
        true,
    )
    .unwrap();
}

fn repository_platform_sources() -> CompilerPlatformSources {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/host must live below the Skiff root")
        .to_path_buf();
    CompilerPlatformSources::new(&root).unwrap()
}

struct TempFixture {
    root: PathBuf,
}

impl TempFixture {
    fn new(name: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "skiff-runtime-host-{name}-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn child(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
