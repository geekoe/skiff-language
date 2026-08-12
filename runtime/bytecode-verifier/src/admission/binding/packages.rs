use skiff_artifact_model::{
    bytecode::opcodes::opcode_table_fingerprint, current_platform_error_projection_registry_ref,
    host_effect_registry_identity, intrinsic_registry_identity,
    native_value_lifecycle_registry_identity, value_lifecycle_policy_identity,
    PlatformErrorProjectionRegistryRef, BYTECODE_ISA_VERSION, BYTECODE_MAGIC,
    BYTECODE_SCHEMA_VERSION,
};
use skiff_runtime_linked_bytecode::{CandidateTable, LinkedBytecodeCandidate};
use skiff_runtime_loader::{HydratedBytecodePackage, HydratedDeploymentBytecode};

use crate::{VerificationError, VerificationLocation};

use super::{row_u32, semantic_violation, table_location};

pub(super) fn prove_owner_and_packages(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    prove_owner(hydrated)?;
    if candidate.packages().len() != hydrated.packages().len() {
        return Err(semantic_violation(
            VerificationLocation::Image,
            format!(
                "candidate package set has {} rows; exact hydration has {}",
                candidate.packages().len(),
                hydrated.packages().len()
            ),
        ));
    }

    for (row_number, (candidate_package, (map_key, hydrated_package))) in candidate
        .packages()
        .iter()
        .zip(hydrated.packages())
        .enumerate()
    {
        let location = table_location(
            CandidateTable::Packages,
            row_u32(CandidateTable::Packages, row_number)?,
        );
        if candidate_package.package_build_id() != map_key
            || hydrated_package.reference().package_build_id != *map_key
        {
            return Err(semantic_violation(
                location,
                "candidate, hydration map, and package reference build ids disagree",
            ));
        }
        prove_package_header(
            candidate_package,
            hydrated_package,
            hydrated.platform_error_projection_registry(),
            location,
        )?;
    }
    Ok(())
}

fn prove_owner(hydrated: &HydratedDeploymentBytecode) -> Result<(), VerificationError> {
    let reference = hydrated.reference();
    let deployment = hydrated.deployment();
    let exact = reference.service_id == deployment.contract.service_id
        && reference.contract_version == deployment.contract.contract_version
        && reference.deployment_revision == deployment.deployment_revision
        && reference.deployment_artifact_identity == deployment.deployment_artifact_identity;
    if !exact {
        return Err(semantic_violation(
            VerificationLocation::Image,
            "opaque hydration owner disagrees with its exact deployment record",
        ));
    }
    let implementation = &deployment.implementation;
    let exact_implementation = hydrated
        .packages()
        .get(&implementation.package_build_id)
        .is_some_and(|package| package.reference() == implementation);
    if !exact_implementation {
        return Err(semantic_violation(
            VerificationLocation::Image,
            "exact implementation package is absent from the hydrated package closure",
        ));
    }
    Ok(())
}

fn prove_package_header(
    candidate: &skiff_runtime_linked_bytecode::LinkedPackageBytecodeProvenance,
    package: &HydratedBytecodePackage,
    deployment_registry_receipt: &PlatformErrorProjectionRegistryRef,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let admitted = package.bytecode();
    let artifact = admitted.artifact();
    let view = admitted.view();
    let expected_fingerprint = opcode_table_fingerprint();
    let expected_native_registry = native_value_lifecycle_registry_identity();
    let expected_policy = value_lifecycle_policy_identity();
    let expected_host_registry = host_effect_registry_identity();
    let expected_intrinsic_registry = intrinsic_registry_identity();
    let package_bytecode_ref = package.artifact().bytecode.as_ref();

    let exact_header = candidate.artifact_ref() == admitted.reference()
        && package_bytecode_ref == Some(admitted.reference())
        && candidate.declared_bytecode_identity()
            == admitted.reference().bytecode_identity.as_str()
        && candidate.declared_bytecode_identity() == artifact.bytecode_identity.as_str()
        && candidate.declared_bytecode_identity() == view.bytecode_identity()
        && candidate.magic() == BYTECODE_MAGIC
        && candidate.magic() == artifact.magic.as_str()
        && candidate.schema_version() == BYTECODE_SCHEMA_VERSION
        && candidate.schema_version() == artifact.schema_version.as_str()
        && candidate.schema_version() == view.schema_version()
        && candidate.isa_version() == BYTECODE_ISA_VERSION
        && candidate.isa_version() == artifact.isa_version.as_str()
        && candidate.isa_version() == view.isa_version()
        && candidate.opcode_table_fingerprint() == expected_fingerprint.as_str()
        && candidate.opcode_table_fingerprint() == artifact.opcode_table_fingerprint.as_str()
        && candidate.opcode_table_fingerprint() == view.opcode_table_fingerprint();
    if !exact_header {
        return Err(semantic_violation(
            location,
            format!(
                "package {} candidate header is not the exact admitted v7 header/reference",
                package.reference().package_build_id
            ),
        ));
    }
    prove_authority_pin(
        package.reference().package_build_id.as_str(),
        "native value lifecycle registry",
        candidate.authorities().native_value_lifecycle_registry(),
        &artifact.native_value_lifecycle_registry,
        view.native_value_lifecycle_registry(),
        expected_native_registry,
        location,
    )?;
    prove_authority_pin(
        package.reference().package_build_id.as_str(),
        "value lifecycle policy",
        candidate.authorities().value_lifecycle_policy(),
        &artifact.value_lifecycle_policy,
        view.value_lifecycle_policy(),
        expected_policy,
        location,
    )?;
    prove_authority_pin(
        package.reference().package_build_id.as_str(),
        "host effect registry",
        candidate.authorities().host_effect_registry(),
        &artifact.host_effect_registry,
        view.host_effect_registry(),
        expected_host_registry,
        location,
    )?;
    prove_authority_pin(
        package.reference().package_build_id.as_str(),
        "intrinsic registry",
        candidate.authorities().intrinsic_registry(),
        &artifact.intrinsic_registry,
        view.intrinsic_registry(),
        expected_intrinsic_registry,
        location,
    )?;
    prove_platform_error_projection_registry_pin(
        package.reference().package_build_id.as_str(),
        candidate,
        deployment_registry_receipt,
        package,
        location,
    )?;
    if candidate.artifact_ref().artifact_path.is_some() {
        return Err(semantic_violation(
            location,
            "candidate package provenance retained a storage path",
        ));
    }
    Ok(())
}

fn prove_platform_error_projection_registry_pin(
    package_build_id: &str,
    candidate: &skiff_runtime_linked_bytecode::LinkedPackageBytecodeProvenance,
    deployment_registry_receipt: &PlatformErrorProjectionRegistryRef,
    package: &HydratedBytecodePackage,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let admitted = package.bytecode();
    let artifact = admitted.artifact();
    let view = admitted.view();
    let pins = [
        candidate.authorities().platform_error_projection_registry(),
        deployment_registry_receipt,
        package.platform_error_projection_registry(),
        &package.artifact().platform_error_projection_registry,
        &artifact.platform_error_projection_registry,
        view.platform_error_projection_registry(),
        current_platform_error_projection_registry_ref(),
    ];
    if pins.windows(2).all(|pair| pair[0] == pair[1]) {
        return Ok(());
    }

    Err(semantic_violation(
        location,
        format!(
            "package {package_build_id} platform error projection registry pin is not exact across candidate, deployment hydration receipt, package hydration receipt, PackageArtifact root, BytecodeArtifact header, validated view, and runtime authority"
        ),
    ))
}

fn prove_authority_pin<T: Eq>(
    package_build_id: &str,
    authority: &str,
    candidate: &T,
    artifact: &T,
    validated_view: &T,
    compile_time: &T,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let exact =
        candidate == artifact && artifact == validated_view && validated_view == compile_time;
    if exact {
        return Ok(());
    }

    Err(semantic_violation(
        location,
        format!(
            "package {package_build_id} {authority} pin is not exact across candidate, admitted artifact, validated view, and compile-time authority"
        ),
    ))
}
