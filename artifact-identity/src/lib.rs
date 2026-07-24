mod artifact_coordinates;
mod artifact_path;
mod artifact_reference;
mod constants;
mod contract;
mod deployment;
mod ecosystem_paths;
mod error;
mod file_ir;
mod framing;
mod identity_labels;
mod legacy_service;
mod operation;
mod package;
mod package_artifact;
pub mod package_resolver;
mod package_test;
mod publication;
mod publication_validation;
mod runtime_assembly;
mod runtime_program;
mod semantic;
mod service_artifact_closure;
mod service_assembly_identity;

pub use artifact_coordinates::{
    package_unit_content_hash, publication_storage_segment, validate_package_unit_artifact_path,
    validate_service_assembly_artifact_path,
};
pub use artifact_path::ArtifactRelativePath;
pub use artifact_reference::{
    package_unit_artifact_ref, service_unit_artifact_ref, PackageUnitArtifactRef,
    ServiceAssemblyArtifactRef, ServiceUnitArtifactRef,
};
pub use constants::{
    ASSEMBLY_IDENTITY_PREFIX, ASSEMBLY_IDENTITY_SCHEMA_MARKER, BUNDLE_IDENTITY_PREFIX,
    CONTRACT_OPERATION_IDENTITY_PREFIX, CONTRACT_OPERATION_IDENTITY_SCHEMA_MARKER,
    DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX, DEPLOYMENT_ARTIFACT_IDENTITY_SCHEMA_MARKER,
    FILE_IR_IDENTITY_PREFIX, OPERATION_ABI_IDENTITY_PREFIX, PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
    PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER, PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
    PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_SCHEMA_MARKER, PACKAGE_ASSEMBLY_IDENTITY_PREFIX,
    PACKAGE_BUILD_IDENTITY_PREFIX, PACKAGE_BUILD_IDENTITY_SCHEMA_MARKER,
    PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX, PACKAGE_LOCAL_ABI_IDENTITY_PREFIX,
    PACKAGE_LOCAL_ABI_IDENTITY_SCHEMA_MARKER, PACKAGE_SCHEMA_INDEX_IDENTITY_PREFIX,
    PACKAGE_SCHEMA_INDEX_IDENTITY_SCHEMA_MARKER, PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX,
    PACKAGE_SCHEMA_TYPE_IDENTITY_SCHEMA_MARKER, PACKAGE_TEST_BUILD_IDENTITY_PREFIX,
    PACKAGE_TEST_ENTRYPOINT_ID_PREFIX, PACKAGE_TEST_ENTRYPOINT_LOCAL_ID_PREFIX,
    PUBLICATION_ABI_IDENTITY_PREFIX, RUNTIME_PROGRAM_BUILD_SCHEMA_MARKER,
    SERVICE_ASSEMBLY_IDENTITY_PREFIX, SERVICE_BUILD_IDENTITY_PREFIX,
    SERVICE_PROTOCOL_IDENTITY_PREFIX, SERVICE_PROTOCOL_IDENTITY_SCHEMA_MARKER,
    SERVICE_UNIT_IDENTITY_PREFIX,
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
    EnvironmentActivationStatePath, PackageArtifactPointerPath, PackageArtifactRecordPath,
    PackageFileIrRecordPath, PackageResourceRecordPath, PackageSchemaIndexRecordPath,
    PackageSchemaTypeRecordPath, RuntimeAssemblyPointerPath, RuntimeAssemblyRecordPath,
    ServiceContractPointerPath, ServiceContractRecordPath, ServiceDeploymentPointerPath,
    ServiceDeploymentRecordPath,
};
pub use error::{ArtifactIdentityError, Result};
pub use file_ir::{
    assign_file_ir_identity, canonical_file_ir_identity_bytes, canonical_file_ir_identity_value,
    file_ir_hash, file_ir_identity, file_ir_with_identity, validate_file_ir_identity,
};
pub use framing::framed_identity;
pub use legacy_service::{
    service_unit_hash, service_unit_identity, service_unit_identity_bytes,
    service_unit_identity_value,
};
pub use operation::{
    operation_abi_hash, operation_abi_identity, public_function_operation_abi_id,
    public_instance_method_operation_abi_id, OperationAbiIdentityInput,
};
pub use package::{
    assign_package_unit_identities, package_build_hash, package_build_identity,
    package_build_identity_projection, package_implementation_links_identity,
    package_local_abi_hash, package_local_abi_identity, package_local_abi_identity_projection,
    validate_package_unit_identities, PackageBuildIdentityProjection,
    PackageLocalAbiIdentityProjection,
};
pub use package_artifact::{
    assign_package_artifact_identities, package_artifact_build_identity,
    package_artifact_build_identity_projection, package_artifact_local_abi_identity,
    package_artifact_local_abi_identity_projection, package_artifact_ref,
    validate_package_artifact_identities, PackageArtifactBuildIdentityProjection,
    PackageArtifactLocalAbiIdentityProjection,
};
pub use package_resolver::{
    ordered_package_build_identities_from_artifact_refs,
    ordered_package_build_identities_from_artifact_root, ordered_package_units_from_artifact_refs,
    ordered_package_units_from_artifact_root, runtime_program_dynamic_build_id_from_artifact_refs,
    runtime_program_dynamic_build_id_from_artifact_root,
};
pub use package_test::{
    canonical_package_test_build_identity_bytes, canonical_package_test_build_identity_value,
    derive_package_test_entrypoint_id, package_test_build_hash, package_test_build_identity,
    package_test_entrypoint_local_id, validate_package_test_assembly_identity,
};
pub use publication::{
    assign_publication_abi_identity, publication_abi_hash, publication_abi_identity,
    publication_abi_identity_bytes,
};
pub use publication_validation::{
    validate_publication_abi_identity, validate_publication_abi_surface,
};
pub use runtime_assembly::{
    assign_runtime_assembly_identity, runtime_assembly_identity,
    runtime_assembly_identity_projection, runtime_assembly_ref, validate_runtime_assembly_identity,
    validate_runtime_assembly_surface, AssemblyIdentityProjection,
};
pub use runtime_program::{
    runtime_program_dynamic_build_id, runtime_program_service_unit_identity_bytes,
    runtime_program_service_unit_identity_bytes_from_json,
    runtime_program_service_unit_identity_value,
    runtime_program_service_unit_identity_value_from_json,
};
pub use semantic::{
    abi_alias_id_from_source_anchor, abi_callable_id_from_source_anchor,
    abi_const_id_from_source_anchor, abi_instance_id_from_source_anchor,
    abi_interface_id_from_source_anchor, abi_symbol_id_fact, abi_type_id_from_source_anchor,
    abi_type_id_key, canonical_interface_instantiation_key, canonical_interface_method_abi_id,
    canonical_interface_method_abi_id_from_parts, interface_instantiation_ref,
    interface_instantiation_ref_for_type_ref, type_ref_abi_key,
};
pub use service_artifact_closure::{
    validate_service_artifact_closure, ValidatedArtifactContent, ValidatedServiceArtifactClosure,
};
pub use service_assembly_identity::{
    service_assembly_hash, service_assembly_identity, service_assembly_identity_projection,
    service_build_identity_from_assembly_identity, service_build_identity_hash,
    validate_service_assembly_identity,
};
pub use skiff_canonical_json::{canonical_json_number, canonical_json_value};

#[cfg(test)]
mod tests;
