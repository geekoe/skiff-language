use crate::input::{PublicationManifest, PublicationResourceInput, SourceTree};

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
        }
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
