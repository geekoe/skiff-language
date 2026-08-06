mod actor;
mod artifact_coordinates;
mod artifact_path;
mod constants;
mod contract;
mod deployment;
mod ecosystem_paths;
mod error;
mod file_ir;
mod framing;
mod gateway;
mod identity_labels;
mod package_artifact;
mod runtime_assembly;
mod semantic;

pub use actor::{actor_abi_identity, actor_implementation_identity, actor_method_identity};
pub use artifact_coordinates::publication_storage_segment;
pub use artifact_path::ArtifactRelativePath;
pub use constants::{
    ACTOR_ABI_IDENTITY_PREFIX, ACTOR_ABI_IDENTITY_SCHEMA_MARKER,
    ACTOR_IMPLEMENTATION_IDENTITY_PREFIX, ACTOR_IMPLEMENTATION_IDENTITY_SCHEMA_MARKER,
    ACTOR_METHOD_IDENTITY_PREFIX, ACTOR_METHOD_IDENTITY_SCHEMA_MARKER, ASSEMBLY_IDENTITY_PREFIX,
    ASSEMBLY_IDENTITY_SCHEMA_MARKER, CONTRACT_OPERATION_IDENTITY_PREFIX,
    CONTRACT_OPERATION_IDENTITY_SCHEMA_MARKER, DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
    DEPLOYMENT_ARTIFACT_IDENTITY_SCHEMA_MARKER, FILE_IR_IDENTITY_PREFIX,
    GATEWAY_ENTRY_IDENTITY_PREFIX, GATEWAY_ENTRY_IDENTITY_SCHEMA_MARKER,
    PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX, PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER,
    PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX, PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_SCHEMA_MARKER,
    PACKAGE_SCHEMA_INDEX_IDENTITY_PREFIX, PACKAGE_SCHEMA_INDEX_IDENTITY_SCHEMA_MARKER,
    PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX, PACKAGE_SCHEMA_TYPE_IDENTITY_SCHEMA_MARKER,
    SERVICE_PROTOCOL_IDENTITY_PREFIX, SERVICE_PROTOCOL_IDENTITY_SCHEMA_MARKER,
};
pub use contract::{
    assign_service_contract_identities, contract_operation_id,
    normalize_contract_operation_contract, normalize_contract_type_shape,
    package_schema_index_identity, package_schema_type_id, service_contract_ref,
    service_protocol_identity, service_protocol_identity_hash,
    service_protocol_identity_projection, validate_package_schema_index,
    validate_package_schema_records, validate_service_contract_identities,
    ServiceProtocolIdentityProjection,
};
pub use deployment::{
    assign_service_deployment_identity, service_deployment_identity,
    service_deployment_identity_projection, service_deployment_ref,
    validate_service_deployment_identity, validate_service_deployment_input,
    validate_service_deployment_ref, validate_service_deployment_surface,
    DeploymentArtifactIdentityProjection,
};
pub use ecosystem_paths::{
    PackageArtifactPointerPath, PackageArtifactRecordPath, PackageFileIrRecordPath,
    PackageResourceRecordPath, PackageSchemaIndexRecordPath, PackageSchemaTypeRecordPath,
    ProfileActivationStatePath, ReleasePointerPath, RuntimeAssemblyPointerPath,
    RuntimeAssemblyRecordPath, ServiceContractPointerPath, ServiceContractRecordPath,
    ServiceDeploymentPointerPath, ServiceDeploymentRecordPath,
};
pub use error::{ArtifactIdentityError, Result};
pub use file_ir::{
    assign_file_ir_identity, canonical_file_ir_identity_bytes, canonical_file_ir_identity_value,
    file_ir_hash, file_ir_identity, file_ir_with_identity, validate_file_ir_identity,
};
pub use framing::framed_identity;
pub use gateway::{
    canonical_gateway_entry_identity_bytes, canonical_websocket_entry_id_bytes,
    gateway_entry_identity, gateway_entry_identity_hash, gateway_entry_identity_projection,
    normalize_gateway_entry_protocol_surface, normalize_gateway_external_schema,
    validate_gateway_entry_protocol_surface, websocket_entry_id, websocket_entry_id_projection,
    GatewayEntryIdentityProjection, WebSocketEntryIdProjection, WEBSOCKET_ENTRY_ID_SCHEMA_MARKER,
};
pub use package_artifact::{
    assign_package_artifact_identities, package_artifact_build_identity,
    package_artifact_build_identity_projection, package_artifact_local_abi_identity,
    package_artifact_local_abi_identity_projection, package_artifact_ref,
    validate_package_artifact_identities, PackageArtifactBuildIdentityProjection,
    PackageArtifactLocalAbiIdentityProjection, ValidatedPackageArtifact,
};
pub use runtime_assembly::{
    assign_runtime_assembly_identity, runtime_assembly_identity,
    runtime_assembly_identity_projection, runtime_assembly_ref, validate_runtime_assembly_identity,
    validate_runtime_assembly_surface, AssemblyIdentityProjection,
};
pub use semantic::{
    abi_alias_id_from_source_anchor, abi_callable_id_from_source_anchor,
    abi_const_id_from_source_anchor, abi_instance_id_from_source_anchor,
    abi_interface_id_from_source_anchor, abi_symbol_id_fact, abi_type_id_from_source_anchor,
    abi_type_id_key, canonical_interface_instantiation_key, canonical_interface_method_abi_id,
    canonical_interface_method_abi_id_from_parts, interface_instantiation_ref,
    interface_instantiation_ref_for_type_ref, type_ref_abi_key,
};
pub use skiff_canonical_json::{canonical_json_number, canonical_json_value};

#[cfg(test)]
mod tests;
