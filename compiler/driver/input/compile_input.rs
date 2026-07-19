use skiff_compiler_input_model::PackageCompileInputMetadata;

use crate::input::source_graph::PackageSourceInput;

impl PackageCompileInputMetadata for PackageSourceInput {
    fn package_dependencies(&self) -> &[crate::input::PackageDependency] {
        &self.manifest.dependencies
    }
}

pub type PackageCompileInput<'a> =
    skiff_compiler_input_model::PackageCompileInput<'a, PackageSourceInput>;
