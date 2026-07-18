mod api_exports;
mod assets;
mod boundary;
mod callables;
mod export_links;
mod model;
mod projection;
mod projection_signatures;
mod runtime_requirements;
mod visible_types;

pub use boundary::project_boundary_callable;
pub use model::{
    PackageArtifactProjectionInput, ProjectedPackageArtifact, ProjectedPackageResource,
};
pub use projection::project_compiled_package_artifact;

#[cfg(test)]
mod tests;
