use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};
use skiff_artifact_identity::{
    package_artifact_ref, PackageArtifactRecordPath, PackageFileIrRecordPath,
    PackageResourceRecordPath,
};
use skiff_artifact_model::{PackageArtifact, PackageArtifactRef};
use skiff_compiler_compiled::BytecodeCompilationHandoff;
use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;
use skiff_compiler_emission::package_artifact::{
    materialize_package_artifact, PublishedPackageArtifact,
};
use skiff_compiler_input::{
    package_config::{discover_package_manifests, package_alias_bindings, PackageManifest},
    package_sources::read_official_package_sources,
    read_publication_resources, CompilerPlatformSources, ManifestOwner,
};
use skiff_compiler_source::{
    prelude_registry::initialize_prelude_registry, source_graph::PublicationSourceGraph,
};
use skiff_deployment::storage::CanonicalArtifactStore;

use crate::{compile_package, PackageCompileInput, PackageSourceInput};

use super::{invalid_input, AuthoringResult};

/// Exact immutable records produced for one compiled package candidate.
///
/// Pointer installation and activation-profile mutation deliberately remain outside
/// this receipt and outside the canonical package record writer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishedPackageArtifactReceipt {
    pub artifact: PackageArtifactRef,
    pub record_path: String,
    pub file_ir_record_paths: Vec<String>,
    pub resource_record_paths: Vec<String>,
}

/// Builds the compiler-owned standard package only from an already validated
/// platform source authority. No arbitrary package root or manifest is
/// accepted, and this function performs no artifact-store writes.
pub fn author_official_std_package(
    platform_sources: &CompilerPlatformSources,
) -> AuthoringResult<PublishedPackageArtifact> {
    initialize_prelude_registry(platform_sources)?;
    author_official_std_package_after_platform_context_guard(platform_sources)
}

fn author_official_std_package_after_platform_context_guard(
    platform_sources: &CompilerPlatformSources,
) -> AuthoringResult<PublishedPackageArtifact> {
    let manifests = discover_package_manifests(platform_sources, platform_sources.root())?;
    let manifest = exact_official_std_manifest(&manifests)?;
    let package_root = manifest
        .provenance
        .path
        .parent()
        .ok_or_else(|| invalid_input("official std manifest has no package root"))?;
    if package_root != platform_sources.std_dir() {
        return Err(invalid_input(format!(
            "official std manifest root {} does not match authorized std root {}",
            package_root.display(),
            platform_sources.std_dir().display()
        )));
    }

    let raw_sources = read_official_package_sources(platform_sources, manifest)?;
    let source_tree = raw_sources.source_tree();
    let source_graph =
        PublicationSourceGraph::parse_raw_publication_sources(&raw_sources.into_source_graph())?;
    let resources = read_publication_resources(package_root, &manifest.resources)?;
    let package = PackageSourceInput::new(
        manifest.publication.clone(),
        source_tree,
        source_graph,
        resources,
    );
    let aliases = package_alias_bindings(&manifest.dependencies, &manifests);
    let input = PackageCompileInput::new(
        platform_sources,
        &package,
        &aliases,
        SKIFF_STD_PUBLICATION_ID,
        false,
    )
    .with_canonical_dependencies(&[], &[])
    .with_available_canonical_packages(&[]);
    compile_package(input)?
        .into_disabled_package()
        .map_err(|_| invalid_input("official std compilation unexpectedly enabled bytecode"))
}

/// Legacy compiler-side writer for bytecode-free canonical PackageArtifact
/// records.
///
/// The emitted candidate is completely validated and all record locations are
/// planned before the first immutable write. Storage paths emitted by earlier
/// compiler stages are treated as non-canonical candidate metadata; this owner
/// replaces both the top-level record path and every nested `artifactPath`
/// with the typed ecosystem-store paths. A missing artifact root is created;
/// a non-directory root is rejected. The deployment storage owner remains an
/// implementation detail behind this path-based compiler facade.
pub fn publish_package_artifact_records(
    artifact_root: &Path,
    published: &PublishedPackageArtifact,
) -> AuthoringResult<PublishedPackageArtifactReceipt> {
    let store = CanonicalArtifactStore::create(artifact_root)?;
    publish_package_artifact_records_to_store(&store, published, None)
}

/// Bytecode-aware compiler-side writer for canonical PackageArtifact records.
///
/// The bytecode record is written before the referencing PackageArtifact so
/// the store never contains a package pointer to a missing bytecode record.
pub fn publish_package_artifact_records_with_bytecode(
    artifact_root: &Path,
    published: &PublishedPackageArtifact,
    bytecode: &BytecodeCompilationHandoff,
) -> AuthoringResult<PublishedPackageArtifactReceipt> {
    let store = CanonicalArtifactStore::create(artifact_root)?;
    publish_package_artifact_records_to_store(&store, published, Some(bytecode))
}

pub(super) fn publish_package_artifact_records_to_store(
    store: &CanonicalArtifactStore,
    published: &PublishedPackageArtifact,
    bytecode: Option<&BytecodeCompilationHandoff>,
) -> AuthoringResult<PublishedPackageArtifactReceipt> {
    validate_bytecode_publication_state(published, bytecode)?;
    let plan = PackagePublicationPlan::new(published)?;

    for (file_ref, file) in plan.artifact.files.iter().zip(plan.file_ir_units.iter()) {
        store.write_file_ir(&plan.reference, file_ref, file)?;
    }
    for (resource_ref, bytes) in plan
        .artifact
        .static_resources
        .iter()
        .zip(plan.resource_blobs.iter())
    {
        store.write_static_resource(&plan.reference, resource_ref, bytes)?;
    }
    for record in published.package_schema_type_records.values() {
        store.write_package_schema_type_record(record)?;
    }
    store.write_package_schema_index(&published.package_schema_index)?;
    if let Some(bytecode) = bytecode {
        store.write_package_bytecode(&plan.reference, bytecode.artifact())?;
    }
    store.write_package_artifact(&plan.artifact)?;

    Ok(plan.receipt)
}

fn validate_bytecode_publication_state(
    published: &PublishedPackageArtifact,
    bytecode: Option<&BytecodeCompilationHandoff>,
) -> AuthoringResult<()> {
    match bytecode {
        Some(handoff) => {
            if published.artifact.bytecode.as_ref() != Some(handoff.reference()) {
                return Err(invalid_input(
                    "bytecode publication handoff does not match PackageArtifact bytecode reference",
                ));
            }
            if published.artifact.bytecode_statement_manifest_identity
                != handoff.statement_manifest_receipt().identity().as_str()
            {
                return Err(invalid_input(
                    "bytecode publication handoff does not match PackageArtifact statement manifest",
                ));
            }
        }
        None => {
            if published.artifact.bytecode.is_some() {
                return Err(invalid_input(
                    "legacy publication cannot write a bytecode-bearing PackageArtifact without its bytecode handoff",
                ));
            }
        }
    }
    Ok(())
}

struct PackagePublicationPlan {
    artifact: PackageArtifact,
    reference: PackageArtifactRef,
    file_ir_units: Vec<skiff_artifact_model::FileIrUnit>,
    resource_blobs: Vec<Vec<u8>>,
    receipt: PublishedPackageArtifactReceipt,
}

impl PackagePublicationPlan {
    fn new(published: &PublishedPackageArtifact) -> AuthoringResult<Self> {
        let materialized = materialize_package_artifact(
            &published.artifact,
            &published.file_ir_units,
            &published.resource_blobs,
        )?;
        let mut artifact = materialized.artifact;
        for file in &mut artifact.files {
            file.artifact_path = None;
        }
        for resource in &mut artifact.static_resources {
            resource.artifact_path = None;
        }
        let reference = package_artifact_ref(&artifact)?;
        let record_path = PackageArtifactRecordPath::new(&reference)?.to_string();

        let files_by_key = published
            .file_ir_units
            .iter()
            .map(|file| {
                (
                    (file.identity.as_str(), file.module_path.as_str()),
                    file.unit.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let resources_by_key = materialized
            .resource_blobs
            .iter()
            .map(|resource| {
                (
                    (resource.sha256.as_str(), resource.byte_len),
                    resource.bytes.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();

        let mut file_ir_units = Vec::with_capacity(artifact.files.len());
        let mut file_ir_record_paths = Vec::with_capacity(artifact.files.len());
        for file_ref in &mut artifact.files {
            let record_path = PackageFileIrRecordPath::new(&reference, file_ref)?.to_string();
            let unit = files_by_key
                .get(&(
                    file_ref.file_ir_identity.as_str(),
                    file_ref.module_path.as_str(),
                ))
                .ok_or_else(|| {
                    invalid_input(format!(
                        "PackageArtifact FileIrUnit {} has no exact emitted typed payload",
                        file_ref.file_ir_identity
                    ))
                })?
                .clone();
            file_ref.artifact_path = Some(record_path.clone());
            file_ir_units.push(unit);
            file_ir_record_paths.push(record_path);
        }

        let mut resource_blobs = Vec::with_capacity(artifact.static_resources.len());
        let mut resource_record_paths = Vec::with_capacity(artifact.static_resources.len());
        for resource_ref in &mut artifact.static_resources {
            let record_path = PackageResourceRecordPath::new(&reference, resource_ref)?.to_string();
            let bytes = resources_by_key
                .get(&(resource_ref.sha256.as_str(), resource_ref.byte_len))
                .ok_or_else(|| {
                    invalid_input(format!(
                        "PackageArtifact resource {} has no exact emitted typed payload",
                        resource_ref.path
                    ))
                })?
                .clone();
            resource_ref.artifact_path = Some(record_path.clone());
            resource_blobs.push(bytes);
            resource_record_paths.push(record_path);
        }

        if package_artifact_ref(&artifact)? != reference {
            return Err(invalid_input(
                "canonical package record paths changed package artifact identity",
            ));
        }

        let receipt = PublishedPackageArtifactReceipt {
            artifact: reference.clone(),
            record_path,
            file_ir_record_paths,
            resource_record_paths,
        };
        Ok(Self {
            artifact,
            reference,
            file_ir_units,
            resource_blobs,
            receipt,
        })
    }
}

fn exact_official_std_manifest(
    manifests: &BTreeMap<(String, String), PackageManifest>,
) -> AuthoringResult<&PackageManifest> {
    let mut matches = manifests
        .values()
        .filter(|manifest| manifest.id.as_str() == SKIFF_STD_PUBLICATION_ID);
    let manifest = matches
        .next()
        .ok_or_else(|| invalid_input("validated platform sources contain no official std"))?;
    if matches.next().is_some() {
        return Err(invalid_input(
            "validated platform sources contain multiple official std manifests",
        ));
    }
    if manifest.provenance.owner != ManifestOwner::CompilerStandardPackage
        || manifest.provenance.synthetic
    {
        return Err(invalid_input(
            "official std manifest is not compiler-owned source provenance",
        ));
    }
    if !manifest.dependencies.is_empty() || !manifest.services.is_empty() {
        return Err(invalid_input(
            "official std manifest must not declare package or contract dependencies",
        ));
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests;
