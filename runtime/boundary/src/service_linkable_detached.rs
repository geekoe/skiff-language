use std::collections::HashSet;

use skiff_runtime_model::{
    request_heap::RequestHeap,
    value::{HeapHandle, HeapNode, RuntimeMap, RuntimeObject, RuntimeObjectFields, RuntimeValue},
};

use crate::service_linkable::ServiceLinkableMaterializationError;

pub(crate) fn reject_detached_interface_graph(
    value: &RuntimeValue,
    heap: &RequestHeap,
) -> Result<(), ServiceLinkableMaterializationError> {
    reject_detached_interface_graph_inner(value, heap, &mut HashSet::new())
}

fn reject_detached_interface_graph_inner(
    value: &RuntimeValue,
    heap: &RequestHeap,
    visited: &mut HashSet<HeapHandle>,
) -> Result<(), ServiceLinkableMaterializationError> {
    let RuntimeValue::Heap(handle) = value else {
        return Ok(());
    };
    if !visited.insert(*handle) {
        return Ok(());
    }
    match heap.get(*handle).map_err(model_error)? {
        HeapNode::Bytes(_) => Ok(()),
        HeapNode::Array(items) => {
            for item in items {
                reject_detached_interface_graph_inner(item, heap, visited)?;
            }
            Ok(())
        }
        HeapNode::Object(object) => {
            for item in object.fields().values() {
                reject_detached_interface_graph_inner(item, heap, visited)?;
            }
            Ok(())
        }
        HeapNode::Map(map) => {
            for item in map.values() {
                reject_detached_interface_graph_inner(item, heap, visited)?;
            }
            Ok(())
        }
        HeapNode::Interface(interface) => Err(
            ServiceLinkableMaterializationError::DetachedInterfaceCarrier {
                carrier: interface.carrier().kind_label(),
            },
        ),
        HeapNode::Exception(_) => Err(ServiceLinkableMaterializationError::TypeMismatch),
    }
}

pub(crate) fn materialize_detached_graph(
    value: &RuntimeValue,
    source: &RequestHeap,
    destination: &mut RequestHeap,
) -> Result<RuntimeValue, ServiceLinkableMaterializationError> {
    DetachedGraphMaterializer::default().clone_value(value, source, destination)
}

#[derive(Default)]
struct DetachedGraphMaterializer {
    active: HashSet<HeapHandle>,
}

impl DetachedGraphMaterializer {
    fn clone_value(
        &mut self,
        value: &RuntimeValue,
        source: &RequestHeap,
        destination: &mut RequestHeap,
    ) -> Result<RuntimeValue, ServiceLinkableMaterializationError> {
        let RuntimeValue::Heap(handle) = value else {
            return Ok(value.clone());
        };
        if !self.active.insert(*handle) {
            return Err(ServiceLinkableMaterializationError::CyclicValueGraph);
        }
        let node = source.get(*handle).map_err(model_error)?;
        let cloned_node = match node {
            HeapNode::Bytes(bytes) => HeapNode::Bytes(bytes.clone()),
            HeapNode::Array(items) => HeapNode::Array(
                items
                    .iter()
                    .map(|item| self.clone_value(item, source, destination))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            HeapNode::Object(object) => HeapNode::Object(RuntimeObject::unshaped(
                object
                    .fields()
                    .iter()
                    .map(|(name, value)| {
                        Ok((name.clone(), self.clone_value(value, source, destination)?))
                    })
                    .collect::<Result<RuntimeObjectFields, _>>()?,
            )),
            HeapNode::Map(map) => HeapNode::Map(
                map.iter()
                    .map(|(key, value)| {
                        Ok((key.clone(), self.clone_value(value, source, destination)?))
                    })
                    .collect::<Result<RuntimeMap, _>>()?,
            ),
            HeapNode::Interface(interface) => {
                return Err(
                    ServiceLinkableMaterializationError::DetachedInterfaceCarrier {
                        carrier: interface.carrier().kind_label(),
                    },
                );
            }
            HeapNode::Exception(_) => {
                return Err(ServiceLinkableMaterializationError::TypeMismatch);
            }
        };
        let cloned = match cloned_node {
            HeapNode::Bytes(bytes) => destination.alloc_bytes(bytes),
            HeapNode::Array(items) => destination.alloc_array(items),
            HeapNode::Object(object) => destination.alloc_object(object),
            HeapNode::Map(map) => destination.alloc_map(map),
            HeapNode::Interface(_) | HeapNode::Exception(_) => unreachable!(),
        }
        .map_err(model_error)?;
        self.active.remove(handle);
        Ok(RuntimeValue::Heap(cloned))
    }
}

pub(crate) fn model_error(
    error: skiff_runtime_model::error::RuntimeModelError,
) -> ServiceLinkableMaterializationError {
    ServiceLinkableMaterializationError::RuntimeModel {
        message: error.to_string(),
    }
}
