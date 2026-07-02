use std::collections::BTreeMap;

use serde::Serialize;
use skiff_artifact_model::AbiIdentityFacts;
use skiff_compiler_core::json_utils::value_sha256;
use skiff_compiler_core::prelude_registry::PRELUDE_REGISTRY_ID;

use crate::{
    emission::artifact::{ArtifactUnit, PACKAGE_ASSEMBLY_KIND, SERVICE_ASSEMBLY_SCHEMA_VERSION},
    emission::{
        artifact_assembly::{
            package_artifact_assembly_path, package_dependency_entries, package_version_index_path,
            PackageAssemblyFileRef, PackageAssemblyPackageObject, PackageVersionIndexModel,
            PublishedPackageArtifacts,
        },
        file_ir_artifacts::published_file_ir_artifacts_from_projection_input,
        identity::{identity, PACKAGE_ASSEMBLY_IDENTITY_PREFIX},
    },
    error::{EmissionError, Result},
    projection::{
        context::{PackageApiSourceProjection, ProjectedPackageDependency},
        source_map::PublicationSourceMap,
        PackageProjectionBundle, ProjectedPackagePublication,
    },
};

pub struct PackageEmissionContext<'a> {
    pub package_id: &'a str,
    pub version: &'a str,
    pub api_source: Option<&'a PackageApiSourceProjection>,
    pub dependencies: &'a [ProjectedPackageDependency],
    pub dependency_artifacts: &'a BTreeMap<String, PublishedPackageArtifacts>,
}

impl<'a> PackageEmissionContext<'a> {
    pub fn new(
        package_id: &'a str,
        version: &'a str,
        api_source: Option<&'a PackageApiSourceProjection>,
        dependencies: &'a [ProjectedPackageDependency],
        dependency_artifacts: &'a BTreeMap<String, PublishedPackageArtifacts>,
    ) -> Self {
        Self {
            package_id,
            version,
            api_source,
            dependencies,
            dependency_artifacts,
        }
    }
}

pub fn build_package_artifacts(
    package_publications: &[ProjectedPackagePublication<'_>],
) -> Result<Vec<PublishedPackageArtifacts>> {
    let mut remaining = package_publications
        .iter()
        .filter(|package| package_artifact_is_published(package))
        .collect::<Vec<_>>();
    let mut package_artifacts = BTreeMap::<String, PublishedPackageArtifacts>::new();

    while !remaining.is_empty() {
        let before = remaining.len();
        let mut index = 0;
        while index < remaining.len() {
            let package = remaining[index];
            if !package
                .manifest()
                .dependencies()
                .iter()
                .all(|dependency| package_artifacts.contains_key(dependency.id()))
            {
                index += 1;
                continue;
            }

            let package = remaining.remove(index);
            let artifact = build_package_artifact(package, &package_artifacts)?;
            package_artifacts.insert(package.manifest().id().to_string(), artifact);
        }
        if remaining.len() == before {
            let package_ids = remaining
                .iter()
                .map(|package| package.manifest().id())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(EmissionError::ContractValidation {
                message: format!("package dependency cycle or missing artifact for {package_ids}"),
            });
        }
    }

    Ok(package_artifacts.into_values().collect())
}

fn package_artifact_is_published(package: &ProjectedPackagePublication<'_>) -> bool {
    package.manifest().id() != PRELUDE_REGISTRY_ID && !package.manifest().provenance().synthetic()
}

fn build_package_artifact(
    package_publication: &ProjectedPackagePublication<'_>,
    package_artifacts: &BTreeMap<String, PublishedPackageArtifacts>,
) -> Result<PublishedPackageArtifacts> {
    let api_source =
        package_publication
            .source
            .api_source()
            .map(|source| PackageApiSourceProjection {
                relative_path: source.relative_path().to_path_buf(),
                content_hash: source.content_hash().to_string(),
            });
    let dependencies = package_publication
        .manifest()
        .dependencies()
        .iter()
        .map(|dependency| ProjectedPackageDependency {
            id: dependency.id().to_string(),
            version: dependency.version().to_string(),
            alias: dependency.alias().map(str::to_string),
            config: dependency.config().clone(),
            collection_name_mapping: dependency.collection_name_mapping().clone(),
        })
        .collect::<Vec<_>>();
    emit_package(
        &package_publication.bundle,
        PackageEmissionContext::new(
            package_publication.manifest().id(),
            package_publication.manifest().version(),
            api_source.as_ref(),
            &dependencies,
            package_artifacts,
        ),
    )
}

pub fn emit_package(
    bundle: &PackageProjectionBundle<'_>,
    context: PackageEmissionContext<'_>,
) -> Result<PublishedPackageArtifacts> {
    let package_file_ir_artifacts =
        published_file_ir_artifacts_from_projection_input(bundle.input)?;
    let file_refs = package_file_ir_artifacts
        .iter()
        .map(PackageAssemblyFileRef::from_published_file_ir_artifact)
        .collect::<Vec<_>>();
    let dependencies =
        package_dependency_entries(context.dependencies, context.dependency_artifacts)?;
    let assembly_hash_model = package_assembly_artifact_model(
        context.package_id,
        context.version,
        None,
        context.api_source,
        &bundle.exports,
        &bundle.abi_identity_projection,
        &file_refs,
        &dependencies,
        &bundle.config_projection,
        &bundle.source_map,
    );
    let hash = value_sha256(&artifact_model_value(&assembly_hash_model));
    let assembly_identity = identity(PACKAGE_ASSEMBLY_IDENTITY_PREFIX, &hash);
    let assembly_path = package_artifact_assembly_path(context.package_id, &hash);
    let index_path = package_version_index_path(context.package_id, context.version);
    let assembly_unit = ArtifactUnit {
        model: package_assembly_artifact_model(
            context.package_id,
            context.version,
            Some(assembly_identity.as_str()),
            context.api_source,
            &bundle.exports,
            &bundle.abi_identity_projection,
            &file_refs,
            &dependencies,
            &bundle.config_projection,
            &bundle.source_map,
        ),
        identity: assembly_identity.clone(),
        hash,
        path: assembly_path.clone(),
    };
    let assembly = assembly_unit.to_published_json();
    let version_index_model = PackageVersionIndexModel::new(
        context.package_id,
        context.version,
        &assembly_identity,
        &assembly_path,
        bundle.exports.clone(),
        bundle.abi_identity_projection.clone(),
        file_refs,
        dependencies,
        &bundle.config_projection,
        bundle.source_map.clone(),
    );
    let index_hash = value_sha256(&artifact_model_value(&version_index_model));
    let version_index_unit = ArtifactUnit {
        model: &version_index_model,
        identity: String::new(),
        hash: index_hash,
        path: index_path,
    };
    let version_index = version_index_unit.to_published_json();

    Ok(PublishedPackageArtifacts {
        package_id: context.package_id.to_string(),
        version: context.version.to_string(),
        exports: bundle.exports.clone(),
        abi_identity_projection: bundle.abi_identity_projection.clone(),
        file_ir_units: package_file_ir_artifacts,
        config_projection: bundle.config_projection.clone(),
        assembly,
        version_index_model,
        version_index,
    })
}

fn package_assembly_artifact_model<'a>(
    package_id: &'a str,
    version: &'a str,
    assembly_identity: Option<&'a str>,
    api_source: Option<&'a PackageApiSourceProjection>,
    exports: &'a crate::projection::package_exports::PackageExports,
    abi_identity_projection: &'a AbiIdentityFacts,
    files: &'a [PackageAssemblyFileRef],
    dependencies: &'a [crate::emission::artifact_assembly::PackageDependencyEntry],
    config_projection: &'a crate::projection::ConfigProjection,
    source_map: &'a PublicationSourceMap,
) -> PackageAssemblyArtifactModel<'a> {
    PackageAssemblyArtifactModel {
        schema_version: SERVICE_ASSEMBLY_SCHEMA_VERSION,
        kind: PACKAGE_ASSEMBLY_KIND,
        package: PackageAssemblyPackageObject {
            id: package_id,
            version,
            assembly_identity,
        },
        api_source: api_source.map(publication_api_source_identity),
        exports,
        abi_identity_projection,
        files,
        dependencies,
        config_shape: &config_projection.shape,
        config_uses: &config_projection.uses,
        config_activation: &config_projection.activation,
        config_requirements: &config_projection.requirements,
        source_map,
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageAssemblyArtifactModel<'a> {
    schema_version: &'static str,
    kind: &'static str,
    package: PackageAssemblyPackageObject<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_source: Option<PublicationApiSourceIdentity<'a>>,
    exports: &'a crate::projection::package_exports::PackageExports,
    abi_identity_projection: &'a AbiIdentityFacts,
    files: &'a [PackageAssemblyFileRef],
    dependencies: &'a [crate::emission::artifact_assembly::PackageDependencyEntry],
    config_shape: &'a crate::projection::ConfigShape,
    config_uses: &'a [crate::projection::ConfigUseEntry],
    config_activation: &'a crate::projection::ConfigActivation,
    config_requirements: &'a crate::projection::ConfigRequirementsProjection,
    source_map: &'a PublicationSourceMap,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicationApiSourceIdentity<'a> {
    relative_path: String,
    content_hash: &'a str,
}

fn publication_api_source_identity(
    source: &PackageApiSourceProjection,
) -> PublicationApiSourceIdentity<'_> {
    PublicationApiSourceIdentity {
        relative_path: source.relative_path.to_string_lossy().into_owned(),
        content_hash: source.content_hash.as_str(),
    }
}

fn artifact_model_value<T>(model: &T) -> serde_json::Value
where
    T: Serialize,
{
    serde_json::to_value(model).expect("package artifact model must serialize")
}
