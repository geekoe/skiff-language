mod assembly_seam;
pub mod error;
pub mod http_plan;
pub mod native_call_plan;
pub mod type_plan;

pub use assembly_seam::{RuntimeAssemblyTypePlanSeamError, RuntimeAssemblyTypePlanTarget};
pub use error::{Error, Result};
pub use http_plan::{
    binary_http_request_parameter_plan, binary_http_response_plan,
    linked_http_response_stream_item_type, linked_type_ref_is_http_response_stream,
};
pub use native_call_plan::{
    program_call_first_type_arg_plan, resolve_call_plan_with_registry, resolve_native_call_plan,
    validate_native_call_artifact,
};
pub use skiff_runtime_model::type_plan::{
    RuntimeRecordFieldPlan, RuntimeTypeNode, RuntimeTypePlan,
};
pub use skiff_runtime_native_contract::{NativeCallValidation, NativeSignatureRegistry};
pub use type_plan::{
    build_recoverable_behavior_index, interface_method_table_from_linked,
    linked_interface_instantiation_runtime_id, linked_type_ref_runtime_key,
    method_tables_runtime_equivalent, recoverable_interface_projection_identity,
    runtime_interface_method_table_id, PlanContext, ProgramTypeView,
    RuntimeRecoverableExpectedTypePlanLinkedExt, RuntimeTypePlanLinkedExt,
};
