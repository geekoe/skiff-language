use std::collections::HashSet;

use skiff_runtime_model::{
    recoverable::{RuntimeRecoverableBoundaryContext, RuntimeRecoverableExpectedTypePlan},
    request_heap::RequestHeap,
    value::{CallbackCapabilityCarrier, HeapHandle, HeapNode, InterfaceCarrier, RuntimeValue},
};

use crate::error::{RecoverableBoundaryError, RecoverableBoundaryErrorCode, Result, RuntimeError};

/// Reject request-scoped callback capabilities before any persistent encoder,
/// behavior hook, rebuild hook, or transport fallback can observe them.
pub fn reject_callback_capability_graph(
    value: &RuntimeValue,
    heap: &RequestHeap,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> Result<()> {
    reject_callback_capability_graph_inner(value, heap, context, expected, "$", &mut HashSet::new())
}

fn reject_callback_capability_graph_inner(
    value: &RuntimeValue,
    heap: &RequestHeap,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
    path: &str,
    visited: &mut HashSet<HeapHandle>,
) -> Result<()> {
    let RuntimeValue::Heap(handle) = value else {
        return Ok(());
    };
    if !visited.insert(*handle) {
        return Ok(());
    }
    match heap.get(*handle)? {
        HeapNode::Bytes(_) => Ok(()),
        HeapNode::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                reject_callback_capability_graph_inner(
                    item,
                    heap,
                    context,
                    expected,
                    &format!("{path}[{index}]"),
                    visited,
                )?;
            }
            Ok(())
        }
        HeapNode::Object(object) => {
            for (field, item) in object.fields() {
                reject_callback_capability_graph_inner(
                    item,
                    heap,
                    context,
                    expected,
                    &format!("{path}.{field}"),
                    visited,
                )?;
            }
            Ok(())
        }
        HeapNode::Map(map) => {
            for (key, item) in map {
                reject_callback_capability_graph_inner(
                    item,
                    heap,
                    context,
                    expected,
                    &format!("{path}.map({})", key.string_payload()),
                    visited,
                )?;
            }
            Ok(())
        }
        HeapNode::Interface(interface) => match interface.carrier() {
            InterfaceCarrier::CallbackCapability(carrier) => Err(
                callback_capability_not_recoverable_error(carrier, path, context, expected),
            ),
            InterfaceCarrier::Local { .. } | InterfaceCarrier::Remote { .. } => Ok(()),
        },
        HeapNode::Exception(_) => Err(RuntimeError::Decode(
            "request-local exception cannot enter a persistent boundary".to_string(),
        )),
    }
}

pub(crate) fn callback_capability_not_recoverable_error(
    carrier: &CallbackCapabilityCarrier,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> RuntimeError {
    RecoverableBoundaryError::new(
        RecoverableBoundaryErrorCode::CallbackCapabilityNotRecoverable,
        "request-scoped callback capability cannot enter a persistent or recoverable boundary",
        context,
        expected,
    )
    .with_detail(serde_json::json!({
        "nodePath": path,
        "ownerRuntimeReplicaId": carrier.owner_runtime_replica_id(),
        "ownerActivationId": carrier.owner_activation_id(),
        "requestGeneration": carrier.request_generation(),
        "interfaceOrAdapterContract": carrier.interface_or_adapter_contract(),
        "opaqueCapabilityId": carrier.opaque_capability_id(),
        "rebuildAttempted": false,
        "fallbackAttempted": false,
    }))
    .into()
}
