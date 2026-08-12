use std::{
    collections::{btree_map::Entry, BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    current_platform_error_projection_registry_ref, BytecodeArtifactRef, ContractOperationId,
    PackageArtifact, PackageArtifactRef, PackageBinding, PackageBuildId, PackageCallableId,
    PackageExecutableCoordinate, PackageRequirement, PackageRequirementKey, PackageSchemaTypeId,
    PlatformErrorProjectionRegistryRef, ServiceContract, ServiceContractRef, ServiceDeployment,
    ServiceDeploymentRef, ServiceRequirementKey,
};

mod manifests;

use manifests::{validate_deployment_manifests, HydratedPackageManifests};

/// Exact content boundary for consumer deployment bytecode hydration.
///
/// Every method is intentionally repeated on this narrow capability instead
/// of inheriting [`crate::DeploymentBytecodeContentResolver`]. The bytecode lane
/// cannot name, resolve or accidentally reopen File IR, schemas or resources.
/// Raw `BytecodeArtifact` content is never accepted; package bytecode crosses
/// the boundary only as its opaque C1-C9 admission token.
pub trait DeploymentBytecodeContentResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>>;

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>>;

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>>;

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
    ServiceContract(ServiceContractRef),
    Package(PackageArtifactRef),
    PackageBytecode {
        package: PackageArtifactRef,
        bytecode: BytecodeArtifactRef,
    },
}

/// Exact cross-record authority that failed while admitting a self-contained
/// bytecode package. The category remains typed while `detail` on the error
/// carries the concrete row/identity diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentBytecodeManifestKind {
    Header,
    FunctionOrigin,
    SyntheticCallback,
    StatementAttribution,
    SelfType,
    Callable,
    Actor,
    InterfaceConformance,
    ConstantRoot,
    SchemaDescriptor,
    PackageReference,
    ServiceOperation,
    RemoteInterface,
}

/// Fail-closed categories produced while hydrating exact deployment bytecode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeploymentBytecodeHydrationError {
    ContentResolution {
        reference: Box<DeploymentBytecodeReference>,
        message: String,
    },
    InvalidContent {
        reference: Box<DeploymentBytecodeReference>,
        message: String,
    },
    ManifestMismatch {
        package: Box<PackageArtifactRef>,
        kind: DeploymentBytecodeManifestKind,
        detail: String,
    },
    PlatformErrorProjectionRegistryMismatch {
        package: Box<PackageArtifactRef>,
        package_artifact: Box<PlatformErrorProjectionRegistryRef>,
        bytecode_header: Box<PlatformErrorProjectionRegistryRef>,
        structurally_validated_view: Box<PlatformErrorProjectionRegistryRef>,
        runtime: Box<PlatformErrorProjectionRegistryRef>,
    },
    MixedPlatformErrorProjectionRegistry {
        implementation: Box<PackageArtifactRef>,
        implementation_registry: Box<PlatformErrorProjectionRegistryRef>,
        package: Box<PackageArtifactRef>,
        package_registry: Box<PlatformErrorProjectionRegistryRef>,
    },
    MissingBytecode {
        package: Box<PackageArtifactRef>,
    },
    ReferenceMismatch {
        expected: Box<DeploymentBytecodeReference>,
        actual: Box<DeploymentBytecodeReference>,
    },
    DuplicatePackage {
        package_build_id: PackageBuildId,
        first: Box<PackageArtifactRef>,
        duplicate: Box<PackageArtifactRef>,
    },
    DuplicatePackageBinding {
        key: PackageRequirementKey,
    },
    MissingPackageBinding {
        key: PackageRequirementKey,
    },
    UnexpectedPackageBinding {
        key: PackageRequirementKey,
    },
    PackageRequirementMismatch {
        key: PackageRequirementKey,
        requirement: Box<PackageRequirement>,
        selected: Box<PackageArtifactRef>,
    },
    ConflictingPackageOwner {
        package_id: String,
        first_build_id: PackageBuildId,
        second_build_id: PackageBuildId,
    },
    MissingSchemaPackageOwner {
        package_id: String,
        stable_schema_key: String,
        type_id: PackageSchemaTypeId,
    },
    DuplicateServiceSlot {
        key: ServiceRequirementKey,
    },
    ContractMismatch {
        key: Option<ServiceRequirementKey>,
        expected: Option<Box<ServiceContractRef>>,
        actual: Option<Box<ServiceContractRef>>,
    },
    MissingOperation {
        key: ServiceRequirementKey,
        contract: Box<ServiceContractRef>,
        operation: ContractOperationId,
    },
    OperationCoverageMismatch {
        contract: Box<ServiceContractRef>,
        expected: BTreeSet<ContractOperationId>,
        actual: BTreeSet<ContractOperationId>,
    },
}

impl fmt::Display for DeploymentBytecodeHydrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContentResolution { reference, message } => {
                write!(formatter, "failed to resolve {reference:?}: {message}")
            }
            Self::InvalidContent { reference, message } => {
                write!(formatter, "invalid content for {reference:?}: {message}")
            }
            Self::ManifestMismatch {
                package,
                kind,
                detail,
            } => write!(
                formatter,
                "package {} has an invalid {kind:?} bytecode manifest: {detail}",
                package.package_build_id
            ),
            Self::PlatformErrorProjectionRegistryMismatch {
                package,
                package_artifact,
                bytecode_header,
                structurally_validated_view,
                runtime,
            } => write!(
                formatter,
                "package {} does not join one platform error projection registry: PackageArtifact {package_artifact:?}, BytecodeArtifact header {bytecode_header:?}, structurally validated view {structurally_validated_view:?}, runtime {runtime:?}",
                package.package_build_id
            ),
            Self::MixedPlatformErrorProjectionRegistry {
                implementation,
                implementation_registry,
                package,
                package_registry,
            } => write!(
                formatter,
                "deployment package closure mixes platform error projection registries: implementation package {} uses {implementation_registry:?}, package {} uses {package_registry:?}",
                implementation.package_build_id, package.package_build_id
            ),
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
            Self::DuplicatePackageBinding { key } => {
                write!(formatter, "package binding {key:?} is repeated")
            }
            Self::MissingPackageBinding { key } => {
                write!(formatter, "package requirement {key:?} has no exact binding")
            }
            Self::UnexpectedPackageBinding { key } => {
                write!(formatter, "package binding {key:?} is outside the consumer closure")
            }
            Self::PackageRequirementMismatch {
                key,
                requirement,
                selected,
            } => write!(
                formatter,
                "package binding {key:?} selects {selected:?}, which mismatches {requirement:?}"
            ),
            Self::ConflictingPackageOwner {
                package_id,
                first_build_id,
                second_build_id,
            } => write!(
                formatter,
                "consumer closure resolves package {package_id} to builds {first_build_id} and {second_build_id}"
            ),
            Self::MissingSchemaPackageOwner {
                package_id,
                stable_schema_key,
                type_id,
            } => write!(
                formatter,
                "PackageSchema reference {package_id}:{stable_schema_key}:{type_id} has no hydrated package owner"
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
            Self::MissingOperation {
                key,
                contract,
                operation,
            } => write!(
                formatter,
                "service dependency {key:?} references missing operation {operation} in {contract:?}"
            ),
            Self::OperationCoverageMismatch {
                contract,
                expected,
                actual,
            } => write!(
                formatter,
                "deployment operation coverage for {contract:?} is {actual:?}, expected {expected:?}"
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
    platform_error_projection_registry: PlatformErrorProjectionRegistryRef,
    manifests: HydratedPackageManifests,
}

impl HydratedBytecodePackage {
    pub(crate) fn checked(
        reference: PackageArtifactRef,
        artifact: Arc<PackageArtifact>,
        bytecode: Arc<ValidatedBytecodeArtifact>,
    ) -> Result<Self, DeploymentBytecodeHydrationError> {
        let platform_error_projection_registry =
            join_package_platform_error_projection_registry(&reference, &artifact, &bytecode)?;
        let actual_reference = exact_package_reference(&artifact);
        if reference != actual_reference {
            return Err(DeploymentBytecodeHydrationError::ReferenceMismatch {
                expected: Box::new(DeploymentBytecodeReference::Package(reference)),
                actual: Box::new(DeploymentBytecodeReference::Package(actual_reference)),
            });
        }
        let declared_bytecode = artifact.bytecode.clone().ok_or_else(|| {
            DeploymentBytecodeHydrationError::MissingBytecode {
                package: Box::new(reference.clone()),
            }
        })?;
        if &declared_bytecode != bytecode.reference() {
            return Err(DeploymentBytecodeHydrationError::ReferenceMismatch {
                expected: Box::new(DeploymentBytecodeReference::PackageBytecode {
                    package: reference.clone(),
                    bytecode: declared_bytecode,
                }),
                actual: Box::new(DeploymentBytecodeReference::PackageBytecode {
                    package: reference,
                    bytecode: bytecode.reference().clone(),
                }),
            });
        }
        let manifests = HydratedPackageManifests::checked(&reference, &artifact, &bytecode)?;
        Ok(Self {
            reference,
            artifact,
            bytecode,
            platform_error_projection_registry,
            manifests,
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

    pub fn platform_error_projection_registry(&self) -> &PlatformErrorProjectionRegistryRef {
        &self.platform_error_projection_registry
    }

    /// Returns the admitted artifact function key for one exact path-free
    /// executable coordinate.
    pub fn function_key_for_executable(
        &self,
        executable: &PackageExecutableCoordinate,
    ) -> Option<&str> {
        self.manifests.function_key_for_executable(executable)
    }

    /// Returns the admitted artifact function key selected by one exact
    /// package callable manifest row.
    pub fn function_key_for_callable(&self, callable: &PackageCallableId) -> Option<&str> {
        self.manifests.function_key_for_callable(callable)
    }

    /// Returns the unique package implementation callable that canonically
    /// owns one ordinary executable origin. Public aliases are never returned.
    pub fn canonical_implementation_callable_for_executable(
        &self,
        executable: &PackageExecutableCoordinate,
    ) -> Option<&PackageCallableId> {
        self.manifests
            .canonical_implementation_callable_for_executable(executable)
    }

    /// Returns the unique package implementation callable that canonically
    /// owns one admitted ordinary function key.
    pub fn canonical_implementation_callable_for_function_key(
        &self,
        function_key: &str,
    ) -> Option<&PackageCallableId> {
        self.manifests
            .canonical_implementation_callable_for_function_key(function_key)
    }

    /// Resolves a canonical implementation callable back to its admitted
    /// ordinary function. Public aliases are intentionally absent.
    pub fn function_key_for_canonical_implementation_callable(
        &self,
        callable: &PackageCallableId,
    ) -> Option<&str> {
        self.manifests
            .function_key_for_canonical_implementation_callable(callable)
    }

    /// Returns the admitted synthetic callback function anchored at one exact
    /// ordinary executable and producer-owned site ordinal.
    pub fn function_key_for_synthetic_callback(
        &self,
        owner: &PackageExecutableCoordinate,
        site_ordinal: u32,
    ) -> Option<&str> {
        self.manifests
            .function_key_for_synthetic_callback(owner, site_ordinal)
    }

    /// Returns the canonical package-owned callable identity for one exact
    /// synthetic callback site.
    pub fn synthetic_callback_callable(
        &self,
        owner: &PackageExecutableCoordinate,
        site_ordinal: u32,
    ) -> Option<&PackageCallableId> {
        self.manifests
            .synthetic_callback_callable(owner, site_ordinal)
    }

    /// Resolves a canonical synthetic callback callable identity back to its
    /// admitted function key.
    pub fn function_key_for_synthetic_callback_callable(
        &self,
        callable: &PackageCallableId,
    ) -> Option<&str> {
        self.manifests
            .function_key_for_synthetic_callback_callable(callable)
    }

    /// Returns the canonical effect-summary owner for either an ordinary or
    /// synthetic admitted function. Public aliases are never returned.
    pub fn canonical_effect_callable_for_function_key(
        &self,
        function_key: &str,
    ) -> Option<&PackageCallableId> {
        self.manifests
            .canonical_effect_callable_for_function_key(function_key)
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
    platform_error_projection_registry: PlatformErrorProjectionRegistryRef,
}

impl HydratedDeploymentBytecode {
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
                expected: Box::new(DeploymentBytecodeReference::ServiceDeployment(reference)),
                actual: Box::new(DeploymentBytecodeReference::ServiceDeployment(
                    actual_reference,
                )),
            });
        }
        validate_contract_store(&contract_store)?;
        let packages = canonical_packages(packages)?;
        let platform_error_projection_registry =
            join_deployment_platform_error_projection_registry(&deployment, &packages)?;
        let service_dependencies =
            canonical_service_dependencies(&deployment, service_dependencies)?;
        validate_required_contracts(&deployment, &contract_store, &service_dependencies)?;
        validate_deployment_manifests(
            &deployment,
            &contract_store,
            &service_dependencies,
            &packages,
        )?;
        Ok(Self {
            reference,
            deployment,
            contract_store,
            service_dependencies,
            packages,
            platform_error_projection_registry,
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

    pub fn platform_error_projection_registry(&self) -> &PlatformErrorProjectionRegistryRef {
        &self.platform_error_projection_registry
    }
}

/// Exact consumer deployment bytecode loader.
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

    /// Hydrates one exact consumer deployment without resolving provider
    /// deployments or release pointers.
    pub fn load(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> Result<HydratedDeploymentBytecode, DeploymentBytecodeHydrationError> {
        let deployment = self
            .resolver
            .resolve_deployment(reference)
            .map_err(
                |error| DeploymentBytecodeHydrationError::ContentResolution {
                    reference: Box::new(DeploymentBytecodeReference::ServiceDeployment(
                        reference.clone(),
                    )),
                    message: error.to_string(),
                },
            )?;
        let actual_reference = exact_deployment_reference(&deployment);
        if reference != &actual_reference {
            return Err(DeploymentBytecodeHydrationError::ReferenceMismatch {
                expected: Box::new(DeploymentBytecodeReference::ServiceDeployment(
                    reference.clone(),
                )),
                actual: Box::new(DeploymentBytecodeReference::ServiceDeployment(
                    actual_reference,
                )),
            });
        }
        validate_deployment_binding_uniqueness(&deployment)?;
        skiff_artifact_identity::validate_service_deployment_ref(reference, &deployment).map_err(
            |error| DeploymentBytecodeHydrationError::InvalidContent {
                reference: Box::new(DeploymentBytecodeReference::ServiceDeployment(
                    reference.clone(),
                )),
                message: error.to_string(),
            },
        )?;

        let own_contract = self.resolve_contract(None, &deployment.contract)?;
        validate_deployment_operation_coverage(&deployment, &own_contract)?;
        let packages = self.load_package_closure(&deployment)?;
        let mut contract_store = BTreeMap::from([(deployment.contract.clone(), own_contract)]);
        let service_dependencies =
            self.load_service_dependencies(&deployment, &packages, &mut contract_store)?;

        HydratedDeploymentBytecode::checked(
            reference.clone(),
            deployment,
            contract_store,
            service_dependencies,
            packages,
        )
    }

    fn load_package_closure(
        &self,
        deployment: &ServiceDeployment,
    ) -> Result<Vec<HydratedBytecodePackage>, DeploymentBytecodeHydrationError> {
        let bindings = deployment
            .package_bindings
            .iter()
            .map(|binding| (binding.key.clone(), binding.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut used_bindings = BTreeSet::new();
        let mut hydrated_by_build = BTreeMap::<PackageBuildId, PackageArtifactRef>::new();
        let mut builds_by_package_id = BTreeMap::<String, PackageBuildId>::new();
        let mut pending = vec![deployment.implementation.clone()];
        let mut packages = Vec::new();

        while let Some(package_reference) = pending.pop() {
            match hydrated_by_build.entry(package_reference.package_build_id.clone()) {
                Entry::Occupied(entry) if entry.get() != &package_reference => {
                    return Err(DeploymentBytecodeHydrationError::DuplicatePackage {
                        package_build_id: package_reference.package_build_id.clone(),
                        first: Box::new(entry.get().clone()),
                        duplicate: Box::new(package_reference),
                    });
                }
                Entry::Occupied(_) => continue,
                Entry::Vacant(entry) => {
                    entry.insert(package_reference.clone());
                }
            }
            if let Some(first_build_id) = builds_by_package_id.insert(
                package_reference.package_id.clone(),
                package_reference.package_build_id.clone(),
            ) {
                if first_build_id != package_reference.package_build_id {
                    return Err(DeploymentBytecodeHydrationError::ConflictingPackageOwner {
                        package_id: package_reference.package_id,
                        first_build_id,
                        second_build_id: package_reference.package_build_id,
                    });
                }
            }

            let artifact = self.resolve_package(&package_reference)?;
            validate_package_service_slot_uniqueness(&package_reference, &artifact)?;
            let requirements = artifact.package_requirements.clone();
            let bytecode_reference = artifact.bytecode.clone().ok_or_else(|| {
                DeploymentBytecodeHydrationError::MissingBytecode {
                    package: Box::new(package_reference.clone()),
                }
            })?;
            let bytecode = self
                .resolver
                .resolve_package_bytecode(&package_reference, &bytecode_reference)
                .map_err(
                    |error| DeploymentBytecodeHydrationError::ContentResolution {
                        reference: Box::new(DeploymentBytecodeReference::PackageBytecode {
                            package: package_reference.clone(),
                            bytecode: bytecode_reference,
                        }),
                        message: error.to_string(),
                    },
                )?;
            let hydrated =
                HydratedBytecodePackage::checked(package_reference.clone(), artifact, bytecode)?;

            for requirement in requirements {
                let key = PackageRequirementKey {
                    caller_package_build_id: package_reference.package_build_id.clone(),
                    package_requirement_alias: requirement.alias.clone(),
                };
                let binding = bindings.get(&key).ok_or_else(|| {
                    DeploymentBytecodeHydrationError::MissingPackageBinding { key: key.clone() }
                })?;
                validate_package_requirement(&key, &requirement, binding)?;
                used_bindings.insert(key);
                pending.push(binding.package.clone());
            }
            packages.push(hydrated);
        }

        if let Some(key) = bindings.keys().find(|key| !used_bindings.contains(*key)) {
            return Err(DeploymentBytecodeHydrationError::UnexpectedPackageBinding {
                key: key.clone(),
            });
        }
        Ok(packages)
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> Result<Arc<PackageArtifact>, DeploymentBytecodeHydrationError> {
        let artifact = self.resolver.resolve_package(reference).map_err(|error| {
            DeploymentBytecodeHydrationError::ContentResolution {
                reference: Box::new(DeploymentBytecodeReference::Package(reference.clone())),
                message: error.to_string(),
            }
        })?;
        let actual_reference = exact_package_reference(&artifact);
        if reference != &actual_reference {
            return Err(DeploymentBytecodeHydrationError::ReferenceMismatch {
                expected: Box::new(DeploymentBytecodeReference::Package(reference.clone())),
                actual: Box::new(DeploymentBytecodeReference::Package(actual_reference)),
            });
        }
        skiff_artifact_identity::validate_package_artifact_identities(&artifact).map_err(
            |error| DeploymentBytecodeHydrationError::InvalidContent {
                reference: Box::new(DeploymentBytecodeReference::Package(reference.clone())),
                message: error.to_string(),
            },
        )?;
        Ok(artifact)
    }

    fn load_service_dependencies(
        &self,
        deployment: &ServiceDeployment,
        packages: &[HydratedBytecodePackage],
        contracts: &mut BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    ) -> Result<Vec<HydratedServiceDependency>, DeploymentBytecodeHydrationError> {
        let selectors = deployment
            .service_selectors
            .iter()
            .map(|selector| (selector.key.clone(), selector.contract.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut dependencies = Vec::new();
        for package in packages {
            for requirement in &package.artifact.service_requirements {
                let key = ServiceRequirementKey {
                    caller_package_build_id: package.reference.package_build_id.clone(),
                    service_requirement_slot: requirement.service_binding_slot,
                };
                let expected_contract = ServiceContractRef {
                    service_id: requirement.contract_requirement.service_id.clone(),
                    contract_version: requirement.contract_requirement.contract_version.clone(),
                    service_protocol_identity: requirement
                        .contract_requirement
                        .expected_protocol_identity
                        .clone(),
                };
                let selected_contract = selectors.get(&key).ok_or_else(|| {
                    DeploymentBytecodeHydrationError::ContractMismatch {
                        key: Some(key.clone()),
                        expected: Some(Box::new(expected_contract.clone())),
                        actual: None,
                    }
                })?;
                if selected_contract != &expected_contract {
                    return Err(DeploymentBytecodeHydrationError::ContractMismatch {
                        key: Some(key),
                        expected: Some(Box::new(expected_contract)),
                        actual: Some(Box::new(selected_contract.clone())),
                    });
                }
                let contract = if let Some(contract) = contracts.get(selected_contract) {
                    Arc::clone(contract)
                } else {
                    let contract = self.resolve_contract(Some(&key), selected_contract)?;
                    contracts.insert(selected_contract.clone(), Arc::clone(&contract));
                    contract
                };
                for operation in &requirement.used_operations {
                    if !contract.operations.contains_key(operation) {
                        return Err(DeploymentBytecodeHydrationError::MissingOperation {
                            key: key.clone(),
                            contract: Box::new(selected_contract.clone()),
                            operation: operation.clone(),
                        });
                    }
                }
                dependencies.push(HydratedServiceDependency::new(
                    key,
                    selected_contract.clone(),
                    requirement.used_operations.clone(),
                ));
            }
        }
        Ok(dependencies)
    }

    fn resolve_contract(
        &self,
        key: Option<&ServiceRequirementKey>,
        reference: &ServiceContractRef,
    ) -> Result<Arc<ServiceContract>, DeploymentBytecodeHydrationError> {
        let contract = self.resolver.resolve_contract(reference).map_err(|error| {
            DeploymentBytecodeHydrationError::ContentResolution {
                reference: Box::new(DeploymentBytecodeReference::ServiceContract(
                    reference.clone(),
                )),
                message: error.to_string(),
            }
        })?;
        let actual_reference = exact_contract_reference(&contract);
        if reference != &actual_reference {
            return Err(DeploymentBytecodeHydrationError::ContractMismatch {
                key: key.cloned(),
                expected: Some(Box::new(reference.clone())),
                actual: Some(Box::new(actual_reference)),
            });
        }
        skiff_artifact_identity::validate_service_contract_identities(&contract).map_err(
            |error| DeploymentBytecodeHydrationError::InvalidContent {
                reference: Box::new(DeploymentBytecodeReference::ServiceContract(
                    reference.clone(),
                )),
                message: error.to_string(),
            },
        )?;
        Ok(contract)
    }
}

fn validate_deployment_binding_uniqueness(
    deployment: &ServiceDeployment,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let mut package_bindings = BTreeSet::new();
    for binding in &deployment.package_bindings {
        if !package_bindings.insert(binding.key.clone()) {
            return Err(DeploymentBytecodeHydrationError::DuplicatePackageBinding {
                key: binding.key.clone(),
            });
        }
    }
    let mut service_slots = BTreeSet::new();
    for selector in &deployment.service_selectors {
        if !service_slots.insert(selector.key.clone()) {
            return Err(DeploymentBytecodeHydrationError::DuplicateServiceSlot {
                key: selector.key.clone(),
            });
        }
    }
    Ok(())
}

fn validate_deployment_operation_coverage(
    deployment: &ServiceDeployment,
    contract: &ServiceContract,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let expected = contract.operations.keys().cloned().collect::<BTreeSet<_>>();
    let actual = deployment
        .operation_bindings
        .iter()
        .map(|binding| binding.contract_operation_id.clone())
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(
            DeploymentBytecodeHydrationError::OperationCoverageMismatch {
                contract: Box::new(deployment.contract.clone()),
                expected,
                actual,
            },
        );
    }
    Ok(())
}

fn validate_package_service_slot_uniqueness(
    reference: &PackageArtifactRef,
    artifact: &PackageArtifact,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let mut slots = BTreeSet::new();
    for requirement in &artifact.service_requirements {
        if !slots.insert(requirement.service_binding_slot) {
            return Err(DeploymentBytecodeHydrationError::DuplicateServiceSlot {
                key: ServiceRequirementKey {
                    caller_package_build_id: reference.package_build_id.clone(),
                    service_requirement_slot: requirement.service_binding_slot,
                },
            });
        }
    }
    Ok(())
}

fn validate_package_requirement(
    key: &PackageRequirementKey,
    requirement: &PackageRequirement,
    binding: &PackageBinding,
) -> Result<(), DeploymentBytecodeHydrationError> {
    if binding.package.package_id != requirement.package_id
        || binding.package.package_version != requirement.exact_version
        || binding.package.package_local_abi_identity != requirement.expected_local_abi
        || requirement
            .expected_package_build
            .as_ref()
            .is_some_and(|expected| expected != &binding.package.package_build_id)
    {
        return Err(
            DeploymentBytecodeHydrationError::PackageRequirementMismatch {
                key: key.clone(),
                requirement: Box::new(requirement.clone()),
                selected: Box::new(binding.package.clone()),
            },
        );
    }
    Ok(())
}

fn join_package_platform_error_projection_registry(
    reference: &PackageArtifactRef,
    artifact: &PackageArtifact,
    bytecode: &ValidatedBytecodeArtifact,
) -> Result<PlatformErrorProjectionRegistryRef, DeploymentBytecodeHydrationError> {
    let package_artifact = &artifact.platform_error_projection_registry;
    let bytecode_header = &bytecode.artifact().platform_error_projection_registry;
    let structurally_validated_view = bytecode.view().platform_error_projection_registry();
    let runtime = current_platform_error_projection_registry_ref();
    if package_artifact != bytecode_header
        || package_artifact != structurally_validated_view
        || package_artifact != runtime
        || bytecode_header != structurally_validated_view
        || bytecode_header != runtime
        || structurally_validated_view != runtime
    {
        return Err(
            DeploymentBytecodeHydrationError::PlatformErrorProjectionRegistryMismatch {
                package: Box::new(reference.clone()),
                package_artifact: Box::new(package_artifact.clone()),
                bytecode_header: Box::new(bytecode_header.clone()),
                structurally_validated_view: Box::new(structurally_validated_view.clone()),
                runtime: Box::new(runtime.clone()),
            },
        );
    }
    Ok(package_artifact.clone())
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
                expected: Some(Box::new(expected.clone())),
                actual: Some(Box::new(actual)),
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
                    first: Box::new(entry.get().reference.clone()),
                    duplicate: Box::new(package.reference),
                });
            }
        }
    }
    Ok(canonical)
}

fn join_deployment_platform_error_projection_registry(
    deployment: &ServiceDeployment,
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<PlatformErrorProjectionRegistryRef, DeploymentBytecodeHydrationError> {
    let implementation = packages
        .get(&deployment.implementation.package_build_id)
        .filter(|package| package.reference() == &deployment.implementation)
        .ok_or_else(|| DeploymentBytecodeHydrationError::ManifestMismatch {
            package: Box::new(deployment.implementation.clone()),
            kind: DeploymentBytecodeManifestKind::PackageReference,
            detail: "implementation package is absent from the exact hydrated closure".to_string(),
        })?;
    let implementation_registry = implementation.platform_error_projection_registry().clone();
    for package in packages.values() {
        let package_registry = package.platform_error_projection_registry();
        if package_registry != &implementation_registry {
            return Err(
                DeploymentBytecodeHydrationError::MixedPlatformErrorProjectionRegistry {
                    implementation: Box::new(implementation.reference().clone()),
                    implementation_registry: Box::new(implementation_registry.clone()),
                    package: Box::new(package.reference().clone()),
                    package_registry: Box::new(package_registry.clone()),
                },
            );
        }
    }
    Ok(implementation_registry)
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
                            expected: Some(Box::new(expected.clone())),
                            actual: Some(Box::new(dependency.contract)),
                        });
                    }
                    None => {
                        return Err(DeploymentBytecodeHydrationError::ContractMismatch {
                            key: Some(key),
                            expected: None,
                            actual: Some(Box::new(dependency.contract)),
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
            expected: Some(Box::new((*contract).clone())),
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
            expected: Some(Box::new(deployment.contract.clone())),
            actual: None,
        });
    }
    for dependency in dependencies.values() {
        if !contracts.contains_key(&dependency.contract) {
            return Err(DeploymentBytecodeHydrationError::ContractMismatch {
                key: Some(dependency.key.clone()),
                expected: Some(Box::new(dependency.contract.clone())),
                actual: None,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
