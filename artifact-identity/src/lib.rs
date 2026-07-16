mod constants;
mod error;
mod file_ir;
mod framing;
mod legacy_service;
mod operation;
mod package;
pub mod package_resolver;
mod package_test;
mod publication;
mod runtime_program;

pub use constants::{
    BUNDLE_IDENTITY_PREFIX, FILE_IR_IDENTITY_PREFIX, OPERATION_ABI_IDENTITY_PREFIX,
    PACKAGE_ABI_IDENTITY_PREFIX, PACKAGE_ASSEMBLY_IDENTITY_PREFIX, PACKAGE_BUILD_IDENTITY_PREFIX,
    PACKAGE_TEST_BUILD_IDENTITY_PREFIX, PACKAGE_TEST_ENTRYPOINT_ID_PREFIX,
    PACKAGE_TEST_ENTRYPOINT_LOCAL_ID_PREFIX, PUBLICATION_ABI_IDENTITY_PREFIX,
    RUNTIME_PROGRAM_BUILD_SCHEMA_MARKER, SERVICE_ASSEMBLY_IDENTITY_PREFIX,
    SERVICE_BUILD_IDENTITY_PREFIX, SERVICE_UNIT_IDENTITY_PREFIX,
};
pub use error::{ArtifactIdentityError, Result};
pub use file_ir::{
    assign_file_ir_identity, canonical_file_ir_identity_bytes, canonical_file_ir_identity_value,
    file_ir_hash, file_ir_identity, file_ir_with_identity, validate_file_ir_identity,
};
pub use legacy_service::{
    service_unit_hash, service_unit_identity, service_unit_identity_bytes,
    service_unit_identity_value,
};
pub use operation::{
    operation_abi_hash, operation_abi_identity, public_function_operation_abi_id,
    public_instance_method_operation_abi_id, OperationAbiIdentityInput,
};
pub use package::{
    assign_package_unit_identities, package_abi_hash, package_abi_identity, package_build_hash,
    package_build_identity, validate_package_unit_identities,
};
pub use package_resolver::{
    ordered_package_build_identities_from_artifact_refs,
    ordered_package_build_identities_from_artifact_root, ordered_package_units_from_artifact_refs,
    ordered_package_units_from_artifact_root, runtime_program_dynamic_build_id_from_artifact_refs,
    runtime_program_dynamic_build_id_from_artifact_root, PackageUnitArtifactRef,
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
pub use runtime_program::{
    runtime_program_dynamic_build_id, runtime_program_service_unit_identity_bytes,
    runtime_program_service_unit_identity_bytes_from_json,
    runtime_program_service_unit_identity_value,
    runtime_program_service_unit_identity_value_from_json,
};
pub use skiff_canonical_json::{canonical_json_number, canonical_json_value};

#[cfg(test)]
mod tests;
