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
mod tests;
