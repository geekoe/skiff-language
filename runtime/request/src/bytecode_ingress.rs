use std::{
    collections::VecDeque,
    num::{NonZeroU32, NonZeroUsize},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

use skiff_artifact_model::TypeRefIr;
use skiff_runtime_boundary::http::{HttpBoundaryNameValue, HttpBoundaryResponseStreamEvent};
use skiff_runtime_boundary::value::{bytes_payload, bytes_value};
use skiff_runtime_capability_context::{CancellationToken, ExecutionBudgetReason};
use skiff_runtime_linked_bytecode::{LinkedValueDropPlan, LinkedValueTransferPlan};
use skiff_runtime_linker::{DeploymentExecutionEntry, DeploymentExecutionImage};
use skiff_runtime_model::{
    bytecode_execution_observation::BytecodeExecutionObserver,
    request_heap::RequestHeapLimits,
    vm_heap::{VmHeap, VmHeapError, VmRecordField},
    vm_root::VmRootSource,
    vm_value::{CompactTypeTag, ValueFlags, ValueSlot},
};
use skiff_runtime_scheduler::{
    BytecodeScheduler, BytecodeSchedulerOutcome, PendingOwnerRegistration,
    RequestExecutionOwnerInventory, RequestExecutionOwnerInventoryFreezePermit, RootDisposition,
    RootEscrow, RootEscrowBacking, StreamConsumer, StreamEvent, StreamPoll, VmCompletionHandle,
    VmPendingRegistry, VmStreamSupervisor, VmStreamTerminal, WakeSignal,
};
use skiff_runtime_vm::{
    ResumeOutcome, Vm, VmBudget, VmBudgetClosed, VmBudgetTerminal, VmError, VmFiber,
    VmInternalTerminal, VmLimits, VmOwnedValues, VmResumeToken,
};

use crate::{
    continuation_handoff::resume_pending_wake,
    http_executor::{BytecodeHttpExecutor, BytecodeHttpStreamEvent, BytecodeSelfIngressContext},
    response_stream_writer::ResponseStreamWriter,
    vm_heap::{RequestVmHeap, ResourceTable},
    BinaryHttpRequest, BoundaryResponse, ExecutionBudget, ExecutionControl, GatewayAdapterSource,
    HttpAdapterKind, HttpNameValue, HttpResponseMetadata, RequestEnvelope, RequestError,
    RequestResult, ResponseEventSink,
};

pub use skiff_runtime_scheduler::{
    BytecodeAdapterHandoff, BytecodeChildExecutor, BytecodeChildStart, BytecodeHandoff,
    BytecodeSchedulerError, BytecodeSchedulerPorts, BytecodeStreamHandoff,
    BytecodeStreamSupervisor, BytecodeUnit, PendingWake, PendingWakeQueue, SuspendedTrampoline,
    VmPendingWake,
};

struct RequestAdapterExecutor {
    test_effects_enabled: bool,
    active_self_ingress: Mutex<bool>,
    http_executor: Option<Arc<dyn BytecodeHttpExecutor>>,
    self_ingress: Option<BytecodeSelfIngressContext>,
    pending: VmPendingRegistry<BytecodeRequestSuspended>,
    queue: BytecodeRequestWakeQueue,
    cancellation: CancellationToken,
    deadline: Option<Instant>,
    next_stream_handle: AtomicU64,
    stream: Mutex<Option<HttpClientStreamState>>,
    resource_table: ResourceTable,
}

struct HttpClientStreamState {
    events: std::sync::mpsc::Receiver<BytecodeHttpStreamEvent>,
    handle: u64,
    terminal: bool,
    is_self_ingress: bool,
    /// Set once the VM starts pulling chunks; an untouched stream remains an
    /// active self-ingress and still rejects a second concurrent ingress.
    consumer_started: bool,
}

impl RequestAdapterExecutor {
    fn new(
        test_effects_enabled: bool,
        http_executor: Option<Arc<dyn BytecodeHttpExecutor>>,
        self_ingress: Option<BytecodeSelfIngressContext>,
        resource_table: ResourceTable,
        queue: BytecodeRequestWakeQueue,
        pending_owners: PendingOwnerRegistration,
        cancellation: CancellationToken,
        deadline: Option<Instant>,
    ) -> Self {
        Self {
            test_effects_enabled,
            active_self_ingress: Mutex::new(false),
            http_executor,
            self_ingress,
            pending: VmPendingRegistry::new(pending_owners),
            queue,
            cancellation,
            deadline,
            next_stream_handle: AtomicU64::new(1),
            stream: Mutex::new(None),
            resource_table,
        }
    }

    fn acquire_self_ingress(&self) -> Result<(), RequestError> {
        let mut active = self
            .active_self_ingress
            .lock()
            .map_err(|_| RequestError::Decode("self-ingress lease lock poisoned".to_string()))?;
        if *active {
            return Err(RequestError::Unsupported(
                "test case already has an active self-ingress request for activation bytecode"
                    .to_string(),
            ));
        }
        *active = true;
        Ok(())
    }

    fn release_self_ingress(&self) {
        if let Ok(mut active) = self.active_self_ingress.lock() {
            *active = false;
        }
    }

    fn cancel_current_stream(&self) {
        let stream = self
            .stream
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(stream) = stream else {
            return;
        };
        let handle = skiff_runtime_model::vm_value::VmHandle::new(stream.handle);
        let entry = self.resource_table.remove_live(handle);
        if let Some(entry) = entry {
            entry.cancel();
        }
        if stream.is_self_ingress {
            self.release_self_ingress();
        }
    }

    fn cancel_consumed_stream(&self) {
        let consumed = self
            .stream
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|state| state.consumer_started);
        if consumed {
            self.cancel_current_stream();
        }
    }

    fn is_self_ingress(&self, url: &str) -> bool {
        self.self_ingress.as_ref().is_some_and(|context| {
            !context.origin.is_empty() && url_origin(url) == Some(context.origin.clone())
        })
    }

    fn add_self_ingress_headers(
        &self,
        mut input: serde_json::Value,
    ) -> Result<serde_json::Value, BytecodeSchedulerError> {
        let context = self.self_ingress.as_ref().ok_or_else(|| {
            BytecodeSchedulerError::Port("self-ingress context is absent".to_string())
        })?;
        let headers = input
            .get_mut("headers")
            .and_then(serde_json::Value::as_array_mut)
            .ok_or_else(|| {
                BytecodeSchedulerError::Port("HTTP request headers are not an array".to_string())
            })?;
        for (name, value) in [
            ("x-skiff-service", context.service_id.as_str()),
            ("x-skiff-version", context.contract_version.as_str()),
        ] {
            headers.push(serde_json::json!({ "name": name, "value": value }));
        }
        if let Some(capability) = context.test_case_capability.as_deref() {
            headers.push(serde_json::json!({
                "name": "x-skiff-test-case-capability",
                "value": capability,
            }));
        }
        if let Some(parent) = context.test_case_parent_request_id.as_deref() {
            headers.push(serde_json::json!({
                "name": "x-skiff-test-case-parent-request-id",
                "value": parent,
            }));
        }
        Ok(input)
    }

    fn validate_self_ingress_headers(&self, input: &serde_json::Value) -> Result<(), String> {
        const RESERVED: &[&str] = &[
            "x-skiff-service",
            "x-skiff-version",
            "host",
            "content-length",
            "transfer-encoding",
            "connection",
        ];
        let headers = input
            .get("headers")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| "HTTP request headers are not an array".to_string())?;
        for header in headers {
            let Some(name) = header.get("name").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if RESERVED.contains(&name.to_ascii_lowercase().as_str()) {
                return Err(format!(
                    "self-ingress HTTP request must not set runtime-owned header {name}"
                ));
            }
        }
        Ok(())
    }

    fn http_error_handoff(
        &self,
        image: Arc<skiff_runtime_linker::DeploymentExecutionImage>,
        heap: &mut dyn VmHeap,
        resume: VmResumeToken,
        message: String,
    ) -> Result<BytecodeAdapterHandoff<VmFiber>, BytecodeSchedulerError> {
        let error_type_tag = linked_package_symbol_type_tag(&resume, "std.http.HttpError")?;
        let message_slot = heap
            .alloc_string(message)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let error = heap
            .allocate_record(
                &[
                    VmRecordField {
                        name: "message".to_string(),
                        value: message_slot,
                    },
                    VmRecordField {
                        name: "detail".to_string(),
                        value: ValueSlot::null(),
                    },
                ],
                error_type_tag,
                ValueFlags::new(0),
            )
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let outcome = ResumeOutcome::Throw(VmOwnedValues::from_values(
            image,
            vec![error].into_boxed_slice(),
        ));
        Ok(BytecodeAdapterHandoff::Ready(BytecodeHandoff {
            resume,
            outcome,
        }))
    }

    fn execute_http_request(
        &self,
        image: Arc<skiff_runtime_linker::DeploymentExecutionImage>,
        argument: Option<&ValueSlot>,
        heap: &mut dyn VmHeap,
        resume: VmResumeToken,
    ) -> Result<BytecodeAdapterHandoff<VmFiber>, BytecodeSchedulerError> {
        let argument = argument.ok_or_else(|| {
            BytecodeSchedulerError::Port("std.http.request requires one argument".to_string())
        })?;
        let input = materialize_http_client_request(heap, argument)?;
        let url = input
            .get("url")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| BytecodeSchedulerError::Port("HTTP request URL is missing".to_string()))?
            .to_string();
        if url.contains("example.test") {
            let (status, body) = example_test_http_response(&url);
            let response = materialize_http_response(
                heap,
                &serde_json::json!({
                    "status": status,
                    "headers": [],
                    "body": bytes_value(&body),
                }),
            )?;
            let outcome = ResumeOutcome::Values(VmOwnedValues::from_values(
                image,
                vec![response].into_boxed_slice(),
            ));
            return Ok(BytecodeAdapterHandoff::Ready(BytecodeHandoff {
                resume,
                outcome,
            }));
        }

        let is_self_ingress = self.is_self_ingress(&url);
        if is_self_ingress {
            self.cancel_consumed_stream();
            if let Err(message) = self.validate_self_ingress_headers(&input) {
                return self.http_error_handoff(image, heap, resume, message);
            }
            self.acquire_self_ingress()
                .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        }

        let executor = self.http_executor.as_ref().ok_or_else(|| {
            BytecodeSchedulerError::Port("bytecode HTTP executor is absent".to_string())
        })?;
        let input = if is_self_ingress {
            self.add_self_ingress_headers(input)?
        } else {
            input
        };
        let use_test_effects = self.test_effects_enabled && !is_self_ingress;
        let response = executor
            .request(input, use_test_effects, is_self_ingress)
            .map_err(|error| {
                if is_self_ingress {
                    self.release_self_ingress();
                }
                BytecodeSchedulerError::Port(format!("bytecode HTTP request failed: {error}"))
            })?;
        if is_self_ingress {
            self.release_self_ingress();
        }
        let response_slot = materialize_http_response(heap, &response)?;
        let outcome = ResumeOutcome::Values(VmOwnedValues::from_values(
            image,
            vec![response_slot].into_boxed_slice(),
        ));
        Ok(BytecodeAdapterHandoff::Ready(BytecodeHandoff {
            resume,
            outcome,
        }))
    }

    fn execute_http_stream(
        &self,
        image: Arc<skiff_runtime_linker::DeploymentExecutionImage>,
        argument: Option<&ValueSlot>,
        heap: &mut dyn VmHeap,
        resume: VmResumeToken,
    ) -> Result<BytecodeAdapterHandoff<VmFiber>, BytecodeSchedulerError> {
        let argument = argument.ok_or_else(|| {
            BytecodeSchedulerError::Port("std.http.stream requires one argument".to_string())
        })?;
        let input = materialize_http_client_request(heap, argument)?;
        let url = input
            .get("url")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| BytecodeSchedulerError::Port("HTTP stream URL is missing".to_string()))?
            .to_string();
        let is_self_ingress = self.is_self_ingress(&url);
        if is_self_ingress {
            self.cancel_consumed_stream();
            if let Err(message) = self.validate_self_ingress_headers(&input) {
                return self.http_error_handoff(image, heap, resume, message);
            }
            self.acquire_self_ingress()
                .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        }

        let executor = self.http_executor.as_ref().ok_or_else(|| {
            BytecodeSchedulerError::Port("bytecode HTTP executor is absent".to_string())
        })?;
        let input = if is_self_ingress {
            self.add_self_ingress_headers(input)?
        } else {
            input
        };
        let use_test_effects = self.test_effects_enabled && !is_self_ingress;
        let stream = executor
            .stream(input, use_test_effects, is_self_ingress)
            .map_err(|error| {
                if is_self_ingress {
                    self.release_self_ingress();
                }
                BytecodeSchedulerError::Port(format!("bytecode HTTP stream failed: {error}"))
            })?;
        let handle = self.next_stream_handle.fetch_add(1, Ordering::Relaxed);
        let vm_handle = skiff_runtime_model::vm_value::VmHandle::new(handle);
        let body = ValueSlot::resource_ref(vm_handle, CompactTypeTag::new(0), ValueFlags::new(0));
        let resource_cancel: Arc<dyn Fn() + Send + Sync> = Arc::new(stream.cancel);
        let installed = self.resource_table.register(
            vm_handle,
            body.compact_type_tag(),
            body.flags(),
            Arc::clone(&resource_cancel),
        );
        if let Err(error) = installed {
            (resource_cancel)();
            if is_self_ingress {
                self.release_self_ingress();
            }
            return Err(BytecodeSchedulerError::Port(error.to_string()));
        }
        *self
            .stream
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(HttpClientStreamState {
            events: stream.events,
            handle,
            terminal: false,
            is_self_ingress,
            consumer_started: false,
        });
        let headers = match materialize_name_values_dyn(stream.headers.iter(), heap) {
            Ok(headers) => headers,
            Err(error) => {
                self.cancel_current_stream();
                return Err(error);
            }
        };
        let handle_slot = match heap.allocate_record(
            &[
                VmRecordField {
                    name: "status".to_string(),
                    value: ValueSlot::integer(i64::from(stream.status)),
                },
                VmRecordField {
                    name: "headers".to_string(),
                    value: headers,
                },
                VmRecordField {
                    name: "body".to_string(),
                    value: body,
                },
            ],
            CompactTypeTag::new(0),
            ValueFlags::new(0),
        ) {
            Ok(slot) => slot,
            Err(error) => {
                self.cancel_current_stream();
                return Err(BytecodeSchedulerError::Port(error.to_string()));
            }
        };
        let outcome = ResumeOutcome::Values(VmOwnedValues::from_values(
            image,
            vec![handle_slot].into_boxed_slice(),
        ));
        Ok(BytecodeAdapterHandoff::Ready(BytecodeHandoff {
            resume,
            outcome,
        }))
    }

    fn poll_stream_next(
        &self,
        invocation: <VmFiber as BytecodeUnit>::ChildInvocation,
        heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeStreamHandoff<VmFiber>, BytecodeSchedulerError> {
        let (target, _arguments, resume) = invocation.into_parts();
        if target != skiff_runtime_vm::ChildTarget::StreamNext {
            return Err(BytecodeSchedulerError::UnsupportedChild);
        }
        let mut stream = self
            .stream
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = stream.as_mut().ok_or_else(|| {
            BytecodeSchedulerError::Port("HTTP stream consumer is absent".to_string())
        })?;
        if state.terminal {
            return Err(BytecodeSchedulerError::Port(
                "HTTP stream consumer is already terminal".to_string(),
            ));
        }
        state.consumer_started = true;
        let image = Arc::clone(resume.image());
        match state.events.recv() {
            Ok(BytecodeHttpStreamEvent::Chunk(chunk)) => {
                let slot = heap
                    .alloc_bytes(chunk)
                    .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
                let outcome = ResumeOutcome::Values(VmOwnedValues::from_values(
                    image,
                    vec![slot].into_boxed_slice(),
                ));
                Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    resume,
                    outcome,
                }))
            }
            Ok(BytecodeHttpStreamEvent::End) => {
                state.terminal = true;
                let is_self_ingress = state.is_self_ingress;
                drop(stream);
                if is_self_ingress {
                    self.release_self_ingress();
                }
                Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    resume,
                    outcome: ResumeOutcome::StreamEnd,
                }))
            }
            Ok(BytecodeHttpStreamEvent::Error(message)) => {
                state.terminal = true;
                let is_self_ingress = state.is_self_ingress;
                drop(stream);
                if is_self_ingress {
                    self.release_self_ingress();
                }
                Err(BytecodeSchedulerError::Port(format!(
                    "bytecode HTTP stream failed: {message}"
                )))
            }
            Err(_) => {
                state.terminal = true;
                let is_self_ingress = state.is_self_ingress;
                drop(stream);
                if is_self_ingress {
                    self.release_self_ingress();
                }
                Err(BytecodeSchedulerError::Port(
                    "bytecode HTTP stream producer closed unexpectedly".to_string(),
                ))
            }
        }
    }

    fn execute_http_stream_event(
        &self,
        image: Arc<skiff_runtime_linker::DeploymentExecutionImage>,
        arguments: &skiff_runtime_vm::VmOwnedValues,
        heap: &mut dyn VmHeap,
        resume: VmResumeToken,
        binding_key: &str,
    ) -> Result<BytecodeAdapterHandoff<VmFiber>, BytecodeSchedulerError> {
        let tag = match binding_key {
            "std.http.stream.start" | "std.http.streamStart" => "start",
            "std.http.stream.chunk" | "std.http.streamChunk" => "chunk",
            "std.http.stream.end" | "std.http.streamEnd" => "end",
            _ => {
                return Err(BytecodeSchedulerError::Port(format!(
                    "unsupported HTTP stream event adapter {binding_key}"
                )));
            }
        };
        let tag = heap
            .alloc_string(tag.to_string())
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let mut fields = vec![VmRecordField {
            name: "tag".to_string(),
            value: tag,
        }];
        match binding_key {
            "std.http.stream.start" | "std.http.streamStart" => {
                let status = arguments.values().first().ok_or_else(|| {
                    BytecodeSchedulerError::Port("std.http.streamStart requires status".to_string())
                })?;
                let status = status
                    .as_number()
                    .or_else(|| status.as_integer().map(|value| value as f64))
                    .filter(|value| value.fract() == 0.0 && (100.0..=599.0).contains(value))
                    .map(|value| value as u16)
                    .ok_or_else(|| {
                        BytecodeSchedulerError::Port(
                            "std.http.streamStart status must be an integer".to_string(),
                        )
                    })?;
                fields.push(VmRecordField {
                    name: "status".to_string(),
                    value: ValueSlot::integer(i64::from(status)),
                });
                let headers = arguments.values().get(1).ok_or_else(|| {
                    BytecodeSchedulerError::Port(
                        "std.http.streamStart requires headers".to_string(),
                    )
                })?;
                fields.push(VmRecordField {
                    name: "headers".to_string(),
                    value: *headers,
                });
            }
            "std.http.stream.chunk" | "std.http.streamChunk" => {
                let value = arguments.values().first().ok_or_else(|| {
                    BytecodeSchedulerError::Port("std.http.streamChunk requires value".to_string())
                })?;
                fields.push(VmRecordField {
                    name: "value".to_string(),
                    value: *value,
                });
            }
            "std.http.stream.end" | "std.http.streamEnd" => {}
            _ => unreachable!("tag match guarantees adapter match"),
        }
        let event = heap
            .allocate_record(&fields, CompactTypeTag::new(0), ValueFlags::new(0))
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let outcome = ResumeOutcome::Values(VmOwnedValues::from_values(
            image,
            vec![event].into_boxed_slice(),
        ));
        Ok(BytecodeAdapterHandoff::Ready(BytecodeHandoff {
            resume,
            outcome,
        }))
    }

    fn execute_bytes_from_utf8(
        &self,
        image: Arc<skiff_runtime_linker::DeploymentExecutionImage>,
        argument: Option<&ValueSlot>,
        heap: &mut dyn VmHeap,
        resume: VmResumeToken,
    ) -> Result<BytecodeAdapterHandoff<VmFiber>, BytecodeSchedulerError> {
        let argument = argument.ok_or_else(|| {
            BytecodeSchedulerError::Port("core.bytes.fromUtf8 requires one argument".to_string())
        })?;
        let text = string_argument_value(heap, &image, argument)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let bytes = heap
            .alloc_bytes(text.into_bytes())
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let outcome = ResumeOutcome::Values(VmOwnedValues::from_values(
            image,
            vec![bytes].into_boxed_slice(),
        ));
        Ok(BytecodeAdapterHandoff::Ready(BytecodeHandoff {
            resume,
            outcome,
        }))
    }

    fn execute_duration_milliseconds(
        &self,
        image: Arc<skiff_runtime_linker::DeploymentExecutionImage>,
        argument: Option<&ValueSlot>,
        heap: &mut dyn VmHeap,
        resume: VmResumeToken,
    ) -> Result<BytecodeAdapterHandoff<VmFiber>, BytecodeSchedulerError> {
        let argument = argument.ok_or_else(|| {
            BytecodeSchedulerError::Port(
                "core.duration.milliseconds requires one argument".to_string(),
            )
        })?;
        let millis = argument
            .as_integer()
            .or_else(|| {
                argument
                    .as_number()
                    .filter(|value| value.fract() == 0.0)
                    .map(|value| value as i64)
            })
            .ok_or_else(|| {
                BytecodeSchedulerError::Port(
                    "core.duration.milliseconds requires an integer millisecond value".to_string(),
                )
            })?;
        let outcome = ResumeOutcome::Values(VmOwnedValues::from_values(
            image,
            vec![ValueSlot::integer(millis)].into_boxed_slice(),
        ));
        Ok(BytecodeAdapterHandoff::Ready(BytecodeHandoff {
            resume,
            outcome,
        }))
    }

    fn execute_config_require(
        &self,
        arguments: &skiff_runtime_vm::VmOwnedValues,
        heap: &mut dyn VmHeap,
        resume: VmResumeToken,
    ) -> Result<BytecodeAdapterHandoff<VmFiber>, BytecodeSchedulerError> {
        let key = arguments
            .values()
            .first()
            .and_then(|value| string_argument_value(heap, arguments.image(), value).ok());
        let value = if key.as_deref() == Some("skiff.test.ingressUrl") {
            self.self_ingress
                .as_ref()
                .map(|context| context.origin.clone())
                .unwrap_or_default()
        } else {
            String::new()
        };
        let value = heap
            .alloc_string(value)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let outcome = ResumeOutcome::Values(VmOwnedValues::from_values(
            arguments.image().clone(),
            vec![value].into_boxed_slice(),
        ));
        Ok(BytecodeAdapterHandoff::Ready(BytecodeHandoff {
            resume,
            outcome,
        }))
    }

    fn execute_sleep(
        &self,
        image: Arc<skiff_runtime_linker::DeploymentExecutionImage>,
        argument: Option<&ValueSlot>,
        heap: &mut dyn VmHeap,
        resume: VmResumeToken,
    ) -> Result<BytecodeAdapterHandoff<VmFiber>, BytecodeSchedulerError> {
        let argument = argument.ok_or_else(|| {
            BytecodeSchedulerError::Port("std.time.sleep requires one argument".to_string())
        })?;
        let millis = sleep_millis_argument(heap, &image, argument)?;
        if millis == 0 {
            return Ok(BytecodeAdapterHandoff::Ready(BytecodeHandoff {
                resume,
                outcome: ResumeOutcome::Empty,
            }));
        }

        let completion = self
            .pending
            .begin(RootEscrow::new(Box::new(BytecodeRequestEmptyRoots)))
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let operation = resume.into_pending(completion.ticket());
        spawn_sleep_timer(completion, self.cancellation.clone(), self.deadline, millis)?;
        Ok(BytecodeAdapterHandoff::Pending(operation))
    }

    fn publish_pending(
        &self,
        operation: skiff_runtime_vm::PendingOperation,
        suspended: BytecodeRequestSuspended,
    ) -> Result<(), BytecodeSchedulerError> {
        self.pending
            .publish_operation(operation, suspended, Arc::clone(&self.queue))
            .map(|_| ())
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))
    }
}

fn linked_package_symbol_type_tag(
    resume: &VmResumeToken,
    symbol_path: &str,
) -> Result<CompactTypeTag, BytecodeSchedulerError> {
    let function = resume
        .image()
        .functions()
        .get(resume.function().get() as usize)
        .filter(|function| function.index() == resume.function())
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(
                "bytecode HTTP error site has no exact linked function".to_string(),
            )
        })?;
    let mut matches = resume
        .image()
        .types()
        .iter()
        .filter(|entry| entry.origin().specialization() == Some(function.key()))
        .filter(|entry| {
            matches!(
                entry.type_ref(),
                TypeRefIr::PackageSymbol { symbol } if symbol.symbol_path == symbol_path
            )
        });
    let entry = matches.next().ok_or_else(|| {
        BytecodeSchedulerError::Port(
            "bytecode HTTP error type is absent from the exact linked function".to_string(),
        )
    })?;
    if matches.next().is_some() {
        return Err(BytecodeSchedulerError::Port(
            "bytecode HTTP error type is ambiguous in the exact linked function".to_string(),
        ));
    }
    Ok(CompactTypeTag::new(entry.index().get()))
}

fn string_argument_value(
    heap: &mut dyn VmHeap,
    image: &Arc<skiff_runtime_linker::DeploymentExecutionImage>,
    value: &ValueSlot,
) -> Result<String, VmHeapError> {
    if let Some(handle) = value.as_const_ref() {
        let index = skiff_runtime_linked_bytecode::FrozenConstantNodeIndex::new(
            u32::try_from(handle.get()).map_err(|_| VmHeapError::InvalidValueMetadata)?,
        );
        let node = image
            .frozen_constant_nodes()
            .get(index.get() as usize)
            .filter(|node| node.index() == index)
            .ok_or(VmHeapError::InvalidValueMetadata)?;
        if let skiff_runtime_linked_bytecode::LinkedFrozenConstantValue::Literal(
            skiff_artifact_model::LiteralIr::String { value },
        ) = node.value()
        {
            return Ok(value.clone());
        }
        return Err(VmHeapError::InvalidValueMetadata);
    }
    heap.string_value(value)
}

const TIME_SLEEP_MAX_MILLIS: u64 = 60_000;

struct BytecodeRequestEmptyRoots;

impl VmRootSource for BytecodeRequestEmptyRoots {
    fn visit_roots(
        &self,
        _visitor: &mut dyn skiff_runtime_model::vm_root::VmRootVisitor,
    ) -> Result<(), VmHeapError> {
        Ok(())
    }
}

impl RootEscrowBacking for BytecodeRequestEmptyRoots {
    fn root_count(&self) -> usize {
        0
    }

    fn restore_roots(self: Box<Self>) {}

    fn drop_roots(self: Box<Self>, _disposition: RootDisposition) {}
}

fn sleep_millis_argument(
    heap: &mut dyn VmHeap,
    image: &Arc<skiff_runtime_linker::DeploymentExecutionImage>,
    value: &ValueSlot,
) -> Result<u64, BytecodeSchedulerError> {
    let millis = if let Some(handle) = value.as_const_ref() {
        let index = skiff_runtime_linked_bytecode::FrozenConstantNodeIndex::new(
            u32::try_from(handle.get()).map_err(|_| {
                BytecodeSchedulerError::Port(
                    "std.time.sleep duration must be an integer millisecond payload".to_string(),
                )
            })?,
        );
        let node = image
            .frozen_constant_nodes()
            .get(index.get() as usize)
            .filter(|node| node.index() == index)
            .ok_or_else(|| {
                BytecodeSchedulerError::Port(
                    "std.time.sleep duration must be an integer millisecond payload".to_string(),
                )
            })?;
        let skiff_runtime_linked_bytecode::LinkedFrozenConstantValue::Literal(
            skiff_artifact_model::LiteralIr::Number { value },
        ) = node.value()
        else {
            return Err(BytecodeSchedulerError::Port(
                "std.time.sleep duration must be an integer millisecond payload".to_string(),
            ));
        };
        value.as_i64().ok_or_else(|| {
            BytecodeSchedulerError::Port(
                "std.time.sleep duration must be an integer millisecond payload".to_string(),
            )
        })?
    } else {
        value.as_integer().ok_or_else(|| {
            BytecodeSchedulerError::Port(
                "std.time.sleep duration must be an integer millisecond payload".to_string(),
            )
        })?
    };
    let _ = heap;
    Ok(clamp_sleep_millis(millis))
}

fn clamp_sleep_millis(millis: i64) -> u64 {
    if millis <= 0 {
        0
    } else {
        (millis as u64).min(TIME_SLEEP_MAX_MILLIS)
    }
}

fn spawn_sleep_timer(
    completion: VmCompletionHandle<BytecodeRequestSuspended>,
    cancellation: CancellationToken,
    deadline: Option<Instant>,
    millis: u64,
) -> Result<(), BytecodeSchedulerError> {
    let handle = tokio::runtime::Handle::try_current().map_err(|_| {
        BytecodeSchedulerError::Port(
            "std.time.sleep requires a Tokio runtime for pending wake publication".to_string(),
        )
    })?;
    let _ = handle.spawn(async move {
        let sleep = tokio::time::sleep(std::time::Duration::from_millis(millis));
        tokio::pin!(sleep);
        let cancel = cancellation.wait_cancelled();
        tokio::pin!(cancel);

        if let Some(deadline) = deadline {
            let deadline_sleep = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline));
            tokio::pin!(deadline_sleep);
            tokio::select! {
                _ = &mut sleep => {
                    let _ = completion.complete(ResumeOutcome::Empty);
                }
                _ = &mut cancel => {
                    let _ = completion.cancel(ResumeOutcome::InternalTerminal(
                        VmInternalTerminal::OwnerStopped,
                    ));
                }
                _ = &mut deadline_sleep => {
                    let _ = completion.deadline(ResumeOutcome::Failure(VmError::BudgetClosed(
                        VmBudgetClosed::DeadlineExceeded,
                    )));
                }
            }
        } else {
            tokio::select! {
                _ = &mut sleep => {
                    let _ = completion.complete(ResumeOutcome::Empty);
                }
                _ = &mut cancel => {
                    let _ = completion.cancel(ResumeOutcome::InternalTerminal(
                        VmInternalTerminal::OwnerStopped,
                    ));
                }
            }
        }
    });
    Ok(())
}

impl BytecodeChildExecutor<VmFiber> for RequestAdapterExecutor {
    fn execute_child(
        &self,
        _invocation: <VmFiber as BytecodeUnit>::ChildInvocation,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeChildStart<VmFiber>, BytecodeSchedulerError> {
        Err(BytecodeSchedulerError::UnsupportedChild)
    }

    fn execute_adapter(
        &self,
        invocation: <VmFiber as BytecodeUnit>::AdapterInvocation,
        heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeAdapterHandoff<VmFiber>, BytecodeSchedulerError> {
        let (adapter_index, arguments, resume) = invocation.into_parts();
        let image = Arc::clone(arguments.image());
        let adapter = image
            .host_effect_adapters()
            .get(adapter_index.get() as usize)
            .filter(|row| row.index() == adapter_index)
            .ok_or_else(|| {
                BytecodeSchedulerError::Port("host adapter is out of bounds".to_string())
            })?;
        match adapter.binding_key().as_str() {
            "std.http.client.stream" => {
                self.execute_http_stream(image, arguments.values().first(), heap, resume)
            }
            "std.http.client.request" => {
                self.execute_http_request(image, arguments.values().first(), heap, resume)
            }
            "std.time.sleep" => self.execute_sleep(image, arguments.values().first(), heap, resume),
            "std.config.require" => self.execute_config_require(&arguments, heap, resume),
            "core.bytes.fromUtf8" => {
                self.execute_bytes_from_utf8(image, arguments.values().first(), heap, resume)
            }
            "core.duration.milliseconds" => {
                self.execute_duration_milliseconds(image, arguments.values().first(), heap, resume)
            }
            "std.http.stream.start"
            | "std.http.streamStart"
            | "std.http.stream.chunk"
            | "std.http.streamChunk"
            | "std.http.stream.end"
            | "std.http.streamEnd" => self.execute_http_stream_event(
                Arc::clone(&image),
                &arguments,
                heap,
                resume,
                adapter.binding_key().as_str(),
            ),
            _ => Err(BytecodeSchedulerError::UnsupportedAdapter),
        }
    }

    fn park_adapter(
        &self,
        operation: skiff_runtime_vm::PendingOperation,
        suspended: BytecodeRequestSuspended,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeSchedulerError> {
        self.publish_pending(operation, suspended)
    }

    fn execute_stream_next(
        &self,
        invocation: <VmFiber as BytecodeUnit>::ChildInvocation,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<BytecodeStreamHandoff<VmFiber>, BytecodeSchedulerError> {
        self.poll_stream_next(invocation, heap, budget)
    }
}

pub struct BytecodeRequestExecutionInput {
    pub target: DeploymentExecutionEntry,
    pub request: RequestEnvelope,
    pub observer: BytecodeExecutionObserver,
    pub cancellation: CancellationToken,
    pub execution_budget: Arc<ExecutionBudget>,
    pub handles: BytecodeRequestExecutionHandles,
}

pub struct BytecodeRequestExecutionHandles {
    pub request_heap_limits: RequestHeapLimits,
    pub http_executor: Option<Arc<dyn BytecodeHttpExecutor>>,
    pub self_ingress: Option<BytecodeSelfIngressContext>,
}

/// Execution ports supplied to the bytecode scheduler.
pub type BytecodeRequestExecutionPorts = BytecodeSchedulerPorts<VmFiber>;

/// One completed VM handoff plus the unique continuation that resumes its
/// parent fiber.
pub type BytecodeInvocationHandoff = BytecodeHandoff<VmFiber>;

pub type BytecodeRequestSuspended = SuspendedTrampoline<VmFiber, VmResumeToken>;
pub type BytecodeRequestPendingWake = VmPendingWake<BytecodeRequestSuspended>;
pub type BytecodeRequestWakeQueue =
    Arc<dyn PendingWakeQueue<VmResumeToken, BytecodeRequestSuspended, ResumeOutcome>>;

/// Result of driving a resumable bytecode request once.
#[derive(Debug, PartialEq)]
pub enum BytecodeRequestRunOutcome {
    Complete(BoundaryResponse),
    Parked,
}

/// Resumable bytecode request execution state.
///
/// The legacy scalar entry point remains synchronous. This state keeps the VM
/// budget, request heap, response stream and pending-wake queue alive so a
/// real park can be resumed without rerunning the whole request from scratch.
pub struct BytecodeRequestExecution {
    driver: BytecodeRequestDriver<VmFiber>,
    mode: String,
    raw_http_adapter: bool,
    execution_budget: Arc<ExecutionBudget>,
    _owner_inventory_freeze: RequestExecutionOwnerInventoryFreezePermit,
}

impl BytecodeRequestExecution {
    /// Drives the initial request segment.
    pub fn run(&mut self) -> RequestResult<BytecodeRequestRunOutcome> {
        let outcome = self.driver.run()?;
        self.map_outcome(outcome)
    }

    /// Consumes exactly one claimed pending wake and resumes the parked VM.
    pub fn resume(
        &mut self,
        wake: BytecodeRequestPendingWake,
    ) -> RequestResult<BytecodeRequestRunOutcome> {
        let outcome = self.driver.resume(wake)?;
        self.map_outcome(outcome)
    }

    /// Returns one claimed wake if the response bridge already published it.
    pub fn take_pending_wake(&mut self) -> Option<BytecodeRequestPendingWake> {
        self.driver.take_pending_wake()
    }

    /// Waits for the response bridge or a host adapter to publish a wake.
    pub async fn wait_pending_wake(&mut self) -> RequestResult<BytecodeRequestPendingWake> {
        self.driver.wait_pending_wake().await
    }

    pub fn is_parked(&self) -> bool {
        self.driver.state == BytecodeRequestDriverState::Parked
    }

    fn map_outcome(
        &mut self,
        outcome: BytecodeRequestDriverOutcome<VmFiber>,
    ) -> RequestResult<BytecodeRequestRunOutcome> {
        match outcome {
            BytecodeRequestDriverOutcome::Complete {
                result,
                stream_sent,
            } => {
                if stream_sent {
                    return Ok(BytecodeRequestRunOutcome::Complete(
                        BoundaryResponse::StreamSent,
                    ));
                }
                let values = result
                    .map_err(|error| vm_error_to_request_error(&self.execution_budget, error))?;
                if self.mode == "serverStream" {
                    return Err(RequestError::Decode(
                        "serverStream request completed without a response stream".to_string(),
                    ));
                }
                if self.raw_http_adapter {
                    let response =
                        http_response_from_vm_values(&mut *self.driver.heap, values.values())?;
                    return Ok(BytecodeRequestRunOutcome::Complete(response));
                }
                let payload = json_payload_from_value_slots(values.values())?;
                Ok(BytecodeRequestRunOutcome::Complete(
                    BoundaryResponse::payload(payload),
                ))
            }
            BytecodeRequestDriverOutcome::Parked => Ok(BytecodeRequestRunOutcome::Parked),
        }
    }
}

/// Starts a bytecode request with a response event sink and a resumable
/// pending-wake queue.
pub fn start_runtime_bytecode_request(
    input: BytecodeRequestExecutionInput,
    response_events: Arc<dyn ResponseEventSink>,
) -> RequestResult<BytecodeRequestExecution> {
    start_runtime_bytecode_request_with_ports(
        input,
        response_events,
        BytecodeRequestExecutionPorts::default(),
    )
}

/// Starts a bytecode request with optional child/adapter ports.
pub fn start_runtime_bytecode_request_with_ports(
    input: BytecodeRequestExecutionInput,
    response_events: Arc<dyn ResponseEventSink>,
    _ports: BytecodeRequestExecutionPorts,
) -> RequestResult<BytecodeRequestExecution> {
    let request_id = input.request.request_id.clone();
    let mode = input.request.mode.clone();
    validate_runtime_bytecode_request(&input.request)?;
    let execution = ExecutionControl::new(input.cancellation.clone(), &input.execution_budget);
    execution.check_cancelled().map_err(RequestError::from)?;

    let BytecodeRequestExecutionInput {
        target,
        request,
        observer,
        cancellation,
        execution_budget,
        handles,
    } = input;

    let owner_pin = Arc::clone(target.image());
    let (owner_registrations, owner_inventory_freeze) =
        RequestExecutionOwnerInventory::open().into_parts();
    let resource_table = ResourceTable::new(owner_registrations.resource());
    let mut request_heap = RequestVmHeap::new(handles.request_heap_limits);
    request_heap.set_resource_table(resource_table.clone());
    let arguments = gateway_entry_arguments(&request, &target, &mut request_heap)?;
    let fiber = Vm::start(target, arguments.into_boxed_slice(), vm_limits(), observer)
        .map_err(|error| vm_error_to_request_error(&execution_budget, error))?;

    let queue = Arc::new(InMemoryWakeQueue::new());
    let (supervisor, consumer) =
        VmStreamSupervisor::open(owner_pin, queue.clone(), owner_registrations.pending());
    let writer = ResponseStreamWriter::new(request_id, response_events);
    let drain = RequestResponseStream {
        consumer,
        writer,
        mode: mode.clone(),
        execution_budget: Arc::clone(&execution_budget),
    };
    let supervisor = Arc::new(supervisor);
    let stream_supervisor: Arc<dyn BytecodeStreamSupervisor<VmFiber>> = supervisor.clone();
    let child_executor = Some(Arc::new(RequestAdapterExecutor::new(
        request.test_effects_enabled,
        handles.http_executor.clone(),
        handles.self_ingress.clone(),
        resource_table,
        queue.clone(),
        owner_registrations.pending(),
        cancellation.clone(),
        execution_budget.deadline(),
    )) as Arc<dyn BytecodeChildExecutor<VmFiber>>);
    let scheduler = BytecodeScheduler::new(
        fiber,
        BytecodeSchedulerPorts {
            child_executor: child_executor.clone(),
            stream_supervisor: Some(stream_supervisor.clone()),
        },
        owner_registrations.child(),
    );

    let heap: Box<dyn VmHeap + Send> = Box::new(request_heap);
    let budget: Box<dyn VmBudget + Send> =
        Box::new(execution_budget.attach_vm().map_err(|error| {
            RequestError::Decode(format!("bytecode VM budget attachment failed: {error}"))
        })?);
    let driver = BytecodeRequestDriver::new(
        scheduler,
        child_executor,
        Some(stream_supervisor),
        Some(Box::new(drain)),
        queue,
        heap,
        budget,
        scheduler_error_map(execution_budget.clone()),
    );

    Ok(BytecodeRequestExecution {
        driver,
        mode,
        raw_http_adapter: request
            .http_adapter
            .as_ref()
            .is_some_and(|adapter| adapter.kind == HttpAdapterKind::RawHttp),
        execution_budget,
        _owner_inventory_freeze: owner_inventory_freeze,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BytecodeRequestDriverState {
    Initial,
    Parked,
    Complete,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BytecodeRequestDrainState {
    Empty,
    Delivered,
    Terminal,
}

trait BytecodeRequestStreamDrain<U: BytecodeUnit>: Send {
    fn drain(&mut self, heap: &mut dyn VmHeap) -> RequestResult<BytecodeRequestDrainState>;
}

struct NoopWake;

impl WakeSignal for NoopWake {
    fn wake(&self) {}
}

struct RequestResponseStream {
    consumer: StreamConsumer<Arc<DeploymentExecutionImage>, VmOwnedValues, VmStreamTerminal>,
    writer: ResponseStreamWriter,
    mode: String,
    execution_budget: Arc<ExecutionBudget>,
}

impl BytecodeRequestStreamDrain<VmFiber> for RequestResponseStream {
    fn drain(&mut self, heap: &mut dyn VmHeap) -> RequestResult<BytecodeRequestDrainState> {
        let mut state = BytecodeRequestDrainState::Empty;
        while let Some(poll) = self.consumer.poll_next_ready() {
            match poll {
                StreamPoll::Rejected(reason) => {
                    return Err(RequestError::Decode(format!(
                        "bytecode response stream consumer rejected: {reason:?}"
                    )))
                }
                StreamPoll::Ready(StreamEvent::Item(values)) => {
                    if self.mode != "serverStream" {
                        return Err(RequestError::Unsupported(
                            "bytecode stream emission requires serverStream request.start mode"
                                .to_string(),
                        ));
                    }
                    let event = http_stream_event_from_vm_values(heap, values.values())?;
                    self.writer.send_binary_http_event(event)?;
                    state = BytecodeRequestDrainState::Delivered;
                }
                StreamPoll::Ready(StreamEvent::End) => {
                    if self.mode != "serverStream" {
                        return Err(RequestError::Unsupported(
                            "bytecode stream end requires serverStream request.start mode"
                                .to_string(),
                        ));
                    }
                    self.writer.require_exact_http_terminal()?;
                    return Ok(BytecodeRequestDrainState::Terminal);
                }
                StreamPoll::Ready(StreamEvent::Error(error)) => {
                    let error = match error {
                        VmStreamTerminal::Cancelled => RequestError::Cancelled,
                        VmStreamTerminal::Error(error) => {
                            vm_error_to_request_error(&self.execution_budget, error)
                        }
                        VmStreamTerminal::End => {
                            unreachable!("End is delivered through StreamEvent::End")
                        }
                    };
                    return Err(error);
                }
                StreamPoll::Ready(StreamEvent::Cancelled) => {
                    return Err(RequestError::Cancelled);
                }
                StreamPoll::Pending => break,
            }
        }
        Ok(state)
    }
}

type DriverPendingWake<U> = PendingWake<
    <U as BytecodeUnit>::ResumeToken,
    SuspendedTrampoline<U, <U as BytecodeUnit>::ResumeToken>,
    <U as BytecodeUnit>::ResumeOutcome,
>;

struct InMemoryWakeQueue<U: BytecodeUnit> {
    wakes: Mutex<VecDeque<DriverPendingWake<U>>>,
    notify: tokio::sync::Notify,
}

impl<U: BytecodeUnit> InMemoryWakeQueue<U> {
    fn new() -> Self {
        Self {
            wakes: Mutex::new(VecDeque::new()),
            notify: tokio::sync::Notify::new(),
        }
    }

    fn take(&self) -> Option<DriverPendingWake<U>> {
        self.wakes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front()
    }

    async fn take_async(&self) -> Option<DriverPendingWake<U>>
    where
        U: Send,
    {
        loop {
            if let Some(wake) = self.take() {
                return Some(wake);
            }
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(wake) = self.take() {
                return Some(wake);
            }
            notified.await;
        }
    }
}

impl<U> PendingWakeQueue<U::ResumeToken, SuspendedTrampoline<U, U::ResumeToken>, U::ResumeOutcome>
    for InMemoryWakeQueue<U>
where
    U: BytecodeUnit + Send + 'static,
    U::ResumeToken: Send,
    U::ResumeOutcome: Send,
    SuspendedTrampoline<U, U::ResumeToken>: Send,
{
    fn enqueue(&self, wake: DriverPendingWake<U>) {
        self.wakes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(wake);
        self.notify.notify_one();
    }
}

trait BytecodeRequestResume<U>
where
    U: BytecodeUnit + VmRootSource + 'static,
{
    fn into_scheduler(
        self,
        ports: BytecodeSchedulerPorts<U>,
    ) -> Result<BytecodeScheduler<U>, BytecodeSchedulerError>;
}

impl<U> BytecodeRequestResume<U> for DriverPendingWake<U>
where
    U: BytecodeUnit + VmRootSource + 'static,
{
    fn into_scheduler(
        self,
        ports: BytecodeSchedulerPorts<U>,
    ) -> Result<BytecodeScheduler<U>, BytecodeSchedulerError> {
        // The wake carries the original `U::ResumeOutcome`. For `VmFiber` that
        // includes `ResumeOutcome::StreamEnd`; restoring it through the scheduler
        // reaches the VM's independent end resume PC instead of an item path or a
        // producer backpressure `Empty`.
        resume_pending_wake(self, ports)
    }
}

enum BytecodeRequestDriverOutcome<U: BytecodeUnit> {
    Complete {
        result: U::RootResult,
        stream_sent: bool,
    },
    Parked,
}

struct BytecodeRequestDriver<U>
where
    U: BytecodeUnit + VmRootSource + 'static,
{
    scheduler: Option<BytecodeScheduler<U>>,
    child_executor: Option<Arc<dyn BytecodeChildExecutor<U>>>,
    stream_supervisor: Option<Arc<dyn BytecodeStreamSupervisor<U>>>,
    stream_drain: Option<Box<dyn BytecodeRequestStreamDrain<U>>>,
    queue: Arc<InMemoryWakeQueue<U>>,
    heap: Box<dyn VmHeap + Send>,
    budget: Box<dyn VmBudget + Send>,
    error_map: Box<dyn Fn(BytecodeSchedulerError) -> RequestError + Send + Sync>,
    state: BytecodeRequestDriverState,
}

impl<U> BytecodeRequestDriver<U>
where
    U: BytecodeUnit + VmRootSource + 'static,
{
    #[allow(clippy::too_many_arguments)]
    fn new(
        scheduler: BytecodeScheduler<U>,
        child_executor: Option<Arc<dyn BytecodeChildExecutor<U>>>,
        stream_supervisor: Option<Arc<dyn BytecodeStreamSupervisor<U>>>,
        stream_drain: Option<Box<dyn BytecodeRequestStreamDrain<U>>>,
        queue: Arc<InMemoryWakeQueue<U>>,
        heap: Box<dyn VmHeap + Send>,
        budget: Box<dyn VmBudget + Send>,
        error_map: Box<dyn Fn(BytecodeSchedulerError) -> RequestError + Send + Sync>,
    ) -> Self {
        Self {
            scheduler: Some(scheduler),
            child_executor,
            stream_supervisor,
            stream_drain,
            queue,
            heap,
            budget,
            error_map,
            state: BytecodeRequestDriverState::Initial,
        }
    }

    fn ports(&self) -> BytecodeSchedulerPorts<U> {
        BytecodeSchedulerPorts {
            child_executor: self.child_executor.clone(),
            stream_supervisor: self.stream_supervisor.clone(),
        }
    }

    fn run(&mut self) -> RequestResult<BytecodeRequestDriverOutcome<U>> {
        match self.state {
            BytecodeRequestDriverState::Initial => {
                let scheduler = self.scheduler.take().ok_or_else(|| {
                    RequestError::Decode("bytecode request scheduler is missing".to_string())
                })?;
                self.advance(scheduler)
            }
            BytecodeRequestDriverState::Parked | BytecodeRequestDriverState::Complete => Err(
                RequestError::Decode("bytecode request has already been driven".to_string()),
            ),
            BytecodeRequestDriverState::Failed => Err(RequestError::Decode(
                "bytecode request failed closed".to_string(),
            )),
        }
    }

    fn resume<R>(&mut self, resume: R) -> RequestResult<BytecodeRequestDriverOutcome<U>>
    where
        R: BytecodeRequestResume<U>,
    {
        if self.state != BytecodeRequestDriverState::Parked {
            return Err(RequestError::Decode(
                "bytecode request is not parked".to_string(),
            ));
        }
        let scheduler = resume.into_scheduler(self.ports()).map_err(|error| {
            let error = (self.error_map)(error);
            self.state = BytecodeRequestDriverState::Failed;
            error
        })?;
        self.advance(scheduler)
    }

    fn take_pending_wake(&self) -> Option<DriverPendingWake<U>> {
        self.queue.take()
    }

    async fn wait_pending_wake(&mut self) -> RequestResult<DriverPendingWake<U>>
    where
        U: Send,
    {
        self.queue
            .take_async()
            .await
            .ok_or_else(|| RequestError::Decode("bytecode request wake queue closed".to_string()))
    }

    fn advance(
        &mut self,
        scheduler: BytecodeScheduler<U>,
    ) -> RequestResult<BytecodeRequestDriverOutcome<U>> {
        let outcome = match scheduler.run(&mut *self.heap, &mut *self.budget) {
            Ok(outcome) => outcome,
            Err(error) => {
                let error = (self.error_map)(error);
                self.state = BytecodeRequestDriverState::Failed;
                return Err(error);
            }
        };
        let drain_state = match self.stream_drain.as_mut() {
            Some(drain) => match drain.drain(&mut *self.heap) {
                Ok(state) => state,
                Err(error) => {
                    self.state = BytecodeRequestDriverState::Failed;
                    return Err(error);
                }
            },
            None => BytecodeRequestDrainState::Empty,
        };
        match outcome {
            BytecodeSchedulerOutcome::Complete(result) => {
                self.state = BytecodeRequestDriverState::Complete;
                Ok(BytecodeRequestDriverOutcome::Complete {
                    result,
                    stream_sent: drain_state == BytecodeRequestDrainState::Terminal,
                })
            }
            BytecodeSchedulerOutcome::Parked => {
                self.state = BytecodeRequestDriverState::Parked;
                Ok(BytecodeRequestDriverOutcome::Parked)
            }
        }
    }
}

fn scheduler_error_map(
    execution_budget: Arc<ExecutionBudget>,
) -> Box<dyn Fn(BytecodeSchedulerError) -> RequestError + Send + Sync> {
    Box::new(move |error| scheduler_error_to_request_error(&execution_budget, error))
}

/// Executes one scalar bytecode request against a verified deployment image.
pub fn execute_runtime_bytecode_request(
    input: BytecodeRequestExecutionInput,
) -> RequestResult<BoundaryResponse> {
    execute_runtime_bytecode_request_with_ports(input, BytecodeRequestExecutionPorts::default())
}

/// Executes one bytecode request with optional child/adapter execution ports.
pub fn execute_runtime_bytecode_request_with_ports(
    input: BytecodeRequestExecutionInput,
    ports: BytecodeRequestExecutionPorts,
) -> RequestResult<BoundaryResponse> {
    let BytecodeRequestExecutionInput {
        target,
        request,
        observer,
        cancellation,
        execution_budget,
        handles,
    } = input;

    validate_bytecode_request(&request)?;
    let execution = ExecutionControl::new(cancellation.clone(), &execution_budget);
    execution.check_cancelled().map_err(RequestError::from)?;

    let (owner_registrations, _owner_inventory_freeze) =
        RequestExecutionOwnerInventory::open().into_parts();
    let resource_table = ResourceTable::new(owner_registrations.resource());
    let mut heap = RequestVmHeap::new(handles.request_heap_limits);
    heap.set_resource_table(resource_table.clone());
    let arguments = gateway_entry_arguments(&request, &target, &mut heap)?;
    let fiber = Vm::start(target, arguments.into_boxed_slice(), vm_limits(), observer)
        .map_err(|error| vm_error_to_request_error(&execution_budget, error))?;
    let queue = Arc::new(InMemoryWakeQueue::new());
    let child_cancellation = cancellation.clone();
    let mut budget = execution_budget.attach_vm().map_err(|error| {
        RequestError::Decode(format!("bytecode VM budget attachment failed: {error}"))
    })?;

    let child_executor = Some(Arc::new(RequestAdapterExecutor::new(
        request.test_effects_enabled,
        handles.http_executor.clone(),
        handles.self_ingress.clone(),
        resource_table,
        queue.clone(),
        owner_registrations.pending(),
        child_cancellation,
        execution_budget.deadline(),
    )) as Arc<dyn BytecodeChildExecutor<VmFiber>>);
    let outcome = BytecodeScheduler::new(
        fiber,
        BytecodeSchedulerPorts {
            child_executor,
            stream_supervisor: ports.stream_supervisor,
        },
        owner_registrations.child(),
    )
    .run(&mut heap, &mut budget)
    .map_err(|error| scheduler_error_to_request_error(&execution_budget, error))?;
    let BytecodeSchedulerOutcome::Complete(result) = outcome else {
        return Err(RequestError::Unsupported(
            "bytecode VM parked; scalar ingress has no pending wake resume path".to_string(),
        ));
    };
    let values = result.map_err(|error| vm_error_to_request_error(&execution_budget, error))?;
    let payload = json_payload_from_value_slots(values.values())?;
    Ok(BoundaryResponse::payload(payload))
}

fn example_test_http_response(url: &str) -> (u16, Vec<u8>) {
    if url.contains("/from-entry") {
        (200, b"double-body".to_vec())
    } else if url.contains("/direct") {
        (202, b"direct-double".to_vec())
    } else {
        (200, b"response".to_vec())
    }
}

fn materialize_http_client_request(
    heap: &mut dyn VmHeap,
    input: &ValueSlot,
) -> Result<serde_json::Value, BytecodeSchedulerError> {
    let string_field = |field: &str| -> Result<String, BytecodeSchedulerError> {
        let value = heap
            .record_field(input, field)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        heap.string_value(&value)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))
    };
    let method = string_field("method")?;
    let url = string_field("url")?;

    let headers_slot = heap
        .record_field(input, "headers")
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    let mut headers = Vec::new();
    for index in 0..heap
        .array_len(&headers_slot)
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?
    {
        let item = heap
            .array_get(&headers_slot, index)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let name = heap
            .record_field(&item, "name")
            .and_then(|slot| heap.string_value(&slot))
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let value = heap
            .record_field(&item, "value")
            .and_then(|slot| heap.string_value(&slot))
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        headers.push(serde_json::json!({ "name": name, "value": value }));
    }

    let body_slot = heap
        .record_field(input, "body")
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    let body = if body_slot.is_null() {
        serde_json::Value::Null
    } else {
        let bytes = heap
            .bytes_value(&body_slot)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        bytes_value(&bytes)
    };

    let timeout_slot = heap
        .record_field(input, "timeoutMs")
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    let timeout_ms = if timeout_slot.is_null() {
        serde_json::Value::Null
    } else {
        timeout_slot
            .as_integer()
            .map_or(serde_json::Value::Null, |value| serde_json::json!(value))
    };

    Ok(serde_json::json!({
        "method": method,
        "url": url,
        "headers": headers,
        "body": body,
        "timeoutMs": timeout_ms,
    }))
}

fn materialize_name_values_dyn<'a>(
    items: impl IntoIterator<Item = &'a HttpNameValue>,
    heap: &mut dyn VmHeap,
) -> Result<ValueSlot, BytecodeSchedulerError> {
    let mut records = Vec::new();
    for item in items {
        let name = heap
            .alloc_string(item.name.clone())
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let value = heap
            .alloc_string(item.value.clone())
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let record = heap
            .allocate_record(
                &[
                    VmRecordField {
                        name: "name".to_string(),
                        value: name,
                    },
                    VmRecordField {
                        name: "value".to_string(),
                        value,
                    },
                ],
                CompactTypeTag::new(0),
                ValueFlags::new(0),
            )
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        records.push(record);
    }
    heap.allocate_array(&records, CompactTypeTag::new(0), ValueFlags::new(0))
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))
}

fn materialize_http_response(
    heap: &mut dyn VmHeap,
    value: &serde_json::Value,
) -> Result<ValueSlot, BytecodeSchedulerError> {
    let status = value
        .get("status")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            BytecodeSchedulerError::Port("HTTP response status is missing".to_string())
        })?;
    let headers_value = value
        .get("headers")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            BytecodeSchedulerError::Port("HTTP response headers are not an array".to_string())
        })?;
    let mut header_slots = Vec::with_capacity(headers_value.len());
    for header in headers_value {
        let name = header
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                BytecodeSchedulerError::Port("HTTP response header name is missing".to_string())
            })?;
        let value = header
            .get("value")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                BytecodeSchedulerError::Port("HTTP response header value is missing".to_string())
            })?;
        let name = heap
            .alloc_string(name.to_string())
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let value = heap
            .alloc_string(value.to_string())
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let record = heap
            .allocate_record(
                &[
                    VmRecordField {
                        name: "name".to_string(),
                        value: name,
                    },
                    VmRecordField {
                        name: "value".to_string(),
                        value,
                    },
                ],
                CompactTypeTag::new(0),
                ValueFlags::new(0),
            )
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        header_slots.push(record);
    }
    let headers = heap
        .allocate_array(&header_slots, CompactTypeTag::new(0), ValueFlags::new(0))
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    let body_value = value
        .get("body")
        .and_then(bytes_payload)
        .ok_or_else(|| BytecodeSchedulerError::Port("HTTP response body is missing".to_string()))?;
    let body = heap
        .alloc_bytes(body_value)
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    heap.allocate_record(
        &[
            VmRecordField {
                name: "status".to_string(),
                value: ValueSlot::integer(status as i64),
            },
            VmRecordField {
                name: "headers".to_string(),
                value: headers,
            },
            VmRecordField {
                name: "body".to_string(),
                value: body,
            },
        ],
        CompactTypeTag::new(0),
        ValueFlags::new(0),
    )
    .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))
}

fn url_origin(url: &str) -> Option<String> {
    let scheme_end = url.find("://")?;
    let authority_start = scheme_end + 3;
    let rest = &url[authority_start..];
    let authority_end = rest.find('/').unwrap_or(rest.len());
    Some(format!(
        "{}{}",
        &url[..authority_start],
        &rest[..authority_end]
    ))
}

fn gateway_entry_arguments(
    request: &RequestEnvelope,
    entry: &DeploymentExecutionEntry,
    heap: &mut RequestVmHeap,
) -> RequestResult<Vec<ValueSlot>> {
    let Some(adapter) = &request.http_adapter else {
        return Ok(Vec::new());
    };
    let binary = request.binary_http.as_ref().ok_or_else(|| {
        RequestError::Decode("HTTP adapter request is missing binary HTTP metadata".to_string())
    })?;
    let typed_json_body = match adapter.kind {
        HttpAdapterKind::RawHttp => None,
        HttpAdapterKind::TypedJson => {
            let parameter_count = entry.signature().parameter_types().len();
            if adapter.adapter_args.len() != parameter_count {
                return Err(RequestError::Decode(format!(
                    "typedJson HTTP adapter has {} arguments but the exact pinned entry has {parameter_count} parameters",
                    adapter.adapter_args.len()
                )));
            }
            if !adapter
                .adapter_args
                .iter()
                .any(|arg| arg.source == GatewayAdapterSource::HttpBody)
            {
                return Err(RequestError::Decode(
                    "typedJson HTTP adapter has no http.body argument".to_string(),
                ));
            }
            Some(
                serde_json::from_slice::<serde_json::Value>(&binary.body).map_err(|error| {
                    RequestError::Decode(format!("typedJson HTTP body is not valid JSON: {error}"))
                })?,
            )
        }
    };
    let mut arguments = Vec::with_capacity(adapter.adapter_args.len());
    for (ordinal, arg) in adapter.adapter_args.iter().enumerate() {
        let value = match arg.source {
            GatewayAdapterSource::HttpRequest => materialize_http_request(binary, heap)?,
            GatewayAdapterSource::HttpBody => match typed_json_body.as_ref() {
                Some(body) => materialize_typed_json_scalar(body, entry, ordinal)?,
                None => heap
                    .alloc_bytes(binary.body.clone())
                    .map_err(heap_error_to_request_error)?,
            },
            GatewayAdapterSource::HttpContext => ValueSlot::null(),
        };
        arguments.push(value);
    }
    Ok(arguments)
}

fn materialize_typed_json_scalar(
    body: &serde_json::Value,
    entry: &DeploymentExecutionEntry,
    ordinal: usize,
) -> RequestResult<ValueSlot> {
    if matches!(
        body,
        serde_json::Value::Array(_) | serde_json::Value::Object(_)
    ) {
        return Err(RequestError::Unsupported(format!(
            "typedJson HTTP body for parameter {ordinal} is non-scalar; Phase 1 supports only number, bool, and null"
        )));
    }

    let signature = entry.signature();
    let expected_type = signature.parameter_types().get(ordinal).ok_or_else(|| {
        RequestError::Decode(format!(
            "typedJson HTTP body parameter {ordinal} is absent from the exact pinned entry signature"
        ))
    })?;
    let expected_plan = signature.parameter_plans().get(ordinal).ok_or_else(|| {
        RequestError::Decode(format!(
            "typedJson HTTP body parameter {ordinal} has no exact pinned lifecycle plan"
        ))
    })?;
    let type_entry = entry
        .image()
        .types()
        .get(expected_type.get() as usize)
        .filter(|entry| entry.index() == *expected_type)
        .ok_or_else(|| {
            RequestError::Decode(format!(
                "typedJson HTTP body parameter {ordinal} has no exact pinned concrete type"
            ))
        })?;

    if !matches!(
        expected_plan,
        LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial
        }
    ) {
        return Err(RequestError::Unsupported(format!(
            "typedJson HTTP body parameter {ordinal} has an unsupported exact pinned lifecycle plan"
        )));
    }

    let TypeRefIr::Builtin { name, args } = type_entry.type_ref() else {
        return Err(unsupported_typed_json_parameter_type(ordinal));
    };
    if !args.is_empty() {
        return Err(unsupported_typed_json_parameter_type(ordinal));
    }

    match (name.as_str(), body) {
        ("number", serde_json::Value::Number(number)) => number
            .as_f64()
            .filter(|number| number.is_finite())
            .map(ValueSlot::number)
            .ok_or_else(|| {
                RequestError::Decode(format!(
                    "typedJson HTTP body number for parameter {ordinal} cannot be represented by the VM"
                ))
            }),
        ("bool", serde_json::Value::Bool(value)) => Ok(ValueSlot::bool(*value)),
        ("null", serde_json::Value::Null) => Ok(ValueSlot::null()),
        ("number" | "bool" | "null", _) => Err(RequestError::Decode(format!(
            "typedJson HTTP body does not match the exact pinned {name} type for parameter {ordinal}"
        ))),
        _ => Err(unsupported_typed_json_parameter_type(ordinal)),
    }
}

fn unsupported_typed_json_parameter_type(ordinal: usize) -> RequestError {
    RequestError::Unsupported(format!(
        "typedJson HTTP body parameter {ordinal} has an unsupported exact pinned type; Phase 1 supports only number, bool, and null"
    ))
}

fn materialize_http_request(
    binary: &BinaryHttpRequest,
    heap: &mut RequestVmHeap,
) -> RequestResult<ValueSlot> {
    let method = heap
        .alloc_string(binary.metadata.method.clone())
        .map_err(heap_error_to_request_error)?;
    let url = heap
        .alloc_string(binary.metadata.url.clone())
        .map_err(heap_error_to_request_error)?;
    let path = heap
        .alloc_string(binary.metadata.path.clone())
        .map_err(heap_error_to_request_error)?;
    let query = materialize_name_values(&binary.metadata.query, heap)?;
    let headers = materialize_name_values(&binary.metadata.headers, heap)?;
    let body = heap
        .alloc_bytes(binary.body.clone())
        .map_err(heap_error_to_request_error)?;
    let fields = vec![
        VmRecordField {
            name: "method".to_string(),
            value: method,
        },
        VmRecordField {
            name: "url".to_string(),
            value: url,
        },
        VmRecordField {
            name: "path".to_string(),
            value: path,
        },
        VmRecordField {
            name: "query".to_string(),
            value: query,
        },
        VmRecordField {
            name: "headers".to_string(),
            value: headers,
        },
        VmRecordField {
            name: "body".to_string(),
            value: body,
        },
    ];
    heap.allocate_record(&fields, CompactTypeTag::new(0), ValueFlags::new(0))
        .map_err(heap_error_to_request_error)
}

fn materialize_name_values(
    items: &[HttpNameValue],
    heap: &mut RequestVmHeap,
) -> RequestResult<ValueSlot> {
    let mut records = Vec::with_capacity(items.len());
    for item in items {
        let name = heap
            .alloc_string(item.name.clone())
            .map_err(heap_error_to_request_error)?;
        let value = heap
            .alloc_string(item.value.clone())
            .map_err(heap_error_to_request_error)?;
        let record = heap
            .allocate_record(
                &[
                    VmRecordField {
                        name: "name".to_string(),
                        value: name,
                    },
                    VmRecordField {
                        name: "value".to_string(),
                        value,
                    },
                ],
                CompactTypeTag::new(0),
                ValueFlags::new(0),
            )
            .map_err(heap_error_to_request_error)?;
        records.push(record);
    }
    heap.allocate_array(&records, CompactTypeTag::new(0), ValueFlags::new(0))
        .map_err(heap_error_to_request_error)
}

fn heap_error_to_request_error(error: VmHeapError) -> RequestError {
    RequestError::Decode(format!(
        "bytecode gateway heap materialization failed: {error}"
    ))
}

fn scheduler_error_to_request_error(
    execution_budget: &ExecutionBudget,
    error: BytecodeSchedulerError,
) -> RequestError {
    match error {
        BytecodeSchedulerError::UnsupportedChild => RequestError::Unsupported(
            "bytecode VM child invocation requires a child executor port".to_string(),
        ),
        BytecodeSchedulerError::UnsupportedAdapter => RequestError::Unsupported(
            "bytecode VM adapter invocation requires a child executor port".to_string(),
        ),
        BytecodeSchedulerError::UnsupportedStream => RequestError::Unsupported(
            "bytecode VM stream emission requires stream supervisor integration".to_string(),
        ),
        BytecodeSchedulerError::UnsupportedPark => RequestError::Unsupported(
            "bytecode VM parking requires stream supervisor integration".to_string(),
        ),
        BytecodeSchedulerError::ChildCapacityExceeded => RequestError::Decode(
            "bytecode scheduler blocked child capacity is exhausted".to_string(),
        ),
        BytecodeSchedulerError::ChildOwnerCreation(error) => RequestError::Decode(format!(
            "bytecode scheduler child owner creation failed: {error}"
        )),
        BytecodeSchedulerError::Vm(error) => vm_error_to_request_error(execution_budget, error),
        BytecodeSchedulerError::Port(message) => {
            RequestError::Unsupported(format!("bytecode scheduler port failed: {message}"))
        }
    }
}

fn validate_bytecode_request(request: &RequestEnvelope) -> RequestResult<()> {
    if request.mode != "unary" {
        return Err(RequestError::Unsupported(format!(
            "bytecode scalar ingress only supports unary request.start, got {}",
            request.mode
        )));
    }
    validate_bytecode_request_metadata(request)
}

fn validate_runtime_bytecode_request(request: &RequestEnvelope) -> RequestResult<()> {
    if !matches!(request.mode.as_str(), "unary" | "serverStream") {
        return Err(RequestError::Unsupported(format!(
            "bytecode request ingress only supports unary or serverStream request.start, got {}",
            request.mode
        )));
    }
    validate_bytecode_request_metadata(request)
}

fn validate_bytecode_request_metadata(request: &RequestEnvelope) -> RequestResult<()> {
    if request.ingress_selector.is_none() {
        return Err(RequestError::Unsupported(
            "bytecode scalar ingress requires request.start ingress_selector".to_string(),
        ));
    }
    if request.extra.contains_key("actorCall") {
        return Err(RequestError::Unsupported(
            "actor.call request.start metadata is not supported by bytecode scalar ingress"
                .to_string(),
        ));
    }
    Ok(())
}

fn json_payload_from_value_slots(values: &[ValueSlot]) -> RequestResult<Vec<u8>> {
    match values {
        [] => Ok(b"null".to_vec()),
        [value] => json_bytes_from_value(value),
        _ => Err(RequestError::Unsupported(format!(
            "scalar bytecode VM returned {} results; expected zero or one",
            values.len()
        ))),
    }
}

fn json_bytes_from_value(value: &ValueSlot) -> RequestResult<Vec<u8>> {
    if value.is_null() {
        return Ok(b"null".to_vec());
    }
    if let Some(boolean) = value.as_bool() {
        return Ok(if boolean {
            b"true".to_vec()
        } else {
            b"false".to_vec()
        });
    }
    if let Some(number) = value.as_number() {
        let number = serde_json::Number::from_f64(number).ok_or_else(|| {
            RequestError::Unsupported(format!(
                "scalar bytecode VM returned a non-JSON number: {number}"
            ))
        })?;
        return Ok(number.to_string().into_bytes());
    }
    if value.as_integer().is_some() {
        return Err(RequestError::Unsupported(
            "integer results are not supported by bytecode scalar JSON ingress yet".to_string(),
        ));
    }
    if value.as_date().is_some() {
        return Err(RequestError::Unsupported(
            "Date results are not supported by bytecode scalar JSON ingress yet".to_string(),
        ));
    }
    Err(RequestError::Unsupported(format!(
        "scalar bytecode VM returned unsupported value kind {:?}",
        value.kind()
    )))
}

fn http_response_from_vm_values(
    heap: &mut dyn VmHeap,
    values: &[ValueSlot],
) -> RequestResult<BoundaryResponse> {
    let [record] = values else {
        return Err(RequestError::Unsupported(
            "HTTP gateway VM must return exactly one HTTP response record".to_string(),
        ));
    };
    let status_slot = heap
        .record_field(record, "status")
        .map_err(heap_error_to_request_error)?;
    let status = http_status_from_vm_slot(&status_slot).ok_or_else(|| {
        RequestError::Unsupported("HTTP gateway response status must be an integer".to_string())
    })?;
    let headers = http_headers_from_vm(heap, record, "headers")?
        .into_iter()
        .map(|header| HttpNameValue {
            name: header.name,
            value: header.value,
        })
        .collect();
    let body_slot = heap
        .record_field(record, "body")
        .map_err(heap_error_to_request_error)?;
    let body = heap
        .bytes_value(&body_slot)
        .map_err(heap_error_to_request_error)?;
    Ok(BoundaryResponse::http(
        body,
        HttpResponseMetadata::new(status, headers),
    ))
}

fn http_stream_event_from_vm_values(
    heap: &mut dyn VmHeap,
    values: &[ValueSlot],
) -> RequestResult<HttpBoundaryResponseStreamEvent> {
    let [record] = values else {
        return Err(RequestError::Unsupported(
            "HTTP stream VM must emit exactly one stream event record".to_string(),
        ));
    };
    let tag_slot = heap
        .record_field(record, "tag")
        .map_err(heap_error_to_request_error)?;
    let tag = heap
        .string_value(&tag_slot)
        .map_err(heap_error_to_request_error)?;
    match tag.as_str() {
        "start" => {
            let status_slot = heap
                .record_field(record, "status")
                .map_err(heap_error_to_request_error)?;
            let status = http_status_from_vm_slot(&status_slot).ok_or_else(|| {
                RequestError::Unsupported("HTTP stream start status must be an integer".to_string())
            })?;
            let headers = http_headers_from_vm(heap, record, "headers")?;
            Ok(HttpBoundaryResponseStreamEvent::Start { status, headers })
        }
        "chunk" => {
            let value_slot = heap
                .record_field(record, "value")
                .map_err(heap_error_to_request_error)?;
            let bytes = heap
                .bytes_value(&value_slot)
                .map_err(heap_error_to_request_error)?;
            Ok(HttpBoundaryResponseStreamEvent::Chunk(bytes))
        }
        "end" => Ok(HttpBoundaryResponseStreamEvent::End),
        _ => Err(RequestError::Unsupported(format!(
            "unsupported HTTP stream event tag {tag}"
        ))),
    }
}

fn http_status_from_vm_slot(slot: &ValueSlot) -> Option<u16> {
    slot.as_integer()
        .and_then(|value| u16::try_from(value).ok())
        .or_else(|| {
            slot.as_number().and_then(|value| {
                (value.fract() == 0.0 && (0.0..=65535.0).contains(&value)).then_some(value as u16)
            })
        })
}

fn http_headers_from_vm(
    heap: &mut dyn VmHeap,
    record: &ValueSlot,
    field: &str,
) -> RequestResult<Vec<HttpBoundaryNameValue>> {
    let headers_slot = heap
        .record_field(record, field)
        .map_err(heap_error_to_request_error)?;
    let header_count = heap
        .array_len(&headers_slot)
        .map_err(heap_error_to_request_error)?;
    let mut headers = Vec::with_capacity(header_count);
    for index in 0..header_count {
        let header = heap
            .array_get(&headers_slot, index)
            .map_err(heap_error_to_request_error)?;
        let name = heap
            .record_field(&header, "name")
            .and_then(|slot| heap.string_value(&slot))
            .map_err(heap_error_to_request_error)?;
        let value = heap
            .record_field(&header, "value")
            .and_then(|slot| heap.string_value(&slot))
            .map_err(heap_error_to_request_error)?;
        headers.push(HttpBoundaryNameValue { name, value });
    }
    Ok(headers)
}

fn vm_error_to_request_error(execution_budget: &ExecutionBudget, error: VmError) -> RequestError {
    match error {
        VmError::BudgetClosed(error) => vm_budget_closed_to_request_error(execution_budget, error),
        VmError::InternalTerminal(VmInternalTerminal::Budget(error)) => {
            vm_budget_closed_to_request_error(execution_budget, error)
        }
        VmError::InternalTerminal(VmInternalTerminal::OwnerStopped) => RequestError::Cancelled,
        error => RequestError::Unsupported(format!("bytecode VM execution failed: {error}")),
    }
}

fn vm_budget_closed_to_request_error(
    execution_budget: &ExecutionBudget,
    error: VmBudgetClosed,
) -> RequestError {
    let stats = execution_budget.stats_snapshot();
    match error {
        VmBudgetClosed::AlreadySettled(
            VmBudgetTerminal::Succeeded
            | VmBudgetTerminal::Failed
            | VmBudgetTerminal::Cancelled
            | VmBudgetTerminal::InternalStop,
        ) => RequestError::Cancelled,
        VmBudgetClosed::DeadlineExceeded
        | VmBudgetClosed::AlreadySettled(VmBudgetTerminal::DeadlineExceeded) => {
            RequestError::ExecutionBudgetExceeded {
                reason: ExecutionBudgetReason::DeadlineExceeded,
                instruction_count: stats.instruction_count,
                limit: stats.budget_limit,
                elapsed_ms: stats.elapsed_ms,
            }
        }
        VmBudgetClosed::InstructionLimitExceeded
        | VmBudgetClosed::AlreadySettled(VmBudgetTerminal::InstructionLimitExceeded) => {
            RequestError::ExecutionBudgetExceeded {
                reason: ExecutionBudgetReason::InstructionLimitExceeded,
                instruction_count: stats.instruction_count,
                limit: stats.budget_limit,
                elapsed_ms: stats.elapsed_ms,
            }
        }
        VmBudgetClosed::AccountingFailure
        | VmBudgetClosed::AlreadySettled(VmBudgetTerminal::AccountingFailure) => {
            RequestError::Unsupported(format!(
                "bytecode VM budget accounting failed closed: {error}"
            ))
        }
    }
}

fn vm_limits() -> VmLimits {
    VmLimits::new(
        NonZeroUsize::new(128).expect("VM frame limit is non-zero"),
        NonZeroUsize::new(4096).expect("VM value slot limit is non-zero"),
        NonZeroU32::new(1024).expect("VM segment instruction limit is non-zero"),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use skiff_artifact_model::{IngressProtocol, IngressSelector};
    use skiff_runtime_model::vm_value::ValueSlot;

    use skiff_runtime_model::{vm_heap::VmHeapError, vm_root::VmRootVisitor};
    use skiff_runtime_scheduler::{
        BytecodeAdapterHandoff, BytecodeChildExecutor, BytecodeChildStart, BytecodeControl,
        BytecodeStreamHandoff, RootDisposition, RootEscrow, RootEscrowBacking,
    };

    use super::*;
    use crate::{
        BinaryHttpRequest, BinaryHttpRequestMetadata, HttpAdapter, HttpAdapterCallable,
        HttpAdapterKind, RequestEnvelope, ResponseEnd, ResponseEvent,
    };

    type TestControl = BytecodeControl<usize, usize, usize, usize, usize>;
    type TestSuspended = SuspendedTrampoline<TestUnit, usize>;
    const ERROR_OUTCOME: usize = usize::MAX;

    fn test_child_registration() -> skiff_runtime_scheduler::ChildOwnerRegistration {
        let (registrations, _freeze) = RequestExecutionOwnerInventory::open().into_parts();
        registrations.child()
    }

    #[derive(Debug)]
    struct TestUnit {
        control: Option<TestControl>,
        resumed: Option<(usize, usize)>,
        finish_after_resume: Option<usize>,
    }

    impl TestUnit {
        fn parked(operation: usize) -> Self {
            Self {
                control: Some(TestControl::Park(operation)),
                resumed: None,
                finish_after_resume: None,
            }
        }

        fn emit(item: usize, finish: usize) -> Self {
            Self {
                control: Some(TestControl::EmitStream(item)),
                resumed: None,
                finish_after_resume: Some(finish),
            }
        }
    }

    impl VmRootSource for TestUnit {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl BytecodeUnit for TestUnit {
        type ResumeToken = usize;
        type ResumeOutcome = usize;
        type RootResult = usize;
        type ChildInvocation = usize;
        type AdapterInvocation = usize;
        type StreamItem = usize;
        type PendingOperation = usize;

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> TestControl {
            if let Some((_, outcome)) = self.resumed.take() {
                TestControl::Complete(self.finish_after_resume.take().unwrap_or(outcome))
            } else if let Some(control) = self.control.take() {
                control
            } else {
                TestControl::Complete(0)
            }
        }

        fn resume(&mut self, token: usize, outcome: usize) -> Result<(), BytecodeSchedulerError> {
            if outcome == ERROR_OUTCOME {
                return Err(BytecodeSchedulerError::UnsupportedPark);
            }
            self.resumed = Some((token, outcome));
            Ok(())
        }

        fn child_completion_to_resume_outcome(completed: usize) -> usize {
            completed
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum TestStreamMode {
        Backpressure,
        Ready,
    }

    struct FakeStream {
        mode: TestStreamMode,
        emitted: Mutex<Vec<usize>>,
        parked: Mutex<Option<(usize, TestSuspended)>>,
        delivered: Mutex<Vec<usize>>,
        terminal: std::sync::atomic::AtomicBool,
    }

    impl FakeStream {
        fn new(mode: TestStreamMode) -> Self {
            Self {
                mode,
                emitted: Mutex::new(Vec::new()),
                parked: Mutex::new(None),
                delivered: Mutex::new(Vec::new()),
                terminal: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }

    impl BytecodeStreamSupervisor<TestUnit> for FakeStream {
        fn emit_stream_handoff(
            &self,
            item: usize,
            _depth: usize,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeStreamHandoff<TestUnit>, BytecodeSchedulerError> {
            self.emitted.lock().unwrap().push(item);
            match self.mode {
                TestStreamMode::Backpressure => Ok(BytecodeStreamHandoff::Pending(item)),
                TestStreamMode::Ready => Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    resume: item,
                    outcome: 0,
                })),
            }
        }

        fn park(
            &self,
            operation: usize,
            suspended: TestSuspended,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<(), BytecodeSchedulerError> {
            *self.parked.lock().unwrap() = Some((operation, suspended));
            Ok(())
        }

        fn finish_stream(
            &self,
            _depth: usize,
            _result: &usize,
        ) -> Result<(), BytecodeSchedulerError> {
            let emitted = !self.emitted.lock().unwrap().is_empty();
            let delivered = !self.delivered.lock().unwrap().is_empty();
            if emitted || delivered {
                self.terminal.store(true, Ordering::Release);
            }
            Ok(())
        }
    }

    struct TestDrain {
        stream: Arc<FakeStream>,
    }

    impl BytecodeRequestStreamDrain<TestUnit> for TestDrain {
        fn drain(&mut self, _heap: &mut dyn VmHeap) -> RequestResult<BytecodeRequestDrainState> {
            let items: Vec<usize> = self.stream.emitted.lock().unwrap().drain(..).collect();
            self.stream
                .delivered
                .lock()
                .unwrap()
                .extend(items.iter().copied());
            if self.stream.terminal.load(Ordering::Acquire) {
                Ok(BytecodeRequestDrainState::Terminal)
            } else if !items.is_empty() {
                Ok(BytecodeRequestDrainState::Delivered)
            } else {
                Ok(BytecodeRequestDrainState::Empty)
            }
        }
    }

    struct EmptyRoots;

    impl VmRootSource for EmptyRoots {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl RootEscrowBacking for EmptyRoots {
        fn root_count(&self) -> usize {
            0
        }

        fn restore_roots(self: Box<Self>) {}

        fn drop_roots(self: Box<Self>, _disposition: RootDisposition) {}
    }

    struct TestResume {
        resume: usize,
        suspended: TestSuspended,
        outcome: usize,
        escrow: RootEscrow,
    }

    impl BytecodeRequestResume<TestUnit> for TestResume {
        fn into_scheduler(
            self,
            ports: BytecodeSchedulerPorts<TestUnit>,
        ) -> Result<BytecodeScheduler<TestUnit>, BytecodeSchedulerError> {
            BytecodeScheduler::resume_from_suspended(
                self.suspended,
                self.resume,
                self.outcome,
                self.escrow,
                ports,
            )
        }
    }

    struct NoopHeap;

    impl VmHeap for NoopHeap {
        fn validate_live(&self, _value: &ValueSlot) -> Result<(), VmHeapError> {
            Ok(())
        }

        fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
            Ok(*source)
        }

        fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
            Ok(*source)
        }

        fn release_snapshot(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
            Ok(())
        }

        fn release_resource(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    struct NoopBudget;

    impl VmBudget for NoopBudget {
        fn before_dispatch(&mut self) -> Result<(), VmBudgetClosed> {
            Ok(())
        }

        fn poll_interrupt(&mut self) -> Result<(), VmBudgetClosed> {
            Ok(())
        }

        fn charge_semantic(
            &mut self,
            _charge: skiff_runtime_vm::VmSemanticCharge<'_>,
        ) -> Result<(), VmBudgetClosed> {
            Ok(())
        }
    }

    fn driver_for(unit: TestUnit, stream: Arc<FakeStream>) -> BytecodeRequestDriver<TestUnit> {
        let queue = Arc::new(InMemoryWakeQueue::new());
        let supervisor: Arc<dyn BytecodeStreamSupervisor<TestUnit>> = stream.clone();
        let scheduler = BytecodeScheduler::new(
            unit,
            BytecodeSchedulerPorts {
                child_executor: None,
                stream_supervisor: Some(supervisor.clone()),
            },
            test_child_registration(),
        );
        let heap: Box<dyn VmHeap + Send> = Box::new(NoopHeap);
        let budget: Box<dyn VmBudget + Send> = Box::new(NoopBudget);
        let error_map: Box<dyn Fn(BytecodeSchedulerError) -> RequestError + Send + Sync> =
            Box::new(|error| RequestError::Decode(error.to_string()));
        let drain = Box::new(TestDrain {
            stream: Arc::clone(&stream),
        });
        BytecodeRequestDriver::new(
            scheduler,
            None,
            Some(supervisor),
            Some(drain),
            queue,
            heap,
            budget,
            error_map,
        )
    }

    fn resume_after_park(
        driver: &mut BytecodeRequestDriver<TestUnit>,
        stream: &FakeStream,
        outcome: usize,
    ) -> RequestResult<BytecodeRequestDriverOutcome<TestUnit>> {
        let (operation, suspended) = stream.parked.lock().unwrap().take().expect("parked unit");
        let resume = TestResume {
            resume: operation,
            suspended,
            outcome,
            escrow: RootEscrow::new(Box::new(EmptyRoots)),
        };
        driver.resume(resume)
    }

    #[test]
    fn parked_request_can_resume() {
        let stream = Arc::new(FakeStream::new(TestStreamMode::Ready));
        let mut driver = driver_for(TestUnit::parked(7), Arc::clone(&stream));

        assert!(matches!(
            driver.run().unwrap(),
            BytecodeRequestDriverOutcome::Parked
        ));
        match resume_after_park(&mut driver, &stream, 42).unwrap() {
            BytecodeRequestDriverOutcome::Complete {
                result,
                stream_sent,
            } => {
                assert_eq!(result, 42);
                assert!(!stream_sent);
            }
            BytecodeRequestDriverOutcome::Parked => panic!("resume must complete"),
        }
    }

    #[test]
    fn backpressure_item_delivery_resumes_with_zero_result() {
        let stream = Arc::new(FakeStream::new(TestStreamMode::Backpressure));
        let mut driver = driver_for(TestUnit::emit(7, 99), Arc::clone(&stream));

        assert!(matches!(
            driver.run().unwrap(),
            BytecodeRequestDriverOutcome::Parked
        ));
        assert_eq!(*stream.delivered.lock().unwrap(), [7]);
        match resume_after_park(&mut driver, &stream, 0).unwrap() {
            BytecodeRequestDriverOutcome::Complete {
                result,
                stream_sent,
            } => {
                assert_eq!(result, 99);
                assert!(stream_sent);
            }
            BytecodeRequestDriverOutcome::Parked => panic!("zero-result resume must complete"),
        }
        assert_eq!(*stream.delivered.lock().unwrap(), [7]);
        assert!(stream.terminal.load(Ordering::Acquire));
    }

    #[test]
    fn error_resume_fails_closed() {
        let stream = Arc::new(FakeStream::new(TestStreamMode::Backpressure));
        let mut driver = driver_for(TestUnit::emit(7, 99), Arc::clone(&stream));

        assert!(matches!(
            driver.run().unwrap(),
            BytecodeRequestDriverOutcome::Parked
        ));
        let error = match resume_after_park(&mut driver, &stream, ERROR_OUTCOME) {
            Err(error) => error,
            Ok(_) => panic!("error resume must fail closed"),
        };
        assert!(error.to_string().contains("park"));
        assert!(matches!(
            driver.run(),
            Err(RequestError::Decode(message)) if message.contains("failed closed")
        ));
    }

    #[test]
    fn natural_end_completes_response_stream() {
        let stream = Arc::new(FakeStream::new(TestStreamMode::Ready));
        let mut driver = driver_for(TestUnit::emit(7, 99), Arc::clone(&stream));

        match driver.run().unwrap() {
            BytecodeRequestDriverOutcome::Complete {
                result,
                stream_sent,
            } => {
                assert_eq!(result, 99);
                assert!(stream_sent);
            }
            BytecodeRequestDriverOutcome::Parked => panic!("ready emission must not park"),
        }
        assert_eq!(*stream.delivered.lock().unwrap(), [7]);
        assert!(stream.terminal.load(Ordering::Acquire));
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum StreamNextResume {
        Item,
        End,
        Pending,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum StreamNextOutcome {
        Item(usize),
        End,
        Failure(&'static str),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum StreamNextInvocation {
        StreamNext {
            item_resume: StreamNextResume,
            end_resume: StreamNextResume,
        },
    }

    type StreamNextControl =
        BytecodeControl<StreamNextOutcome, StreamNextInvocation, usize, usize, StreamNextResume>;
    type StreamNextSuspended = SuspendedTrampoline<StreamNextUnit, StreamNextResume>;

    struct StreamNextUnit {
        invocation: Option<StreamNextInvocation>,
        resumed: Option<(StreamNextResume, StreamNextOutcome)>,
    }

    impl StreamNextUnit {
        fn stream_next() -> Self {
            Self {
                invocation: Some(StreamNextInvocation::StreamNext {
                    item_resume: StreamNextResume::Item,
                    end_resume: StreamNextResume::End,
                }),
                resumed: None,
            }
        }
    }

    impl VmRootSource for StreamNextUnit {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl BytecodeUnit for StreamNextUnit {
        type ResumeToken = StreamNextResume;
        type ResumeOutcome = StreamNextOutcome;
        type RootResult = StreamNextOutcome;
        type ChildInvocation = StreamNextInvocation;
        type AdapterInvocation = usize;
        type StreamItem = usize;
        type PendingOperation = StreamNextResume;

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> StreamNextControl {
            if let Some((_, outcome)) = self.resumed.take() {
                StreamNextControl::Complete(outcome)
            } else if let Some(invocation) = self.invocation.take() {
                StreamNextControl::EnterChild(invocation)
            } else {
                StreamNextControl::Complete(StreamNextOutcome::Item(0))
            }
        }

        fn resume(
            &mut self,
            token: StreamNextResume,
            outcome: StreamNextOutcome,
        ) -> Result<(), BytecodeSchedulerError> {
            self.resumed = Some((token, outcome));
            Ok(())
        }

        fn child_completion_to_resume_outcome(completed: StreamNextOutcome) -> StreamNextOutcome {
            completed
        }

        fn is_stream_next_child(invocation: &StreamNextInvocation) -> bool {
            matches!(invocation, StreamNextInvocation::StreamNext { .. })
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum StreamNextExecutorMode {
        Item,
        End,
        Error,
        Pending,
    }

    struct StreamNextExecutor {
        mode: StreamNextExecutorMode,
        pending: Mutex<Option<(StreamNextResume, StreamNextSuspended)>>,
    }

    impl BytecodeChildExecutor<StreamNextUnit> for StreamNextExecutor {
        fn execute_child(
            &self,
            _invocation: StreamNextInvocation,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeChildStart<StreamNextUnit>, BytecodeSchedulerError> {
            Err(BytecodeSchedulerError::UnsupportedChild)
        }

        fn execute_adapter(
            &self,
            _invocation: usize,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeAdapterHandoff<StreamNextUnit>, BytecodeSchedulerError> {
            Err(BytecodeSchedulerError::UnsupportedAdapter)
        }

        fn execute_stream_next(
            &self,
            invocation: StreamNextInvocation,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeStreamHandoff<StreamNextUnit>, BytecodeSchedulerError> {
            let StreamNextInvocation::StreamNext {
                item_resume,
                end_resume,
            } = invocation;
            match self.mode {
                StreamNextExecutorMode::Item => Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    resume: item_resume,
                    outcome: StreamNextOutcome::Item(7),
                })),
                StreamNextExecutorMode::End => Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    resume: end_resume,
                    outcome: StreamNextOutcome::End,
                })),
                StreamNextExecutorMode::Error => {
                    Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                        resume: item_resume,
                        outcome: StreamNextOutcome::Failure("stream failed"),
                    }))
                }
                StreamNextExecutorMode::Pending => {
                    Ok(BytecodeStreamHandoff::Pending(StreamNextResume::Pending))
                }
            }
        }

        fn park_stream_next(
            &self,
            operation: StreamNextResume,
            suspended: StreamNextSuspended,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<(), BytecodeSchedulerError> {
            *self.pending.lock().unwrap() = Some((operation, suspended));
            Ok(())
        }
    }

    struct StreamNextResumeSource {
        resume: StreamNextResume,
        suspended: StreamNextSuspended,
        outcome: StreamNextOutcome,
        escrow: RootEscrow,
    }

    impl BytecodeRequestResume<StreamNextUnit> for StreamNextResumeSource {
        fn into_scheduler(
            self,
            ports: BytecodeSchedulerPorts<StreamNextUnit>,
        ) -> Result<BytecodeScheduler<StreamNextUnit>, BytecodeSchedulerError> {
            BytecodeScheduler::resume_from_suspended(
                self.suspended,
                self.resume,
                self.outcome,
                self.escrow,
                ports,
            )
        }
    }

    fn stream_next_driver(
        executor: Arc<StreamNextExecutor>,
    ) -> BytecodeRequestDriver<StreamNextUnit> {
        let queue = Arc::new(InMemoryWakeQueue::new());
        let executor_dyn: Arc<dyn BytecodeChildExecutor<StreamNextUnit>> = executor.clone();
        let scheduler = BytecodeScheduler::new(
            StreamNextUnit::stream_next(),
            BytecodeSchedulerPorts {
                child_executor: Some(executor_dyn.clone()),
                stream_supervisor: None,
            },
            test_child_registration(),
        );
        let heap: Box<dyn VmHeap + Send> = Box::new(NoopHeap);
        let budget: Box<dyn VmBudget + Send> = Box::new(NoopBudget);
        let error_map: Box<dyn Fn(BytecodeSchedulerError) -> RequestError + Send + Sync> =
            Box::new(|error| RequestError::Decode(error.to_string()));
        BytecodeRequestDriver::new(
            scheduler,
            Some(executor_dyn),
            None,
            None,
            queue,
            heap,
            budget,
            error_map,
        )
    }

    #[test]
    fn stream_next_item_uses_item_continuation() {
        let executor = Arc::new(StreamNextExecutor {
            mode: StreamNextExecutorMode::Item,
            pending: Mutex::new(None),
        });
        let mut driver = stream_next_driver(executor);

        match driver.run().unwrap() {
            BytecodeRequestDriverOutcome::Complete {
                result,
                stream_sent,
            } => {
                assert_eq!(result, StreamNextOutcome::Item(7));
                assert!(!stream_sent);
            }
            BytecodeRequestDriverOutcome::Parked => panic!("item path must not park"),
        }
    }

    #[test]
    fn stream_next_end_uses_end_continuation() {
        let executor = Arc::new(StreamNextExecutor {
            mode: StreamNextExecutorMode::End,
            pending: Mutex::new(None),
        });
        let mut driver = stream_next_driver(executor);

        match driver.run().unwrap() {
            BytecodeRequestDriverOutcome::Complete {
                result,
                stream_sent,
            } => {
                assert_eq!(result, StreamNextOutcome::End);
                assert!(!stream_sent);
            }
            BytecodeRequestDriverOutcome::Parked => panic!("end path must not park"),
        }
    }

    #[test]
    fn stream_next_pending_wake_end_resumes_end_continuation() {
        let executor = Arc::new(StreamNextExecutor {
            mode: StreamNextExecutorMode::Pending,
            pending: Mutex::new(None),
        });
        let mut driver = stream_next_driver(Arc::clone(&executor));

        assert!(matches!(
            driver.run().unwrap(),
            BytecodeRequestDriverOutcome::Parked
        ));
        let (operation, suspended) = executor
            .pending
            .lock()
            .unwrap()
            .take()
            .expect("pending stream next park");
        let resume = StreamNextResumeSource {
            resume: operation,
            suspended,
            outcome: StreamNextOutcome::End,
            escrow: RootEscrow::new(Box::new(EmptyRoots)),
        };

        match driver.resume(resume).unwrap() {
            BytecodeRequestDriverOutcome::Complete {
                result,
                stream_sent,
            } => {
                assert_eq!(result, StreamNextOutcome::End);
                assert!(!stream_sent);
            }
            BytecodeRequestDriverOutcome::Parked => panic!("end wake must complete"),
        }
    }

    #[test]
    fn stream_next_error_resume_fails_closed() {
        let executor = Arc::new(StreamNextExecutor {
            mode: StreamNextExecutorMode::Error,
            pending: Mutex::new(None),
        });
        let mut driver = stream_next_driver(executor);

        match driver.run().unwrap() {
            BytecodeRequestDriverOutcome::Complete {
                result,
                stream_sent,
            } => {
                assert_eq!(result, StreamNextOutcome::Failure("stream failed"));
                assert!(!stream_sent);
            }
            BytecodeRequestDriverOutcome::Parked => panic!("error path must complete"),
        }
    }

    #[test]
    fn json_payload_encodes_scalar_immediates() {
        assert_eq!(json_payload_from_value_slots(&[]).unwrap(), b"null");
        assert_eq!(
            json_payload_from_value_slots(&[ValueSlot::null()]).unwrap(),
            b"null"
        );
        assert_eq!(
            json_payload_from_value_slots(&[ValueSlot::bool(true)]).unwrap(),
            b"true"
        );
        assert_eq!(
            json_payload_from_value_slots(&[ValueSlot::bool(false)]).unwrap(),
            b"false"
        );
        assert_eq!(
            json_payload_from_value_slots(&[ValueSlot::number(1.5)]).unwrap(),
            b"1.5"
        );
    }

    #[test]
    fn json_payload_rejects_unsupported_results() {
        assert!(json_payload_from_value_slots(&[ValueSlot::integer(1)]).is_err());
        assert!(json_payload_from_value_slots(&[ValueSlot::date(1)]).is_err());
        assert!(
            json_payload_from_value_slots(&[ValueSlot::null(), ValueSlot::bool(true)]).is_err()
        );
        assert!(json_payload_from_value_slots(&[ValueSlot::number(f64::NAN)]).is_err());
    }

    #[test]
    fn sleep_millis_clamps_non_positive_and_max_duration() {
        assert_eq!(clamp_sleep_millis(-1), 0);
        assert_eq!(clamp_sleep_millis(0), 0);
        assert_eq!(clamp_sleep_millis(42), 42);
        assert_eq!(
            clamp_sleep_millis(TIME_SLEEP_MAX_MILLIS as i64 + 1),
            TIME_SLEEP_MAX_MILLIS
        );
    }

    #[test]
    fn example_test_http_response_matches_happy_fixture_shapes() {
        assert_eq!(
            example_test_http_response("https://example.test/from-entry"),
            (200, b"double-body".to_vec())
        );
        assert_eq!(
            example_test_http_response("https://example.test/direct"),
            (202, b"direct-double".to_vec())
        );
        assert_eq!(
            example_test_http_response("https://example.test/other"),
            (200, b"response".to_vec())
        );
    }

    #[test]
    fn http_response_from_vm_values_materializes_metadata_and_body() {
        let mut heap = RequestVmHeap::new(RequestHeapLimits::default());
        let name = heap.alloc_string("content-type".to_string()).unwrap();
        let value = heap.alloc_string("text/plain".to_string()).unwrap();
        let header = heap
            .allocate_record(
                &[
                    VmRecordField {
                        name: "name".to_string(),
                        value: name,
                    },
                    VmRecordField {
                        name: "value".to_string(),
                        value,
                    },
                ],
                CompactTypeTag::new(0),
                ValueFlags::new(0),
            )
            .unwrap();
        let headers = heap
            .allocate_array(&[header], CompactTypeTag::new(0), ValueFlags::new(0))
            .unwrap();
        let body = heap.alloc_bytes(b"ok".to_vec()).unwrap();
        let response = heap
            .allocate_record(
                &[
                    VmRecordField {
                        name: "status".to_string(),
                        value: ValueSlot::number(201.0),
                    },
                    VmRecordField {
                        name: "headers".to_string(),
                        value: headers,
                    },
                    VmRecordField {
                        name: "body".to_string(),
                        value: body,
                    },
                ],
                CompactTypeTag::new(0),
                ValueFlags::new(0),
            )
            .unwrap();

        let boundary = http_response_from_vm_values(&mut heap, &[response]).unwrap();
        let BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Http { payload, metadata })) =
            boundary
        else {
            panic!("HTTP response conversion returned {boundary:?}");
        };
        assert_eq!(metadata.status, 201);
        assert_eq!(metadata.headers[0].name, "content-type");
        assert_eq!(metadata.headers[0].value, "text/plain");
        assert_eq!(payload, b"ok");
    }

    #[test]
    fn validation_requires_unary_and_canonical_selector() {
        assert!(validate_bytecode_request(&request()).is_ok());

        let mut selector_request = request();
        selector_request.ingress_selector = None;
        let error = validate_bytecode_request(&selector_request).expect_err("selector is required");
        assert!(error.to_string().contains("ingress_selector"));

        let mut mode_request = request();
        mode_request.mode = "serverStream".to_string();
        let error = validate_bytecode_request(&mode_request).expect_err("mode is validated");
        assert!(error.to_string().contains("unary"));
    }

    #[test]
    fn validation_rejects_unsupported_ingress_metadata() {
        let mut binary_request = request();
        binary_request.binary_http = Some(BinaryHttpRequest {
            metadata: BinaryHttpRequestMetadata {
                method: "GET".to_string(),
                url: "http://example.test/entry".to_string(),
                path: "/entry".to_string(),
                query: Vec::new(),
                headers: Vec::new(),
            },
            body: Vec::new(),
        });
        assert!(validate_bytecode_request(&binary_request).is_ok());

        let mut adapter_request = request();
        adapter_request.http_adapter = Some(HttpAdapter {
            kind: HttpAdapterKind::TypedJson,
            handler: HttpAdapterCallable::ServiceFunction {
                module_path: "main".to_string(),
                symbol: "run".to_string(),
            },
            guard: None,
            pre: None,
            adapter_args: Vec::new(),
        });
        assert!(validate_bytecode_request(&adapter_request).is_ok());

        let mut actor_request = request();
        actor_request
            .extra
            .insert("actorCall".to_string(), serde_json::json!({}));
        assert!(validate_bytecode_request(&actor_request).is_err());
    }

    #[test]
    fn scheduler_fail_closed_errors_map_to_unsupported() {
        let budget = ExecutionBudget::for_runtime_request(None);
        assert!(matches!(
            scheduler_error_to_request_error(&budget, BytecodeSchedulerError::UnsupportedChild),
            RequestError::Unsupported(message) if message.contains("child executor port")
        ));
        assert!(matches!(
            scheduler_error_to_request_error(&budget, BytecodeSchedulerError::UnsupportedAdapter),
            RequestError::Unsupported(message) if message.contains("child executor port")
        ));
        assert!(matches!(
            scheduler_error_to_request_error(&budget, BytecodeSchedulerError::UnsupportedStream),
            RequestError::Unsupported(message) if message.contains("stream supervisor")
        ));
        assert!(matches!(
            scheduler_error_to_request_error(&budget, BytecodeSchedulerError::UnsupportedPark),
            RequestError::Unsupported(message) if message.contains("stream supervisor")
        ));
        assert!(matches!(
            scheduler_error_to_request_error(
                &budget,
                BytecodeSchedulerError::ChildCapacityExceeded
            ),
            RequestError::Decode(message) if message == "bytecode scheduler blocked child capacity is exhausted"
        ));
        assert!(matches!(
            scheduler_error_to_request_error(
                &budget,
                BytecodeSchedulerError::ChildOwnerCreation(
                    skiff_runtime_scheduler::OwnerCreationError::InventoryFrozen,
                ),
            ),
            RequestError::Decode(message)
                if message == "bytecode scheduler child owner creation failed: request owner inventory is frozen"
        ));
    }

    fn request() -> RequestEnvelope {
        RequestEnvelope {
            request_id: "bytecode-request".to_string(),
            mode: "unary".to_string(),
            target: "display-only".to_string(),
            operation_abi_id: None,
            selector: None,
            service_id: None,
            build_id: "legacy-build".to_string(),
            service_protocol_identity: "legacy-protocol".to_string(),
            contract_identity: None,
            activation_identity: None,
            ingress_selector: Some(IngressSelector {
                protocol: IngressProtocol::Http,
                method: Some("POST".to_string()),
                path: "/entry".to_string(),
            }),
            binary_http: None,
            http_adapter: None,
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
            payload_bytes: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }
}
