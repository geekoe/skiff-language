use skiff_compiler_input_model::PublicationInputMetadata;

use crate::input::source_graph::Publication;

impl PublicationInputMetadata for Publication {
    fn package_dependencies(&self) -> &[crate::input::PackageDependency] {
        &self.manifest.dependencies
    }
}

pub(crate) type PublicationInputCore<'a> =
    skiff_compiler_input_model::PublicationInputCore<'a, Publication>;
pub(crate) type PackagePublicationInput<'a> =
    skiff_compiler_input_model::PackagePublicationInput<'a, Publication>;
pub(crate) type ServicePublicationInput<'a> =
    skiff_compiler_input_model::ServicePublicationInput<'a, Publication>;
pub(crate) type PublicationInput<'a> =
    skiff_compiler_input_model::PublicationInput<'a, Publication>;
