use std::{collections::BTreeMap, ops::Deref};

use skiff_artifact_model::PackageArtifact;
use skiff_compiler_input::CompilerPlatformSources;
use skiff_compiler_input_model::{PackageCompileInputMetadata, PackageContractCompileDependency};
use skiff_compiler_projection_input::ResolvedPackageSchema;
use skiff_deployment::storage::CanonicalArtifactStore;

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
    canonical: CanonicalPackageCompileInput<'a>,
    resolved_package_schemas: &'a [ResolvedPackageSchema],
    canonical_artifact_store: Option<&'a CanonicalArtifactStore>,
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
            canonical: CanonicalPackageCompileInput::new(package, package_aliases, package_id),
            resolved_package_schemas: &[],
            canonical_artifact_store: None,
        }
    }

    pub fn platform_sources(&self) -> &CompilerPlatformSources {
        self.platform_sources
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

    /// Supplies store-verified schema records for exact dependency bindings.
    ///
    /// The driver selects only bindings that are actually required after File
    /// IR closes compiler-owned dependencies such as `std`.
    pub fn with_resolved_package_schemas(
        mut self,
        resolved_package_schemas: &'a [ResolvedPackageSchema],
    ) -> Self {
        self.resolved_package_schemas = resolved_package_schemas;
        self
    }

    pub fn resolved_package_schemas(&self) -> &'a [ResolvedPackageSchema] {
        self.resolved_package_schemas
    }

    /// Gives the compiler driver read-only access to canonical dependency
    /// records. Projection crates never receive this filesystem owner.
    pub fn with_canonical_artifact_store(
        mut self,
        canonical_artifact_store: &'a CanonicalArtifactStore,
    ) -> Self {
        self.canonical_artifact_store = Some(canonical_artifact_store);
        self
    }

    pub(crate) fn canonical_artifact_store(&self) -> Option<&'a CanonicalArtifactStore> {
        self.canonical_artifact_store
    }
}

impl<'a> Deref for PackageCompileInput<'a> {
    type Target = CanonicalPackageCompileInput<'a>;

    fn deref(&self) -> &Self::Target {
        &self.canonical
    }
}
