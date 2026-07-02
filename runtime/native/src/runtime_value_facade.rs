//! Native-owned facade for runtime value, heap, and type-plan dependencies.

pub use skiff_runtime_boundary::value::{bytes_payload, bytes_value, decode_base64, encode_base64};
pub use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{runtime_map_has, ActorRef, HeapNode, RuntimeValue},
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
};

#[cfg(test)]
pub use skiff_runtime_boundary::type_descriptor::RuntimeTypePlanDescriptorExt;
