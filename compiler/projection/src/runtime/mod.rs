//! Runtime projection helpers used by publish orchestration and artifact
//! assembly.

mod entrypoints;
mod gateway;
mod operation_effects;
mod operation_validation;
mod operations;
mod schema;
mod service_operations;
pub mod source_runtime;
pub mod storage_metadata;
mod timeout_validation;
mod type_normalization;
mod type_ref_text;
mod websocket_bindings;
mod websocket_manifest;
mod websocket_validation;

pub use entrypoints::{
    build_entry_point_artifacts, EntryOperationCallable, EntryOperationSpec,
    PackageGatewayProjection,
};
pub use gateway::{gateway_entry, timeout_entry, GatewayEntry, TimeoutEntry};
pub use operation_effects::effect_summary_for_signature;
pub use operation_validation::validate_runtime_operation_modes;
pub use operations::{
    interface_modules, service_operation_adapter_symbol, service_operation_entries,
    OperationEntryIr,
};
pub use schema::{package_runtime_schema_for_type_ref, package_runtime_schema_for_type_spec};
pub use service_operations::{
    build_artifact_operations, build_public_instance_artifact_operations,
    build_public_instance_runtime_operations, build_runtime_operations, raw_http_gateway_operation,
};
pub use source_runtime::compile_error_to_publication_error;
pub use storage_metadata::validate_service_storage_projection_namespace;
pub use timeout_validation::validate_timeout_targets;
pub use type_normalization::{
    is_connection_message_type, is_gateway_connect_result_type, is_http_request_type,
    is_http_response_stream_event_type, is_http_response_type, is_nullable_http_response_type,
    is_projection_connection_message_type, is_projection_gateway_connect_result_type,
    is_projection_null_or_void_type, is_projection_string_type,
    is_projection_websocket_connection_type, is_projection_websocket_receive_event_type,
    is_websocket_connect_request_type, is_websocket_receive_event_root, normalize_type_name,
    projection_gateway_connect_result_context_type, projection_type_matches_text,
};
pub use type_ref_text::{
    entry_function_type_ref_source_text, entry_type_source_text_with_named_types, response_type_ir,
    type_ref_ir_source_text_with_local_types,
};
pub use websocket_bindings::{
    validate_operation_adapter_args, validate_receive_adapter_args,
    validate_websocket_adapter_sources,
};
pub use websocket_manifest::build_websocket_manifest;
pub use websocket_validation::validate_websocket_gateway;
