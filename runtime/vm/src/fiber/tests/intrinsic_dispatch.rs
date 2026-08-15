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
  final suffix = "ri".concat("ght")
  var joined = base.concat(suffix)
  joined = joined.concat("")
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
        Self::build_source(
            "example.com/vm-p5-intrinsic-dispatch",
            "/vm/intrinsic-dispatch",
            INTRINSIC_DISPATCH_SOURCE,
            "intrinsic-dispatch",
        )
    }

    fn build_source(package_id: &str, path: &str, source: &str, temp_label: &str) -> Self {
        static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("VM crate has a repository root")
            .to_path_buf();
        let temp = std::env::temp_dir().join(format!(
            "skiff-vm-p5-{temp_label}-{}-{}-{}",
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
            format!("id: {package_id}\nversion: 1.0.0\n"),
        )
        .unwrap();
        std::fs::write(
            fixture_root.join("service.yml"),
            format!("id: {package_id}\n"),
        )
        .unwrap();
        std::fs::write(fixture_root.join("api.yml"), "{}\n").unwrap();
        std::fs::write(
            fixture_root.join("http.yml"),
            format!(
                "run:\n  method: POST\n  path: {path}\n  kind: rawHttp\n  handler: main.run\n  adapterArgs:\n    - param: request\n      source: {{ kind: http.request }}\n"
            ),
        )
        .unwrap();
        std::fs::write(fixture_root.join("main.skiff"), source).unwrap();

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
                    && binding.selector.path == path
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

const HOST_RESUME_SOURCE: &str = r#"
import std

function headers() -> Array<std.http.HttpHeader> { return [] }

function run(request: std.http.HttpRequest) -> Stream<std.http.HttpResponseStreamEvent> {
  final response = std.http.request(std.http.HttpClientRequest {
    method: "GET",
    url: "http://127.0.0.1/owned-values",
    headers: headers(),
    body: null,
    timeoutMs: 1,
  })
  emit({ tag: "chunk", value: response.body })
  return null
}
"#;

fn host_resume_fixture() -> &'static IntrinsicDispatchFixture {
    static FIXTURE: OnceLock<IntrinsicDispatchFixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        IntrinsicDispatchFixture::build_source(
            "example.com/vm-p5-owned-values-resume",
            "/vm/owned-values-resume",
            HOST_RESUME_SOURCE,
            "owned-values-resume",
        )
    })
}

pub(super) fn host_result_resume_token() -> crate::VmResumeToken {
    let fixture = host_resume_fixture();
    let entry = fixture.target();
    let function_index = entry.function();
    let function = fixture
        .image
        .functions()
        .get(function_index.get() as usize)
        .filter(|function| function.index() == function_index)
        .expect("host fixture root function remains linked");
    let (instruction_ordinal, instruction) = function
        .instructions()
        .iter()
        .enumerate()
        .find(|(_, instruction)| instruction.opcode() == Opcode::InvokeHost)
        .expect("host fixture emits one exact InvokeHost");
    let instruction_index = InstructionIndex::new(instruction_ordinal as u32);
    let adapter = instruction
        .resolved_operands()
        .iter()
        .find(|operand| operand.operand_ordinal() == 0)
        .and_then(|operand| match operand.target() {
            LinkedInstructionTarget::HostEffectAdapter(adapter) => Some(adapter),
            _ => None,
        })
        .expect("InvokeHost resolves its exact adapter");
    let resume_site = instruction
        .resolved_operands()
        .iter()
        .find(|operand| operand.operand_ordinal() == 3)
        .and_then(|operand| match operand.target() {
            LinkedInstructionTarget::ResumeSite(resume) => Some(resume),
            _ => None,
        })
        .expect("InvokeHost resolves its exact resume site");
    let resume = fixture
        .image
        .resume_sites()
        .get(resume_site)
        .filter(|resume| resume.function() == function_index && resume.site() == instruction_index)
        .expect("InvokeHost resume descriptor matches its instruction");
    crate::VmResumeToken::new(
        Arc::clone(&fixture.image),
        1,
        function_index,
        instruction_index,
        resume.resume(),
        resume.end_resume(),
        resume_site,
        resume.expected_stack_height_before_result(),
        u32::try_from(resume.result_types().len()).unwrap(),
        VmResumeAuthority::Adapter(adapter),
    )
}

pub(super) fn host_resume_fiber_and_token(
    heap: &mut IntrinsicDispatchHeap,
) -> (VmFiber, crate::VmResumeToken) {
    let fixture = host_resume_fixture();
    let entry = fixture.target();
    let [parameter_type] = entry.signature().parameter_types() else {
        panic!("host resume fixture has one exact gateway request parameter")
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
    let mut fiber = Vm::start(entry, Box::new([request]), vm_limits(), observer)
        .expect("host resume fixture accepts its exact request carrier");
    loop {
        match fiber
            .dispatch_one(heap)
            .expect("host fixture reaches its exact adapter handoff")
        {
            DispatchOutcome::Continue => {}
            DispatchOutcome::Handoff(VmControl::EnterAdapter(invocation)) => {
                let (_, arguments, resume) = invocation.into_parts();
                arguments
                    .release(heap)
                    .expect("adapter arguments release through their exact plans");
                return (fiber, resume);
            }
            DispatchOutcome::Handoff(_) => panic!("host fixture exposes one adapter handoff"),
            DispatchOutcome::Complete(_) | DispatchOutcome::Throw(_) => {
                panic!("host fixture must suspend before completion")
            }
        }
    }
}

#[derive(Clone)]
pub(super) enum IntrinsicDispatchValue {
    Opaque,
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<ValueSlot>),
    Record(Vec<VmRecordField>),
}

pub(super) struct IntrinsicDispatchEntry {
    owners: usize,
    value: IntrinsicDispatchValue,
}

#[derive(Default)]
pub(super) struct IntrinsicDispatchHeap {
    next_handle: u64,
    pub(super) entries: BTreeMap<u64, IntrinsicDispatchEntry>,
    pub(super) transfer_attempts: usize,
    pub(super) fail_transfer_at: Option<usize>,
    pub(super) change_transfer_at: Option<usize>,
    pub(super) share_attempts: usize,
    pub(super) fail_share_at: Option<usize>,
    pub(super) release_attempts: usize,
    pub(super) release_history: Vec<u64>,
    pub(super) fail_release_at: Option<usize>,
    typed_string_allocations: usize,
    fail_typed_string_at: Option<usize>,
    pub(super) typed_bytes_allocations: usize,
    pub(super) fail_typed_bytes_at: Option<usize>,
    record_allocations: usize,
    fail_record_allocation: bool,
    pub(super) fail_bytes_read: bool,
    pub(super) array_push_attempts: usize,
    pub(super) fail_array_push_at: Option<usize>,
    pub(super) fail_validate_handle: Option<u64>,
    pub(super) writable_prepare_attempts: usize,
    pub(super) fail_writable_prepare_at: Option<usize>,
    pub(super) writable_commit_attempts: usize,
    pub(super) fail_writable_commit_at: Option<usize>,
}

impl IntrinsicDispatchHeap {
    pub(super) fn allocate(
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
            .as_handle()
            .map(VmHandle::get)
            .ok_or(VmHeapError::InvalidValueMetadata)
    }

    pub(super) fn owner_count(&self, handle: u64) -> usize {
        self.entries.get(&handle).map_or(0, |entry| entry.owners)
    }

    pub(super) fn debug_inventory(&self) -> Vec<(u64, usize, &'static str)> {
        self.entries
            .iter()
            .map(|(handle, entry)| {
                let kind = match &entry.value {
                    IntrinsicDispatchValue::Opaque => "opaque",
                    IntrinsicDispatchValue::String(_) => "string",
                    IntrinsicDispatchValue::Bytes(_) => "bytes",
                    IntrinsicDispatchValue::Array(_) => "array",
                    IntrinsicDispatchValue::Record(_) => "record",
                };
                (*handle, entry.owners, kind)
            })
            .collect()
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
                IntrinsicDispatchValue::Array(elements) => elements.clone(),
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

    fn child_at(
        &self,
        container: &ValueSlot,
        segment: &PinnedWritablePathSegment,
    ) -> Result<ValueSlot, VmHeapError> {
        let handle = Self::handle(container)?;
        let entry = self
            .entries
            .get(&handle)
            .ok_or(VmHeapError::InvalidValueMetadata)?;
        match (segment, &entry.value) {
            (
                PinnedWritablePathSegment::DenseField { field },
                IntrinsicDispatchValue::Record(fields),
            ) => fields
                .iter()
                .find(|candidate| candidate.name == *field)
                .map(|candidate| candidate.value)
                .ok_or(VmHeapError::InvalidValueMetadata),
            (
                PinnedWritablePathSegment::ArrayIndex { index },
                IntrinsicDispatchValue::Array(items),
            ) => items
                .get(*index)
                .copied()
                .ok_or(VmHeapError::InvalidValueMetadata),
            (PinnedWritablePathSegment::MapKey { .. }, _)
            | (_, IntrinsicDispatchValue::Opaque)
            | (_, IntrinsicDispatchValue::String(_))
            | (_, IntrinsicDispatchValue::Bytes(_))
            | (PinnedWritablePathSegment::DenseField { .. }, IntrinsicDispatchValue::Array(_))
            | (PinnedWritablePathSegment::ArrayIndex { .. }, IntrinsicDispatchValue::Record(_)) => {
                Err(VmHeapError::InvalidValueMetadata)
            }
        }
    }

    fn replace_child(
        &mut self,
        container: &ValueSlot,
        segment: &PinnedWritablePathSegment,
        value: ValueSlot,
    ) -> Result<(), VmHeapError> {
        self.validate_live(&value)?;
        let handle = Self::handle(container)?;
        let old = {
            let entry = self
                .entries
                .get_mut(&handle)
                .ok_or(VmHeapError::InvalidValueMetadata)?;
            match (segment, &mut entry.value) {
                (
                    PinnedWritablePathSegment::DenseField { field },
                    IntrinsicDispatchValue::Record(fields),
                ) => {
                    let destination = fields
                        .iter_mut()
                        .find(|candidate| candidate.name == *field)
                        .ok_or(VmHeapError::InvalidValueMetadata)?;
                    std::mem::replace(&mut destination.value, value)
                }
                (
                    PinnedWritablePathSegment::ArrayIndex { index },
                    IntrinsicDispatchValue::Array(items),
                ) => {
                    let destination = items
                        .get_mut(*index)
                        .ok_or(VmHeapError::InvalidValueMetadata)?;
                    std::mem::replace(destination, value)
                }
                _ => return Err(VmHeapError::InvalidValueMetadata),
            }
        };
        if let Some(old) = old.as_request_heap_ref() {
            self.release_handle(old.get())?;
        }
        Ok(())
    }

    fn clone_container(&mut self, container: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.validate_live(container)?;
        let handle = Self::handle(container)?;
        let value = self
            .entries
            .get(&handle)
            .ok_or(VmHeapError::InvalidValueMetadata)?
            .value
            .clone();
        let value = match value {
            IntrinsicDispatchValue::Array(mut items) => {
                for item in &mut items {
                    if item.as_request_heap_ref().is_some() {
                        *item = self.snapshot_share(item)?;
                    }
                }
                IntrinsicDispatchValue::Array(items)
            }
            IntrinsicDispatchValue::Record(mut fields) => {
                for field in &mut fields {
                    if field.value.as_request_heap_ref().is_some() {
                        field.value = self.snapshot_share(&field.value)?;
                    }
                }
                IntrinsicDispatchValue::Record(fields)
            }
            IntrinsicDispatchValue::Opaque
            | IntrinsicDispatchValue::String(_)
            | IntrinsicDispatchValue::Bytes(_) => return Err(VmHeapError::InvalidValueMetadata),
        };
        let tag = container
            .compact_type_tag()
            .ok_or(VmHeapError::InvalidValueMetadata)?;
        Ok(self.allocate(value, tag, container.flags()))
    }
}

impl VmHeap for IntrinsicDispatchHeap {
    fn validate_live(&self, value: &ValueSlot) -> Result<(), VmHeapError> {
        if value
            .as_handle()
            .is_some_and(|handle| self.fail_validate_handle == Some(handle.get()))
        {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::ValidateLive,
                message: "injected live validation failure".to_string(),
            });
        }
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
                    .as_handle()
                    .is_some_and(|handle| self.entries.contains_key(&handle.get())) =>
            {
                Ok(())
            }
            Some(skiff_runtime_model::vm_value::ValueKind::ResourceRef)
                if value
                    .as_handle()
                    .is_some_and(|handle| self.entries.contains_key(&handle.get())) =>
            {
                Ok(())
            }
            _ => Err(VmHeapError::InvalidValueMetadata),
        }
    }

    fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.validate_live(source)?;
        self.share_attempts += 1;
        if self.fail_share_at == Some(self.share_attempts) {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::SnapshotShare,
                message: "injected snapshot share failure".to_string(),
            });
        }
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
        if self.change_transfer_at == Some(self.transfer_attempts) {
            let handle = source
                .as_handle()
                .ok_or(VmHeapError::InvalidValueMetadata)?;
            let tag = source
                .compact_type_tag()
                .ok_or(VmHeapError::InvalidValueMetadata)?;
            let flags = ValueFlags::new(source.flags().bits() ^ 1);
            return match source.kind() {
                Some(skiff_runtime_model::vm_value::ValueKind::RequestHeapRef) => {
                    Ok(ValueSlot::request_heap_ref(handle, tag, flags))
                }
                Some(skiff_runtime_model::vm_value::ValueKind::ResourceRef) => {
                    Ok(ValueSlot::resource_ref(handle, tag, flags))
                }
                _ => Err(VmHeapError::InvalidValueMetadata),
            };
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
        self.release_history.push(handle);
        self.release_handle(handle)
    }

    fn release_resource(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
        self.release_attempts += 1;
        if self.fail_release_at == Some(self.release_attempts) {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::ReleaseResource,
                message: "injected resource release failure".to_string(),
            });
        }
        let handle = Self::handle(owner)?;
        self.release_history.push(handle);
        self.release_handle(handle)
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
        self.typed_bytes_allocations += 1;
        if self.fail_typed_bytes_at == Some(self.typed_bytes_allocations) {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::AllocateRepresentation,
                message: "injected intrinsic bytes allocation failure".to_string(),
            });
        }
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

    fn allocate_array(
        &mut self,
        elements: &[ValueSlot],
        tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        for element in elements {
            self.validate_live(element)?;
        }
        Ok(self.allocate(IntrinsicDispatchValue::Array(elements.to_vec()), tag, flags))
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
            | IntrinsicDispatchValue::Array(_)
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
            | IntrinsicDispatchValue::Array(_)
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

    fn array_push_owned(&mut self, array: &ValueSlot, item: ValueSlot) -> Result<(), VmHeapError> {
        self.array_push_attempts += 1;
        self.validate_live(array)?;
        self.validate_live(&item)?;
        if self.fail_array_push_at == Some(self.array_push_attempts) {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::ArrayPushOwned,
                message: "injected owned array push failure".to_string(),
            });
        }
        let handle = Self::handle(array)?;
        let IntrinsicDispatchValue::Array(items) = &mut self
            .entries
            .get_mut(&handle)
            .ok_or(VmHeapError::InvalidValueMetadata)?
            .value
        else {
            return Err(VmHeapError::InvalidValueMetadata);
        };
        items.push(item);
        Ok(())
    }

    fn array_get(&self, array: &ValueSlot, index: usize) -> Result<ValueSlot, VmHeapError> {
        self.validate_live(array)?;
        let handle = Self::handle(array)?;
        let IntrinsicDispatchValue::Array(items) = &self
            .entries
            .get(&handle)
            .ok_or(VmHeapError::InvalidValueMetadata)?
            .value
        else {
            return Err(VmHeapError::InvalidValueMetadata);
        };
        items
            .get(index)
            .copied()
            .ok_or(VmHeapError::InvalidValueMetadata)
    }

    fn array_len(&self, array: &ValueSlot) -> Result<usize, VmHeapError> {
        self.validate_live(array)?;
        let handle = Self::handle(array)?;
        let IntrinsicDispatchValue::Array(items) = &self
            .entries
            .get(&handle)
            .ok_or(VmHeapError::InvalidValueMetadata)?
            .value
        else {
            return Err(VmHeapError::InvalidValueMetadata);
        };
        Ok(items.len())
    }

    fn prepare_writable_path(
        &mut self,
        root: &ValueSlot,
        segments: &[VmHeapPathSegment],
        selectors: &[ValueSlot],
    ) -> Result<WritablePathPreparation, VmHeapError> {
        self.writable_prepare_attempts += 1;
        if self.fail_writable_prepare_at == Some(self.writable_prepare_attempts) {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::PrepareWritablePath,
                message: "injected writable path preparation failure".to_string(),
            });
        }
        self.validate_live(root)?;
        let mut selector = 0usize;
        let mut current = *root;
        let mut pinned = Vec::with_capacity(segments.len());
        let mut containers = Vec::with_capacity(segments.len());
        for segment in segments {
            containers.push(current);
            let resolved = match segment {
                VmHeapPathSegment::DenseField { field } => PinnedWritablePathSegment::DenseField {
                    field: field.clone(),
                },
                VmHeapPathSegment::ArrayIndex => {
                    let value = selectors
                        .get(selector)
                        .ok_or(VmHeapError::InvalidValueMetadata)?;
                    selector += 1;
                    PinnedWritablePathSegment::ArrayIndex {
                        index: skiff_runtime_model::vm_heap::collection_index(value)
                            .ok_or(VmHeapError::InvalidValueMetadata)?,
                    }
                }
                VmHeapPathSegment::MapKey => return Err(VmHeapError::InvalidValueMetadata),
            };
            current = self.child_at(&current, &resolved)?;
            pinned.push(resolved);
        }
        if selector != selectors.len() {
            return Err(VmHeapError::InvalidValueMetadata);
        }
        WritablePathPreparation::new(
            *root,
            pinned.into_boxed_slice(),
            containers.into_boxed_slice(),
            Some(current),
        )
    }

    fn commit_writable_path(
        &mut self,
        prepared: WritablePathPreparation,
        value: ValueSlot,
    ) -> Result<ValueSlot, VmHeapError> {
        self.writable_commit_attempts += 1;
        if self.fail_writable_commit_at == Some(self.writable_commit_attempts) {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::CommitWritablePath,
                message: "injected writable path commit failure".to_string(),
            });
        }
        self.validate_live(&prepared.root())?;
        self.validate_live(&value)?;
        let exclusive = prepared.containers().iter().all(|container| {
            Self::handle(container)
                .ok()
                .and_then(|handle| self.entries.get(&handle))
                .is_some_and(|entry| entry.owners == 1)
        });
        if exclusive {
            let container = prepared
                .containers()
                .last()
                .copied()
                .ok_or(VmHeapError::InvalidValueMetadata)?;
            let segment = prepared
                .segments()
                .last()
                .ok_or(VmHeapError::InvalidValueMetadata)?;
            self.replace_child(&container, segment, value)?;
            return Ok(prepared.root());
        }

        // The admitted Phase 1-5 mutation fixtures exercise a one-segment
        // copy-on-write path. Keep the fake heap deliberately narrow so a
        // test cannot accidentally claim broader VM heap semantics.
        if prepared.segments().len() != 1 {
            return Err(VmHeapError::InvalidValueMetadata);
        }
        let replacement = self.clone_container(&prepared.root())?;
        self.replace_child(&replacement, &prepared.segments()[0], value)?;
        Ok(replacement)
    }
}

#[derive(Default)]
pub(super) struct IntrinsicRootHandles(pub(super) Vec<u64>);

impl VmRootVisitor for IntrinsicRootHandles {
    fn visit_root(&mut self, root: &ValueSlot) -> Result<(), VmHeapError> {
        if let Some(handle) = root.as_handle() {
            self.0.push(handle.get());
        }
        Ok(())
    }
}

pub(super) fn current_intrinsic(fiber: &VmFiber) -> Option<(String, TypeIndex)> {
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

pub(super) fn drive_to_opcode_occurrence(
    fiber: &mut VmFiber,
    heap: &mut IntrinsicDispatchHeap,
    target: Opcode,
    occurrence: usize,
) {
    let mut seen = 0;
    for _ in 0..10_000 {
        if current_opcode(fiber) == Some(target) {
            seen += 1;
            if seen == occurrence {
                return;
            }
        }
        assert!(matches!(
            fiber.dispatch_one(heap).expect("drive to target opcode"),
            DispatchOutcome::Continue
        ));
    }
    panic!("fixture did not reach {target:?} occurrence {occurrence} within the step cap");
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

pub(super) fn intrinsic_fiber(heap: &mut IntrinsicDispatchHeap) -> VmFiber {
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

pub(super) fn drive_intrinsic_fiber_to_completion(
    fiber: &mut VmFiber,
    heap: &mut IntrinsicDispatchHeap,
) {
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

pub(super) fn drive_to_owned_concat(fiber: &mut VmFiber, heap: &mut IntrinsicDispatchHeap) {
    let mut concat_count = 0;
    for _ in 0..10_000 {
        if current_intrinsic(fiber).is_some_and(|(key, _)| {
            if key == "receiver:string.concat@1" {
                concat_count += 1;
            }
            key == "receiver:string.concat@1" && concat_count == 3
        }) {
            return;
        }
        assert!(matches!(
            fiber.dispatch_one(heap).expect("drive to second concat"),
            DispatchOutcome::Continue
        ));
    }
    panic!("intrinsic fixture did not reach its two-owner concat within the step cap");
}

pub(super) fn drive_to_intrinsic_key(
    fiber: &mut VmFiber,
    heap: &mut IntrinsicDispatchHeap,
    expected_key: &str,
) {
    for _ in 0..10_000 {
        if current_intrinsic(fiber).is_some_and(|(key, _)| key == expected_key) {
            return;
        }
        assert!(matches!(
            fiber.dispatch_one(heap).expect("drive to intrinsic key"),
            DispatchOutcome::Continue
        ));
    }
    panic!("intrinsic fixture did not reach {expected_key}");
}

pub(super) fn drive_to_root_return(fiber: &mut VmFiber, heap: &mut IntrinsicDispatchHeap) {
    for _ in 0..10_000 {
        if fiber.frames.len() == 1 && current_opcode(fiber) == Some(Opcode::Return) {
            return;
        }
        match fiber.dispatch_one(heap).expect("drive to root return") {
            DispatchOutcome::Continue => {}
            DispatchOutcome::Handoff(VmControl::EmitStream(item)) => {
                let resume = item.release(heap).expect("release emitted chunk owner");
                fiber
                    .resume(resume, ResumeOutcome::Empty)
                    .expect("resume after emitted chunk");
            }
            DispatchOutcome::Complete(_) => panic!("fixture completed before its root Return"),
            DispatchOutcome::Handoff(_) => panic!("fixture exposes only EmitStream handoff"),
            DispatchOutcome::Throw(_) => panic!("fixture must not throw"),
        }
    }
    panic!("fixture did not reach its root Return within the step cap");
}

pub(super) fn heap_owner_total(heap: &IntrinsicDispatchHeap) -> usize {
    heap.entries.values().map(|entry| entry.owners).sum()
}

pub(super) fn live_string_owner(fiber: &VmFiber, heap: &IntrinsicDispatchHeap) -> ValueSlot {
    fiber
        .values
        .iter()
        .copied()
        .zip(fiber.live_values.iter().copied())
        .find_map(|(value, live)| {
            let handle = value.as_request_heap_ref()?;
            let entry = heap.entries.get(&handle.get())?;
            (live && matches!(&entry.value, IntrinsicDispatchValue::String(_))).then_some(value)
        })
        .expect("the intrinsic fixture keeps a request-owned string live")
}

fn next_intrinsic_stream_item(
    fiber: &mut VmFiber,
    heap: &mut IntrinsicDispatchHeap,
) -> crate::StreamItem {
    loop {
        match fiber
            .dispatch_one(heap)
            .expect("drive intrinsic fixture to EmitStream")
        {
            DispatchOutcome::Continue => {}
            DispatchOutcome::Handoff(VmControl::EmitStream(item)) => return item,
            DispatchOutcome::Handoff(_) => {
                panic!("intrinsic fixture has only EmitStream handoff")
            }
            DispatchOutcome::Complete(_) => panic!("fixture completed before EmitStream"),
            DispatchOutcome::Throw(_) => panic!("intrinsic fixture must not throw"),
        }
    }
}

#[test]
fn stream_item_release_failure_returns_the_rooted_carrier_for_exact_retry() {
    let mut heap = IntrinsicDispatchHeap::default();
    let mut fiber = intrinsic_fiber(&mut heap);
    let item = next_intrinsic_stream_item(&mut fiber, &mut heap);
    let [root] = item.item().values() else {
        panic!("EmitStream transfers exactly one item owner")
    };
    let handle = root.as_request_heap_ref().unwrap().get();
    heap.fail_release_at = Some(heap.release_attempts + 1);

    let failure = item
        .release(&mut heap)
        .expect_err("the injected first release must return its carrier");

    assert!(matches!(
        failure.error(),
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::ReleaseSnapshot,
            ..
        })
    ));
    assert_eq!(heap.owner_count(handle), 1);
    let mut roots = IntrinsicRootHandles::default();
    failure.visit_roots(&mut roots).unwrap();
    assert_eq!(roots.0, [handle]);

    let attempts_after_failure = heap.release_attempts;
    let (item, _error) = failure.into_parts();
    heap.fail_release_at = None;
    let resume = item
        .release(&mut heap)
        .expect("the unchanged carrier can retry on the same heap");
    assert_eq!(heap.owner_count(handle), 0);
    assert_eq!(heap.release_attempts, attempts_after_failure + 1);
    assert_eq!(
        heap.release_history
            .iter()
            .filter(|released| **released == handle)
            .count(),
        1
    );
    fiber
        .resume(resume, ResumeOutcome::Empty)
        .expect("the same continuation resumes after release retry");
    drive_intrinsic_fiber_to_completion(&mut fiber, &mut heap);
    assert!(heap.entries.is_empty());
    assert_eq!(
        heap.release_history
            .iter()
            .filter(|released| **released == handle)
            .count(),
        1,
        "the retried item owner must never be released twice"
    );
}

#[test]
fn stream_item_release_failure_moves_exact_owner_into_terminal_escrow() {
    let mut heap = IntrinsicDispatchHeap::default();
    let mut fiber = intrinsic_fiber(&mut heap);
    let item = next_intrinsic_stream_item(&mut fiber, &mut heap);
    let [root] = item.item().values() else {
        panic!("EmitStream transfers exactly one item owner")
    };
    let handle = root.as_request_heap_ref().unwrap().get();
    heap.fail_release_at = Some(heap.release_attempts + 1);

    let failure = item
        .release(&mut heap)
        .expect_err("the injected release must return its rooted carrier");
    let mut carrier_roots = IntrinsicRootHandles::default();
    failure.visit_roots(&mut carrier_roots).unwrap();
    assert_eq!(carrier_roots.0, [handle]);
    assert_eq!(heap.owner_count(handle), 1);

    let attempts_after_failure = heap.release_attempts;
    let (mut escrow, error) = failure.into_terminal_escrow();
    assert!(matches!(
        error,
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::ReleaseSnapshot,
            ..
        })
    ));
    assert_eq!(escrow.root_count(), 1);
    assert_eq!(escrow.unresolved_count(), 0, "the exact plan must survive");
    let mut escrow_roots = IntrinsicRootHandles::default();
    escrow.visit_roots(&mut escrow_roots).unwrap();
    assert_eq!(escrow_roots.0, [handle]);
    assert_eq!(
        heap.debug_inventory()
            .iter()
            .filter(|(entry, _, _)| *entry == handle)
            .map(|(_, owners, _)| *owners)
            .collect::<Vec<_>>(),
        [1]
    );

    heap.fail_release_at = None;
    escrow
        .release_all(&mut heap)
        .expect("terminal escrow releases through the captured exact plan");
    assert!(escrow.is_empty());
    assert_eq!(escrow.root_count(), 0);
    assert_eq!(heap.owner_count(handle), 0);
    assert!(heap
        .debug_inventory()
        .iter()
        .all(|(entry, _, _)| *entry != handle));
    let mut released_roots = IntrinsicRootHandles::default();
    escrow.visit_roots(&mut released_roots).unwrap();
    assert!(released_roots.0.is_empty());
    assert_eq!(heap.release_attempts, attempts_after_failure + 1);
    assert_eq!(
        heap.release_history
            .iter()
            .filter(|released| **released == handle)
            .count(),
        1
    );

    let attempts_after_release = heap.release_attempts;
    escrow
        .release_all(&mut heap)
        .expect("an empty terminal escrow is idempotent");
    assert_eq!(heap.release_attempts, attempts_after_release);
    assert_eq!(
        heap.release_history
            .iter()
            .filter(|released| **released == handle)
            .count(),
        1,
        "terminal cleanup must never release the consumed owner twice"
    );
}

#[test]
fn host_arguments_release_failure_returns_unreleased_suffix_escrow() {
    let mut heap = IntrinsicDispatchHeap::default();
    let image = Arc::clone(&intrinsic_dispatch_fixture().image);
    let plan = intrinsic_snapshot_plan();
    let first = heap.allocate(
        IntrinsicDispatchValue::Opaque,
        compact_tag(1),
        ValueFlags::new(0),
    );
    let second = heap.allocate(
        IntrinsicDispatchValue::Opaque,
        compact_tag(2),
        ValueFlags::new(0),
    );
    let values = crate::VmOwnedValues::new_exact(
        image,
        Box::new([first, second]),
        Box::new([plan.clone(), plan.clone()]),
    );
    let arguments = crate::VmHostEffectArguments::new(
        values,
        Box::new([plan.clone(), plan]),
        FunctionIndex::new(0),
        InstructionIndex::new(0),
    );
    heap.fail_release_at = Some(heap.release_attempts + 2);

    let failure = arguments
        .release(&mut heap)
        .expect_err("the injected second release must return its suffix carrier");
    assert_eq!(
        heap.owner_count(first.as_request_heap_ref().unwrap().get()),
        0
    );
    assert_eq!(
        heap.owner_count(second.as_request_heap_ref().unwrap().get()),
        1
    );
    let mut roots = IntrinsicRootHandles::default();
    failure.visit_roots(&mut roots).unwrap();
    assert_eq!(roots.0, [second.as_request_heap_ref().unwrap().get()]);

    let (error, mut escrow) = failure.into_terminal_escrow();
    assert!(matches!(
        error,
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::ReleaseSnapshot,
            ..
        })
    ));
    assert_eq!(escrow.root_count(), 1);
    heap.fail_release_at = None;
    escrow
        .release_all(&mut heap)
        .expect("the retained suffix releases through the captured exact plans");
    assert_eq!(
        heap.owner_count(second.as_request_heap_ref().unwrap().get()),
        0
    );
    assert!(escrow.is_empty());
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
    assert_eq!(seen.get("receiver:string.concat@1"), Some(&4));
    assert_eq!(seen.get("core.bytes.fromUtf8"), Some(&2));
    assert_eq!(seen.get("receiver:bytes.toUtf8String@1"), Some(&1));
    assert!(
        heap.entries.is_empty(),
        "frame exit releases every result owner"
    );
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
