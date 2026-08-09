use std::{
    collections::{btree_map::Entry, BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    BytecodeArtifactRef, ContractOperationId, PackageArtifact, PackageArtifactRef, PackageBuildId,
    ServiceContract, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
    ServiceRequirementKey,
};

use crate::runtime_assembly::RuntimeAssemblyContentResolver;

/// Exact content boundary for consumer deployment bytecode hydration.
///
/// The bytecode method returns the opaque C1-C9 admission token. Raw
/// `BytecodeArtifact` content is never exposed to the loader or its callers.
pub trait DeploymentBytecodeContentResolver: RuntimeAssemblyContentResolver {
    fn resolve_package_bytecode(
        &self,
        package: &PackageArtifactRef,
        reference: &BytecodeArtifactRef,
    ) -> anyhow::Result<Arc<ValidatedBytecodeArtifact>>;
}

/// Typed reference values retained in a hydration mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeploymentBytecodeReference {
    ServiceDeployment(ServiceDeploymentRef),
    Package(PackageArtifactRef),
    PackageBytecode {
        package: PackageArtifactRef,
        bytecode: BytecodeArtifactRef,
    },
}

/// Fail-closed categories produced while hydrating exact deployment bytecode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeploymentBytecodeHydrationError {
    MissingBytecode {
        package: PackageArtifactRef,
    },
    ReferenceMismatch {
        expected: DeploymentBytecodeReference,
        actual: DeploymentBytecodeReference,
    },
    DuplicatePackage {
        package_build_id: PackageBuildId,
        first: PackageArtifactRef,
        duplicate: PackageArtifactRef,
    },
    DuplicateServiceSlot {
        key: ServiceRequirementKey,
    },
    ContractMismatch {
        key: Option<ServiceRequirementKey>,
        expected: Option<ServiceContractRef>,
        actual: Option<ServiceContractRef>,
    },
}

impl fmt::Display for DeploymentBytecodeHydrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBytecode { package } => write!(
                formatter,
                "package {} has no bytecode artifact reference",
                package.package_build_id
            ),
            Self::ReferenceMismatch { expected, actual } => write!(
                formatter,
                "deployment bytecode reference mismatch: expected {expected:?}, actual {actual:?}"
            ),
            Self::DuplicatePackage {
                package_build_id,
                first,
                duplicate,
            } => write!(
                formatter,
                "package build {package_build_id} is hydrated more than once: {first:?} and {duplicate:?}"
            ),
            Self::DuplicateServiceSlot { key } => {
                write!(formatter, "service dependency slot {key:?} is repeated")
            }
            Self::ContractMismatch {
                key,
                expected,
                actual,
            } => write!(
                formatter,
                "consumer contract mismatch at service slot {key:?}: expected {expected:?}, actual {actual:?}"
            ),
        }
    }
}

impl std::error::Error for DeploymentBytecodeHydrationError {}

/// One exact PackageArtifact paired with its admitted bytecode image.
#[derive(Debug)]
pub struct HydratedBytecodePackage {
    reference: PackageArtifactRef,
    artifact: Arc<PackageArtifact>,
    bytecode: Arc<ValidatedBytecodeArtifact>,
}

impl HydratedBytecodePackage {
    #[allow(dead_code)]
    pub(crate) fn checked(
        reference: PackageArtifactRef,
        artifact: Arc<PackageArtifact>,
        bytecode: Arc<ValidatedBytecodeArtifact>,
    ) -> Result<Self, DeploymentBytecodeHydrationError> {
        let actual_reference = exact_package_reference(&artifact);
        if reference != actual_reference {
            return Err(DeploymentBytecodeHydrationError::ReferenceMismatch {
                expected: DeploymentBytecodeReference::Package(reference),
                actual: DeploymentBytecodeReference::Package(actual_reference),
            });
        }
        let declared_bytecode = artifact.bytecode.clone().ok_or_else(|| {
            DeploymentBytecodeHydrationError::MissingBytecode {
                package: reference.clone(),
            }
        })?;
        if &declared_bytecode != bytecode.reference() {
            return Err(DeploymentBytecodeHydrationError::ReferenceMismatch {
                expected: DeploymentBytecodeReference::PackageBytecode {
                    package: reference.clone(),
                    bytecode: declared_bytecode,
                },
                actual: DeploymentBytecodeReference::PackageBytecode {
                    package: reference,
                    bytecode: bytecode.reference().clone(),
                },
            });
        }
        Ok(Self {
            reference,
            artifact,
            bytecode,
        })
    }

    pub fn reference(&self) -> &PackageArtifactRef {
        &self.reference
    }

    pub fn artifact(&self) -> &Arc<PackageArtifact> {
        &self.artifact
    }

    pub fn bytecode(&self) -> &Arc<ValidatedBytecodeArtifact> {
        &self.bytecode
    }
}

/// Consumer-side symbolic service dependency facts.
///
/// No provider deployment, provider build, release pointer or executable is
/// retained in this row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydratedServiceDependency {
    key: ServiceRequirementKey,
    contract: ServiceContractRef,
    used_operations: BTreeSet<ContractOperationId>,
}

impl HydratedServiceDependency {
    #[allow(dead_code)]
    pub(crate) fn new(
        key: ServiceRequirementKey,
        contract: ServiceContractRef,
        used_operations: BTreeSet<ContractOperationId>,
    ) -> Self {
        Self {
            key,
            contract,
            used_operations,
        }
    }

    pub fn key(&self) -> &ServiceRequirementKey {
        &self.key
    }

    pub fn contract(&self) -> &ServiceContractRef {
        &self.contract
    }

    pub fn used_operations(&self) -> &BTreeSet<ContractOperationId> {
        &self.used_operations
    }
}

/// Exact, consumer-only input to deployment bytecode linking.
#[derive(Debug)]
pub struct HydratedDeploymentBytecode {
    reference: ServiceDeploymentRef,
    deployment: Arc<ServiceDeployment>,
    contract_store: BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    service_dependencies: BTreeMap<ServiceRequirementKey, HydratedServiceDependency>,
    packages: BTreeMap<PackageBuildId, HydratedBytecodePackage>,
}

impl HydratedDeploymentBytecode {
    #[allow(dead_code)]
    pub(crate) fn checked(
        reference: ServiceDeploymentRef,
        deployment: Arc<ServiceDeployment>,
        contract_store: BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
        service_dependencies: Vec<HydratedServiceDependency>,
        packages: Vec<HydratedBytecodePackage>,
    ) -> Result<Self, DeploymentBytecodeHydrationError> {
        let actual_reference = exact_deployment_reference(&deployment);
        if reference != actual_reference {
            return Err(DeploymentBytecodeHydrationError::ReferenceMismatch {
                expected: DeploymentBytecodeReference::ServiceDeployment(reference),
                actual: DeploymentBytecodeReference::ServiceDeployment(actual_reference),
            });
        }
        validate_contract_store(&contract_store)?;
        let packages = canonical_packages(packages)?;
        let service_dependencies =
            canonical_service_dependencies(&deployment, service_dependencies)?;
        validate_required_contracts(&deployment, &contract_store, &service_dependencies)?;
        Ok(Self {
            reference,
            deployment,
            contract_store,
            service_dependencies,
            packages,
        })
    }

    pub fn reference(&self) -> &ServiceDeploymentRef {
        &self.reference
    }

    pub fn deployment(&self) -> &Arc<ServiceDeployment> {
        &self.deployment
    }

    pub fn contract_store(&self) -> &BTreeMap<ServiceContractRef, Arc<ServiceContract>> {
        &self.contract_store
    }

    pub fn service_dependencies(
        &self,
    ) -> &BTreeMap<ServiceRequirementKey, HydratedServiceDependency> {
        &self.service_dependencies
    }

    pub fn packages(&self) -> &BTreeMap<PackageBuildId, HydratedBytecodePackage> {
        &self.packages
    }
}

/// Resolver handle for the future deployment bytecode load algorithm.
///
/// The interface deliberately exposes no `load` or public parts constructor
/// until exact closure hydration is implemented.
pub struct DeploymentBytecodeLoader<'a, R: ?Sized> {
    resolver: &'a R,
}

impl<'a, R> DeploymentBytecodeLoader<'a, R>
where
    R: DeploymentBytecodeContentResolver + ?Sized,
{
    pub fn new(resolver: &'a R) -> Self {
        Self { resolver }
    }

    pub fn resolver(&self) -> &'a R {
        self.resolver
    }
}

fn exact_package_reference(artifact: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: artifact.package_id.clone(),
        package_version: artifact.package_version.clone(),
        package_build_id: artifact.package_build_id.clone(),
        package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
    }
}

fn exact_deployment_reference(deployment: &ServiceDeployment) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: deployment.contract.service_id.clone(),
        contract_version: deployment.contract.contract_version.clone(),
        deployment_revision: deployment.deployment_revision.clone(),
        deployment_artifact_identity: deployment.deployment_artifact_identity.clone(),
    }
}

fn exact_contract_reference(contract: &ServiceContract) -> ServiceContractRef {
    ServiceContractRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        service_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

fn validate_contract_store(
    contracts: &BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    for (expected, contract) in contracts {
        let actual = exact_contract_reference(contract);
        if expected != &actual {
            return Err(DeploymentBytecodeHydrationError::ContractMismatch {
                key: None,
                expected: Some(expected.clone()),
                actual: Some(actual),
            });
        }
    }
    Ok(())
}

fn canonical_packages(
    packages: Vec<HydratedBytecodePackage>,
) -> Result<BTreeMap<PackageBuildId, HydratedBytecodePackage>, DeploymentBytecodeHydrationError> {
    let mut canonical = BTreeMap::<PackageBuildId, HydratedBytecodePackage>::new();
    for package in packages {
        let build_id = package.reference.package_build_id.clone();
        match canonical.entry(build_id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(package);
            }
            Entry::Occupied(entry) => {
                return Err(DeploymentBytecodeHydrationError::DuplicatePackage {
                    package_build_id: build_id,
                    first: entry.get().reference.clone(),
                    duplicate: package.reference,
                });
            }
        }
    }
    Ok(canonical)
}

fn canonical_service_dependencies(
    deployment: &ServiceDeployment,
    dependencies: Vec<HydratedServiceDependency>,
) -> Result<
    BTreeMap<ServiceRequirementKey, HydratedServiceDependency>,
    DeploymentBytecodeHydrationError,
> {
    let mut selectors = BTreeMap::new();
    for selector in &deployment.service_selectors {
        if selectors
            .insert(selector.key.clone(), selector.contract.clone())
            .is_some()
        {
            return Err(DeploymentBytecodeHydrationError::DuplicateServiceSlot {
                key: selector.key.clone(),
            });
        }
    }

    let mut canonical = BTreeMap::new();
    for dependency in dependencies {
        let key = dependency.key.clone();
        match canonical.entry(key.clone()) {
            Entry::Occupied(_) => {
                return Err(DeploymentBytecodeHydrationError::DuplicateServiceSlot { key });
            }
            Entry::Vacant(entry) => {
                match selectors.get(&key) {
                    Some(expected) if expected != &dependency.contract => {
                        return Err(DeploymentBytecodeHydrationError::ContractMismatch {
                            key: Some(key),
                            expected: Some(expected.clone()),
                            actual: Some(dependency.contract),
                        });
                    }
                    None => {
                        return Err(DeploymentBytecodeHydrationError::ContractMismatch {
                            key: Some(key),
                            expected: None,
                            actual: Some(dependency.contract),
                        });
                    }
                    Some(_) => {}
                }
                entry.insert(dependency);
            }
        }
    }

    if let Some((key, contract)) = selectors
        .iter()
        .find(|(key, _)| !canonical.contains_key(*key))
    {
        return Err(DeploymentBytecodeHydrationError::ContractMismatch {
            key: Some((*key).clone()),
            expected: Some((*contract).clone()),
            actual: None,
        });
    }
    Ok(canonical)
}

fn validate_required_contracts(
    deployment: &ServiceDeployment,
    contracts: &BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    dependencies: &BTreeMap<ServiceRequirementKey, HydratedServiceDependency>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    if !contracts.contains_key(&deployment.contract) {
        return Err(DeploymentBytecodeHydrationError::ContractMismatch {
            key: None,
            expected: Some(deployment.contract.clone()),
            actual: None,
        });
    }
    for dependency in dependencies.values() {
        if !contracts.contains_key(&dependency.contract) {
            return Err(DeploymentBytecodeHydrationError::ContractMismatch {
                key: Some(dependency.key.clone()),
                expected: Some(dependency.contract.clone()),
                actual: None,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
