use std::{collections::BTreeMap, ops::Deref};

use skiff_artifact_model::PackageArtifact;
use skiff_compiler_input::{CompilerPlatformPackageAuthority, CompilerPlatformSources};
use skiff_compiler_input_model::{PackageCompileInputMetadata, PackageContractCompileDependency};

use crate::input::{PackageDependency, PackageSourceInput};

impl PackageCompileInputMetadata for PackageSourceInput {
    fn package_dependencies(&self) -> &[PackageDependency] {
        &self.manifest.dependencies
    }
}

type CanonicalPackageCompileInput<'a> =
    skiff_compiler_input_model::PackageCompileInput<'a, PackageSourceInput>;

/// The compiler-library package input plus its explicit platform trust owner.
pub struct PackageCompileInput<'a> {
    platform_sources: &'a CompilerPlatformSources,
    platform_package_authority: Option<&'a CompilerPlatformPackageAuthority>,
    canonical: CanonicalPackageCompileInput<'a>,
}

impl<'a> PackageCompileInput<'a> {
    pub fn new(
        platform_sources: &'a CompilerPlatformSources,
        package: &'a PackageSourceInput,
        package_aliases: &'a BTreeMap<String, Vec<String>>,
        package_id: &'a str,
    ) -> Self {
        Self {
            platform_sources,
            platform_package_authority: None,
            canonical: CanonicalPackageCompileInput::new(package, package_aliases, package_id),
        }
    }

    pub fn with_platform_package_authority(
        mut self,
        authority: &'a CompilerPlatformPackageAuthority,
    ) -> Self {
        assert_eq!(
            self.canonical.package_id,
            authority.package_id(),
            "compiler platform authority must match the exact package"
        );
        self.platform_package_authority = Some(authority);
        self
    }

    pub fn platform_sources(&self) -> &CompilerPlatformSources {
        self.platform_sources
    }

    pub(crate) fn platform_package_authority(&self) -> Option<&CompilerPlatformPackageAuthority> {
        self.platform_package_authority
    }

    pub fn with_canonical_dependencies(
        mut self,
        dependency_packages: &'a [PackageArtifact],
        contract_dependencies: &'a [PackageContractCompileDependency],
    ) -> Self {
        self.canonical = self
            .canonical
            .with_canonical_dependencies(dependency_packages, contract_dependencies);
        self
    }

    pub fn with_available_canonical_packages(
        mut self,
        available_packages: &'a [PackageArtifact],
    ) -> Self {
        self.canonical = self
            .canonical
            .with_available_canonical_packages(available_packages);
        self
    }
}

impl<'a> Deref for PackageCompileInput<'a> {
    type Target = CanonicalPackageCompileInput<'a>;

    fn deref(&self) -> &Self::Target {
        &self.canonical
    }
}
