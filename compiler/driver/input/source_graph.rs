#[cfg(test)]
use std::path::PathBuf;

use crate::{
    input::{PublicationManifest, ResolvedPackageGraph, SourceTree},
    shared::publication_error::PublicationError,
};
use skiff_compiler_input::RawPublication as RawInputPublication;

pub(crate) use skiff_compiler_source::source_graph::{CompilerSourceFile, PublicationSourceGraph};
#[cfg(test)]
pub(crate) use skiff_compiler_source::source_graph::{
    CompilerSourceRole, ParsedSourceFile, SourceOrigin,
};

#[derive(Debug, Clone)]
pub(crate) struct Publication {
    pub(crate) manifest: PublicationManifest,
    pub(crate) source_tree: SourceTree,
    pub(crate) source_graph: PublicationSourceGraph,
    pub(crate) package_graph: ResolvedPackageGraph,
}

impl Publication {
    pub(crate) fn new(
        manifest: PublicationManifest,
        source_tree: SourceTree,
        source_graph: PublicationSourceGraph,
        package_graph: ResolvedPackageGraph,
    ) -> Self {
        Self {
            manifest,
            source_tree,
            source_graph,
            package_graph,
        }
    }

    pub(crate) fn production_sources(&self) -> Vec<CompilerSourceFile> {
        self.source_graph.production_files()
    }
}

/// Shared stage-1 input-assembly core for both publication kinds.
///
/// Building a `Publication` from a manifest, source tree, source graph, and
/// resolved package graph is the one segment the service and package input lines
/// have in common. Each kind keeps its own overlay that produces these four
/// components, then funnels through this pure constructor.
#[cfg(test)]
pub(crate) fn assemble_publication(
    manifest: PublicationManifest,
    source_tree: SourceTree,
    source_graph: PublicationSourceGraph,
    package_graph: ResolvedPackageGraph,
) -> Publication {
    Publication::new(manifest, source_tree, source_graph, package_graph)
}

pub(crate) fn publication_from_raw(
    raw: RawInputPublication,
) -> Result<Publication, PublicationError> {
    let source_graph = PublicationSourceGraph::parse_raw_publication_sources(&raw.source_graph)?;
    Ok(publication_from_raw_with_source_graph(raw, source_graph))
}

pub(crate) fn publication_from_raw_with_source_graph(
    raw: RawInputPublication,
    source_graph: PublicationSourceGraph,
) -> Publication {
    Publication::new(
        raw.manifest,
        raw.source_tree,
        source_graph,
        raw.package_graph,
    )
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PackageSourceVisibility {
    Export { public_module_path: String },
    Private,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct PackagePublicationSources {
    root: PathBuf,
    package_id: String,
    files: Vec<ParsedSourceFile>,
    visibility_by_path: std::collections::BTreeMap<PathBuf, PackageSourceVisibility>,
}

#[cfg(test)]
impl PackagePublicationSources {
    pub(crate) fn with_visibility(
        root: PathBuf,
        package_id: impl Into<String>,
        files: Vec<ParsedSourceFile>,
        visibility_by_path: std::collections::BTreeMap<PathBuf, PackageSourceVisibility>,
    ) -> Result<Self, PublicationError> {
        let package_id = package_id.into();
        validate_package_publication_sources(&package_id, &files, &visibility_by_path)?;
        Ok(Self {
            root,
            package_id,
            files,
            visibility_by_path,
        })
    }

    pub(crate) fn into_compiler_sources(self) -> Vec<CompilerSourceFile> {
        let Self {
            root: _root,
            package_id: _package_id,
            files,
            visibility_by_path: _visibility_by_path,
        } = self;
        files
            .into_iter()
            .map(|source| CompilerSourceFile::from_parsed_file(source, CompilerSourceRole::Package))
            .collect()
    }

    pub(crate) fn into_source_graph(self) -> PublicationSourceGraph {
        PublicationSourceGraph::from_compiler_sources(self.into_compiler_sources())
    }

    pub(crate) fn source_tree(&self) -> SourceTree {
        SourceTree {
            root: self.root.clone(),
            sources: self
                .files
                .iter()
                .map(|source| crate::input::SourceTreeFile {
                    module_path: source.module_path.clone(),
                    file_path: source.relative_path.clone(),
                    is_test_file: source.is_test_file,
                    byte_len: source.text.len() as u64,
                })
                .collect(),
        }
    }

    pub(crate) fn files(&self) -> &[ParsedSourceFile] {
        &self.files
    }
}

#[cfg(test)]
fn validate_package_publication_sources(
    package_id: &str,
    files: &[ParsedSourceFile],
    visibility_by_path: &std::collections::BTreeMap<PathBuf, PackageSourceVisibility>,
) -> Result<(), PublicationError> {
    let file_paths = files
        .iter()
        .map(|source| source.relative_path.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut violations = Vec::new();

    for source in files {
        match source.origin() {
            SourceOrigin::Package {
                package_id: source_package_id,
            } if source_package_id == package_id => {}
            SourceOrigin::Package {
                package_id: source_package_id,
            } => violations.push(format!(
                "{} belongs to package {source_package_id}, expected {package_id}",
                source.relative_path.display()
            )),
            SourceOrigin::Service => violations.push(format!(
                "{} is not a package source",
                source.relative_path.display()
            )),
        }
        if !visibility_by_path.contains_key(&source.relative_path) {
            violations.push(format!(
                "{} has no package visibility",
                source.relative_path.display()
            ));
        }
    }

    for (path, visibility) in visibility_by_path {
        if !file_paths.contains(path) {
            violations.push(format!(
                "{} has package visibility but no parsed source",
                path.display()
            ));
        }
        if let PackageSourceVisibility::Export { public_module_path } = visibility {
            if public_module_path.is_empty() {
                violations.push(format!("{} has empty public module path", path.display()));
            }
        }
    }

    if violations.is_empty() {
        return Ok(());
    }

    Err(PublicationError::ContractValidation {
        message: violations
            .into_iter()
            .map(|violation| format!("- {violation}"))
            .collect::<Vec<_>>()
            .join("\n"),
    })
}
