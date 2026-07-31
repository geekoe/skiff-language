use crate::{
    request_heap::RequestHeap,
    runtime_value_graph::RuntimeValueGraph,
    value::{CallbackCapabilityCarrier, HeapNode, InterfaceCarrier, InterfaceValue, RuntimeValue},
};

#[test]
fn graph_access_exposes_interface_wrapper_without_object_fallback() {
    let mut heap = RequestHeap::default();
    let handle = heap
        .alloc_interface(InterfaceValue::new(
            "pkg.Reader".to_string(),
            InterfaceCarrier::CallbackCapability(CallbackCapabilityCarrier::new(
                "runtime-a",
                "activation-a",
                1,
                "pkg.Reader",
                "reader-default",
            )),
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

#[test]
fn callback_capability_graph_access_never_falls_back_to_object_shape() {
    let mut heap = RequestHeap::default();
    let handle = heap
        .alloc_interface(InterfaceValue::new(
            "contract:reader".to_string(),
            InterfaceCarrier::CallbackCapability(CallbackCapabilityCarrier::new(
                "runtime-a",
                "activation-a",
                7,
                "contract:reader",
                "capability-1",
            )),
        ))
        .expect("callback capability should allocate");
    let value = RuntimeValue::Heap(handle);
    let graph = RuntimeValueGraph::new(&heap);

    let interface = graph
        .interface(&value)
        .expect("callback capability should stay an interface wrapper");
    assert!(matches!(
        interface.carrier(),
        InterfaceCarrier::CallbackCapability(_)
    ));
    let error = graph
        .object_fields_or_error(&value, "expected runtime object")
        .expect_err("opaque callback route must not expose object fields");
    assert!(error.to_string().contains("callback capability"));
}
