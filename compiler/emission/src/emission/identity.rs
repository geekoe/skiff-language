pub use skiff_artifact_identity::{
    assign_file_ir_identity, assign_package_unit_identities, derive_package_test_entrypoint_id,
    file_ir_identity, package_abi_identity, package_build_identity, package_test_build_hash,
    package_test_build_identity, package_test_entrypoint_local_id,
    public_function_operation_abi_id, public_instance_method_operation_abi_id,
    publication_abi_identity, runtime_program_dynamic_build_id,
    runtime_program_dynamic_build_id_from_artifact_root,
    runtime_program_service_unit_identity_bytes_from_json, service_unit_hash,
    service_unit_identity, validate_package_test_assembly_identity, ArtifactIdentityError,
    BUNDLE_IDENTITY_PREFIX, FILE_IR_IDENTITY_PREFIX, OPERATION_ABI_IDENTITY_PREFIX,
    PACKAGE_ABI_IDENTITY_PREFIX, PACKAGE_ASSEMBLY_IDENTITY_PREFIX, PACKAGE_BUILD_IDENTITY_PREFIX,
    PACKAGE_TEST_BUILD_IDENTITY_PREFIX, PACKAGE_TEST_ENTRYPOINT_ID_PREFIX,
    PACKAGE_TEST_ENTRYPOINT_LOCAL_ID_PREFIX, PUBLICATION_ABI_IDENTITY_PREFIX,
    SERVICE_ASSEMBLY_IDENTITY_PREFIX, SERVICE_BUILD_IDENTITY_PREFIX, SERVICE_UNIT_IDENTITY_PREFIX,
};

pub fn identity(prefix: &str, hash: &str) -> String {
    format!("{prefix}:{hash}")
}
