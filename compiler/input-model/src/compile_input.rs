use std::collections::BTreeMap;

use skiff_artifact_model::{ContractRequirement, PackageArtifact, ServiceContract};

use crate::PackageDependency;

pub trait PackageCompileInputMetadata {
    fn package_dependencies(&self) -> &[PackageDependency];
}

/// One validated contract dependency supplied to package compilation.
///
/// The requirement owns the source alias and expected identity; the contract
/// is the independently published, code-free protocol artifact. No provider
/// package, build, deployment, route, or executable fact crosses this input.
#[derive(Debug, Clone)]
pub struct PackageContractCompileDependency {
    pub requirement: ContractRequirement,
    pub contract: ServiceContract,
}

/// The only production input for user source compilation.
///
/// Package dependencies are canonical PackageArtifacts and service calls are
/// compiled solely against ServiceContracts. Legacy service configuration is
/// adapted before this boundary and is never stored here.
pub struct PackageCompileInput<'a, P: ?Sized> {
    pub package: &'a P,
    pub package_id: &'a str,
    pub package_aliases: &'a BTreeMap<String, Vec<String>>,
    pub package_dependencies: &'a [PackageDependency],
    pub dependency_packages: &'a [PackageArtifact],
    pub contract_dependencies: &'a [PackageContractCompileDependency],
}

impl<'a, P: PackageCompileInputMetadata + ?Sized> PackageCompileInput<'a, P> {
    pub fn new(
        package: &'a P,
        package_aliases: &'a BTreeMap<String, Vec<String>>,
        package_id: &'a str,
    ) -> Self {
        Self {
            package,
            package_id,
            package_aliases,
            package_dependencies: package.package_dependencies(),
            dependency_packages: &[],
            contract_dependencies: &[],
        }
    }

    pub fn with_canonical_dependencies(
        mut self,
        dependency_packages: &'a [PackageArtifact],
        contract_dependencies: &'a [PackageContractCompileDependency],
    ) -> Self {
        self.dependency_packages = dependency_packages;
        self.contract_dependencies = contract_dependencies;
        self
    }
}
