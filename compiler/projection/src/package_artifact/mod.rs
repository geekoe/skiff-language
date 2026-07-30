mod api_exports;
mod assets;
mod boundary;
mod callables;
mod config_requirements;
mod export_links;
mod model;
mod projection;
mod runtime_requirements;
mod schema;
mod visible_types;

pub use boundary::project_boundary_callable;
pub use model::{
    PackageArtifactProjectionInput, ProjectedPackageArtifact, ProjectedPackageResource,
};
pub use projection::project_compiled_package_artifact;

#[cfg(test)]
mod tests;
