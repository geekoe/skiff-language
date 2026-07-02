use serde_json::Value;

use crate::{
    input::{
        source_graph::publication_from_raw, Publication, PublicationManifest, ResolvedPackage,
    },
    shared::publication_error::PublicationError,
};

#[cfg(test)]
use crate::emission::artifact_assembly::PublishedPackageArtifacts;
#[cfg(test)]
use crate::input::source_graph::{CompilerSourceFile, PublicationSourceGraph};
#[cfg(test)]
use crate::input::{assemble_publication, ResolvedPackageGraph};
#[cfg(test)]
use crate::input::{SourceTree, SourceTreeFile};
#[cfg(test)]
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub(crate) struct PackagePublicationJob {
    pub(crate) publication: Publication,
    pub(crate) dependency_config: Value,
}

impl PackagePublicationJob {
    pub(crate) fn new(publication: Publication, dependency_config: Value) -> Self {
        Self {
            publication,
            dependency_config,
        }
    }

    pub(crate) fn manifest(&self) -> &PublicationManifest {
        &self.publication.manifest
    }

    pub(crate) fn config(&self) -> &Value {
        &self.dependency_config
    }
}

pub(crate) fn build_package_jobs(
    packages: Vec<ResolvedPackage>,
) -> Result<Vec<PackagePublicationJob>, PublicationError> {
    skiff_compiler_input::build_package_jobs(packages)?
        .into_iter()
        .map(package_publication_job_from_raw)
        .collect()
}

pub(crate) fn package_publication_job_from_raw(
    raw: skiff_compiler_input::RawPackagePublicationJob,
) -> Result<PackagePublicationJob, PublicationError> {
    let publication = publication_from_raw(raw.publication)?;
    Ok(PackagePublicationJob::new(
        publication,
        raw.dependency_config,
    ))
}

#[cfg(test)]
fn build_package_artifact_from_sources(
    package: &ResolvedPackage,
    sources: &[CompilerSourceFile],
    available: &BTreeMap<crate::input::PackageManifestKey, crate::input::PackageManifest>,
    _package_artifacts: &BTreeMap<String, PublishedPackageArtifacts>,
) -> Result<PublishedPackageArtifacts, PublicationError> {
    let source_graph = PublicationSourceGraph::from_compiler_sources(sources.to_vec());
    let source_tree = SourceTree {
        root: package
            .manifest
            .provenance
            .path
            .parent()
            .expect("package manifest has parent directory")
            .to_path_buf(),
        sources: sources
            .iter()
            .map(|source| SourceTreeFile {
                module_path: source.module_path.clone(),
                file_path: source.relative_path.clone(),
                is_test_file: source.is_test_file,
                byte_len: source.text.len() as u64,
            })
            .collect(),
    };
    let publication = assemble_publication(
        package.manifest.publication.clone(),
        source_tree,
        source_graph,
        ResolvedPackageGraph::declared_only(package.manifest.dependencies.clone()),
    );
    let job = PackagePublicationJob::new(publication.clone(), package.config.clone());
    let compiled_packages = crate::pipeline::compile_package_jobs(vec![job], available)?;
    let compiled_package = compiled_packages
        .first()
        .expect("single package job compiles to one publication");
    let package_projection_inputs =
        skiff_compiler_compiled::projection_input::build_package_projection_inputs(
            &compiled_packages,
        );
    let prelude_projection = crate::shared::prelude_registry::projection_prelude_context();
    let package_projections = skiff_compiler_projection::project_package_publications(
        &package_projection_inputs,
        &prelude_projection,
    )?;
    let artifacts =
        crate::emission::package_artifacts::build_package_artifacts(&package_projections)?;
    artifacts
        .into_iter()
        .find(|artifact| artifact.package_id == compiled_package.id())
        .ok_or_else(|| PublicationError::ContractValidation {
            message: format!(
                "package {} did not emit package artifacts",
                compiled_package.id()
            ),
        })
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf};

    use super::*;
    use crate::{
        input::{
            ManifestOwner, ManifestProvenance, PackageApi, PackageDependency, PackageManifest,
            PublicationApiEntry, PublicationManifest,
        },
        test_support::project_fixtures::TestDir,
    };
    use skiff_compiler_core::id::PublicationId;

    #[test]
    fn package_publication_graph_preserves_declared_dependencies() {
        let temp = TestDir::new("skiff-compiler", "package-publication-dependencies");
        let package_root = temp.path().join("package");
        std::fs::create_dir_all(&package_root).unwrap();
        let dependency = PackageDependency {
            id: "example.com/base".to_string(),
            version: "1.2.3".to_string(),
            alias: Some("base".to_string()),
            config: serde_json::json!({ "mode": "test" }),
            collection_name_mapping: BTreeMap::new(),
        };
        let manifest = PackageManifest::new(PublicationManifest::new(
            PublicationId::parse("example.com/facade").unwrap(),
            "0.1.0".to_string(),
            PackageApi::default(),
            vec![dependency.clone()],
            ManifestProvenance::file(
                package_root.join("package.yml"),
                ManifestOwner::UserOrBuiltinPackage,
            ),
        ));
        let package = ResolvedPackage {
            manifest,
            config: crate::input::empty_dependency_config(),
        };

        let publications = build_package_jobs(vec![package]).unwrap();

        assert_eq!(publications.len(), 1);
        let declared_dependencies = publications[0]
            .publication
            .package_graph
            .declared_dependencies();
        assert_eq!(declared_dependencies, &[dependency]);
    }

    #[test]
    fn official_std_private_package_files_use_dotted_internal_namespace() {
        let manifest = official_std_manifest(PackageApi::from_entries(vec![
            PublicationApiEntry::for_source("http.request", "http", "request"),
        ]));
        let package = ResolvedPackage {
            manifest,
            config: crate::input::empty_dependency_config(),
        };
        let sources = vec![
            test_source(
                "http.skiff",
                "std.http",
                r#"
                    function request() -> string { return "ok" }
                "#,
            ),
            test_source(
                "helper.skiff",
                "std.__private.helper",
                r#"
                    type HelperState { value: string }
                    function helper() -> HelperState {
                      return { value: "internal" }
                    }
                "#,
            ),
        ];

        let artifact = build_package_artifact_from_sources(
            &package,
            &sources,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        let module_paths = artifact.assembly.value["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file["modulePath"].as_str().unwrap())
            .collect::<Vec<_>>();

        assert!(module_paths.contains(&"std.http"));
        assert!(module_paths.contains(&"std.__private.helper"));
        assert!(
            module_paths
                .iter()
                .all(|module_path| !module_path.contains('/')),
            "published package file module paths must be dotted identities: {module_paths:?}"
        );
    }

    #[test]
    fn package_artifacts_emit_abi_identity_projection() {
        let package_root = PathBuf::from("/tmp/example.com/models");
        let manifest = PackageManifest::new(PublicationManifest::new(
            skiff_compiler_core::id::PublicationId::parse("example.com/models").unwrap(),
            "0.1.0".to_string(),
            PackageApi::from_entries(vec![PublicationApiEntry::for_source(
                "public.PublicUser",
                "models",
                "PublicUser",
            )]),
            Vec::new(),
            ManifestProvenance::file(
                package_root.join("package.yml"),
                ManifestOwner::UserOrBuiltinPackage,
            ),
        ));
        let package = ResolvedPackage {
            manifest,
            config: crate::input::empty_dependency_config(),
        };
        let sources = vec![test_source(
            "models.skiff",
            "models",
            r#"
                    type PublicUser { payload: PrivatePayload }
                    type PrivatePayload { value: string }
                "#,
        )];

        let artifact = build_package_artifact_from_sources(
            &package,
            &sources,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();

        let abi = &artifact.abi_identity_projection;
        let public_user = abi
            .public_symbols
            .get("public.PublicUser")
            .expect("public type export should carry AbiSymbolId");
        let public_user_type_id = match public_user {
            skiff_compiler_core::artifact::AbiSymbolIdFact::Type { abi_type_id } => abi_type_id,
            other => panic!("public.PublicUser should be a type symbol, got {other:?}"),
        };
        assert_eq!(
            abi.type_nameability.get(public_user_type_id),
            Some(&skiff_compiler_core::artifact::TypeNameability::PublicNameable)
        );
        assert!(
            abi.type_nameability.values().any(|nameability| *nameability
                == skiff_compiler_core::artifact::TypeNameability::ClosureOnly),
            "private return type must be present in ABI closure identity map"
        );
        assert!(
            !abi.public_symbols
                .keys()
                .any(|public_path| public_path.contains("PrivatePayload")),
            "closure-only type must not enter public export table"
        );
        assert_eq!(
            artifact.assembly.value["abiIdentityProjection"],
            artifact.version_index.value["abiIdentityProjection"],
            "assembly and version index should carry the same ABI identity facts"
        );
        assert_eq!(
            artifact.assembly.value["abiIdentityProjection"]["publicSymbols"]["public.PublicUser"]
                ["kind"],
            "type"
        );
        assert!(
            artifact.assembly.value["abiIdentityProjection"]["typeNameability"]
                .as_object()
                .expect("typeNameability must be an object")
                .values()
                .any(|value| value == "closureOnly")
        );
    }

    fn official_std_manifest(api: PackageApi) -> PackageManifest {
        PackageManifest::new(PublicationManifest::new(
            skiff_compiler_core::id::PublicationId::parse("skiff.run/std").unwrap(),
            "1.0.0".to_string(),
            api,
            Vec::new(),
            ManifestProvenance::file(
                PathBuf::from("/tmp/skiff.run/std/package.yml"),
                ManifestOwner::CompilerStandardPackage,
            ),
        ))
    }

    fn test_source(relative_path: &str, module_path: &str, text: &str) -> CompilerSourceFile {
        CompilerSourceFile::parse(
            PathBuf::from(relative_path),
            module_path.to_string(),
            false,
            false,
            text.to_string(),
            relative_path,
        )
        .unwrap()
    }
}
