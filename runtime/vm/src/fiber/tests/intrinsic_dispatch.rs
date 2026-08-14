use super::*;

#[test]
fn operand_push_preflight_failure_never_partially_installs_or_advances_height() {
    let fixture = ObservationFixture::build(
        "example.com/fiber-push-transaction",
        "function run() -> number { return 1 }",
    );
    let observer = BytecodeExecutionObserver::new(
        Arc::new(RecordingSink::default()),
        observation_correlation(),
    );
    let mut fiber = fixture.start(vm_limits(), observer, Box::<[ValueSlot]>::default());
    let frame = fiber.current_frame().unwrap().clone();
    let destination = frame.operand_base() + frame.operand_height();
    let retained =
        ValueSlot::request_heap_ref(VmHandle::new(77), compact_tag(12), ValueFlags::new(0));
    fiber.values[destination] = retained;
    fiber.live_values[destination] = true;

    let error = fiber
        .push_operand(ValueSlot::number(2.0))
        .expect_err("a live destination must reject the push before commit");

    assert!(matches!(error, VmError::LiveDestination { .. }));
    assert_eq!(fiber.current_frame().unwrap().operand_height(), 0);
    assert!(fiber.values[destination] == retained);
    assert!(fiber.live_values[destination]);
}

const INTRINSIC_DISPATCH_SOURCE: &str = r#"
import std

function run(request: std.http.HttpRequest) -> Stream<std.http.HttpResponseStreamEvent> {
  final base = "left".concat("-")
  final joined = base.concat("right")
  final body = bytes.fromUtf8(joined)
  final text = body.toUtf8String()
  emit({ tag: "chunk", value: bytes.fromUtf8(text) })
  return null
}
"#;

struct IntrinsicDispatchFixture {
    image: Arc<DeploymentExecutionImage>,
    selector: IngressSelector,
    gateway_identity: GatewayEntryIdentity,
}

impl IntrinsicDispatchFixture {
    fn build() -> Self {
        static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("VM crate has a repository root")
            .to_path_buf();
        let temp = std::env::temp_dir().join(format!(
            "skiff-vm-p5-intrinsic-dispatch-{}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let fixture_root = temp.join("source");
        let artifact_root = temp.join("artifacts");
        std::fs::create_dir_all(&fixture_root).unwrap();
        std::fs::create_dir_all(&artifact_root).unwrap();
        std::fs::write(
            fixture_root.join("package.yml"),
            "id: example.com/vm-p5-intrinsic-dispatch\nversion: 1.0.0\n",
        )
        .unwrap();
        std::fs::write(
            fixture_root.join("service.yml"),
            "id: example.com/vm-p5-intrinsic-dispatch\n",
        )
        .unwrap();
        std::fs::write(fixture_root.join("api.yml"), "{}\n").unwrap();
        std::fs::write(
            fixture_root.join("http.yml"),
            "run:\n  method: POST\n  path: /vm/intrinsic-dispatch\n  kind: rawHttp\n  handler: main.run\n  adapterArgs:\n    - param: request\n      source: { kind: http.request }\n",
        )
        .unwrap();
        std::fs::write(fixture_root.join("main.skiff"), INTRINSIC_DISPATCH_SOURCE).unwrap();

        let platform_sources = CompilerPlatformSources::new(&repository_root)
            .expect("open repository platform sources");
        seed_official_std_package(&platform_sources, &artifact_root)
            .expect("seed canonical std into the intrinsic fixture store");
        let receipt = build_authoring_object(
            &platform_sources,
            AuthoringObject::Package,
            &fixture_root,
            &artifact_root,
            "skiff-test",
            true,
        )
        .unwrap_or_else(|error| panic!("production authoring accepts intrinsic fixture: {error}"));
        let deployment_reference =
            serde_json::from_value::<skiff_artifact_model::ServiceDeploymentRef>(
                receipt
                    .pointer("/serviceDeploymentReceipt/deployment")
                    .cloned()
                    .expect("authoring receipt carries deployment"),
            )
            .expect("authoring deployment receipt remains typed");
        let resolver = FilesystemDeploymentBytecodeContentResolver::open(&artifact_root)
            .expect("open intrinsic fixture resolver");
        let deployment = resolver
            .resolve_deployment(&deployment_reference)
            .expect("read intrinsic fixture deployment");
        let ingress = deployment
            .ingress
            .iter()
            .find(|binding| {
                binding.selector.protocol == IngressProtocol::Http
                    && binding.selector.method.as_deref() == Some("POST")
                    && binding.selector.path == "/vm/intrinsic-dispatch"
            })
            .expect("intrinsic fixture publishes its exact HTTP ingress");
        let selector = ingress.selector.clone();
        let gateway_identity = deployment
            .gateway_entries
            .get(&ingress.gateway_entry_key)
            .expect("intrinsic ingress pins a gateway entry")
            .gateway_entry_identity
            .clone();
        let hydrated = DeploymentBytecodeLoader::new(&resolver)
            .load(&deployment_reference)
            .expect("load intrinsic fixture closure");
        let image = Arc::new(
            link_deployment_execution_image(hydrated, &link_limits())
                .expect("link intrinsic fixture image"),
        );
        std::fs::remove_dir_all(temp).unwrap();
        Self {
            image,
            selector,
            gateway_identity,
        }
    }

    fn target(&self) -> DeploymentExecutionEntry {
        self.image
            .http_gateway_entry(&self.selector, &self.gateway_identity)
            .expect("intrinsic fixture gateway entry remains linked")
    }
}

fn intrinsic_dispatch_fixture() -> &'static IntrinsicDispatchFixture {
    static FIXTURE: OnceLock<IntrinsicDispatchFixture> = OnceLock::new();
    FIXTURE.get_or_init(IntrinsicDispatchFixture::build)
}

#[derive(Clone)]
enum IntrinsicDispatchValue {
    Opaque,
    String(String),
    Bytes(Vec<u8>),
    Record(Vec<VmRecordField>),
}

struct IntrinsicDispatchEntry {
    owners: usize,
    value: IntrinsicDispatchValue,
}

#[derive(Default)]
struct IntrinsicDispatchHeap {
    next_handle: u64,
    entries: BTreeMap<u64, IntrinsicDispatchEntry>,
    transfer_attempts: usize,
    fail_transfer_at: Option<usize>,
    release_attempts: usize,
    fail_release_at: Option<usize>,
    typed_string_allocations: usize,
    fail_typed_string_at: Option<usize>,
    record_allocations: usize,
    fail_record_allocation: bool,
    fail_bytes_read: bool,
    array_push_attempts: usize,
}

impl IntrinsicDispatchHeap {
    fn allocate(
        &mut self,
        value: IntrinsicDispatchValue,
        tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> ValueSlot {
        self.next_handle += 1;
        let handle = self.next_handle;
        assert!(self
            .entries
            .insert(handle, IntrinsicDispatchEntry { owners: 1, value })
            .is_none());
        ValueSlot::request_heap_ref(VmHandle::new(handle), tag, flags)
    }

    fn handle(value: &ValueSlot) -> Result<u64, VmHeapError> {
        value
            .as_request_heap_ref()
            .map(VmHandle::get)
            .ok_or(VmHeapError::InvalidValueMetadata)
    }

    fn owner_count(&self, handle: u64) -> usize {
        self.entries.get(&handle).map_or(0, |entry| entry.owners)
    }

    fn release_handle(&mut self, handle: u64) -> Result<(), VmHeapError> {
        let children = {
            let entry = self
                .entries
                .get_mut(&handle)
                .ok_or(VmHeapError::InvalidValueMetadata)?;
            entry.owners = entry
                .owners
                .checked_sub(1)
                .ok_or(VmHeapError::InvalidValueMetadata)?;
            if entry.owners != 0 {
                return Ok(());
            }
            match &entry.value {
                IntrinsicDispatchValue::Record(fields) => {
                    fields.iter().map(|field| field.value).collect::<Vec<_>>()
                }
                IntrinsicDispatchValue::Opaque
                | IntrinsicDispatchValue::String(_)
                | IntrinsicDispatchValue::Bytes(_) => Vec::new(),
            }
        };
        self.entries.remove(&handle);
        for child in children {
            if let Some(child) = child.as_request_heap_ref() {
                self.release_handle(child.get())?;
            }
        }
        Ok(())
    }
}

impl VmHeap for IntrinsicDispatchHeap {
    fn validate_live(&self, value: &ValueSlot) -> Result<(), VmHeapError> {
        match value.kind() {
            Some(
                skiff_runtime_model::vm_value::ValueKind::Null
                | skiff_runtime_model::vm_value::ValueKind::Bool
                | skiff_runtime_model::vm_value::ValueKind::Number
                | skiff_runtime_model::vm_value::ValueKind::Integer
                | skiff_runtime_model::vm_value::ValueKind::Date,
            ) => Ok(()),
            Some(skiff_runtime_model::vm_value::ValueKind::RequestHeapRef)
                if value
                    .as_request_heap_ref()
                    .is_some_and(|handle| self.entries.contains_key(&handle.get())) =>
            {
                Ok(())
            }
            _ => Err(VmHeapError::InvalidValueMetadata),
        }
    }

    fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.validate_live(source)?;
        let handle = Self::handle(source)?;
        self.entries
            .get_mut(&handle)
            .ok_or(VmHeapError::InvalidValueMetadata)?
            .owners += 1;
        Ok(*source)
    }

    fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.validate_live(source)?;
        self.transfer_attempts += 1;
        if self.fail_transfer_at == Some(self.transfer_attempts) {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::TransferOwner,
                message: "injected record field transfer failure".to_string(),
            });
        }
        Ok(*source)
    }

    fn release_snapshot(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
        self.release_attempts += 1;
        if self.fail_release_at == Some(self.release_attempts) {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::ReleaseSnapshot,
                message: "injected intrinsic release failure".to_string(),
            });
        }
        let handle = Self::handle(owner)?;
        self.release_handle(handle)
    }

    fn release_resource(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::ReleaseResource,
            kind: skiff_runtime_model::vm_value::ValueKind::ResourceRef,
        })
    }

    fn alloc_typed_string(
        &mut self,
        value: String,
        tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.typed_string_allocations += 1;
        if self.fail_typed_string_at == Some(self.typed_string_allocations) {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::AllocateRepresentation,
                message: "injected record field materialization failure".to_string(),
            });
        }
        Ok(self.allocate(IntrinsicDispatchValue::String(value), tag, flags))
    }

    fn alloc_typed_bytes(
        &mut self,
        value: Vec<u8>,
        tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        Ok(self.allocate(IntrinsicDispatchValue::Bytes(value), tag, flags))
    }

    fn allocate_record(
        &mut self,
        fields: &[VmRecordField],
        tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        for field in fields {
            self.validate_live(&field.value)?;
        }
        self.record_allocations += 1;
        if self.fail_record_allocation {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::AllocateRecord,
                message: "injected record allocation failure".to_string(),
            });
        }
        Ok(self.allocate(IntrinsicDispatchValue::Record(fields.to_vec()), tag, flags))
    }

    fn string_value(&self, value: &ValueSlot) -> Result<String, VmHeapError> {
        let handle = Self::handle(value)?;
        match &self
            .entries
            .get(&handle)
            .ok_or(VmHeapError::InvalidValueMetadata)?
            .value
        {
            IntrinsicDispatchValue::String(value) => Ok(value.clone()),
            IntrinsicDispatchValue::Opaque
            | IntrinsicDispatchValue::Bytes(_)
            | IntrinsicDispatchValue::Record(_) => Err(VmHeapError::InvalidValueMetadata),
        }
    }

    fn bytes_value(&self, value: &ValueSlot) -> Result<Vec<u8>, VmHeapError> {
        if self.fail_bytes_read {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::RepresentationPayload,
                message: "injected intrinsic bytes read failure".to_string(),
            });
        }
        let handle = Self::handle(value)?;
        match &self
            .entries
            .get(&handle)
            .ok_or(VmHeapError::InvalidValueMetadata)?
            .value
        {
            IntrinsicDispatchValue::Bytes(value) => Ok(value.clone()),
            IntrinsicDispatchValue::Opaque
            | IntrinsicDispatchValue::String(_)
            | IntrinsicDispatchValue::Record(_) => Err(VmHeapError::InvalidValueMetadata),
        }
    }

    fn record_field(&self, record: &ValueSlot, field: &str) -> Result<ValueSlot, VmHeapError> {
        let handle = Self::handle(record)?;
        let IntrinsicDispatchValue::Record(fields) = &self
            .entries
            .get(&handle)
            .ok_or(VmHeapError::InvalidValueMetadata)?
            .value
        else {
            return Err(VmHeapError::InvalidValueMetadata);
        };
        fields
            .iter()
            .find(|candidate| candidate.name == field)
            .map(|candidate| candidate.value)
            .ok_or(VmHeapError::InvalidValueMetadata)
    }

    fn get_dense_field(
        &self,
        record: &ValueSlot,
        field_ordinal: usize,
    ) -> Result<ValueSlot, VmHeapError> {
        let handle = Self::handle(record)?;
        let IntrinsicDispatchValue::Record(fields) = &self
            .entries
            .get(&handle)
            .ok_or(VmHeapError::InvalidValueMetadata)?
            .value
        else {
            return Err(VmHeapError::InvalidValueMetadata);
        };
        fields
            .get(field_ordinal)
            .map(|field| field.value)
            .ok_or(VmHeapError::InvalidValueMetadata)
    }

    fn array_push_owned(
        &mut self,
        _array: &ValueSlot,
        _item: ValueSlot,
    ) -> Result<(), VmHeapError> {
        self.array_push_attempts += 1;
        Err(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::ArrayPushOwned,
            message: "legacy intrinsic must never reach heap mutation".to_string(),
        })
    }
}

#[derive(Default)]
struct IntrinsicRootHandles(Vec<u64>);

impl VmRootVisitor for IntrinsicRootHandles {
    fn visit_root(&mut self, root: &ValueSlot) -> Result<(), VmHeapError> {
        if let Some(handle) = root.as_handle() {
            self.0.push(handle.get());
        }
        Ok(())
    }
}

fn current_intrinsic(fiber: &VmFiber) -> Option<(String, TypeIndex)> {
    let frame = fiber.current_frame().ok()?;
    let instruction = fiber
        .function(frame.function())
        .ok()?
        .instructions()
        .get(frame.instruction().get() as usize)?;
    if instruction.opcode() != Opcode::InvokeIntrinsic {
        return None;
    }
    let LinkedInstructionTarget::Intrinsic(index) = fiber
        .resolved_target(frame.function(), frame.instruction(), instruction, 0)
        .ok()?
    else {
        return None;
    };
    let intrinsic = fiber
        .execution_image()
        .intrinsics()
        .get(index.get() as usize)
        .filter(|intrinsic| intrinsic.index() == index)?;
    let key = match intrinsic.kind() {
        LinkedIntrinsicKind::Static(target) => target.canonical_key().as_str(),
        LinkedIntrinsicKind::Receiver(target) => target.canonical_key,
    };
    Some((
        key.to_string(),
        *intrinsic.signature().result_types().first()?,
    ))
}

fn current_opcode(fiber: &VmFiber) -> Option<Opcode> {
    let frame = fiber.current_frame().ok()?;
    fiber
        .function(frame.function())
        .ok()?
        .instructions()
        .get(frame.instruction().get() as usize)
        .map(|instruction| instruction.opcode())
}

fn drive_to_new_record(fiber: &mut VmFiber, heap: &mut IntrinsicDispatchHeap) {
    for _ in 0..10_000 {
        if current_opcode(fiber) == Some(Opcode::NewRecord) {
            return;
        }
        assert!(matches!(
            fiber.dispatch_one(heap).expect("drive to NewRecord"),
            DispatchOutcome::Continue
        ));
    }
    panic!("intrinsic fixture did not reach NewRecord within the step cap");
}

fn new_record_operand_window(fiber: &VmFiber) -> (usize, usize, Vec<ValueSlot>) {
    let frame = fiber.current_frame().expect("NewRecord frame").clone();
    let decoded = fiber
        .function(frame.function())
        .expect("NewRecord function")
        .instructions()
        .get(frame.instruction().get() as usize)
        .expect("NewRecord instruction");
    assert_eq!(decoded.opcode(), Opcode::NewRecord);
    let field_count = fiber
        .operand_usize(decoded, 1, frame.function(), frame.instruction())
        .expect("NewRecord field count");
    let (_, start, values) = fiber
        .borrow_operands(field_count)
        .expect("NewRecord operand window");
    (frame.operand_height(), start, values)
}

fn assert_new_record_window_rooted(
    fiber: &VmFiber,
    expected_height: usize,
    start: usize,
    field_count: usize,
) {
    assert_eq!(
        fiber.current_frame().unwrap().operand_height(),
        expected_height
    );
    let mut roots = IntrinsicRootHandles::default();
    fiber.visit_roots(&mut roots).unwrap();
    for index in start..start + field_count {
        assert!(fiber.live_values[index], "field operand {index} stays live");
        if let Some(handle) = fiber.values[index].as_handle() {
            assert!(
                roots.0.contains(&handle.get()),
                "field operand {index} remains visible to root visitation"
            );
        }
    }
}

fn intrinsic_fiber(heap: &mut IntrinsicDispatchHeap) -> VmFiber {
    let fixture = intrinsic_dispatch_fixture();
    let entry = fixture.target();
    let [parameter_type] = entry.signature().parameter_types() else {
        panic!("intrinsic fixture retains one exact HTTP request parameter")
    };
    let request = heap.allocate(
        IntrinsicDispatchValue::Opaque,
        compact_tag(parameter_type.get()),
        ValueFlags::new(0),
    );
    let observer = BytecodeExecutionObserver::new(
        Arc::new(RecordingSink::default()),
        observation_correlation(),
    );
    Vm::start(entry, Box::new([request]), vm_limits(), observer)
        .expect("start exact server-stream intrinsic entry")
}

fn drive_intrinsic_fiber_to_completion(fiber: &mut VmFiber, heap: &mut IntrinsicDispatchHeap) {
    for _ in 0..10_000 {
        match fiber
            .dispatch_one(heap)
            .expect("intrinsic fixture dispatch")
        {
            DispatchOutcome::Continue => {}
            DispatchOutcome::Complete(_) => return,
            DispatchOutcome::Handoff(VmControl::EmitStream(item)) => {
                let resume = item.release(heap).expect("release emitted chunk owner");
                fiber
                    .resume(resume, ResumeOutcome::Empty)
                    .expect("resume after emitted chunk");
            }
            DispatchOutcome::Handoff(_) => panic!("intrinsic fixture has only EmitStream handoff"),
            DispatchOutcome::Throw(_) => panic!("intrinsic fixture must not throw"),
        }
    }
    panic!("intrinsic fixture did not terminate within the step cap");
}

#[test]
fn intrinsic_dispatch_executes_phase_5_string_and_bytes_ops_with_typed_rooted_results() {
    let mut heap = IntrinsicDispatchHeap::default();
    let mut fiber = intrinsic_fiber(&mut heap);
    let mut seen = BTreeMap::<String, usize>::new();
    let mut completed = false;

    for _ in 0..10_000 {
        let intrinsic = current_intrinsic(&fiber);
        let outcome = fiber.dispatch_one(&mut heap).expect("intrinsic dispatch");
        if let Some((key, result_type)) = intrinsic {
            *seen.entry(key).or_default() += 1;
            let frame = fiber.current_frame().expect("result remains in the frame");
            let result_index = frame.operand_base() + frame.operand_height() - 1;
            let result = fiber.values[result_index];
            assert_eq!(
                result.compact_type_tag().map(CompactTypeTag::type_index),
                Some(result_type.get()),
                "intrinsic result must carry its exact linked signature type"
            );
            let handle = result
                .as_request_heap_ref()
                .expect("string/bytes result is request-owned");
            let mut roots = IntrinsicRootHandles::default();
            fiber.visit_roots(&mut roots).unwrap();
            assert!(
                roots.0.contains(&handle.get()),
                "the committed result owner must be visible to root visitation"
            );
        }
        match outcome {
            DispatchOutcome::Continue => {}
            DispatchOutcome::Complete(_) => {
                completed = true;
                break;
            }
            DispatchOutcome::Handoff(VmControl::EmitStream(item)) => {
                let resume = item
                    .release(&mut heap)
                    .expect("release emitted chunk owner");
                fiber
                    .resume(resume, ResumeOutcome::Empty)
                    .expect("resume after emitted chunk");
            }
            DispatchOutcome::Handoff(_) => panic!("intrinsic fixture has only EmitStream handoff"),
            DispatchOutcome::Throw(_) => panic!("intrinsic fixture must not throw"),
        }
    }

    assert!(
        completed,
        "intrinsic fixture must complete within the step cap"
    );
    assert_eq!(seen.get("receiver:string.concat@1"), Some(&2));
    assert_eq!(seen.get("core.bytes.fromUtf8"), Some(&2));
    assert_eq!(seen.get("receiver:bytes.toUtf8String@1"), Some(&1));
    assert!(
        heap.entries.is_empty(),
        "frame exit releases every result owner"
    );
}

#[test]
fn intrinsic_dispatch_read_failure_releases_only_the_borrowed_operand_owner() {
    let mut heap = IntrinsicDispatchHeap::default();
    let mut fiber = intrinsic_fiber(&mut heap);

    for _ in 0..10_000 {
        if current_intrinsic(&fiber).is_some_and(|(key, _)| key == "receiver:bytes.toUtf8String@1")
        {
            break;
        }
        assert!(matches!(
            fiber.dispatch_one(&mut heap).unwrap(),
            DispatchOutcome::Continue
        ));
    }
    let frame = fiber.current_frame().unwrap().clone();
    let input_index = frame.operand_base() + frame.operand_height() - 1;
    let input = fiber.values[input_index];
    let handle = input.as_request_heap_ref().unwrap().get();
    assert_eq!(heap.owner_count(handle), 2, "slot plus borrowed operand");
    heap.fail_bytes_read = true;

    let error = match fiber.dispatch_one(&mut heap) {
        Err(error) => error,
        Ok(_) => panic!("bytes payload read failure must fail the intrinsic"),
    };

    assert!(matches!(
        error,
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::RepresentationPayload,
            ..
        })
    ));
    assert_eq!(heap.owner_count(handle), 1);
    let mut roots = IntrinsicRootHandles::default();
    fiber.visit_roots(&mut roots).unwrap();
    assert_eq!(roots.0.iter().filter(|root| **root == handle).count(), 1);
}

#[test]
fn intrinsic_dispatch_first_release_failure_keeps_operand_rooted_until_retry() {
    let mut heap = IntrinsicDispatchHeap::default();
    let mut fiber = intrinsic_fiber(&mut heap);
    let mut concat_count = 0;

    for _ in 0..10_000 {
        if current_intrinsic(&fiber).is_some_and(|(key, _)| {
            if key == "receiver:string.concat@1" {
                concat_count += 1;
            }
            key == "receiver:string.concat@1" && concat_count == 2
        }) {
            break;
        }
        assert!(matches!(
            fiber.dispatch_one(&mut heap).unwrap(),
            DispatchOutcome::Continue
        ));
    }
    let frame = fiber.current_frame().unwrap().clone();
    let first_argument_index = frame.operand_base() + frame.operand_height() - 2;
    let first_argument = fiber.values[first_argument_index];
    let handle = first_argument.as_request_heap_ref().unwrap().get();
    let height = frame.operand_height();
    assert_eq!(heap.owner_count(handle), 2, "slot plus borrowed operand");
    heap.fail_release_at = Some(heap.release_attempts + 1);

    let error = match fiber.dispatch_one(&mut heap) {
        Err(error) => error,
        Ok(_) => panic!("the first owned argument release is injected to fail"),
    };

    assert!(matches!(
        error,
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::ReleaseSnapshot,
            ..
        })
    ));
    assert_eq!(fiber.current_frame().unwrap().operand_height(), height);
    assert!(fiber.values[first_argument_index] == first_argument);
    assert!(fiber.live_values[first_argument_index]);
    assert_eq!(heap.owner_count(handle), 2);
    let mut roots = IntrinsicRootHandles::default();
    fiber.visit_roots(&mut roots).unwrap();
    assert_eq!(roots.0.iter().filter(|root| **root == handle).count(), 2);

    heap.fail_release_at = None;
    assert!(matches!(
        fiber.dispatch_one(&mut heap).unwrap(),
        DispatchOutcome::Continue
    ));
    drive_intrinsic_fiber_to_completion(&mut fiber, &mut heap);
    assert!(heap.entries.is_empty());
}

#[test]
fn new_record_first_materialization_failure_keeps_the_whole_operand_window_rooted() {
    let mut heap = IntrinsicDispatchHeap::default();
    let mut fiber = intrinsic_fiber(&mut heap);
    drive_to_new_record(&mut fiber, &mut heap);
    let (height, start, values) = new_record_operand_window(&fiber);
    heap.fail_typed_string_at = Some(heap.typed_string_allocations + 1);

    let error = match fiber.dispatch_one(&mut heap) {
        Err(error) => error,
        Ok(_) => panic!("first field materialization failure must abort NewRecord"),
    };

    assert!(matches!(
        error,
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::AllocateRepresentation,
            ..
        })
    ));
    assert_new_record_window_rooted(&fiber, height, start, values.len());
    assert!(&fiber.values[start..start + values.len()] == values.as_slice());

    heap.fail_typed_string_at = None;
    assert!(matches!(
        fiber.dispatch_one(&mut heap).unwrap(),
        DispatchOutcome::Continue
    ));
    drive_intrinsic_fiber_to_completion(&mut fiber, &mut heap);
    assert!(heap.entries.is_empty());
}

#[test]
fn new_record_middle_transfer_failure_roots_materialized_prefix_and_original_suffix() {
    let mut heap = IntrinsicDispatchHeap::default();
    let mut fiber = intrinsic_fiber(&mut heap);
    drive_to_new_record(&mut fiber, &mut heap);
    let (height, start, values) = new_record_operand_window(&fiber);
    heap.fail_transfer_at = Some(heap.transfer_attempts + 1);

    let error = match fiber.dispatch_one(&mut heap) {
        Err(error) => error,
        Ok(_) => panic!("second field transfer failure must abort NewRecord"),
    };

    assert!(matches!(
        error,
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::TransferOwner,
            ..
        })
    ));
    assert_new_record_window_rooted(&fiber, height, start, values.len());
    assert!(matches!(
        fiber.values[start].kind(),
        Some(skiff_runtime_model::vm_value::ValueKind::RequestHeapRef)
    ));
    assert!(fiber.values[start + 1] == values[1]);

    heap.fail_transfer_at = None;
    assert!(matches!(
        fiber.dispatch_one(&mut heap).unwrap(),
        DispatchOutcome::Continue
    ));
    drive_intrinsic_fiber_to_completion(&mut fiber, &mut heap);
    assert!(heap.entries.is_empty());
}

#[test]
fn new_record_allocation_failure_keeps_all_transferred_fields_rooted_until_retry() {
    let mut heap = IntrinsicDispatchHeap::default();
    let mut fiber = intrinsic_fiber(&mut heap);
    drive_to_new_record(&mut fiber, &mut heap);
    let (height, start, values) = new_record_operand_window(&fiber);
    heap.fail_record_allocation = true;

    let error = match fiber.dispatch_one(&mut heap) {
        Err(error) => error,
        Ok(_) => panic!("record allocation failure must leave the window installed"),
    };

    assert!(matches!(
        error,
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::AllocateRecord,
            ..
        })
    ));
    assert_eq!(heap.record_allocations, 1);
    assert_new_record_window_rooted(&fiber, height, start, values.len());
    assert!(fiber.values[start..start + values.len()]
        .iter()
        .all(|value| matches!(
            value.kind(),
            Some(skiff_runtime_model::vm_value::ValueKind::RequestHeapRef)
        )));

    heap.fail_record_allocation = false;
    assert!(matches!(
        fiber.dispatch_one(&mut heap).unwrap(),
        DispatchOutcome::Continue
    ));
    drive_intrinsic_fiber_to_completion(&mut fiber, &mut heap);
    assert!(heap.entries.is_empty());
}

fn no_intrinsic_effects() -> CallableMayEffects {
    CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_pending: false,
        pending_effect_categories: Vec::new(),
        inout_path_effects: Vec::new(),
    }
}

#[test]
fn legacy_array_push_intrinsic_executor_fails_before_mutation_with_both_operands_rooted() {
    let fixture = ObservationFixture::build(
        "example.com/fiber-legacy-array-push",
        "function run() -> number { return 1 + 2 }",
    );
    let observer = BytecodeExecutionObserver::new(
        Arc::new(RecordingSink::default()),
        observation_correlation(),
    );
    let mut fiber = fixture.start(vm_limits(), observer, Box::<[ValueSlot]>::default());
    let mut heap = IntrinsicDispatchHeap::default();
    let array = heap.allocate(
        IntrinsicDispatchValue::Opaque,
        compact_tag(10),
        ValueFlags::new(0),
    );
    let item = heap.allocate(
        IntrinsicDispatchValue::Opaque,
        compact_tag(11),
        ValueFlags::new(0),
    );
    fiber.push_operand(array).unwrap();
    fiber.push_operand(item).unwrap();
    let snapshot = intrinsic_snapshot_plan();
    let target = LinkedIntrinsicTarget::new(
        IntrinsicIndex::new(0),
        LinkedIntrinsicKind::Receiver(
            builtin_receiver_op(BuiltinReceiverRoot::Array, BuiltinReceiverMethod::Push, 1)
                .expect("canonical Array.push receiver op"),
        ),
        LinkedNativeCallableSignature::new(
            Box::new([TypeIndex::new(10), TypeIndex::new(11)]),
            Box::new([ParamModeIr::Value, ParamModeIr::Value]),
            Box::new([snapshot.clone(), snapshot]),
            Box::new([TypeIndex::new(12)]),
            Box::new([LinkedValueTransferPlan::SnapshotShare {
                drop: LinkedValueDropPlan::Trivial,
            }]),
            no_intrinsic_effects(),
        )
        .expect("legacy intrinsic signature remains structurally valid"),
    );
    let frame = fiber.current_frame().unwrap().clone();
    let mut executor = LifecycleExecutor::new(&mut heap);

    let error = match fiber.execute_resolved_intrinsic(
        &mut executor,
        frame.function(),
        frame.instruction(),
        &target,
        2,
        1,
    ) {
        Err(error) => error,
        Ok(_) => panic!("legacy Array.push must fail before borrowing or mutation"),
    };
    drop(executor);

    assert!(matches!(
        error,
        VmError::FullValueLifecyclePlanUnavailable {
            opcode: Opcode::InvokeIntrinsic,
            ..
        }
    ));
    assert_eq!(heap.array_push_attempts, 0);
    assert_eq!(fiber.current_frame().unwrap().operand_height(), 2);
    let operand_base = fiber.current_frame().unwrap().operand_base();
    assert!(fiber.values[operand_base] == array);
    assert!(fiber.values[operand_base + 1] == item);
    assert!(fiber.live_values[operand_base]);
    assert!(fiber.live_values[operand_base + 1]);
    let mut roots = IntrinsicRootHandles::default();
    fiber.visit_roots(&mut roots).unwrap();
    assert!(roots
        .0
        .contains(&array.as_request_heap_ref().unwrap().get()));
    assert!(roots.0.contains(&item.as_request_heap_ref().unwrap().get()));
}
