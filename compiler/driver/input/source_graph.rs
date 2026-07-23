use crate::input::{PublicationManifest, PublicationResourceInput, SourceTree};

use skiff_artifact_model::PackageArtifactRef;
use skiff_compiler_source::source_graph::CompilerSourceFile;
pub use skiff_compiler_source::source_graph::PublicationSourceGraph;

/// Parsed package code and manifest facts consumed by one PackageCompileInput.
///
/// This is package code only: service configuration, provider selection,
/// deployment and routes cannot enter the compiler through this boundary.
#[derive(Debug, Clone)]
pub struct PackageSourceInput {
    pub(crate) manifest: PublicationManifest,
    pub(crate) source_tree: SourceTree,
    pub(crate) source_graph: PublicationSourceGraph,
    pub(crate) resources: Vec<PublicationResourceInput>,
    pub(crate) test_overlay: Option<PackageTestOverlayInput>,
}

/// Compiler-internal test overlay. It binds private transformed test functions
/// to the exact already-published production coordinate whose source graph is
/// being compiled; it is not a package dependency or a publication surface.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct PackageTestOverlayInput {
    pub production: PackageArtifactRef,
    pub private_sources: Vec<CompilerSourceFile>,
}

impl PackageSourceInput {
    pub fn new(
        manifest: PublicationManifest,
        source_tree: SourceTree,
        source_graph: PublicationSourceGraph,
        resources: Vec<PublicationResourceInput>,
    ) -> Self {
        Self {
            manifest,
            source_tree,
            source_graph,
            resources,
            test_overlay: None,
        }
    }

    #[doc(hidden)]
    pub fn with_test_overlay(
        mut self,
        production: PackageArtifactRef,
        private_sources: Vec<CompilerSourceFile>,
    ) -> Self {
        self.test_overlay = Some(PackageTestOverlayInput {
            production,
            private_sources,
        });
        self
    }

    pub fn manifest(&self) -> &PublicationManifest {
        &self.manifest
    }

    pub fn source_tree(&self) -> &SourceTree {
        &self.source_tree
    }

    pub fn resources(&self) -> &[PublicationResourceInput] {
        &self.resources
    }

    pub(crate) fn production_sources(&self) -> Vec<CompilerSourceFile> {
        self.source_graph.production_files()
    }

    pub(crate) fn compile_sources(&self) -> Vec<CompilerSourceFile> {
        let mut sources = self.production_sources();
        if let Some(overlay) = &self.test_overlay {
            sources.extend(overlay.private_sources.iter().cloned());
        }
        sources
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use skiff_compiler_source::source_graph::{ParsedSourceFile, SourceFileMeta};

    #[test]
    fn package_source_rejects_removed_provider_syntax() {
        let path = ".skiff-packages/skiff~run~~mongo/1.0.0/mongo.skiff";
        let error = ParsedSourceFile::parse(
            SourceFileMeta::package(
                "skiff.run/mongo",
                PathBuf::from("mongo.skiff"),
                "mongo".to_string(),
            ),
            "provider mongo\n\nexport type MongoTarget {}\n".to_string(),
            path,
        )
        .expect_err("removed provider syntax must fail while parsing package source");
        let message = error.to_string();

        assert!(message.contains(path), "unexpected error: {message}");
        assert!(
            message.contains("legacy provider syntax has been removed"),
            "unexpected error: {message}"
        );
    }
}
