use crate::{PublicationManifest, ResolvedPackageGraph};

#[derive(Debug, Clone)]
pub struct RawPublication {
    pub manifest: PublicationManifest,
    pub source_tree: crate::source_tree::SourceTree,
    pub source_graph: skiff_compiler_input_model::RawPublicationSourceGraph,
    pub package_graph: ResolvedPackageGraph,
}

impl RawPublication {
    pub fn new(
        manifest: PublicationManifest,
        source_tree: crate::source_tree::SourceTree,
        source_graph: skiff_compiler_input_model::RawPublicationSourceGraph,
        package_graph: ResolvedPackageGraph,
    ) -> Self {
        Self {
            manifest,
            source_tree,
            source_graph,
            package_graph,
        }
    }
}

pub fn assemble_publication(
    manifest: PublicationManifest,
    source_tree: crate::source_tree::SourceTree,
    source_graph: skiff_compiler_input_model::RawPublicationSourceGraph,
    package_graph: ResolvedPackageGraph,
) -> RawPublication {
    RawPublication::new(manifest, source_tree, source_graph, package_graph)
}
