use std::{
    collections::HashMap,
    num::NonZeroUsize,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
};

use skiff_artifact_model::{GatewayEntryIdentity, IngressProtocol, IngressSelector};
use skiff_compiler::authoring::{
    build_authoring_object, seed_official_std_package, AuthoringObject,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_linker::{
    link_deployment_execution_image, DeploymentExecutionEntry, DeploymentExecutionImage, LinkLimits,
};
use skiff_runtime_loader::{DeploymentBytecodeLoader, FilesystemDeploymentBytecodeContentResolver};
use skiff_runtime_model::{
    bytecode_execution_observation::{BytecodeExecutionCorrelation, BytecodeExecutionObserver},
    vm_heap::{VmHeap, VmHeapError, VmHeapOperation, VmRecordField},
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{CompactTypeTag, ValueFlags, ValueSlot},
};

use super::*;

const FIXTURE_PACKAGE: &str = "test.skiff/v5-server-stream-shapes";
const FIXTURE_PATH: &str = "/phase-5/server-stream-shapes";
const RELEASE_FAILURE: &str = "injected absent-supervisor stream item release failure";

static NEXT_FIXTURE_TEMP: AtomicU64 = AtomicU64::new(0);

struct AbsentSupervisorFixture {
    image: Arc<DeploymentExecutionImage>,
    selector: IngressSelector,
    gateway_identity: GatewayEntryIdentity,
}

impl AbsentSupervisorFixture {
    fn build() -> Self {
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("request crate has a repository root")
            .to_path_buf();
        let fixture_root = repository_root
            .join("runtime/linker/src/bytecode/tests/fixtures/v5-server-stream-shapes");
        let artifact_root = std::env::temp_dir().join(format!(
            "skiff-request-p5-absent-stream-supervisor-{}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE_TEMP.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("test clock follows the Unix epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&artifact_root).expect("create fixture artifact root");
        let platform_sources = skiff_compiler::CompilerPlatformSources::new(&repository_root)
            .expect("open repository platform sources");
        seed_official_std_package(&platform_sources, &artifact_root)
            .expect("seed canonical std into the fixture store");
        let receipt = build_authoring_object(
            &platform_sources,
            AuthoringObject::Package,
            &fixture_root,
            &artifact_root,
            "skiff-test",
            true,
        )
        .unwrap_or_else(|error| {
            panic!("production authoring accepts absent-supervisor fixture: {error}")
        });
        let deployment_reference =
            serde_json::from_value::<skiff_artifact_model::ServiceDeploymentRef>(
                receipt
                    .pointer("/serviceDeploymentReceipt/deployment")
                    .cloned()
                    .expect("authoring receipt carries deployment"),
            )
            .expect("authoring deployment receipt remains typed");
        let store = CanonicalArtifactStore::open(&artifact_root).expect("open fixture store");
        let deployment = store
            .read_service_deployment(&deployment_reference)
            .expect("read fixture deployment");
        let ingress = deployment
            .ingress
            .iter()
            .find(|binding| {
                binding.selector.protocol == IngressProtocol::Http
                    && binding.selector.method.as_deref() == Some("POST")
                    && binding.selector.path == FIXTURE_PATH
            })
            .expect("fixture publishes its exact server-stream ingress");
        let selector = ingress.selector.clone();
        let gateway_identity = deployment
            .gateway_entries
            .get(&ingress.gateway_entry_key)
            .expect("fixture ingress pins a gateway entry")
            .gateway_entry_identity
            .clone();
        let resolver = FilesystemDeploymentBytecodeContentResolver::open(&artifact_root)
            .expect("open fixture bytecode resolver");
        let hydrated = DeploymentBytecodeLoader::new(&resolver)
            .load(&deployment_reference)
            .expect("load fixture bytecode closure");
        let image = Arc::new(
            link_deployment_execution_image(hydrated, &generous_link_limits())
                .expect("link fixture execution image"),
        );
        std::fs::remove_dir_all(&artifact_root).expect("remove fixture artifact root");
        Self {
            image,
            selector,
            gateway_identity,
        }
    }

    fn target(&self) -> DeploymentExecutionEntry {
        self.image
            .http_gateway_entry(&self.selector, &self.gateway_identity)
            .expect("fixture gateway remains linked")
    }
}

fn absent_supervisor_fixture() -> &'static AbsentSupervisorFixture {
    static FIXTURE: OnceLock<AbsentSupervisorFixture> = OnceLock::new();
    FIXTURE.get_or_init(AbsentSupervisorFixture::build)
}

fn generous_link_limits() -> LinkLimits {
    LinkLimits {
        max_packages: u64::MAX,
        max_root_specializations: u64::MAX,
        max_specializations: u64::MAX,
        max_code_words_per_function: u64::MAX,
        max_total_code_words: u64::MAX,
        max_relocations_per_function: u64::MAX,
        max_total_relocations: u64::MAX,
        max_image_table_entries: u64::MAX,
        max_total_image_table_entries: u64::MAX,
        max_total_function_table_entries: u64::MAX,
        max_type_nesting_depth: u64::MAX,
        max_expanded_type_nodes: u64::MAX,
        max_expanded_type_bytes: u64::MAX,
        max_constant_graph_nodes: u64::MAX,
        max_constant_graph_edges: u64::MAX,
    }
}

fn first_stream_item_type(target: &DeploymentExecutionEntry) -> u32 {
    let function = target
        .image()
        .functions()
        .get(target.function().get() as usize)
        .filter(|row| row.index() == target.function())
        .expect("server-stream entry function remains exact");
    function
        .instructions()
        .iter()
        .enumerate()
        .find(|(_, instruction)| instruction.opcode() == Opcode::EmitStream)
        .and_then(|(position, _)| function.stack_map().entries().get(position))
        .and_then(|entry| entry.stack_before().last())
        .map(|value| value.ty().get())
        .expect("linked EmitStream has one exact item carrier")
}

struct ReleaseTrace {
    item_type: u32,
    fail_next: AtomicBool,
    attempts: AtomicUsize,
    successes: AtomicUsize,
}

impl ReleaseTrace {
    fn new(item_type: u32) -> Arc<Self> {
        Arc::new(Self {
            item_type,
            fail_next: AtomicBool::new(true),
            attempts: AtomicUsize::new(0),
            successes: AtomicUsize::new(0),
        })
    }
}

struct FailOnceStreamItemHeap {
    inner: RequestVmHeap,
    trace: Arc<ReleaseTrace>,
}

impl FailOnceStreamItemHeap {
    fn new(trace: Arc<ReleaseTrace>) -> Self {
        Self {
            inner: RequestVmHeap::new(RequestHeapLimits::default()),
            trace,
        }
    }
}

impl VmHeap for FailOnceStreamItemHeap {
    fn validate_live(&self, value: &ValueSlot) -> Result<(), VmHeapError> {
        self.inner.validate_live(value)
    }

    fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.inner.snapshot_share(source)
    }

    fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.inner.transfer_owner(source)
    }

    fn release_snapshot(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
        let is_item = owner
            .compact_type_tag()
            .is_some_and(|tag| tag.type_index() == self.trace.item_type);
        if is_item {
            self.trace.attempts.fetch_add(1, Ordering::AcqRel);
            if self.trace.fail_next.swap(false, Ordering::AcqRel) {
                return Err(VmHeapError::HeapOperationFailed {
                    operation: VmHeapOperation::ReleaseSnapshot,
                    message: RELEASE_FAILURE.to_string(),
                });
            }
        }
        self.inner.release_snapshot(owner)?;
        if is_item {
            self.trace.successes.fetch_add(1, Ordering::AcqRel);
        }
        Ok(())
    }

    fn release_resource(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
        self.inner.release_resource(owner)
    }

    fn allocate_array(
        &mut self,
        elements: &[ValueSlot],
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner.allocate_array(elements, compact_type_tag, flags)
    }

    fn allocate_record(
        &mut self,
        fields: &[VmRecordField],
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner.allocate_record(fields, compact_type_tag, flags)
    }

    fn alloc_typed_bytes(
        &mut self,
        value: Vec<u8>,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner.alloc_typed_bytes(value, compact_type_tag, flags)
    }

    fn alloc_typed_string(
        &mut self,
        value: String,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner
            .alloc_typed_string(value, compact_type_tag, flags)
    }

    fn string_value(&self, value: &ValueSlot) -> Result<String, VmHeapError> {
        self.inner.string_value(value)
    }

    fn bytes_value(&self, value: &ValueSlot) -> Result<Vec<u8>, VmHeapError> {
        self.inner.bytes_value(value)
    }
}

struct CollectRoots(Vec<ValueSlot>);

impl VmRootVisitor for CollectRoots {
    fn visit_root(&mut self, root: &ValueSlot) -> Result<(), VmHeapError> {
        self.0.push(*root);
        Ok(())
    }
}

struct UnusedWriter;

impl crate::BytecodeServerStreamWriterPort for UnusedWriter {
    fn flush(
        &self,
        _frame: crate::BytecodeServerStreamFrame,
        _execution: crate::OwnedExecutionControl,
    ) -> crate::BytecodeServerStreamWriteFuture {
        panic!("an absent stream supervisor cannot call its transport writer")
    }
}

fn absent_supervisor_input(
    fixture: &AbsentSupervisorFixture,
    heap: Box<dyn VmHeap + Send>,
) -> BytecodeRequestExecutionInput {
    let mut request = RequestEnvelope {
        request_id: "phase-5-absent-stream-supervisor".to_string(),
        mode: "serverStream".to_string(),
        target: "display-only".to_string(),
        operation_abi_id: None,
        selector: None,
        service_id: Some(FIXTURE_PACKAGE.to_string()),
        build_id: "legacy-build".to_string(),
        service_protocol_identity: "legacy-protocol".to_string(),
        contract_identity: None,
        activation_identity: None,
        ingress_selector: Some(fixture.selector.clone()),
        binary_http: Some(crate::BinaryHttpRequest {
            metadata: crate::BinaryHttpRequestMetadata {
                method: "POST".to_string(),
                url: format!("https://example.test{FIXTURE_PATH}"),
                path: FIXTURE_PATH.to_string(),
                query: Vec::new(),
                headers: Vec::new(),
            },
            body: b"chunk".to_vec(),
        }),
        http_adapter: None,
        test_effects_enabled: false,
        test_effect_doubles: HashMap::new(),
        payload_bytes: Vec::new(),
        extra: serde_json::Map::new(),
    };
    request.http_adapter = Some(crate::HttpAdapter {
        kind: HttpAdapterKind::RawHttp,
        handler: crate::HttpAdapterCallable::PackageFunction {
            package_id: FIXTURE_PACKAGE.to_string(),
            symbol_path: "main.stream".to_string(),
        },
        guard: None,
        pre: None,
        adapter_args: vec![crate::GatewayAdapterArg {
            param: "request".to_string(),
            source: crate::GatewayAdapterSource::HttpRequest,
        }],
    });
    BytecodeRequestExecutionInput {
        target: fixture.target(),
        request,
        observer: BytecodeExecutionObserver::noop(BytecodeExecutionCorrelation {
            router_session_id: "phase-5-absent-stream-supervisor".to_string(),
            request_id: "phase-5-absent-stream-supervisor".to_string(),
        }),
        cancellation: CancellationToken::new(),
        execution_budget: Arc::new(ExecutionBudget::for_runtime_request(None)),
        handles: BytecodeRequestExecutionHandles {
            request_heap_limits: RequestHeapLimits::default(),
            max_response_bytes: NonZeroUsize::new(1024).expect("response limit is non-zero"),
        },
        http_client: None,
        server_stream_writer: Some(Arc::new(UnusedWriter)),
        child_composition: Default::default(),
        heap: Some(heap),
    }
}

#[test]
fn phase_5_absent_stream_supervisor_release_failure_reaches_terminal_retry_escrow() {
    let fixture = absent_supervisor_fixture();
    let target = fixture.target();
    let trace = ReleaseTrace::new(first_stream_item_type(&target));
    let input = absent_supervisor_input(
        fixture,
        Box::new(FailOnceStreamItemHeap::new(Arc::clone(&trace))),
    );
    let mut context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
    let start = start_bytecode_request(input, context.resource_table())
        .expect("production request start accepts the real stream fixture");
    let (wake_queue, wake_receiver) = RequestPendingWakeQueue::new();
    let resources = context.resource_table();
    let stream_registrar = BytecodeHttpStreamRegistrar::new(resources.clone());
    let runtime = Arc::new(RequestPendingRuntime {
        registry: Arc::new(RequestPendingRegistry::new(
            context
                .take_pending_registration()
                .expect("fresh request context retains pending registration"),
        )),
        wake_queue,
        budget: Arc::clone(&start.execution_budget),
        resources,
        http_client: start.http_client.clone(),
        execution_control: start.execution_control.clone(),
        stream_registrar,
        child_composition: Default::default(),
        db_session: Arc::new(Mutex::new(None)),
        cleanup_roots: Mutex::new(Vec::new()),
        materialization_escrows: Mutex::new(Vec::new()),
        manual_sleep_completion: Mutex::new(None),
    });
    let mut context = context.with_ports(BytecodeSchedulerPorts {
        child_executor: Some(Arc::new(BytecodeHostExecutor {
            runtime: Arc::clone(&runtime),
            child_composition: Default::default(),
            child_heap_factory: Arc::new(crate::bytecode_children::FailClosedChildHeapFactory),
            observer: BytecodeExecutionObserver::noop(BytecodeExecutionCorrelation {
                router_session_id: "phase-5-absent-stream".to_string(),
                request_id: "request".to_string(),
            }),
        })),
        // The request passed the real server-stream admission above. Omitting
        // only this port drives the production absent-supervisor rejection.
        stream_supervisor: None,
        child_stream_supervisors: Arc::new(Mutex::new(Vec::new())),
    });
    let BytecodeStart {
        fiber,
        mut heap,
        mut budget,
        execution_budget,
        mode,
        raw_http_adapter,
        http_client: _,
        server_stream,
        execution_control: _,
    } = start;
    assert!(server_stream.is_some());
    context.install_root(fiber);
    let error = match context.start_drive(&mut *heap, &mut *budget) {
        Err(error) => error,
        Ok(_) => panic!("the absent supervisor must reject the emitted item"),
    };

    let mut roots = CollectRoots(Vec::new());
    error
        .visit_roots(&mut roots)
        .expect("owner-bearing scheduler error exposes its exact item root");
    let [root] = roots.0.as_slice() else {
        panic!("the rejected stream item retains exactly one root")
    };
    assert_eq!(heap.validate_live(root), Ok(()));
    assert!(error.to_string().contains(RELEASE_FAILURE));
    assert_eq!(trace.attempts.load(Ordering::Acquire), 1);
    assert_eq!(trace.successes.load(Ordering::Acquire), 0);

    let terminal = ParkedBytecodeRequest {
        context,
        heap,
        budget,
        execution_budget,
        runtime,
        wake_receiver,
        mode,
        raw_http_adapter,
    }
    .terminal(error);
    let ControlledBytecodeDrive::Complete(driven) = terminal else {
        panic!("an absent supervisor is a terminal request failure")
    };
    let DrivenBytecodeRequest {
        result,
        retention,
        owner_inventory,
    } = driven;
    assert!(matches!(
        result,
        Err(RequestError::Unsupported(message)) if message.contains(RELEASE_FAILURE)
    ));
    assert!(retention.cleanup_roots.is_empty());
    let mut retained_roots = CollectRoots(Vec::new());
    assert!(retention.scheduler_failure_owner.is_some());
    retention
        .visit_roots(&mut retained_roots)
        .expect("request retention exposes the exact terminal root");
    assert!(retained_roots.0.as_slice() == roots.0.as_slice());
    assert_eq!(
        retention
            .heap
            .as_deref()
            .expect("terminal retention keeps the request heap")
            .validate_live(root),
        Ok(())
    );
    assert_eq!(trace.attempts.load(Ordering::Acquire), 1);
    assert_eq!(trace.successes.load(Ordering::Acquire), 0);
    let snapshot = owner_inventory.into_snapshot();
    assert_eq!(snapshot.pending.current, 0);
    assert_eq!(snapshot.resource.current, 0);
    assert_eq!(snapshot.child.current, 0);

    drop(retention);
    assert_eq!(trace.attempts.load(Ordering::Acquire), 2);
    assert_eq!(trace.successes.load(Ordering::Acquire), 1);
}

#[test]
fn phase_6_child_stream_missing_authority_fails_closed_before_http_decode() {
    let fixture = absent_supervisor_fixture();
    let target = fixture.target();
    let item_type = skiff_runtime_linked_bytecode::TypeIndex::new(first_stream_item_type(&target));

    crate::bytecode_server_stream::validate_stream_producer_authority(
        &fixture.image,
        target.function(),
        item_type,
        0,
    )
    .expect("the root server-stream entry retains its exact stream-result authority");
    crate::bytecode_server_stream::validate_stream_producer_authority(
        &fixture.image,
        target.function(),
        item_type,
        1,
    )
    .expect("a child producer with exact stream-result authority passes the same checked seam");

    let error = crate::bytecode_server_stream::validate_stream_producer_authority(
        &fixture.image,
        target.function(),
        skiff_runtime_linked_bytecode::TypeIndex::new(0),
        1,
    )
    .expect_err("a child item that is not the linked Stream<T> item must fail closed");
    assert!(
        matches!(
            error,
            BytecodeSchedulerError::Port(ref message)
                if message.contains("lacks the exact linked stream-result authority")
        ),
        "missing authority must stay a typed port failure: {error}"
    );
}
