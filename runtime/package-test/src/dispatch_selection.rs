use std::{
    collections::BTreeSet,
    fmt, fs,
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;
use skiff_artifact_identity::{
    derive_package_test_entrypoint_id, validate_package_test_assembly_identity,
    PACKAGE_TEST_BUILD_IDENTITY_PREFIX, PACKAGE_TEST_ENTRYPOINT_LOCAL_ID_PREFIX,
};
use skiff_artifact_model::{
    PackageTestAssembly, PackageTestAssemblyKind, PackageTestEntrypoint, PackageTestEntrypointKind,
    PackageTestFileIrRef, PackageTestPackageUnitRef,
};

use super::PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX;

const PACKAGE_TEST_ACTIVATION_ID_PREFIX: &str = "skiff-package-test-run-v1:";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ArtifactRootRelativePath {
    path: PathBuf,
}

impl ArtifactRootRelativePath {
    fn new(path: impl AsRef<Path>, label: &str) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !is_safe_artifact_root_relative_path(path) {
            anyhow::bail!(
                "{} path {} must be relative and stay inside artifacts root",
                label,
                path.display()
            );
        }
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    fn parse(path: &str, label: &str) -> anyhow::Result<Self> {
        Self::new(Path::new(path), label)
    }

    fn as_path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Display for ArtifactRootRelativePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.path.display())
    }
}

pub(super) fn publication_storage_segment(value: &str, label: &str) -> anyhow::Result<String> {
    validate_publication_id(value, label)?;
    Ok(value.replace('.', "~").replace('/', "~~"))
}

fn validate_publication_id(value: &str, label: &str) -> anyhow::Result<()> {
    if value.is_empty() || value.len() > 63 || value == "std" {
        anyhow::bail!("{label} {value} must be a publication id");
    }
    if value != value.trim()
        || value.bytes().any(|byte| byte.is_ascii_control())
        || value.contains("://")
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains("//")
        || value.contains('~')
        || value
            .bytes()
            .any(|byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'))
    {
        anyhow::bail!("{label} {value} must be a publication id");
    }

    let Some((authority, local)) = value.split_once('/') else {
        anyhow::bail!("{label} {value} must be a publication id");
    };
    validate_authority(authority, label, value)?;
    if local.is_empty()
        || local
            .split('/')
            .any(|segment| !is_valid_local_segment(segment))
    {
        anyhow::bail!("{label} {value} must be a publication id");
    }
    Ok(())
}

fn validate_authority(authority: &str, label: &str, value: &str) -> anyhow::Result<()> {
    let labels = authority.split('.').collect::<Vec<_>>();
    if labels.len() < 2 || labels.iter().any(|item| !is_valid_authority_label(item)) {
        anyhow::bail!("{label} {value} must be a publication id");
    }
    Ok(())
}

fn is_valid_authority_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    !bytes.is_empty()
        && bytes[0] != b'-'
        && bytes.last() != Some(&b'-')
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn is_valid_local_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_lowercase()
        && bytes.last() != Some(&b'-')
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_' || *byte == b'-'
        })
}

fn is_safe_artifact_root_relative_path(path: &Path) -> bool {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return false;
    }
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageTestDispatchSelection {
    pub package_id: String,
    pub package_version: String,
    pub test_build_identity: String,
    pub entrypoint_id: String,
    pub activation_id: String,
}

impl PackageTestDispatchSelection {
    pub fn build_selection(&self) -> PackageTestBuildSelection {
        PackageTestBuildSelection {
            package_id: self.package_id.clone(),
            package_version: self.package_version.clone(),
            test_build_identity: self.test_build_identity.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageTestBuildSelection {
    pub package_id: String,
    pub package_version: String,
    pub test_build_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedPackageTestBuild {
    pub artifact_root: PathBuf,
    pub assembly_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedPackageTestDispatch {
    pub artifact_root: PathBuf,
    pub assembly_path: PathBuf,
    pub entrypoint_id: String,
}

#[derive(Debug, Clone)]
pub struct PackageTestBuildArtifact {
    pub validated: ValidatedPackageTestBuild,
    pub assembly: PackageTestAssembly,
}

#[derive(Debug, Clone)]
pub struct PackageTestDispatchArtifact {
    pub validated: ValidatedPackageTestDispatch,
    pub assembly: PackageTestAssembly,
    pub entrypoint: PackageTestEntrypoint,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PackageTestDevPointer {
    schema_version: String,
    package_id: String,
    package_version: String,
    test_build_identity: String,
    package_test_assembly: PackageTestAssemblyPointer,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PackageTestAssemblyPointer {
    assembly_path: String,
    assembly_identity: String,
}

#[cfg(test)]
pub(crate) fn validate_package_test_dispatch_from_artifact_roots(
    artifact_roots: &[PathBuf],
    selection: &PackageTestDispatchSelection,
) -> anyhow::Result<ValidatedPackageTestDispatch> {
    Ok(
        load_package_test_dispatch_artifact_from_artifact_roots(artifact_roots, selection)?
            .validated,
    )
}

pub fn load_package_test_dispatch_artifact_from_artifact_roots(
    artifact_roots: &[PathBuf],
    selection: &PackageTestDispatchSelection,
) -> anyhow::Result<PackageTestDispatchArtifact> {
    let build = load_package_test_build_artifact_from_artifact_roots(
        artifact_roots,
        &selection.build_selection(),
    )?;
    let entrypoint = select_package_test_entrypoint(&build.assembly, selection)?;

    Ok(PackageTestDispatchArtifact {
        validated: ValidatedPackageTestDispatch {
            artifact_root: build.validated.artifact_root,
            assembly_path: build.validated.assembly_path,
            entrypoint_id: selection.entrypoint_id.clone(),
        },
        assembly: build.assembly,
        entrypoint,
    })
}

pub fn load_package_test_build_artifact_from_artifact_roots(
    artifact_roots: &[PathBuf],
    selection: &PackageTestBuildSelection,
) -> anyhow::Result<PackageTestBuildArtifact> {
    if artifact_roots.is_empty() {
        anyhow::bail!(
            "no artifact roots are configured for package-test dispatch packageId {} testBuildIdentity {}",
            selection.package_id,
            selection.test_build_identity
        );
    }

    let package_path = publication_storage_segment(&selection.package_id, "packageId")?;
    let test_build_hash = identity_hash(
        &selection.test_build_identity,
        PACKAGE_TEST_BUILD_IDENTITY_PREFIX,
        "testBuildIdentity",
    )?;
    let pointer_relative = ArtifactRootRelativePath::new(
        PathBuf::from("dev")
            .join("package-tests")
            .join(&package_path)
            .join(format!("{test_build_hash}.json")),
        "package test dev pointer",
    )?;

    let mut missing = Vec::new();
    for artifact_root in artifact_roots {
        let pointer_path = artifact_root.join(pointer_relative.as_path());
        if !pointer_path.is_file() {
            missing.push(pointer_path);
            continue;
        }
        return validate_package_test_build_from_pointer(
            artifact_root,
            &pointer_path,
            &package_path,
            test_build_hash,
            selection,
        );
    }

    anyhow::bail!(
        "package-test dev pointer {} was not found in artifact roots {}",
        pointer_relative,
        missing
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn validate_package_test_build_from_pointer(
    artifact_root: &Path,
    pointer_path: &Path,
    package_path: &str,
    test_build_hash: &str,
    selection: &PackageTestBuildSelection,
) -> anyhow::Result<PackageTestBuildArtifact> {
    let pointer_text = fs::read_to_string(pointer_path)
        .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", pointer_path.display()))?;
    let pointer: PackageTestDevPointer = serde_json::from_str(&pointer_text)
        .map_err(|error| anyhow::anyhow!("failed to parse {}: {error}", pointer_path.display()))?;
    validate_dev_pointer(pointer_path, &pointer, selection)?;

    let assembly_pointer = &pointer.package_test_assembly;
    let assembly_relative = ArtifactRootRelativePath::parse(
        &assembly_pointer.assembly_path,
        "package test assembly pointer assemblyPath",
    )?;
    let expected_assembly_relative = ArtifactRootRelativePath::new(
        PathBuf::from("assemblies")
            .join("package-tests")
            .join(package_path)
            .join(format!("{test_build_hash}.json")),
        "package test assembly",
    )?;
    if assembly_relative != expected_assembly_relative {
        anyhow::bail!(
            "{} assemblyPath {} must be {} for packageId {} testBuildIdentity {}",
            pointer_path.display(),
            assembly_relative,
            expected_assembly_relative,
            selection.package_id,
            selection.test_build_identity
        );
    }
    if assembly_pointer.assembly_identity != selection.test_build_identity {
        anyhow::bail!(
            "{} assemblyIdentity {} must equal testBuildIdentity {}",
            pointer_path.display(),
            assembly_pointer.assembly_identity,
            selection.test_build_identity
        );
    }

    let assembly_path = artifact_root.join(assembly_relative.as_path());
    let assembly_text = fs::read_to_string(&assembly_path)
        .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", assembly_path.display()))?;
    let assembly: PackageTestAssembly = serde_json::from_str(&assembly_text).map_err(|error| {
        anyhow::anyhow!(
            "failed to parse package test assembly {}: {error}",
            assembly_path.display()
        )
    })?;
    validate_assembly(&assembly, selection, test_build_hash)?;

    Ok(PackageTestBuildArtifact {
        validated: ValidatedPackageTestBuild {
            artifact_root: artifact_root.to_path_buf(),
            assembly_path,
        },
        assembly,
    })
}

pub(super) fn validate_package_test_activation_id(value: &str) -> anyhow::Result<()> {
    let Some(rest) = value.strip_prefix(PACKAGE_TEST_ACTIVATION_ID_PREFIX) else {
        anyhow::bail!(
            "package-test activationId must start with {PACKAGE_TEST_ACTIVATION_ID_PREFIX}, got {value}"
        );
    };
    if rest.is_empty() {
        anyhow::bail!("package-test activationId must not have an empty run suffix");
    }
    if rest.contains("..") {
        anyhow::bail!("package-test activationId must not contain .., got {value}");
    }
    if rest
        .bytes()
        .any(|byte| matches!(byte, b'/' | b'\\' | b'%') || byte.is_ascii_whitespace())
    {
        anyhow::bail!(
            "package-test activationId suffix must be a single URL/path safe segment, got {value}"
        );
    }
    if !rest.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'~' | b'-')
    }) {
        anyhow::bail!("package-test activationId suffix must match [A-Za-z0-9._:~-]+, got {value}");
    }
    Ok(())
}

fn validate_dev_pointer(
    pointer_path: &Path,
    pointer: &PackageTestDevPointer,
    selection: &PackageTestBuildSelection,
) -> anyhow::Result<()> {
    if pointer.schema_version != "skiff-package-test-dev-pointer-v1" {
        anyhow::bail!(
            "{} schemaVersion must be skiff-package-test-dev-pointer-v1, got {}",
            pointer_path.display(),
            pointer.schema_version
        );
    }
    if pointer.package_id != selection.package_id {
        anyhow::bail!(
            "{} packageId {} does not match dispatch packageId {}",
            pointer_path.display(),
            pointer.package_id,
            selection.package_id
        );
    }
    if pointer.package_version != selection.package_version {
        anyhow::bail!(
            "{} packageVersion {} does not match dispatch packageVersion {}",
            pointer_path.display(),
            pointer.package_version,
            selection.package_version
        );
    }
    if pointer.test_build_identity != selection.test_build_identity {
        anyhow::bail!(
            "{} testBuildIdentity {} does not match dispatch testBuildIdentity {}",
            pointer_path.display(),
            pointer.test_build_identity,
            selection.test_build_identity
        );
    }
    if pointer.package_test_assembly.assembly_path.is_empty() {
        anyhow::bail!("{} assemblyPath is required", pointer_path.display());
    }
    Ok(())
}

fn validate_assembly(
    assembly: &PackageTestAssembly,
    selection: &PackageTestBuildSelection,
    test_build_hash: &str,
) -> anyhow::Result<()> {
    if assembly.kind != PackageTestAssemblyKind::PackageTest {
        anyhow::bail!("package test assembly kind must be packageTest");
    }
    if assembly.package_id != selection.package_id {
        anyhow::bail!(
            "package test assembly packageId {} does not match dispatch packageId {}",
            assembly.package_id,
            selection.package_id
        );
    }
    if assembly.package_version != selection.package_version {
        anyhow::bail!(
            "package test assembly packageVersion {} does not match dispatch packageVersion {}",
            assembly.package_version,
            selection.package_version
        );
    }
    if identity_hash(
        &assembly.test_build_identity,
        PACKAGE_TEST_BUILD_IDENTITY_PREFIX,
        "assembly testBuildIdentity",
    )? != test_build_hash
    {
        anyhow::bail!(
            "package test assembly testBuildIdentity {} does not match assembly path hash {}",
            assembly.test_build_identity,
            test_build_hash
        );
    }
    validate_package_test_assembly_identity(assembly).map_err(|error| {
        anyhow::anyhow!("package test assembly identity validation failed: {error}")
    })?;

    validate_package_unit_ref(&assembly.production_package_unit, "productionPackageUnit")?;
    for (index, dependency) in assembly.dependency_package_units.iter().enumerate() {
        validate_package_unit_ref(dependency, &format!("dependencyPackageUnits[{index}]"))?;
    }
    for (index, file) in assembly.test_files.iter().enumerate() {
        validate_file_ir_ref(file, &format!("testFiles[{index}]"))?;
    }
    validate_link_policy_projection(assembly)?;

    for (index, entrypoint) in assembly.test_entrypoints.iter().enumerate() {
        validate_entrypoint(assembly, entrypoint, index)?;
    }
    Ok(())
}

pub(super) fn select_package_test_entrypoint(
    assembly: &PackageTestAssembly,
    selection: &PackageTestDispatchSelection,
) -> anyhow::Result<PackageTestEntrypoint> {
    validate_package_test_activation_id(&selection.activation_id)?;
    if assembly.package_id != selection.package_id {
        anyhow::bail!(
            "package test assembly packageId {} does not match dispatch packageId {}",
            assembly.package_id,
            selection.package_id
        );
    }
    if assembly.package_version != selection.package_version {
        anyhow::bail!(
            "package test assembly packageVersion {} does not match dispatch packageVersion {}",
            assembly.package_version,
            selection.package_version
        );
    }
    if assembly.test_build_identity != selection.test_build_identity {
        anyhow::bail!(
            "package test assembly testBuildIdentity {} does not match dispatch testBuildIdentity {}",
            assembly.test_build_identity,
            selection.test_build_identity
        );
    }
    let Some((index, entrypoint)) = assembly
        .test_entrypoints
        .iter()
        .enumerate()
        .find(|(_, entrypoint)| entrypoint.entrypoint_id == selection.entrypoint_id)
    else {
        anyhow::bail!(
            "package test entrypointId {} is not listed in assembly {}",
            selection.entrypoint_id,
            selection.test_build_identity
        );
    };
    validate_entrypoint(assembly, entrypoint, index)?;
    Ok(entrypoint.clone())
}

fn validate_entrypoint(
    assembly: &PackageTestAssembly,
    entrypoint: &PackageTestEntrypoint,
    index: usize,
) -> anyhow::Result<()> {
    identity_hash(
        &entrypoint.entrypoint_local_id,
        PACKAGE_TEST_ENTRYPOINT_LOCAL_ID_PREFIX,
        &format!("package test entrypoints[{index}].entrypointLocalId"),
    )?;
    let derived = derive_package_test_entrypoint_id(
        &assembly.test_build_identity,
        &entrypoint.entrypoint_local_id,
    )
    .map_err(|error| anyhow::anyhow!("failed to derive package test entrypoint id: {error}"))?;
    if entrypoint.entrypoint_id != derived {
        anyhow::bail!(
            "package test persisted entrypoints[{index}].entrypointId {} does not match derived id {}",
            entrypoint.entrypoint_id,
            derived
        );
    }
    if entrypoint.kind != PackageTestEntrypointKind::TestOnly {
        anyhow::bail!(
            "package test entrypoint {} must have kind testOnly",
            entrypoint.entrypoint_id
        );
    }
    let test_file_identities = assembly
        .test_files
        .iter()
        .map(|file| file.file_ir_identity.as_str())
        .collect::<BTreeSet<_>>();
    if !test_file_identities.contains(entrypoint.owner_test_file.file_ir_identity.as_str()) {
        anyhow::bail!(
            "package test entrypoint {} owner file {} is not listed in testFiles",
            entrypoint.entrypoint_id,
            entrypoint.owner_test_file.file_ir_identity
        );
    }
    let owner_test_file = assembly
        .test_files
        .iter()
        .find(|file| file.file_ir_identity == entrypoint.owner_test_file.file_ir_identity)
        .expect("owner test file identity was checked above");
    if owner_test_file != &entrypoint.owner_test_file {
        anyhow::bail!(
            "package test entrypoint {} ownerTestFile must exactly match the testFiles ref for {}",
            entrypoint.entrypoint_id,
            entrypoint.owner_test_file.file_ir_identity
        );
    }
    if entrypoint.source_path != owner_test_file.source_path {
        anyhow::bail!(
            "package test entrypoint {} sourcePath {} must match ownerTestFile sourcePath {}",
            entrypoint.entrypoint_id,
            entrypoint.source_path,
            owner_test_file.source_path
        );
    }
    if entrypoint.module_path != owner_test_file.module_path {
        anyhow::bail!(
            "package test entrypoint {} modulePath {} must match ownerTestFile modulePath {}",
            entrypoint.entrypoint_id,
            entrypoint.module_path,
            owner_test_file.module_path
        );
    }
    let Some(link_policy_owner_scope) =
        assembly.link_policy.test_file_scopes.iter().find(|scope| {
            scope.owner_test_file_identity == entrypoint.owner_test_file.file_ir_identity
        })
    else {
        anyhow::bail!(
            "package test entrypoint {} owner file {} is not listed in linkPolicy.testFileScopes",
            entrypoint.entrypoint_id,
            entrypoint.owner_test_file.file_ir_identity
        );
    };
    if !link_policy_owner_scope
        .entrypoint_local_ids
        .iter()
        .any(|id| id == &entrypoint.entrypoint_local_id)
    {
        anyhow::bail!(
            "package test entrypoint {} local id {} is not authorized by linkPolicy.testFileScopes owner file {}",
            entrypoint.entrypoint_id,
            entrypoint.entrypoint_local_id,
            entrypoint.owner_test_file.file_ir_identity
        );
    }
    if entrypoint.executable_ref.file_ir_identity != entrypoint.owner_test_file.file_ir_identity {
        anyhow::bail!(
            "package test entrypoint {} executable file {} must match owner file {}",
            entrypoint.entrypoint_id,
            entrypoint.executable_ref.file_ir_identity,
            entrypoint.owner_test_file.file_ir_identity
        );
    }
    Ok(())
}

fn validate_link_policy_projection(assembly: &PackageTestAssembly) -> anyhow::Result<()> {
    validate_current_package_production_scope(assembly)?;
    validate_test_file_scopes(assembly)?;
    validate_dependency_public_scopes(assembly)?;
    Ok(())
}

fn validate_current_package_production_scope(assembly: &PackageTestAssembly) -> anyhow::Result<()> {
    let reference = &assembly.production_package_unit;
    let scope = &assembly.link_policy.current_package_production;
    if scope.package_id != reference.package_id
        || scope.version != reference.version
        || scope.build_identity != reference.build_identity
    {
        anyhow::bail!(
            "linkPolicy.currentPackageProduction must match productionPackageUnit packageId/version/buildIdentity"
        );
    }
    if !scope.allow_private {
        anyhow::bail!("linkPolicy.currentPackageProduction.allowPrivate must be true");
    }
    validate_sha256_digest(
        &scope.files_digest,
        "linkPolicy.currentPackageProduction.filesDigest",
    )?;
    validate_sha256_digest(
        &scope.implementation_links_digest,
        "linkPolicy.currentPackageProduction.implementationLinksDigest",
    )?;
    Ok(())
}

fn validate_test_file_scopes(assembly: &PackageTestAssembly) -> anyhow::Result<()> {
    if assembly.link_policy.test_file_scopes.len() != assembly.test_files.len() {
        anyhow::bail!(
            "linkPolicy.testFileScopes count {} must match testFiles count {}",
            assembly.link_policy.test_file_scopes.len(),
            assembly.test_files.len()
        );
    }

    let mut seen = BTreeSet::new();
    for (index, (scope, file)) in assembly
        .link_policy
        .test_file_scopes
        .iter()
        .zip(&assembly.test_files)
        .enumerate()
    {
        if !seen.insert(scope.owner_test_file_identity.as_str()) {
            anyhow::bail!(
                "linkPolicy.testFileScopes[{index}] duplicates owner file {}",
                scope.owner_test_file_identity
            );
        }
        if scope.owner_test_file_identity != file.file_ir_identity
            || scope.source_path != file.source_path
            || scope.module_path != file.module_path
        {
            anyhow::bail!(
                "linkPolicy.testFileScopes[{index}] must match testFiles[{index}] identity/source/module"
            );
        }
        validate_sha256_digest(
            &scope.allowed_local_link_digest,
            &format!("linkPolicy.testFileScopes[{index}].allowedLocalLinkDigest"),
        )?;
    }

    Ok(())
}

fn validate_dependency_public_scopes(assembly: &PackageTestAssembly) -> anyhow::Result<()> {
    if assembly.link_policy.dependency_public_scopes.len()
        != assembly.dependency_package_units.len()
    {
        anyhow::bail!(
            "linkPolicy.dependencyPublicScopes count {} must match dependencyPackageUnits count {}",
            assembly.link_policy.dependency_public_scopes.len(),
            assembly.dependency_package_units.len()
        );
    }
    for (index, (scope, reference)) in assembly
        .link_policy
        .dependency_public_scopes
        .iter()
        .zip(&assembly.dependency_package_units)
        .enumerate()
    {
        if scope.package_id != reference.package_id
            || scope.version != reference.version
            || scope.build_identity != reference.build_identity
            || scope.public_abi_identity != reference.public_abi_identity
        {
            anyhow::bail!(
                "linkPolicy.dependencyPublicScopes[{index}] must match dependencyPackageUnits[{index}] identity fields"
            );
        }
        if scope.allow_private {
            anyhow::bail!("linkPolicy.dependencyPublicScopes[{index}].allowPrivate must be false");
        }
        validate_sha256_digest(
            &scope.public_export_digest,
            &format!("linkPolicy.dependencyPublicScopes[{index}].publicExportDigest"),
        )?;
        identity_hash(
            &scope.implementation_links_digest,
            PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX,
            &format!("linkPolicy.dependencyPublicScopes[{index}].implementationLinksDigest"),
        )?;
    }
    Ok(())
}

fn validate_package_unit_ref(
    reference: &PackageTestPackageUnitRef,
    label: &str,
) -> anyhow::Result<()> {
    let build_hash = identity_hash(
        &reference.build_identity,
        "skiff-package-build-v1:sha256",
        &format!("{label}.buildIdentity"),
    )?;
    identity_hash(
        &reference.public_abi_identity,
        "skiff-package-abi-v1:sha256",
        &format!("{label}.publicAbiIdentity"),
    )?;
    identity_hash(
        &reference.implementation_links_identity,
        PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX,
        &format!("{label}.implementationLinksIdentity"),
    )?;
    validate_hash_path_suffix(
        &reference.unit_path,
        build_hash,
        &format!("{label}.unitPath"),
    )?;
    Ok(())
}

fn validate_file_ir_ref(reference: &PackageTestFileIrRef, label: &str) -> anyhow::Result<()> {
    let file_hash = identity_hash(
        &reference.file_ir_identity,
        "skiff-file-ir-v3:sha256",
        &format!("{label}.fileIrIdentity"),
    )?;
    validate_hash_path_suffix(
        &reference.file_ir_path,
        file_hash,
        &format!("{label}.fileIrPath"),
    )?;
    Ok(())
}

fn validate_hash_path_suffix(path: &str, expected_hash: &str, label: &str) -> anyhow::Result<()> {
    let artifact_path = ArtifactRootRelativePath::parse(path, label)?;
    let file_name = artifact_path
        .as_path()
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("{label} must end with <hash>.json"))?;
    let Some(hash) = file_name.strip_suffix(".json") else {
        anyhow::bail!("{label} must end with <hash>.json");
    };
    if hash != expected_hash {
        anyhow::bail!("{label} hash {hash} does not match identity hash {expected_hash}");
    }
    Ok(())
}

pub(super) fn identity_hash<'a>(
    identity: &'a str,
    prefix: &str,
    label: &str,
) -> anyhow::Result<&'a str> {
    let Some(hash) = identity.strip_prefix(&format!("{prefix}:")) else {
        anyhow::bail!("{label} must use {prefix}:<64 lowercase hex>, got {identity}");
    };
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        anyhow::bail!("{label} must use {prefix}:<64 lowercase hex>, got {identity}");
    }
    Ok(hash)
}

fn validate_sha256_digest(digest: &str, label: &str) -> anyhow::Result<()> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        anyhow::bail!("{label} must be <64 lowercase hex>, got {digest}");
    }
    Ok(())
}
