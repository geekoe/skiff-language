use crate::{
    error::{Result, RuntimeModelError as RuntimeError},
    request_heap::RequestHeap,
    value::{HeapNode, InterfaceValue, RuntimeMap, RuntimeObjectFields, RuntimeValue},
};

pub struct RuntimeValueGraph<'a> {
    heap: &'a RequestHeap,
}

impl<'a> RuntimeValueGraph<'a> {
    pub const fn new(heap: &'a RequestHeap) -> Self {
        Self { heap }
    }

    pub fn bytes(&self, value: &RuntimeValue) -> Result<&'a [u8]> {
        self.bytes_or_error(value, "expected runtime bytes")
    }

    pub fn bytes_or_error(&self, value: &RuntimeValue, expected_message: &str) -> Result<&'a [u8]> {
        match value {
            RuntimeValue::Heap(handle) => match self.heap.get(*handle)? {
                HeapNode::Bytes(bytes) => Ok(bytes.as_slice()),
                HeapNode::Interface(value) => {
                    Err(interface_graph_error(value, expected_message, "bytes"))
                }
                _ => Err(RuntimeError::Decode(expected_message.to_string())),
            },
            _ => Err(RuntimeError::Decode(expected_message.to_string())),
        }
    }

    pub fn array(&self, value: &RuntimeValue) -> Result<&'a [RuntimeValue]> {
        self.array_or_error(value, "expected runtime array")
    }

    pub fn array_or_error(
        &self,
        value: &RuntimeValue,
        expected_message: &str,
    ) -> Result<&'a [RuntimeValue]> {
        match value {
            RuntimeValue::Heap(handle) => match self.heap.get(*handle)? {
                HeapNode::Array(items) => Ok(items.as_slice()),
                HeapNode::Interface(value) => {
                    Err(interface_graph_error(value, expected_message, "array"))
                }
                _ => Err(RuntimeError::Decode(expected_message.to_string())),
            },
            _ => Err(RuntimeError::Decode(expected_message.to_string())),
        }
    }

    pub fn map(&self, value: &RuntimeValue) -> Result<&'a RuntimeMap> {
        self.map_or_error(value, "expected runtime map")
    }

    pub fn map_or_error(
        &self,
        value: &RuntimeValue,
        expected_message: &str,
    ) -> Result<&'a RuntimeMap> {
        match value {
            RuntimeValue::Heap(handle) => match self.heap.get(*handle)? {
                HeapNode::Map(map) => Ok(map),
                HeapNode::Interface(value) => {
                    Err(interface_graph_error(value, expected_message, "map"))
                }
                _ => Err(RuntimeError::Decode(expected_message.to_string())),
            },
            _ => Err(RuntimeError::Decode(expected_message.to_string())),
        }
    }

    pub fn interface(&self, value: &RuntimeValue) -> Result<&'a InterfaceValue> {
        self.interface_or_error(value, "expected runtime interface value")
    }

    pub fn interface_or_error(
        &self,
        value: &RuntimeValue,
        expected_message: &str,
    ) -> Result<&'a InterfaceValue> {
        match value {
            RuntimeValue::Heap(handle) => match self.heap.get(*handle)? {
                HeapNode::Interface(value) => Ok(value),
                _ => Err(RuntimeError::Decode(expected_message.to_string())),
            },
            _ => Err(RuntimeError::Decode(expected_message.to_string())),
        }
    }

    pub fn object_fields_or_error(
        &self,
        value: &RuntimeValue,
        expected_message: &str,
    ) -> Result<&'a RuntimeObjectFields> {
        match value {
            RuntimeValue::Heap(handle) => match self.heap.get(*handle)? {
                HeapNode::Object(object) => Ok(object.fields()),
                HeapNode::Interface(value) => {
                    Err(interface_graph_error(value, expected_message, "object"))
                }
                _ => Err(RuntimeError::Decode(expected_message.to_string())),
            },
            _ => Err(RuntimeError::Decode(expected_message.to_string())),
        }
    }
}

fn interface_graph_error(
    value: &InterfaceValue,
    expected_message: &str,
    requested_shape: &str,
) -> RuntimeError {
    RuntimeError::Decode(format!(
        "{expected_message}: {} is not a runtime {requested_shape}",
        value.diagnostic_label()
    ))
}

#[cfg(test)]
mod tests {
    use crate::{
        request_heap::RequestHeap,
        runtime_value_graph::RuntimeValueGraph,
        value::{HeapNode, InterfaceCarrier, InterfaceValue, RuntimeValue},
        value::{RemoteOperationSlot, RemoteOperationTable},
    };

    #[test]
    fn graph_access_exposes_interface_wrapper_without_object_fallback() {
        let mut heap = RequestHeap::default();
        let handle = heap
            .alloc_interface(InterfaceValue::new(
                "pkg.Reader".to_string(),
                InterfaceCarrier::Remote {
                    dependency_ref: "reader-service".to_string(),
                    public_instance_key: "readers/default".to_string(),
                    operations: RemoteOperationTable::new(
                        "remote:reader".to_string(),
                        "pkg.Reader".to_string(),
                        vec![RemoteOperationSlot::new(
                            0,
                            "method:pkg.Reader:read".to_string(),
                            "operation:reader:read".to_string(),
                        )],
                    ),
                },
            ))
            .expect("interface should allocate");
        let value = RuntimeValue::Heap(handle);
        let graph = RuntimeValueGraph::new(&heap);

        assert_eq!(
            graph
                .interface(&value)
                .expect("interface should be accessible")
                .interface(),
            "pkg.Reader"
        );
        let error = graph
            .object_fields_or_error(&value, "expected runtime object")
            .unwrap_err();
        assert!(
            error.to_string().contains("any interface pkg.Reader"),
            "unexpected error: {error}"
        );
        assert!(matches!(
            heap.get(handle).expect("interface should resolve"),
            HeapNode::Interface(_)
        ));
    }
}
