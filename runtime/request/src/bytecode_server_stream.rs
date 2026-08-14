use std::{sync::Arc, task::Poll};

use skiff_artifact_model::{LiteralIr, TypeRefIr};
use skiff_runtime_linked_bytecode::{LinkedShapeEntry, ShapeIndex, TypeIndex};
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::{error::RuntimeErrorPayload, vm_heap::VmHeap, vm_value::ValueSlot};
use skiff_runtime_scheduler::{
    BytecodeHandoff, BytecodeSchedulerError, BytecodeStreamHandoff, BytecodeStreamSupervisor,
    PendingWakeQueue, RequestResourceHandle, RequestResourceTable, RequestResourceTermination,
    RequestServerStreamEventKind, RequestServerStreamPhase, RequestServerStreamReservation,
    RequestServerStreamReserveError,
};
use skiff_runtime_vm::{
    PendingOperation, ResumeOutcome, StreamItem, VmBudget, VmError, VmFiber, VmInternalTerminal,
    VmResult, VmResumeToken,
};

use crate::{
    bytecode_host_effects::{
        BytecodeServerStreamFrame, BytecodeServerStreamWriteFailure,
        SharedBytecodeServerStreamWriterPort,
    },
    bytecode_ingress::{
        array_element_type, poll_future_once, require_exact_slot_type_ref, required_slot_type,
        shape_field_type, validate_builtin_type, validate_record_carrier_fields,
        RequestPendingOutcome, RequestPendingRuntime, VmSuspended,
    },
    HttpNameValue,
};

enum DecodedServerStreamFrame {
    Start {
        status: u16,
        headers: Vec<HttpNameValue>,
    },
    Chunk(Vec<u8>),
    End,
}

impl DecodedServerStreamFrame {
    fn reservation_kind(&self) -> RequestServerStreamEventKind {
        match self {
            Self::Start { .. } => RequestServerStreamEventKind::Start,
            Self::Chunk(payload) => RequestServerStreamEventKind::Chunk {
                payload_bytes: payload.len(),
            },
            Self::End => RequestServerStreamEventKind::End,
        }
    }

    fn into_writer_frame(
        self,
        reservation: RequestServerStreamReservation,
    ) -> Result<BytecodeServerStreamFrame, BytecodeSchedulerError> {
        match self {
            Self::Start { status, headers } => {
                if reservation.sequence().is_some() {
                    return Err(BytecodeSchedulerError::Port(
                        "server-stream start unexpectedly received a chunk sequence".to_string(),
                    ));
                }
                Ok(BytecodeServerStreamFrame::Start { status, headers })
            }
            Self::Chunk(payload) => Ok(BytecodeServerStreamFrame::Chunk {
                sequence: reservation.sequence().ok_or_else(|| {
                    BytecodeSchedulerError::Port(
                        "server-stream chunk is missing its table-owned sequence".to_string(),
                    )
                })?,
                payload,
            }),
            Self::End => {
                if reservation.sequence().is_some() {
                    return Err(BytecodeSchedulerError::Port(
                        "server-stream end unexpectedly received a chunk sequence".to_string(),
                    ));
                }
                Ok(BytecodeServerStreamFrame::End)
            }
        }
    }
}

fn decode_server_stream_headers(
    image: &DeploymentExecutionImage,
    headers_type: TypeIndex,
    headers: &ValueSlot,
    heap: &mut dyn VmHeap,
) -> Result<Vec<HttpNameValue>, BytecodeSchedulerError> {
    let header_type = array_element_type(image, headers_type)?;
    // NewArrayBuilder stores the exact element TypeIndex in the array slot.
    // Equal ABI rows may have different image-local indices, so compare their
    // complete linked TypeRef rather than requiring index identity.
    require_exact_slot_type_ref(image, headers, header_type, "server-stream headers")?;
    let count = heap
        .array_len(headers)
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut decoded = Vec::with_capacity(count);
    for index in 0..count {
        let header = heap
            .array_get(headers, index)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        require_exact_slot_type_ref(image, &header, header_type, "server-stream header")?;
        validate_record_carrier_fields(heap, &header, &["name", "value"], "server-stream header")?;
        let name = heap
            .record_field(&header, "name")
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        validate_builtin_type(
            image,
            required_slot_type(&name, "server-stream header name")?,
            "string",
        )?;
        let name = heap
            .string_value(&name)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let value = heap
            .record_field(&header, "value")
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        validate_builtin_type(
            image,
            required_slot_type(&value, "server-stream header value")?,
            "string",
        )?;
        let value = heap
            .string_value(&value)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        decoded.push(HttpNameValue { name, value });
    }
    Ok(decoded)
}

fn validate_server_stream_tag_type(
    image: &DeploymentExecutionImage,
    shape: &LinkedShapeEntry,
    expected: &str,
) -> Result<(), BytecodeSchedulerError> {
    let tag_type = shape_field_type(shape, "tag")?;
    let entry = image
        .types()
        .get(tag_type.get() as usize)
        .filter(|entry| entry.index() == tag_type)
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(format!(
                "server-stream discriminator type {} is absent from the linked image",
                tag_type.get()
            ))
        })?;
    if !matches!(
        entry.type_ref(),
        TypeRefIr::Literal {
            value: LiteralIr::String { value }
        } if value == expected
    ) {
        return Err(BytecodeSchedulerError::Port(format!(
            "server-stream {expected:?} variant does not retain its exact linked discriminator"
        )));
    }
    Ok(())
}

fn exact_server_stream_variant_shape<'a>(
    image: &'a DeploymentExecutionImage,
    item_shape: ShapeIndex,
    expected_fields: &[&str],
) -> Result<&'a LinkedShapeEntry, BytecodeSchedulerError> {
    let shape = image
        .shapes()
        .get(item_shape.get() as usize)
        .filter(|shape| shape.index() == item_shape)
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(format!(
                "server-stream item references missing linked shape {}",
                item_shape.get()
            ))
        })?;
    if shape.fields().len() != expected_fields.len()
        || !shape
            .fields()
            .iter()
            .zip(expected_fields)
            .all(|(field, expected)| field.name() == *expected)
    {
        return Err(BytecodeSchedulerError::Port(format!(
            "server-stream item shape {} does not carry exact linked {:?} fields",
            item_shape.get(),
            expected_fields
        )));
    }
    Ok(shape)
}

fn decode_server_stream_frame(
    image: &DeploymentExecutionImage,
    item_type: TypeIndex,
    item_shape: ShapeIndex,
    values: &[ValueSlot],
    heap: &mut dyn VmHeap,
) -> Result<DecodedServerStreamFrame, BytecodeSchedulerError> {
    let [item] = values else {
        return Err(BytecodeSchedulerError::Port(
            "server-stream EmitStream must carry exactly one item".to_string(),
        ));
    };
    require_exact_slot_type_ref(image, item, item_type, "server-stream item")?;
    let shape = image
        .shapes()
        .get(item_shape.get() as usize)
        .filter(|shape| shape.index() == item_shape)
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(format!(
                "server-stream item references missing linked shape {}",
                item_shape.get()
            ))
        })?;
    require_exact_slot_type_ref(
        image,
        item,
        shape.nominal_type(),
        "server-stream item shape",
    )?;
    let tag_value = heap
        .record_field(item, "tag")
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    let tag = heap
        .string_value(&tag_value)
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    match tag.as_str() {
        "start" => {
            let shape = exact_server_stream_variant_shape(
                image,
                item_shape,
                &["headers", "status", "tag"],
            )?;
            validate_record_carrier_fields(
                heap,
                item,
                &["headers", "status", "tag"],
                "server-stream start item",
            )?;
            validate_server_stream_tag_type(image, shape, "start")?;
            require_exact_slot_type_ref(
                image,
                &tag_value,
                shape_field_type(shape, "tag")?,
                "server-stream start discriminator",
            )?;
            let status_type = shape_field_type(shape, "status")?;
            validate_builtin_type(image, status_type, "integer")?;
            let status = heap
                .record_field(item, "status")
                .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
            // Immediate scalar slots do not carry a linked type tag. Numeric
            // source constants use the VM's Number carrier even when their
            // linked field authority is the closed `integer` type.
            let status = status
                .as_integer()
                .and_then(|status| u16::try_from(status).ok())
                .or_else(|| {
                    status.as_number().and_then(|status| {
                        (status.is_finite()
                            && status.fract() == 0.0
                            && (100.0..=599.0).contains(&status))
                        .then_some(status as u16)
                    })
                })
                .filter(|status| (100..=599).contains(status))
                .ok_or_else(|| {
                    BytecodeSchedulerError::Port(
                        "server-stream start status is not a valid HTTP status".to_string(),
                    )
                })?;
            let headers_type = shape_field_type(shape, "headers")?;
            let headers = heap
                .record_field(item, "headers")
                .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
            let headers = decode_server_stream_headers(image, headers_type, &headers, heap)?;
            Ok(DecodedServerStreamFrame::Start { status, headers })
        }
        "chunk" => {
            let shape = exact_server_stream_variant_shape(image, item_shape, &["tag", "value"])?;
            validate_record_carrier_fields(
                heap,
                item,
                &["tag", "value"],
                "server-stream chunk item",
            )?;
            validate_server_stream_tag_type(image, shape, "chunk")?;
            require_exact_slot_type_ref(
                image,
                &tag_value,
                shape_field_type(shape, "tag")?,
                "server-stream chunk discriminator",
            )?;
            let value_type = shape_field_type(shape, "value")?;
            validate_builtin_type(image, value_type, "bytes")?;
            let value = heap
                .record_field(item, "value")
                .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
            require_exact_slot_type_ref(image, &value, value_type, "server-stream chunk")?;
            let payload = heap
                .bytes_value(&value)
                .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
            Ok(DecodedServerStreamFrame::Chunk(payload))
        }
        "end" => {
            let shape = exact_server_stream_variant_shape(image, item_shape, &["tag"])?;
            validate_record_carrier_fields(heap, item, &["tag"], "server-stream end item")?;
            validate_server_stream_tag_type(image, shape, "end")?;
            require_exact_slot_type_ref(
                image,
                &tag_value,
                shape_field_type(shape, "tag")?,
                "server-stream end discriminator",
            )?;
            Ok(DecodedServerStreamFrame::End)
        }
        _ => Err(BytecodeSchedulerError::Port(
            "server-stream item has an unknown linked discriminator".to_string(),
        )),
    }
}

fn prepare_root_server_stream_frame<T>(
    depth: usize,
    decode: impl FnOnce() -> Result<T, BytecodeSchedulerError>,
) -> Result<T, BytecodeSchedulerError> {
    if depth != 0 {
        return Err(BytecodeSchedulerError::Port(
            "server-stream response emission is restricted to the exact root producer".to_string(),
        ));
    }
    decode()
}

fn release_stream_item_after_decode<T>(
    prepared: Result<T, BytecodeSchedulerError>,
    item: StreamItem,
    heap: &mut dyn VmHeap,
) -> Result<(T, VmResumeToken), BytecodeSchedulerError> {
    let released = item.release(heap).map_err(BytecodeSchedulerError::from);
    match (prepared, released) {
        (Ok(prepared), Ok(resume)) => Ok((prepared, resume)),
        (Err(error), Ok(_resume)) => Err(error),
        (_, Err(release_error)) => Err(release_error),
    }
}

pub(super) struct BytecodeServerStreamSupervisor {
    runtime: Arc<RequestPendingRuntime>,
    handle: RequestResourceHandle,
    writer: SharedBytecodeServerStreamWriterPort,
}

impl BytecodeServerStreamSupervisor {
    pub(super) fn new(
        runtime: Arc<RequestPendingRuntime>,
        handle: RequestResourceHandle,
        writer: SharedBytecodeServerStreamWriterPort,
    ) -> Self {
        Self {
            runtime,
            handle,
            writer,
        }
    }

    fn reserve_failure_outcome(&self, error: RequestServerStreamReserveError) -> ResumeOutcome {
        let (termination, outcome) = match error {
            RequestServerStreamReserveError::ResponseLimitExceeded {
                limit_bytes,
                emitted_bytes,
                chunk_bytes,
            } => (
                RequestResourceTermination::ResponseLimitExceeded,
                ResumeOutcome::Failure(VmError::HostEffectFailure(RuntimeErrorPayload {
                    code: "ResourceLimitExceeded".to_string(),
                    message: format!("HTTP response exceeds max size of {limit_bytes} bytes"),
                    status: None,
                    details: Some(serde_json::json!({
                        "resource": "http.response",
                        "maxBytes": limit_bytes,
                        "emittedBytes": emitted_bytes,
                        "chunkBytes": chunk_bytes,
                    })),
                })),
            ),
            other => (
                RequestResourceTermination::HostError,
                resource_failure_outcome(other.to_string()),
            ),
        };
        match self.runtime.resources.terminate(&self.handle, termination) {
            Ok(_) => outcome,
            Err(error) => resource_failure_outcome(error.to_string()),
        }
    }
}

impl BytecodeStreamSupervisor<VmFiber> for BytecodeServerStreamSupervisor {
    fn emit_stream_handoff(
        &self,
        item: StreamItem,
        depth: usize,
        heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeStreamHandoff<VmFiber>, BytecodeSchedulerError> {
        let prepared = prepare_root_server_stream_frame(depth, || {
            decode_server_stream_frame(
                item.resume().image(),
                item.item_type(),
                item.item_shape(),
                item.item().values(),
                heap,
            )
        });
        // This is the sole ownership exit for every decode result. The
        // transport future below can therefore retain only its owned,
        // heap-free frame; pending root escrow stays empty.
        let (decoded, resume) = release_stream_item_after_decode(prepared, item, heap)?;
        let reservation = match self
            .runtime
            .resources
            .reserve_server_stream_event(&self.handle, decoded.reservation_kind())
        {
            Ok(reservation) => reservation,
            Err(error) => {
                return Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    resume,
                    outcome: self.reserve_failure_outcome(error),
                }));
            }
        };
        let frame = match decoded.into_writer_frame(reservation) {
            Ok(frame) => frame,
            Err(error) => {
                let _ = self
                    .runtime
                    .resources
                    .terminate(&self.handle, RequestResourceTermination::HostError);
                return Err(error);
            }
        };
        let mut future = self
            .writer
            .flush(frame, self.runtime.execution_control.clone());
        match poll_future_once(future.as_mut()) {
            Poll::Ready(result) => {
                if matches!(&result, Err(BytecodeServerStreamWriteFailure::Cancelled)) {
                    let _ = self.runtime.budget.request_cancel();
                }
                let outcome = self.runtime.ready_terminal().unwrap_or_else(|| {
                    materialize_server_stream_flush_outcome(
                        &self.runtime.resources,
                        reservation,
                        result,
                    )
                });
                Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    resume,
                    outcome,
                }))
            }
            Poll::Pending => self
                .runtime
                .begin_server_stream_pending(resume, future, reservation)
                .map(BytecodeStreamHandoff::Pending),
        }
    }

    fn park(
        &self,
        operation: PendingOperation,
        suspended: VmSuspended,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeSchedulerError> {
        let queue: Arc<dyn PendingWakeQueue<VmResumeToken, VmSuspended, RequestPendingOutcome>> =
            self.runtime.wake_queue.clone();
        match self
            .runtime
            .registry
            .publish_operation_or_abandon(operation, suspended, queue)
        {
            Ok(_) => Ok(()),
            Err(error) => {
                let message = error.to_string();
                drop(error);
                Err(terminalize_server_stream_publication_failure(
                    &self.runtime.resources,
                    &self.handle,
                    message,
                ))
            }
        }
    }

    fn finish_stream(&self, depth: usize, result: &VmResult) -> Result<(), BytecodeSchedulerError> {
        if depth != 0 || result.is_err() {
            return Ok(());
        }
        let snapshot = self
            .runtime
            .resources
            .server_stream_snapshot(&self.handle)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        if snapshot.phase != RequestServerStreamPhase::Ended || snapshot.flush_in_progress {
            return Err(BytecodeSchedulerError::Port(
                "server-stream producer completed before an acknowledged end frame".to_string(),
            ));
        }
        Ok(())
    }
}

fn terminalize_server_stream_publication_failure(
    resources: &RequestResourceTable,
    handle: &RequestResourceHandle,
    message: String,
) -> BytecodeSchedulerError {
    match resources.terminate(handle, RequestResourceTermination::HostError) {
        Ok(_) => BytecodeSchedulerError::Port(message),
        Err(cleanup_error) => BytecodeSchedulerError::Port(format!(
            "{message}; server-stream publication cleanup failed: {cleanup_error}"
        )),
    }
}

fn resource_failure_outcome(message: String) -> ResumeOutcome {
    ResumeOutcome::Failure(VmError::HostEffectFailure(RuntimeErrorPayload {
        code: "InternalError".to_string(),
        message,
        status: None,
        details: None,
    }))
}

pub(super) fn materialize_server_stream_flush_outcome(
    resources: &RequestResourceTable,
    reservation: RequestServerStreamReservation,
    result: Result<(), BytecodeServerStreamWriteFailure>,
) -> ResumeOutcome {
    match result {
        Ok(()) => match resources.complete_server_stream_flush(reservation) {
            Ok(_) => ResumeOutcome::Empty,
            Err(error) => {
                let _ = resources.terminate(
                    &reservation.handle(),
                    RequestResourceTermination::WriterFailed,
                );
                resource_failure_outcome(error.to_string())
            }
        },
        Err(BytecodeServerStreamWriteFailure::Cancelled) => {
            match resources.terminate(&reservation.handle(), RequestResourceTermination::Cancelled)
            {
                Ok(_) => ResumeOutcome::InternalTerminal(VmInternalTerminal::OwnerStopped),
                Err(error) => resource_failure_outcome(error.to_string()),
            }
        }
        Err(BytecodeServerStreamWriteFailure::DeadlineExceeded) => {
            let message = "server-stream writer reported a deadline before the request budget won";
            match resources.terminate(&reservation.handle(), RequestResourceTermination::HostError)
            {
                Ok(_) => resource_failure_outcome(message.to_string()),
                Err(error) => resource_failure_outcome(error.to_string()),
            }
        }
        Err(BytecodeServerStreamWriteFailure::RouterDisconnected) => {
            let outcome =
                resource_failure_outcome("Router disconnected during server-stream flush".into());
            match resources.terminate(
                &reservation.handle(),
                RequestResourceTermination::RouterDisconnected,
            ) {
                Ok(_) => outcome,
                Err(error) => resource_failure_outcome(error.to_string()),
            }
        }
        Err(BytecodeServerStreamWriteFailure::WriterFailed(message)) => {
            let outcome = resource_failure_outcome(message);
            match resources.terminate(
                &reservation.handle(),
                RequestResourceTermination::WriterFailed,
            ) {
                Ok(_) => outcome,
                Err(error) => resource_failure_outcome(error.to_string()),
            }
        }
        Err(BytecodeServerStreamWriteFailure::InvalidProviderContract(message)) => {
            let outcome = resource_failure_outcome(message);
            match resources.terminate(&reservation.handle(), RequestResourceTermination::HostError)
            {
                Ok(_) => outcome,
                Err(error) => resource_failure_outcome(error.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use skiff_runtime_scheduler::{BytecodeSchedulerPorts, RequestExecutionContext};

    use super::*;

    #[test]
    fn phase_5_stream_publication_failure_closes_in_flight_resource() {
        let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let resources = context.resource_table();
        let handle = resources
            .register_server_response_stream(std::num::NonZeroUsize::new(16).unwrap())
            .unwrap();
        let _reservation = resources
            .reserve_server_stream_event(&handle, RequestServerStreamEventKind::Start)
            .unwrap();

        let error = terminalize_server_stream_publication_failure(
            &resources,
            &handle,
            "injected sealed publication failure".to_string(),
        );

        assert!(matches!(
            error,
            BytecodeSchedulerError::Port(message)
                if message == "injected sealed publication failure"
        ));
        assert_eq!(resources.snapshot().live, 0);
        assert_eq!(resources.snapshot().terminal, 0);
        let snapshot = context.into_not_started();
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
    }
}
