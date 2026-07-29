use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{
    service_deployment_ref, validate_package_artifact_identities,
    validate_service_contract_identities, validate_service_deployment_identity,
    ValidatedPackageArtifact,
};
use skiff_artifact_model::{
    PackageArtifact, PackageArtifactRef, PackageBuildId, ServiceContract, ServiceContractRef,
    ServiceDeployment, ServiceDeploymentRef, ServiceProtocolIdentity,
};

use super::{AssemblyResolutionError, AssemblyResult};

pub(super) struct CandidateIndex<'a> {
    deployments: BTreeMap<ServiceDeploymentRef, &'a ServiceDeployment>,
    providers: BTreeMap<ServiceContractRef, BTreeSet<ServiceDeploymentRef>>,
    provider_protocols: BTreeMap<(String, String), BTreeSet<ServiceProtocolIdentity>>,
    contracts: BTreeMap<ServiceContractRef, &'a ServiceContract>,
    contract_protocols: BTreeMap<(String, String), BTreeSet<ServiceProtocolIdentity>>,
    packages: BTreeMap<PackageBuildId, (PackageArtifactRef, &'a PackageArtifact)>,
}

impl<'a> CandidateIndex<'a> {
    pub(super) fn new(
        deployments: &'a [ServiceDeployment],
        contracts: &'a [ServiceContract],
        packages: &'a [PackageArtifact],
    ) -> AssemblyResult<Self> {
        let mut index = Self {
            deployments: BTreeMap::new(),
            providers: BTreeMap::new(),
            provider_protocols: BTreeMap::new(),
            contracts: BTreeMap::new(),
            contract_protocols: BTreeMap::new(),
            packages: BTreeMap::new(),
        };
        index.index_deployments(deployments)?;
        index.index_contracts(contracts)?;
        index.index_packages(packages)?;
        Ok(index)
    }

    pub(super) fn new_with_validated_packages(
        deployments: &'a [ServiceDeployment],
        contracts: &'a [ServiceContract],
        packages: &'a [PackageArtifact],
        validated_packages: &[ValidatedPackageArtifact],
    ) -> AssemblyResult<Self> {
        let mut index = Self {
            deployments: BTreeMap::new(),
            providers: BTreeMap::new(),
            provider_protocols: BTreeMap::new(),
            contracts: BTreeMap::new(),
            contract_protocols: BTreeMap::new(),
            packages: BTreeMap::new(),
        };
        index.index_deployments(deployments)?;
        index.index_contracts(contracts)?;
        index.index_validated_packages(packages, validated_packages)?;
        Ok(index)
    }

    fn index_deployments(&mut self, deployments: &'a [ServiceDeployment]) -> AssemblyResult<()> {
        for deployment in deployments {
            validate_service_deployment_identity(deployment)?;
            let reference = service_deployment_ref(deployment);
            self.deployments
                .entry(reference.clone())
                .or_insert(deployment);
            self.providers
                .entry(deployment.contract.clone())
                .or_default()
                .insert(reference);
            self.provider_protocols
                .entry(contract_coordinate(&deployment.contract))
                .or_default()
                .insert(deployment.contract.service_protocol_identity.clone());
        }
        Ok(())
    }

    fn index_contracts(&mut self, contracts: &'a [ServiceContract]) -> AssemblyResult<()> {
        for contract in contracts {
            validate_service_contract_identities(contract)?;
            let reference = service_contract_ref(contract);
            self.contracts.entry(reference.clone()).or_insert(contract);
            self.contract_protocols
                .entry(contract_coordinate(&reference))
                .or_default()
                .insert(reference.service_protocol_identity);
        }
        Ok(())
    }

    fn index_packages(&mut self, packages: &'a [PackageArtifact]) -> AssemblyResult<()> {
        for package in packages {
            validate_package_artifact_identities(package)?;
            let reference = package_artifact_ref(package);
            self.insert_package(reference, package)?;
        }
        Ok(())
    }

    fn index_validated_packages(
        &mut self,
        packages: &'a [PackageArtifact],
        validated_packages: &[ValidatedPackageArtifact],
    ) -> AssemblyResult<()> {
        for (package, validated) in packages.iter().zip(validated_packages) {
            self.insert_package(validated.reference().clone(), package)?;
        }
        Ok(())
    }

    fn insert_package(
        &mut self,
        reference: PackageArtifactRef,
        package: &'a PackageArtifact,
    ) -> AssemblyResult<()> {
        match self.packages.entry(reference.package_build_id.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert((reference, package));
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                if entry.get().0 != reference {
                    return Err(AssemblyResolutionError::ConflictingCandidatePackageBuild {
                        build_id: reference.package_build_id.clone(),
                        first: entry.get().0.clone(),
                        second: reference,
                    });
                }
            }
        }
        Ok(())
    }

    pub(super) fn contains_deployment(&self, reference: &ServiceDeploymentRef) -> bool {
        self.deployments.contains_key(reference)
    }

    pub(super) fn deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> Option<&'a ServiceDeployment> {
        self.deployments.get(reference).copied()
    }

    pub(super) fn contract(
        &self,
        reference: &ServiceContractRef,
    ) -> AssemblyResult<&'a ServiceContract> {
        if let Some(contract) = self.contracts.get(reference) {
            return Ok(*contract);
        }
        let coordinate = contract_coordinate(reference);
        match self.contract_protocols.get(&coordinate) {
            Some(available) => Err(AssemblyResolutionError::ContractProtocolMismatch {
                service_id: reference.service_id.clone(),
                contract_version: reference.contract_version.clone(),
                expected: reference.service_protocol_identity.clone(),
                available: available.iter().cloned().collect(),
            }),
            None => Err(AssemblyResolutionError::MissingContract(reference.clone())),
        }
    }

    pub(super) fn package(
        &self,
        reference: &PackageArtifactRef,
    ) -> AssemblyResult<&'a PackageArtifact> {
        let Some((available, package)) = self.packages.get(&reference.package_build_id) else {
            return Err(AssemblyResolutionError::MissingPackageArtifact(
                reference.clone(),
            ));
        };
        if available != reference {
            return Err(AssemblyResolutionError::PackageReferenceMismatch {
                expected: reference.clone(),
                available: available.clone(),
            });
        }
        Ok(*package)
    }

    pub(super) fn provider(
        &self,
        contract: &ServiceContractRef,
    ) -> AssemblyResult<ServiceDeploymentRef> {
        if let Some(providers) = self.providers.get(contract) {
            if providers.len() == 1 {
                return Ok(providers.iter().next().expect("one provider").clone());
            }
            return Err(AssemblyResolutionError::AmbiguousServiceProvider {
                contract: contract.clone(),
                providers: providers.iter().cloned().collect(),
            });
        }

        let coordinate = contract_coordinate(contract);
        match self.provider_protocols.get(&coordinate) {
            Some(available) => Err(AssemblyResolutionError::ServiceProviderProtocolMismatch {
                service_id: contract.service_id.clone(),
                contract_version: contract.contract_version.clone(),
                expected: contract.service_protocol_identity.clone(),
                available: available.iter().cloned().collect(),
            }),
            None => Err(AssemblyResolutionError::MissingServiceProvider(
                contract.clone(),
            )),
        }
    }
}

fn contract_coordinate(reference: &ServiceContractRef) -> (String, String) {
    (
        reference.service_id.clone(),
        reference.contract_version.clone(),
    )
}

fn service_contract_ref(contract: &ServiceContract) -> ServiceContractRef {
    ServiceContractRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        service_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

fn package_artifact_ref(package: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: package.package_id.clone(),
        package_version: package.package_version.clone(),
        package_build_id: package.package_build_id.clone(),
        package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
    }
}
