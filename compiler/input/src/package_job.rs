use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::{
    assemble_publication_with_resources, error::InputAssemblyError,
    package_sources::read_resolved_package_sources, PublicationManifest, RawPublication,
    ResolvedPackage, ResolvedPackageGraph,
};

#[derive(Debug, Clone)]
pub struct RawPackagePublicationJob {
    pub publication: RawPublication,
    pub dependency_config: Value,
}

impl RawPackagePublicationJob {
    pub fn new(publication: RawPublication, dependency_config: Value) -> Self {
        Self {
            publication,
            dependency_config,
        }
    }

    pub fn manifest(&self) -> &PublicationManifest {
        &self.publication.manifest
    }

    pub fn config(&self) -> &Value {
        &self.dependency_config
    }
}

pub fn build_package_jobs(
    packages: Vec<ResolvedPackage>,
) -> Result<Vec<RawPackagePublicationJob>, InputAssemblyError> {
    let package_ids = packages
        .iter()
        .map(|package| package.manifest.id.to_string())
        .collect::<BTreeSet<_>>();
    let mut remaining = packages;
    let mut jobs_by_id = BTreeMap::<String, RawPackagePublicationJob>::new();
    let mut ordered = Vec::new();

    while !remaining.is_empty() {
        let before = remaining.len();
        let mut index = 0;
        while index < remaining.len() {
            if !package_publication_dependencies_ready(&remaining[index], &package_ids, &jobs_by_id)
            {
                index += 1;
                continue;
            }
            let package = remaining.remove(index);
            let job = build_package_publication_job(package)?;
            jobs_by_id.insert(job.manifest().id.to_string(), job.clone());
            ordered.push(job);
        }

        if remaining.len() == before {
            for package in remaining {
                let job = build_package_publication_job(package)?;
                ordered.push(job);
            }
            break;
        }
    }

    Ok(ordered)
}

fn package_publication_dependencies_ready(
    package: &ResolvedPackage,
    package_ids: &BTreeSet<String>,
    jobs_by_id: &BTreeMap<String, RawPackagePublicationJob>,
) -> bool {
    package
        .manifest
        .dependencies
        .iter()
        .filter(|dependency| package_ids.contains(&dependency.id))
        .all(|dependency| jobs_by_id.contains_key(&dependency.id))
}

fn build_package_publication_job(
    package: ResolvedPackage,
) -> Result<RawPackagePublicationJob, InputAssemblyError> {
    let package_root = package
        .manifest
        .provenance
        .path
        .parent()
        .expect("package manifest has parent directory")
        .to_path_buf();
    let sources = read_resolved_package_sources(&package)?;
    let source_tree = sources.source_tree();
    let source_graph = sources.into_source_graph();
    let manifest = package.manifest.into_publication();
    let resources = crate::read_publication_resources(&package_root, &manifest.resources)?;
    let dependency_config = package.config;
    let package_graph = ResolvedPackageGraph::declared_only(manifest.dependencies.clone());
    let publication = assemble_publication_with_resources(
        manifest,
        source_tree,
        source_graph,
        package_graph,
        resources,
    );
    Ok(RawPackagePublicationJob::new(
        publication,
        dependency_config,
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        package_config::{empty_dependency_config, PackageApi, PackageManifest},
        ManifestOwner, ManifestProvenance, PackageDependency, PublicationManifest,
        PublicationResourceSpec,
    };
    use skiff_compiler_core::id::PublicationId;

    #[test]
    fn package_publication_graph_preserves_declared_dependencies() {
        let temp = TestDir::new("skiff-compiler-input", "package-publication-dependencies");
        let package_root = temp.path().join("package");
        std::fs::create_dir_all(&package_root).unwrap();
        std::fs::write(
            package_root.join("main.skiff"),
            "type Main { value: string }\n",
        )
        .unwrap();
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
            config: empty_dependency_config(),
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
    fn package_publication_job_collects_resources_outside_source_tree() {
        let temp = TestDir::new("skiff-compiler-input", "package-publication-resources");
        let package_root = temp.path().join("package");
        std::fs::create_dir_all(package_root.join("prompts")).unwrap();
        std::fs::write(
            package_root.join("main.skiff"),
            "type Main { value: string }\n",
        )
        .unwrap();
        std::fs::write(package_root.join("prompts/system.md"), "system prompt\n").unwrap();
        let manifest = PackageManifest::new(PublicationManifest::new_with_resources(
            PublicationId::parse("example.com/facade").unwrap(),
            "0.1.0".to_string(),
            PackageApi::default(),
            Vec::new(),
            vec![PublicationResourceSpec::new("prompts/system.md")],
            ManifestProvenance::file(
                package_root.join("package.yml"),
                ManifestOwner::UserOrBuiltinPackage,
            ),
        ));
        let package = ResolvedPackage {
            manifest,
            config: empty_dependency_config(),
        };

        let publications = build_package_jobs(vec![package]).unwrap();

        let publication = &publications[0].publication;
        assert_eq!(publication.resources.len(), 1);
        assert_eq!(publication.resources[0].path, "prompts/system.md");
        assert_eq!(publication.resources[0].byte_len, 14);
        assert_eq!(publication.source_tree.sources.len(), 1);
        assert_eq!(
            publication.source_tree.sources[0].file_path,
            std::path::Path::new("main.skiff")
        );
    }

    struct TestDir {
        path: std::path::PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str, name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("{prefix}-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("test dir should be created");
            Self { path }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
