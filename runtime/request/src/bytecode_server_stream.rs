use std::{sync::Arc, task::Poll};

use skiff_artifact_model::TypeRefIr;
use skiff_runtime_linked_bytecode::{FunctionIndex, LinkedShapeEntry, ShapeIndex, TypeIndex};
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::{error::RuntimeErrorPayload, vm_heap::VmHeap, vm_value::ValueSlot};
use skiff_runtime_scheduler::{
    BytecodeHandoff, BytecodeParkFailure, BytecodeParkRequest, BytecodePortFailure,
    BytecodeSchedulerError, BytecodeStreamHandoff, BytecodeStreamSupervisor, PendingWakeQueue,
    RequestResourceHandle, RequestResourceTable, RequestResourceTermination,
    RequestServerStreamEventKind, RequestServerStreamPhase, RequestServerStreamReservation,
    RequestServerStreamReserveError,
};
use skiff_runtime_vm::{
    ResumeOutcome, StreamItem, StreamItemReleaseError, VmBudget, VmCompletion, VmError, VmFiber,
    VmInternalTerminal, VmResumeToken,
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

fn validate_server_stream_tag_carrier(
    image: &DeploymentExecutionImage,
    shape: &LinkedShapeEntry,
    expected: &str,
) -> Result<(), BytecodeSchedulerError> {
    let tag_type = shape_field_type(shape, "tag")?;
    validate_builtin_type(image, tag_type, "string").map_err(|_| {
        BytecodeSchedulerError::Port(format!(
            "server-stream {expected:?} variant does not retain its exact linked string discriminator carrier"
        ))
    })
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
            validate_server_stream_tag_carrier(image, shape, "start")?;
            require_exact_slot_type_ref(
                image,
                &tag_value,
                shape_field_type(shape, "tag")?,
                "server-stream start discriminator",
            )?;
            let status_type = shape_field_type(shape, "status")?;
            validate_builtin_type(image, status_type, "number")?;
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
            validate_server_stream_tag_carrier(image, shape, "chunk")?;
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
            validate_server_stream_tag_carrier(image, shape, "end")?;
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

/// Prepares one server-stream frame from any producer depth.
///
/// Root HTTP producers and remote-interface child producers both emit the same
/// linked `std.http.HttpResponseStreamEvent` frame shape. X6's child mux routes
/// nested remote stream frames through the same transport supervisor instead of
/// closing them at depth > 0.
fn prepare_server_stream_frame<T>(
    _depth: usize,
    decode: impl FnOnce() -> Result<T, BytecodeSchedulerError>,
) -> Result<T, BytecodeSchedulerError> {
    decode()
}

/// Checks the exact stream-result authority carried by every stream producer.
///
/// Root HTTP producers were already admitted against the canonical gateway at
/// request ingress. Child producers must also retain the linked stream-result
/// authority; without it the runtime cannot prove that the emitted item is the
/// boundary fact the caller is allowed to consume, so the request fails closed
/// before any HTTP frame decode or transport write.
pub(crate) fn validate_stream_producer_authority(
    image: &DeploymentExecutionImage,
    function: FunctionIndex,
    item_type: TypeIndex,
    depth: usize,
) -> Result<(), BytecodeSchedulerError> {
    let Some(function) = image
        .functions()
        .get(usize::try_from(function.get()).unwrap_or(usize::MAX))
        .filter(|row| row.index() == function)
    else {
        return Err(BytecodeSchedulerError::Port(format!(
            "server-stream item at depth {depth} references a missing function"
        )));
    };
    let Some(stream_result) = function.stream_result_type_ref() else {
        return Err(BytecodeSchedulerError::Port(format!(
            "server-stream item at depth {depth} lacks the exact linked stream-result authority"
        )));
    };
    let stream_type = image
        .types()
        .get(usize::try_from(stream_result.get()).unwrap_or(usize::MAX))
        .filter(|row| row.index() == stream_result)
        .map(|row| row.type_ref())
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(format!(
                "server-stream item at depth {depth} references a missing stream-result type"
            ))
        })?;
    let TypeRefIr::Builtin { name, args } = stream_type else {
        return Err(BytecodeSchedulerError::Port(format!(
            "server-stream item at depth {depth} stream-result type is not an exact Stream carrier"
        )));
    };
    if name != "Stream" || args.len() != 1 {
        return Err(BytecodeSchedulerError::Port(format!(
            "server-stream item at depth {depth} stream-result type is not an exact Stream carrier"
        )));
    }
    let item_type_ref = image
        .types()
        .get(usize::try_from(item_type.get()).unwrap_or(usize::MAX))
        .filter(|row| row.index() == item_type)
        .map(|row| row.type_ref())
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(format!(
                "server-stream item at depth {depth} references a missing item type"
            ))
        })?;
    if item_type_ref != &args[0] {
        return Err(BytecodeSchedulerError::Port(format!(
            "server-stream item at depth {depth} lacks the exact linked stream-result authority"
        )));
    }
    Ok(())
}

enum ReleaseAfterDecodeFailure<R, E> {
    Continuation {
        reason: BytecodeSchedulerError,
        resume: R,
    },
    Terminal {
        primary: Option<BytecodeSchedulerError>,
        failure: E,
    },
}

fn combine_release_after_decode<T, R, E>(
    prepared: Result<T, BytecodeSchedulerError>,
    released: Result<R, E>,
) -> Result<(T, R), ReleaseAfterDecodeFailure<R, E>> {
    match (prepared, released) {
        (Ok(prepared), Ok(resume)) => Ok((prepared, resume)),
        (Err(reason), Ok(resume)) => {
            Err(ReleaseAfterDecodeFailure::Continuation { reason, resume })
        }
        (prepared, Err(failure)) => Err(ReleaseAfterDecodeFailure::Terminal {
            primary: prepared.err(),
            failure,
        }),
    }
}

fn release_stream_item_after_decode<T>(
    prepared: Result<T, BytecodeSchedulerError>,
    item: StreamItem,
    heap: &mut dyn VmHeap,
) -> Result<(T, VmResumeToken), BytecodePortFailure<StreamItem, VmResumeToken>> {
    let released: Result<VmResumeToken, StreamItemReleaseError> = item.release(heap);
    match combine_release_after_decode(prepared, released) {
        Ok(released) => Ok(released),
        Err(ReleaseAfterDecodeFailure::Continuation { reason, resume }) => {
            Err(BytecodePortFailure::continuation(reason, resume))
        }
        // Decoding already selected the request-visible failure. The sealed
        // terminal carrier keeps the independent release diagnostic beside
        // its exact escrow instead of replacing it.
        Err(ReleaseAfterDecodeFailure::Terminal {
            primary: Some(primary),
            failure,
        }) => Err(BytecodePortFailure::terminal_stream_release_with_primary(
            primary, failure,
        )),
        Err(ReleaseAfterDecodeFailure::Terminal {
            primary: None,
            failure,
        }) => Err(BytecodePortFailure::terminal_stream_release(failure)),
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
        producer_heap: &mut dyn VmHeap,
        _consumer_heap: Option<&mut dyn VmHeap>,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeStreamHandoff<VmFiber>, BytecodePortFailure<StreamItem, VmResumeToken>>
    {
        let prepared = prepare_server_stream_frame(depth, || {
            validate_stream_producer_authority(
                item.resume().image(),
                item.resume().function(),
                item.item_type(),
                depth,
            )?;
            decode_server_stream_frame(
                item.resume().image(),
                item.item_type(),
                item.item_shape(),
                item.item().values(),
                producer_heap,
            )
        });
        // This is the sole ownership exit for every decode result. The
        // transport future below can therefore retain only its owned,
        // heap-free frame; pending root escrow stays empty.
        let (decoded, resume) = release_stream_item_after_decode(prepared, item, producer_heap)?;
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
            Err(reason) => {
                let reason = terminate_server_stream_after_continuation_failure(
                    &self.runtime.resources,
                    &self.handle,
                    reason,
                );
                return Err(BytecodePortFailure::continuation(reason, resume));
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
            Poll::Pending => {
                match self
                    .runtime
                    .begin_server_stream_pending(resume, future, reservation)
                {
                    Ok(operation) => Ok(BytecodeStreamHandoff::Pending(operation)),
                    Err(failure) => {
                        let (reason, resume) = failure.into_parts();
                        let reason = terminate_server_stream_after_continuation_failure(
                            &self.runtime.resources,
                            &self.handle,
                            reason,
                        );
                        Err(BytecodePortFailure::continuation(reason, resume))
                    }
                }
            }
        }
    }

    fn park(
        &self,
        request: BytecodeParkRequest<VmFiber>,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeParkFailure<VmFiber>> {
        let queue: Arc<dyn PendingWakeQueue<VmResumeToken, VmSuspended, RequestPendingOutcome>> =
            self.runtime.wake_queue.clone();
        let (operation, suspended) = request.into_parts();
        match self
            .runtime
            .registry
            .publish_operation_or_abandon(operation, suspended, queue)
        {
            Ok(_) => Ok(()),
            Err(error) => {
                let reason = terminalize_server_stream_publication_failure(
                    &self.runtime.resources,
                    &self.handle,
                    error.reason().to_string(),
                );
                Err(BytecodeParkFailure::pending_draft(
                    reason,
                    error.into_draft(),
                ))
            }
        }
    }

    fn finish_stream(
        &self,
        depth: usize,
        completion: &VmCompletion,
    ) -> Result<(), BytecodeSchedulerError> {
        if !finish_stream_requires_end(
            depth,
            completion.returned_values().map(|values| values.len()),
        ) {
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

fn finish_stream_requires_end(depth: usize, returned_value_count: Option<usize>) -> bool {
    depth == 0 && returned_value_count == Some(0)
}

fn terminate_server_stream_after_continuation_failure(
    resources: &RequestResourceTable,
    handle: &RequestResourceHandle,
    reason: BytecodeSchedulerError,
) -> BytecodeSchedulerError {
    match resources.terminate(handle, RequestResourceTermination::HostError) {
        Ok(_) => reason,
        Err(cleanup_error) => BytecodeSchedulerError::Port(format!(
            "{reason}; server-stream continuation cleanup failed: {cleanup_error}"
        )),
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
                let message = error.to_string();
                let message = match resources.terminate(
                    &reservation.handle(),
                    RequestResourceTermination::WriterFailed,
                ) {
                    Ok(_) => message,
                    Err(cleanup_error) => {
                        format!("{message}; server-stream flush cleanup failed: {cleanup_error}")
                    }
                };
                resource_failure_outcome(message)
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
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use skiff_runtime_scheduler::{BytecodeSchedulerPorts, RequestExecutionContext};

    use super::*;

    #[test]
    fn nested_remote_stream_frames_share_the_server_stream_seam() {
        for depth in [0, 1, 3] {
            let decoded =
                prepare_server_stream_frame(depth, || Ok::<_, BytecodeSchedulerError>(depth))
                    .expect("nested child stream emission must use the same frame seam");
            assert_eq!(decoded, depth);
        }
    }

    #[test]
    fn phase_5_stream_decode_failure_with_successful_release_returns_continuation() {
        let prepared = Err::<(), _>(BytecodeSchedulerError::Port(
            "injected decode failure".to_string(),
        ));
        let released = Ok::<_, ()>("exact-resume");

        let failure = combine_release_after_decode(prepared, released)
            .expect_err("decode failure cannot become a successful handoff");

        match failure {
            ReleaseAfterDecodeFailure::Continuation { reason, resume } => {
                assert!(matches!(
                    reason,
                    BytecodeSchedulerError::Port(message)
                        if message == "injected decode failure"
                ));
                assert_eq!(resume, "exact-resume");
            }
            ReleaseAfterDecodeFailure::Terminal { .. } => {
                panic!("a successful release must return the exact continuation")
            }
        }
    }

    struct ExactReleaseRetry {
        exact_plan: &'static str,
        attempts: Arc<AtomicUsize>,
    }

    impl ExactReleaseRetry {
        fn retry(self) -> &'static str {
            assert_eq!(self.attempts.fetch_add(1, Ordering::AcqRel), 0);
            self.exact_plan
        }
    }

    #[test]
    fn phase_5_stream_release_failure_retains_decode_primary_and_exact_retry_owner() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let prepared = Err::<(), _>(BytecodeSchedulerError::Port(
            "primary decode failure".to_string(),
        ));
        let released = Err::<(), _>(ExactReleaseRetry {
            exact_plan: "linked-plan-17",
            attempts: Arc::clone(&attempts),
        });

        let failure = combine_release_after_decode(prepared, released)
            .expect_err("a failed exact release must stay terminal");

        let ReleaseAfterDecodeFailure::Terminal {
            primary: Some(primary),
            failure,
        } = failure
        else {
            panic!("decode plus release failure must retain both primary and owner")
        };
        assert!(matches!(
            primary,
            BytecodeSchedulerError::Port(message) if message == "primary decode failure"
        ));
        assert_eq!(attempts.load(Ordering::Acquire), 0);
        assert_eq!(failure.retry(), "linked-plan-17");
        assert_eq!(attempts.load(Ordering::Acquire), 1);
    }

    #[test]
    fn phase_5_stream_finish_borrows_only_root_empty_success_for_end_check() {
        assert!(finish_stream_requires_end(0, Some(0)));
        assert!(!finish_stream_requires_end(1, Some(0)));
        assert!(!finish_stream_requires_end(0, Some(1)));
        assert!(
            !finish_stream_requires_end(0, None),
            "thrown and failed completions retain their owner without a fabricated stream result"
        );
    }

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

    #[test]
    fn phase_5_stream_continuation_failure_closes_resource_without_replacing_reason() {
        let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let resources = context.resource_table();
        let handle = resources
            .register_server_response_stream(std::num::NonZeroUsize::new(16).unwrap())
            .unwrap();
        let _reservation = resources
            .reserve_server_stream_event(&handle, RequestServerStreamEventKind::Start)
            .unwrap();

        let reason = terminate_server_stream_after_continuation_failure(
            &resources,
            &handle,
            BytecodeSchedulerError::UnsupportedStream,
        );

        assert!(matches!(reason, BytecodeSchedulerError::UnsupportedStream));
        assert_eq!(resources.snapshot().live, 0);
        let snapshot = context.into_not_started();
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
    }

    #[test]
    fn phase_5_stream_end_ack_is_idempotent_without_a_second_terminal() {
        let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let resources = context.resource_table();
        let handle = resources
            .register_server_response_stream(std::num::NonZeroUsize::new(16).unwrap())
            .unwrap();
        let start = resources
            .reserve_server_stream_event(&handle, RequestServerStreamEventKind::Start)
            .unwrap();
        assert!(matches!(
            materialize_server_stream_flush_outcome(&resources, start, Ok(())),
            ResumeOutcome::Empty
        ));
        let end = resources
            .reserve_server_stream_event(&handle, RequestServerStreamEventKind::End)
            .unwrap();
        assert!(matches!(
            materialize_server_stream_flush_outcome(&resources, end, Ok(())),
            ResumeOutcome::Empty
        ));
        assert!(matches!(
            materialize_server_stream_flush_outcome(&resources, end, Ok(())),
            ResumeOutcome::Empty
        ));
        let snapshot = resources.server_stream_snapshot(&handle).unwrap();
        assert_eq!(snapshot.phase, RequestServerStreamPhase::Ended);
        assert!(!snapshot.flush_in_progress);

        resources
            .terminate(&handle, RequestResourceTermination::RequestCompleted)
            .unwrap();
        let snapshot = context.into_not_started();
        assert_eq!(snapshot.resource.current, 0);
    }

    #[test]
    fn phase_6_server_stream_supervisor_preserves_ordered_chunks_and_end() {
        let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let resources = context.resource_table();
        let handle = resources
            .register_server_response_stream(std::num::NonZeroUsize::new(16).unwrap())
            .unwrap();
        let start = resources
            .reserve_server_stream_event(&handle, RequestServerStreamEventKind::Start)
            .unwrap();
        assert!(matches!(
            materialize_server_stream_flush_outcome(&resources, start, Ok(())),
            ResumeOutcome::Empty
        ));
        for expected_sequence in 0_u64..3 {
            let chunk = resources
                .reserve_server_stream_event(
                    &handle,
                    RequestServerStreamEventKind::Chunk { payload_bytes: 1 },
                )
                .unwrap();
            assert_eq!(
                chunk.sequence(),
                Some(expected_sequence),
                "the shared supervisor must allocate exact ordered chunk sequences"
            );
            assert!(matches!(
                materialize_server_stream_flush_outcome(&resources, chunk, Ok(())),
                ResumeOutcome::Empty
            ));
        }
        let end = resources
            .reserve_server_stream_event(&handle, RequestServerStreamEventKind::End)
            .unwrap();
        assert!(matches!(
            materialize_server_stream_flush_outcome(&resources, end, Ok(())),
            ResumeOutcome::Empty
        ));
        let snapshot = resources.server_stream_snapshot(&handle).unwrap();
        assert_eq!(snapshot.phase, RequestServerStreamPhase::Ended);
        assert_eq!(snapshot.next_sequence, 3);

        resources
            .terminate(&handle, RequestResourceTermination::RequestCompleted)
            .unwrap();
        let snapshot = context.into_not_started();
        assert_eq!(snapshot.resource.current, 0);
    }

    #[test]
    fn phase_6_server_stream_supervisor_backpressures_one_flush_permit() {
        let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let resources = context.resource_table();
        let handle = resources
            .register_server_response_stream(std::num::NonZeroUsize::new(16).unwrap())
            .unwrap();
        let start = resources
            .reserve_server_stream_event(&handle, RequestServerStreamEventKind::Start)
            .unwrap();
        assert!(matches!(
            resources.reserve_server_stream_event(
                &handle,
                RequestServerStreamEventKind::Chunk { payload_bytes: 1 },
            ),
            Err(RequestServerStreamReserveError::FlushInProgress)
        ));
        assert!(matches!(
            materialize_server_stream_flush_outcome(&resources, start, Ok(())),
            ResumeOutcome::Empty
        ));
        let chunk = resources
            .reserve_server_stream_event(
                &handle,
                RequestServerStreamEventKind::Chunk { payload_bytes: 1 },
            )
            .unwrap();
        assert_eq!(chunk.sequence(), Some(0));

        resources
            .terminate(&handle, RequestResourceTermination::RequestCompleted)
            .unwrap();
        let snapshot = context.into_not_started();
        assert_eq!(snapshot.resource.current, 0);
    }

    #[test]
    fn phase_5_stream_cancel_wins_once_and_late_ack_cannot_revive_resource() {
        let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let resources = context.resource_table();
        let handle = resources
            .register_server_response_stream(std::num::NonZeroUsize::new(16).unwrap())
            .unwrap();
        let reservation = resources
            .reserve_server_stream_event(&handle, RequestServerStreamEventKind::Start)
            .unwrap();

        assert!(matches!(
            materialize_server_stream_flush_outcome(
                &resources,
                reservation,
                Err(BytecodeServerStreamWriteFailure::Cancelled),
            ),
            ResumeOutcome::InternalTerminal(VmInternalTerminal::OwnerStopped)
        ));
        assert!(matches!(
            materialize_server_stream_flush_outcome(&resources, reservation, Ok(())),
            ResumeOutcome::Failure(_)
        ));
        assert_eq!(resources.snapshot().live, 0);
        assert_eq!(resources.snapshot().terminal, 0);
        let snapshot = context.into_not_started();
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
    }
}
