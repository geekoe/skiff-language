use skiff_compiler_input_model::PackageCompileInputMetadata;

use crate::input::source_graph::Publication;

impl PackageCompileInputMetadata for Publication {
    fn package_dependencies(&self) -> &[crate::input::PackageDependency] {
        &self.manifest.dependencies
    }
}

pub(crate) type PackageCompileInput<'a> =
    skiff_compiler_input_model::PackageCompileInput<'a, Publication>;
