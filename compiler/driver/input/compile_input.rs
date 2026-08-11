use std::{collections::BTreeMap, ops::Deref, path::Path};

use crate::input::{PackageDependency, PackageSourceInput};
use skiff_artifact_model::PackageArtifact;
use skiff_compiler_input::CompilerPlatformSources;
use skiff_compiler_input_model::{PackageCompileInputMetadata, PackageContractCompileDependency};

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
    canonical: CanonicalPackageCompileInput<'a>,
    canonical_artifact_root: Option<&'a Path>,
    test_service: bool,
    emit_bytecode: bool,
}

impl<'a> PackageCompileInput<'a> {
    /// Creates one package request with an explicit bytecode-lane decision.
    ///
    /// Compiler authoring enables bytecode by default. Pass `false` for the
    /// explicit legacy-only File-IR lane.
    pub fn new(
        platform_sources: &'a CompilerPlatformSources,
        package: &'a PackageSourceInput,
        package_aliases: &'a BTreeMap<String, Vec<String>>,
        package_id: &'a str,
        emit_bytecode: bool,
    ) -> Self {
        Self {
            platform_sources,
            canonical: CanonicalPackageCompileInput::new(package, package_aliases, package_id),
            canonical_artifact_root: None,
            test_service: false,
            emit_bytecode,
        }
    }

    pub fn platform_sources(&self) -> &CompilerPlatformSources {
        self.platform_sources
    }

    /// Whether this exact compile request selected the bytecode lane.
    ///
    /// The value is a required constructor argument. The driver never reads
    /// an environment variable or silently changes an enabled request into a
    /// legacy-only compilation.
    pub fn emit_bytecode(&self) -> bool {
        self.emit_bytecode
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

    /// Gives the compiler facade the root of canonical dependency records.
    ///
    /// The facade opens the deployment-owned store internally. Compiler
    /// callers never exchange the deployment storage owner through this
    /// package input.
    pub fn with_canonical_artifact_root(mut self, canonical_artifact_root: &'a Path) -> Self {
        self.canonical_artifact_root = Some(canonical_artifact_root);
        self
    }

    pub(crate) fn canonical_artifact_root(&self) -> Option<&'a Path> {
        self.canonical_artifact_root
    }

    /// Enables the test-service-only dependency visibility mode. This flag is
    /// compiler workflow authority; artifact shape remains ordinary.
    pub fn for_test_service(mut self) -> Self {
        self.test_service = true;
        self
    }

    pub(crate) fn is_test_service(&self) -> bool {
        self.test_service
    }
}

impl<'a> Deref for PackageCompileInput<'a> {
    type Target = CanonicalPackageCompileInput<'a>;

    fn deref(&self) -> &Self::Target {
        &self.canonical
    }
}
