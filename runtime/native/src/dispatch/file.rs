use bytes::Bytes;
use serde_json::Value;
use skiff_runtime_boundary::file::{
    create_options_from_wire, immutable_file_from_wire, FileCreateOptions, ImmutableFileRef,
};
use skiff_runtime_boundary::{contract::RuntimeBoundaryContract, plan::BoundaryUse};
use skiff_runtime_capability_context::FileCapabilityError;

use super::{unsupported_native_target, RuntimeNativeInvocation};
use crate::error::{Result, RuntimeError};
use crate::{
    call_helpers::runtime_string_arg,
    capability::{NativeFileCapability, NativeFileChunkFuture, NativeFileSourceStreamCapability},
    runtime_value_facade::{
        bytes_payload, RequestHeap, RequestHeapLimits, RuntimeTypeNode, RuntimeTypePlan,
        RuntimeValue,
    },
};

pub(super) struct FileNativeDispatch;

impl FileNativeDispatch {
    pub(super) fn matches(target: &str) -> bool {
        matches!(
            target,
            "std.file.create"
                | "std.file.createText"
                | "std.file.read"
                | "std.file.readText"
                | "std.file.info"
                | "std.file.delete"
                | "std.file.createFromStream"
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn dispatch<FileContext>(
        file_context: &FileContext,
        file_source_stream_context: &impl NativeFileSourceStreamCapability,
        request_heap_limits: RequestHeapLimits,
        invocation: &RuntimeNativeInvocation,
        diagnostic_target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue>
    where
        FileContext: NativeFileCapability,
    {
        let binding_key = invocation.binding_key();
        let output = match binding_key {
            "std.file.create" => {
                let content = bytes_arg_from_plan(diagnostic_target, invocation, &args, 0, heap)?;
                let options =
                    file_options_arg(diagnostic_target, invocation, &args, 1, None, heap)?;
                file_context
                    .create_file(diagnostic_target, Bytes::from(content), options)
                    .await?
            }
            "std.file.createText" => {
                let content = string_arg_from_plan(diagnostic_target, invocation, &args, 0, heap)?;
                let options = file_options_arg(
                    diagnostic_target,
                    invocation,
                    &args,
                    1,
                    Some("text/plain; charset=utf-8"),
                    heap,
                )?;
                file_context
                    .create_file(
                        diagnostic_target,
                        Bytes::from(content.into_bytes()),
                        options,
                    )
                    .await?
            }
            "std.file.read" => {
                let file = file_arg_from_plan(diagnostic_target, invocation, &args, 0, heap)?;
                file_context
                    .read_file_wire(diagnostic_target, &file)
                    .await?
            }
            "std.file.readText" => {
                let file = file_arg_from_plan(diagnostic_target, invocation, &args, 0, heap)?;
                file_context
                    .read_text_file(diagnostic_target, &file)
                    .await?
            }
            "std.file.info" => {
                let file = file_arg_from_plan(diagnostic_target, invocation, &args, 0, heap)?;
                file_context.file_info(diagnostic_target, &file).await?
            }
            "std.file.delete" => {
                let file = file_arg_from_plan(diagnostic_target, invocation, &args, 0, heap)?;
                file_context.delete_file(diagnostic_target, &file).await?;
                Value::Null
            }
            "std.file.createFromStream" => {
                let stream = stream_arg_from_plan(diagnostic_target, invocation, &args, 0, heap)?;
                let options =
                    file_options_arg(diagnostic_target, invocation, &args, 1, None, heap)?;
                let item_plan =
                    file_stream_item_plan(diagnostic_target, invocation.arg_plan(0)?)?.clone();
                let source_context = file_source_stream_context.clone();
                file_context
                    .create_file_from_chunks(
                        diagnostic_target,
                        options,
                        Box::new(move || {
                            let source_context = source_context.clone();
                            let stream = stream.clone();
                            let item_plan = item_plan.clone();
                            let request_heap_limits = request_heap_limits.clone();
                            Box::pin(async move {
                                let Some(item) =
                                    source_context.next_file_source_stream_item(&stream).await?
                                else {
                                    return Ok(None);
                                };
                                let mut item_heap = RequestHeap::new(request_heap_limits.clone());
                                let codec = RuntimeBoundaryContract::default().codec_for_expected(
                                    &item_plan,
                                    BoundaryUse::TypedJson,
                                    "std.file.createFromStream item",
                                );
                                let value = codec.from_wire_json(&item, &mut item_heap).map_err(
                                    |error| {
                                        file_capability_error_from_native(RuntimeError::from(error))
                                    },
                                )?;
                                let wire = codec.to_wire_json(&value, &mut item_heap).map_err(
                                    |error| {
                                        file_capability_error_from_native(RuntimeError::from(error))
                                    },
                                )?;
                                let bytes = bytes_payload(&wire).ok_or_else(|| {
                                    FileCapabilityError::Decode(
                                        "std.file.createFromStream item must be bytes".to_string(),
                                    )
                                })?;
                                Ok(Some(Bytes::from(bytes)))
                            }) as NativeFileChunkFuture<'_>
                        }),
                    )
                    .await?
            }
            _ => return Err(unsupported_native_target(binding_key)),
        };

        invocation.native_boundary()?.from_wire_return(
            &output,
            &format!("{diagnostic_target} response"),
            heap,
        )
    }
}

fn file_capability_error_from_native(error: RuntimeError) -> FileCapabilityError {
    match error {
        RuntimeError::Decode(message) => FileCapabilityError::Decode(message),
        RuntimeError::DecodeTarget { target, message } => {
            FileCapabilityError::Decode(format!("decode error for {target}: {message}"))
        }
        RuntimeError::BytesDecode { target, message } => {
            FileCapabilityError::Decode(format!("bytes decode error for {target}: {message}"))
        }
        RuntimeError::DbDecode { target, message } => {
            FileCapabilityError::Decode(format!("db decode error for {target}: {message}"))
        }
        RuntimeError::FileError { message } => FileCapabilityError::File(message),
        RuntimeError::Cancelled => FileCapabilityError::Execution(
            skiff_runtime_capability_context::ExecutionControlError::Cancelled,
        ),
        RuntimeError::ExecutionBudgetExceeded {
            reason,
            instruction_count,
            limit,
            elapsed_ms,
        } => FileCapabilityError::Execution(
            skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(
                skiff_runtime_capability_context::ExecutionBudgetFailure {
                    reason: file_capability_budget_reason(reason),
                    instruction_count,
                    limit,
                    elapsed_ms,
                },
            ),
        ),
        RuntimeError::ResourceLimitExceeded {
            resource,
            reason,
            limit,
            current,
            requested_delta,
        } => FileCapabilityError::ResourceLimitExceeded {
            resource,
            reason,
            limit,
            current,
            requested_delta,
        },
        RuntimeError::InvalidArtifact(message)
        | RuntimeError::HttpError { message, detail: _ }
        | RuntimeError::Unsupported(message) => FileCapabilityError::Decode(message),
        RuntimeError::Recoverable(error) => FileCapabilityError::Stream(
            skiff_runtime_capability_context::StreamRuntimeError::producer(
                RuntimeError::Recoverable(error),
            ),
        ),
        RuntimeError::Opaque(error) => FileCapabilityError::Stream(
            skiff_runtime_capability_context::StreamRuntimeError::Producer(error),
        ),
        RuntimeError::Json(error) => FileCapabilityError::Stream(
            skiff_runtime_capability_context::StreamRuntimeError::producer(RuntimeError::Json(
                error,
            )),
        ),
    }
}

fn file_capability_budget_reason(
    reason: crate::error::BudgetReason,
) -> skiff_runtime_capability_context::ExecutionBudgetReason {
    match reason {
        crate::error::BudgetReason::Cancelled => {
            skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled
        }
        crate::error::BudgetReason::DeadlineExceeded => {
            skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded
        }
        crate::error::BudgetReason::InstructionLimitExceeded => {
            skiff_runtime_capability_context::ExecutionBudgetReason::InstructionLimitExceeded
        }
    }
}

fn bytes_arg_from_plan(
    target: &str,
    invocation: &RuntimeNativeInvocation,
    args: &[RuntimeValue],
    index: usize,
    heap: &mut RequestHeap,
) -> Result<Vec<u8>> {
    let arg = args
        .get(index)
        .ok_or_else(|| RuntimeError::Decode(format!("{target} requires argument {index}")))?;
    let wire = invocation.native_boundary()?.to_wire_arg(
        index,
        arg,
        &format!("{target} argument {index}"),
        heap,
    )?;
    bytes_payload(&wire)
        .ok_or_else(|| RuntimeError::Decode(format!("{target} argument {index} must be bytes")))
}

fn string_arg_from_plan(
    target: &str,
    invocation: &RuntimeNativeInvocation,
    args: &[RuntimeValue],
    index: usize,
    heap: &mut RequestHeap,
) -> Result<String> {
    let arg = args
        .get(index)
        .ok_or_else(|| RuntimeError::Decode(format!("{target} requires argument {index}")))?;
    let coerced = invocation.native_boundary()?.coerce_arg(
        index,
        arg,
        &format!("{target} argument {index}"),
        heap,
    )?;
    runtime_string_arg(&coerced, &format!("{target} argument {index}")).map(str::to_string)
}

fn file_arg_from_plan(
    target: &str,
    invocation: &RuntimeNativeInvocation,
    args: &[RuntimeValue],
    index: usize,
    heap: &mut RequestHeap,
) -> Result<ImmutableFileRef> {
    let arg = args
        .get(index)
        .ok_or_else(|| RuntimeError::Decode(format!("{target} requires file")))?;
    let wire =
        invocation
            .native_boundary()?
            .to_wire_arg(index, arg, &format!("{target} file"), heap)?;
    Ok(immutable_file_from_wire(&wire, target)?)
}

fn file_options_arg(
    target: &str,
    invocation: &RuntimeNativeInvocation,
    args: &[RuntimeValue],
    index: usize,
    default_content_type: Option<&str>,
    heap: &mut RequestHeap,
) -> Result<FileCreateOptions> {
    let Some(arg) = args.get(index) else {
        return Ok(create_options_from_wire(
            None,
            default_content_type,
            target,
        )?);
    };
    let wire = invocation.native_boundary()?.to_wire_arg(
        index,
        arg,
        &format!("{target} options"),
        heap,
    )?;
    Ok(create_options_from_wire(
        Some(&wire),
        default_content_type,
        target,
    )?)
}

fn stream_arg_from_plan(
    target: &str,
    invocation: &RuntimeNativeInvocation,
    args: &[RuntimeValue],
    index: usize,
    heap: &mut RequestHeap,
) -> Result<Value> {
    let arg = args
        .get(index)
        .ok_or_else(|| RuntimeError::Decode(format!("{target} requires stream")))?;
    invocation
        .native_boundary()?
        .to_wire_arg(index, arg, &format!("{target} stream"), heap)
}

fn file_stream_item_plan<'a>(
    target: &str,
    stream_plan: &'a RuntimeTypePlan,
) -> Result<&'a RuntimeTypePlan> {
    match stream_plan.node() {
        RuntimeTypeNode::Stream(item) if matches!(item.node(), RuntimeTypeNode::Bytes) => Ok(item),
        RuntimeTypeNode::Stream(_) => Err(RuntimeError::InvalidArtifact(format!(
            "{target} source must be Stream<bytes>"
        ))),
        _ => Err(RuntimeError::InvalidArtifact(format!(
            "{target} source argument is not a Stream"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;

    use skiff_runtime_model::error::{RuntimeErrorPayload, TypeIdentity, WirePayload};

    #[derive(Debug)]
    struct DummyWirePayload;

    impl fmt::Display for DummyWirePayload {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("dummy producer payload")
        }
    }

    impl std::error::Error for DummyWirePayload {}

    impl WirePayload for DummyWirePayload {
        fn payload(&self) -> RuntimeErrorPayload {
            RuntimeErrorPayload {
                code: "test.FileProducer".to_string(),
                message: "dummy producer payload".to_string(),
                status: None,
                details: Some(serde_json::json!({ "producer": true })),
            }
        }

        fn catch_projection(&self) -> Option<(TypeIdentity, serde_json::Value)> {
            Some((
                TypeIdentity::builtin("test.FileProducerCatch"),
                serde_json::json!({ "caught": true }),
            ))
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[test]
    fn file_capability_error_from_native_preserves_opaque_producer_payload() {
        let error = file_capability_error_from_native(RuntimeError::Opaque(Box::new(
            DummyWirePayload,
        )));

        match error {
            FileCapabilityError::Stream(
                skiff_runtime_capability_context::StreamRuntimeError::Producer(error),
            ) => {
                assert_eq!(error.payload().code, "test.FileProducer");
                assert_eq!(
                    error.catch_projection(),
                    Some((
                        TypeIdentity::builtin("test.FileProducerCatch"),
                        serde_json::json!({ "caught": true }),
                    ))
                );
            }
            error => panic!("expected stream producer, got {error:?}"),
        }
    }
}
