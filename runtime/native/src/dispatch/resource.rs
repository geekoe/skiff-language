use serde_json::json;
use skiff_runtime_boundary::{contract::RuntimeBoundaryContract, plan::BoundaryUse};
use skiff_runtime_model::{
    LoadedPublicationResource, PublicationResourcePath, RuntimeProgramResourceLookupError,
};

use super::{unsupported_native_target, RuntimeNativeInvocation};
use crate::{
    call_helpers::runtime_string_arg,
    capability::NativeResourceCapability,
    error::{Result, RuntimeError},
    runtime_value_facade::{RequestHeap, RuntimeValue},
};

pub(super) struct ResourceNativeDispatch;

impl ResourceNativeDispatch {
    pub(super) fn matches(target: &str) -> bool {
        matches!(
            target,
            "std.resource.bytes"
                | "std.resource.text"
                | "std.resource.json"
                | "std.resource.info"
                | "std.resource.exists"
        )
    }

    pub(super) fn dispatch<ResourceContext>(
        resource_context: &ResourceContext,
        invocation: &RuntimeNativeInvocation,
        diagnostic_target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue>
    where
        ResourceContext: NativeResourceCapability,
    {
        let binding_key = invocation.binding_key();
        match binding_key {
            "std.resource.bytes" => {
                let (path, resource) =
                    resource_arg(resource_context, invocation, diagnostic_target, &args, heap)?;
                let value = RuntimeValue::Heap(heap.alloc_bytes(resource.bytes.as_ref())?);
                invocation.native_boundary()?.coerce_return(
                    &value,
                    &format!("{diagnostic_target} response for {path}"),
                    heap,
                )
            }
            "std.resource.text" => {
                let (path, resource) =
                    resource_arg(resource_context, invocation, diagnostic_target, &args, heap)?;
                let text = resource_text(path.as_str(), resource)?;
                invocation.native_boundary()?.coerce_return(
                    &RuntimeValue::String(text.to_string()),
                    &format!("{diagnostic_target} response for {path}"),
                    heap,
                )
            }
            "std.resource.json" => {
                let (path, resource) =
                    resource_arg(resource_context, invocation, diagnostic_target, &args, heap)?;
                let text = resource_text(path.as_str(), resource)?;
                RuntimeBoundaryContract::default()
                    .codec_for_expected(
                        invocation.return_plan()?,
                        BoundaryUse::JsonValueProjection,
                        format!("std.resource.json resource {path}"),
                    )
                    .decode_json_text(text, heap)
                    .map_err(|error| resource_json_decode_error(path.as_str(), error.into()))
            }
            "std.resource.info" => {
                let (path, resource) =
                    resource_arg(resource_context, invocation, diagnostic_target, &args, heap)?;
                let value = json!({
                    "path": path.as_str(),
                    "size": resource.meta.byte_len,
                    "sha256": &resource.meta.sha256,
                    "contentType": &resource.meta.content_type,
                });
                invocation.native_boundary()?.from_wire_return(
                    &value,
                    &format!("{diagnostic_target} response for {path}"),
                    heap,
                )
            }
            "std.resource.exists" => {
                let path = resource_path_arg(invocation, diagnostic_target, &args, heap)?;
                let exists = match PublicationResourcePath::parse(&path) {
                    Ok(path) => lookup_resource(resource_context, invocation, &path)?.is_some(),
                    Err(_) => false,
                };
                invocation.native_boundary()?.coerce_return(
                    &RuntimeValue::Bool(exists),
                    &format!("{diagnostic_target} response"),
                    heap,
                )
            }
            _ => Err(unsupported_native_target(binding_key)),
        }
    }
}

fn resource_arg<'a>(
    resource_context: &'a impl NativeResourceCapability,
    invocation: &RuntimeNativeInvocation,
    diagnostic_target: &str,
    args: &[RuntimeValue],
    heap: &mut RequestHeap,
) -> Result<(PublicationResourcePath, &'a LoadedPublicationResource)> {
    let path = resource_path_arg(invocation, diagnostic_target, args, heap)?;
    let parsed = PublicationResourcePath::parse(&path)
        .map_err(|error| RuntimeError::resource_error(path.clone(), error.to_string()))?;
    let resource = lookup_resource(resource_context, invocation, &parsed)?.ok_or_else(|| {
        RuntimeError::resource_error(
            parsed.as_str(),
            format!("publication resource {} is not declared", parsed.as_str()),
        )
    })?;
    Ok((parsed, resource))
}

fn resource_path_arg(
    invocation: &RuntimeNativeInvocation,
    diagnostic_target: &str,
    args: &[RuntimeValue],
    heap: &mut RequestHeap,
) -> Result<String> {
    let arg = args
        .first()
        .ok_or_else(|| RuntimeError::Decode(format!("{diagnostic_target} requires path")))?;
    let value = invocation.native_boundary()?.coerce_arg(
        0,
        arg,
        &format!("{diagnostic_target} path"),
        heap,
    )?;
    runtime_string_arg(&value, &format!("{diagnostic_target} path")).map(str::to_string)
}

fn lookup_resource<'a>(
    resource_context: &'a impl NativeResourceCapability,
    invocation: &RuntimeNativeInvocation,
    path: &PublicationResourcePath,
) -> Result<Option<&'a LoadedPublicationResource>> {
    resource_context
        .resources()
        .lookup(invocation.resource_owner()?, path.as_str())
        .map_err(resource_lookup_error)
}

fn resource_lookup_error(error: RuntimeProgramResourceLookupError) -> RuntimeError {
    RuntimeError::InvalidArtifact(error.to_string())
}

fn resource_text<'a>(path: &str, resource: &'a LoadedPublicationResource) -> Result<&'a str> {
    std::str::from_utf8(resource.bytes.as_ref()).map_err(|error| {
        RuntimeError::resource_error(path, format!("resource is not valid UTF-8: {error}"))
    })
}

fn resource_json_decode_error(path: &str, error: RuntimeError) -> RuntimeError {
    match error {
        RuntimeError::Decode(message) => {
            RuntimeError::decode_target("std.resource.json", format!("resource {path}: {message}"))
        }
        RuntimeError::DecodeTarget { target, message } => RuntimeError::decode_target(
            "std.resource.json",
            format!("resource {path}: decode error for {target}: {message}"),
        ),
        other => other,
    }
}
