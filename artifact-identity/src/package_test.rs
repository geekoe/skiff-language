use serde::Serialize;
use serde_json::Value;
use skiff_artifact_model::{ConfigAndEffectMetadata, PackageTestAssembly, PackageTestEntrypoint};

use crate::framing::{canonical_ir_bytes, identity, sha256_hex};
use crate::{
    ArtifactIdentityError, Result, PACKAGE_TEST_BUILD_IDENTITY_PREFIX,
    PACKAGE_TEST_ENTRYPOINT_ID_PREFIX, PACKAGE_TEST_ENTRYPOINT_LOCAL_ID_PREFIX,
};
use skiff_canonical_json::canonical_json_value;

pub fn package_test_build_hash(assembly: &PackageTestAssembly) -> Result<String> {
    Ok(sha256_hex(&canonical_package_test_build_identity_bytes(
        assembly,
    )?))
}

pub fn package_test_build_identity(assembly: &PackageTestAssembly) -> Result<String> {
    Ok(identity(
        PACKAGE_TEST_BUILD_IDENTITY_PREFIX,
        &package_test_build_hash(assembly)?,
    ))
}

pub fn canonical_package_test_build_identity_value(
    assembly: &PackageTestAssembly,
) -> Result<Value> {
    let value = serde_json::to_value(PackageTestBuildIdentityPayload::from_assembly(assembly))
        .map_err(ArtifactIdentityError::SerializePackageTestBuildIdentity)?;
    Ok(canonical_json_value(&value))
}

pub fn canonical_package_test_build_identity_bytes(
    assembly: &PackageTestAssembly,
) -> Result<Vec<u8>> {
    let value = canonical_package_test_build_identity_value(assembly)?;
    serde_json::to_vec(&value).map_err(ArtifactIdentityError::SerializePackageTestBuildIdentity)
}

pub fn validate_package_test_assembly_identity(assembly: &PackageTestAssembly) -> Result<()> {
    let computed_build = package_test_build_identity(assembly)?;
    if assembly.test_build_identity != computed_build {
        return Err(ArtifactIdentityError::PackageTestBuildIdentityMismatch {
            declared: assembly.test_build_identity.clone(),
            computed: computed_build,
        });
    }

    for entrypoint in &assembly.test_entrypoints {
        let computed = derive_package_test_entrypoint_id(
            &assembly.test_build_identity,
            &entrypoint.entrypoint_local_id,
        )?;
        if entrypoint.entrypoint_id != computed {
            return Err(ArtifactIdentityError::PackageTestEntrypointIdMismatch {
                entrypoint_local_id: entrypoint.entrypoint_local_id.clone(),
                declared: entrypoint.entrypoint_id.clone(),
                computed,
            });
        }
    }

    Ok(())
}

pub fn package_test_entrypoint_local_id(
    package_id: &str,
    package_version: &str,
    source_path: &str,
    test_ordinal: u32,
    normalized_test_name: &str,
) -> Result<String> {
    let hash = sha256_hex(&canonical_ir_bytes(
        &PackageTestEntrypointLocalIdPayload {
            schema: "skiff-package-test-entrypoint-local-v1",
            package_id,
            package_version,
            source_path,
            test_ordinal,
            normalized_test_name,
        },
        ArtifactIdentityError::SerializePackageTestBuildIdentity,
    )?);
    Ok(identity(PACKAGE_TEST_ENTRYPOINT_LOCAL_ID_PREFIX, &hash))
}

pub fn derive_package_test_entrypoint_id(
    test_build_identity: &str,
    entrypoint_local_id: &str,
) -> Result<String> {
    let hash = sha256_hex(&canonical_ir_bytes(
        &PackageTestEntrypointIdPayload {
            schema: "skiff-package-test-entrypoint-v1",
            test_build_identity,
            entrypoint_local_id,
        },
        ArtifactIdentityError::SerializePackageTestBuildIdentity,
    )?);
    Ok(identity(PACKAGE_TEST_ENTRYPOINT_ID_PREFIX, &hash))
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageTestBuildIdentityPayload<'a> {
    schema_version: &'a str,
    kind: skiff_artifact_model::PackageTestAssemblyKind,
    package_id: &'a str,
    package_version: &'a str,
    production_package_unit: &'a skiff_artifact_model::PackageTestPackageUnitRef,
    dependency_package_units: &'a [skiff_artifact_model::PackageTestPackageUnitRef],
    test_file_identities: Vec<&'a str>,
    link_policy: &'a skiff_artifact_model::PackageTestLinkPolicy,
    config_and_effect_metadata: &'a ConfigAndEffectMetadata,
    test_entrypoints: Vec<PackageTestEntrypointIdentityProjection<'a>>,
}

impl<'a> PackageTestBuildIdentityPayload<'a> {
    fn from_assembly(assembly: &'a PackageTestAssembly) -> Self {
        Self {
            schema_version: &assembly.schema_version,
            kind: assembly.kind,
            package_id: &assembly.package_id,
            package_version: &assembly.package_version,
            production_package_unit: &assembly.production_package_unit,
            dependency_package_units: &assembly.dependency_package_units,
            test_file_identities: assembly
                .test_files
                .iter()
                .map(|file| file.file_ir_identity.as_str())
                .collect(),
            link_policy: &assembly.link_policy,
            config_and_effect_metadata: &assembly.config_and_effect_metadata,
            test_entrypoints: assembly
                .test_entrypoints
                .iter()
                .map(PackageTestEntrypointIdentityProjection::from_entrypoint)
                .collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageTestEntrypointIdentityProjection<'a> {
    entrypoint_local_id: &'a str,
    display_name: &'a str,
    source_path: &'a str,
    module_path: &'a str,
    owner_test_file_identity: &'a str,
    executable_ref: &'a skiff_artifact_model::PackageTestExecutableRef,
    default_run: bool,
    config_and_effect_metadata: &'a ConfigAndEffectMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_expected_error: Option<&'a skiff_artifact_model::PackageTestRuntimeExpectedError>,
}

impl<'a> PackageTestEntrypointIdentityProjection<'a> {
    fn from_entrypoint(entrypoint: &'a PackageTestEntrypoint) -> Self {
        Self {
            entrypoint_local_id: &entrypoint.entrypoint_local_id,
            display_name: &entrypoint.display_name,
            source_path: &entrypoint.source_path,
            module_path: &entrypoint.module_path,
            owner_test_file_identity: &entrypoint.owner_test_file.file_ir_identity,
            executable_ref: &entrypoint.executable_ref,
            default_run: entrypoint.default_run,
            config_and_effect_metadata: &entrypoint.config_and_effect_metadata,
            runtime_expected_error: entrypoint.runtime_expected_error.as_ref(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageTestEntrypointLocalIdPayload<'a> {
    schema: &'static str,
    package_id: &'a str,
    package_version: &'a str,
    source_path: &'a str,
    test_ordinal: u32,
    normalized_test_name: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageTestEntrypointIdPayload<'a> {
    schema: &'static str,
    test_build_identity: &'a str,
    entrypoint_local_id: &'a str,
}
