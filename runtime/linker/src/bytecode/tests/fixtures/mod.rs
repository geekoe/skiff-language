mod artifact;
mod package;
mod records;

use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    BytecodeArtifact, BytecodeArtifactRef, CallableEffectSummary, CallableMayEffects,
    CallableProvenanceSummary, CallableSemanticFacts, ContractOperationId, PackageArtifact,
    PackageArtifactRef, PackageCallableId, ServiceContract, ServiceContractRef, ServiceDeployment,
    ServiceDeploymentRef,
};
use skiff_runtime_loader::{
    DeploymentBytecodeContentResolver, DeploymentBytecodeLoader, HydratedDeploymentBytecode,
};

use artifact::bytecode_artifact;

pub(super) const ROOT_CALLABLE: &str = "pkg-callable:example.bytecode-link:top-level:fixture.root";
pub(super) const HELPER_CALLABLE: &str =
    "pkg-callable:example.bytecode-link:top-level:fixture.helper";
pub(super) const ENTRY_ALIAS: &str = "pkg-callable:example.bytecode-link:fixture.public_root";
pub(super) const ROOT_FUNCTION: &str = "fixture::root";
pub(super) const HELPER_FUNCTION: &str = "fixture::helper";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RootProgram {
    LocalCall,
    ServiceDependency,
    Interface,
    Host,
    Intrinsic,
    FromType,
}

impl RootProgram {
    pub(super) const fn root_has_parameter(self) -> bool {
        matches!(self, Self::Interface)
    }
}

pub(super) struct Fixture {
    resolver: InMemoryResolver,
    deployment_reference: ServiceDeploymentRef,
    pub(super) package_reference: PackageArtifactRef,
    pub(super) bytecode_reference: BytecodeArtifactRef,
    pub(super) operation: ContractOperationId,
}

impl Fixture {
    pub(super) fn exact_local() -> Self {
        Self::new(RootProgram::LocalCall, false)
    }

    pub(super) fn aliased_entry() -> Self {
        Self::new(RootProgram::LocalCall, true)
    }

    pub(super) fn interface() -> Self {
        Self::new(RootProgram::Interface, false)
    }

    pub(super) fn host() -> Self {
        Self::new(RootProgram::Host, false)
    }

    pub(super) fn intrinsic() -> Self {
        Self::new(RootProgram::Intrinsic, false)
    }

    pub(super) fn from_type() -> Self {
        Self::new(RootProgram::FromType, false)
    }

    pub(super) fn service_dependency() -> Self {
        Self::new(RootProgram::ServiceDependency, false)
    }

    pub(super) fn hydrate(&self) -> HydratedDeploymentBytecode {
        DeploymentBytecodeLoader::new(&self.resolver)
            .load(&self.deployment_reference)
            .unwrap()
    }

    fn new(program: RootProgram, entry_alias: bool) -> Self {
        let bytecode = artifact::admitted_bytecode(program);
        let alias = entry_alias.then(|| PackageCallableId::new(ENTRY_ALIAS));
        let mut package = package::package(&bytecode, program, alias.as_ref());

        let mut contracts = BTreeMap::new();
        let (own_contract, own_contract_reference, operation) = records::contract(
            "example.bytecode-link-service",
            "run",
            records::contract_has_parameter(program),
        );
        contracts.insert(own_contract_reference.clone(), own_contract);

        let service_selector = if program == RootProgram::ServiceDependency {
            let (provider, provider_reference, provider_operation) =
                records::contract("example.bytecode-link-provider", "call", false);
            records::add_service_requirement(
                &mut package,
                &provider_reference,
                &provider_operation,
            );
            let package_reference = records::package_reference(&package);
            contracts.insert(provider_reference.clone(), provider);
            Some(records::service_selector(
                &package_reference,
                provider_reference,
            ))
        } else {
            None
        };

        let package_reference = records::package_reference(&package);
        let entry_callable = alias.unwrap_or_else(|| PackageCallableId::new(ROOT_CALLABLE));
        let (deployment, deployment_reference) = records::deployment(
            package_reference.clone(),
            own_contract_reference,
            operation.clone(),
            entry_callable,
            service_selector,
        );
        let bytecode_reference = bytecode.reference().clone();
        let resolver = InMemoryResolver {
            deployment_reference: deployment_reference.clone(),
            deployment,
            contracts,
            packages: BTreeMap::from([(package_reference.clone(), Arc::new(package))]),
            bytecodes: BTreeMap::from([(
                (
                    package_reference.clone(),
                    bytecode_reference.bytecode_identity.clone(),
                ),
                bytecode,
            )]),
        };
        Self {
            resolver,
            deployment_reference,
            package_reference,
            bytecode_reference,
            operation,
        }
    }
}

struct InMemoryResolver {
    deployment_reference: ServiceDeploymentRef,
    deployment: Arc<ServiceDeployment>,
    contracts: BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    packages: BTreeMap<PackageArtifactRef, Arc<PackageArtifact>>,
    bytecodes: BTreeMap<(PackageArtifactRef, String), Arc<ValidatedBytecodeArtifact>>,
}

impl DeploymentBytecodeContentResolver for InMemoryResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        if reference != &self.deployment_reference {
            anyhow::bail!("unexpected deployment reference")
        }
        Ok(Arc::clone(&self.deployment))
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        self.contracts
            .get(reference)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing contract {reference:?}"))
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        self.packages
            .get(reference)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing package {reference:?}"))
    }

    fn resolve_package_bytecode(
        &self,
        package: &PackageArtifactRef,
        reference: &BytecodeArtifactRef,
    ) -> anyhow::Result<Arc<ValidatedBytecodeArtifact>> {
        self.bytecodes
            .get(&(package.clone(), reference.bytecode_identity.clone()))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing bytecode for {package:?}"))
    }
}

pub(super) fn corrupt_relocation_artifact() -> BytecodeArtifact {
    let mut artifact = bytecode_artifact(RootProgram::LocalCall);
    artifact
        .image
        .functions
        .get_mut(ROOT_FUNCTION)
        .unwrap()
        .relocations[0] = skiff_artifact_model::BytecodeRelocation::HostEffectRef(
        skiff_artifact_model::HostEffectReference {
            target: skiff_artifact_model::NativeTarget {
                namespace: "fixture".to_string(),
                symbol: "drift".to_string(),
                binding_key: Some("fixture.drift".to_string()),
                metadata: BTreeMap::new(),
            },
            signature: skiff_artifact_model::HostEffectSignature {
                parameter_types: Vec::new(),
                parameter_modes: Vec::new(),
                parameter_plans: Vec::new(),
                result_types: Vec::new(),
                result_plans: Vec::new(),
                effects: no_effects(),
            },
        },
    );
    artifact
}

pub(super) fn corrupt_relocation_index_artifact() -> BytecodeArtifact {
    let mut artifact = bytecode_artifact(RootProgram::LocalCall);
    artifact
        .image
        .functions
        .get_mut(ROOT_FUNCTION)
        .unwrap()
        .words[1] = 1;
    artifact
}

fn analyzed_facts() -> CallableSemanticFacts {
    CallableSemanticFacts {
        effects: CallableEffectSummary::Analyzed {
            effects: no_effects(),
        },
        provenance: CallableProvenanceSummary::Analyzed {
            return_origins: Vec::new(),
            direct_return_origins: Vec::new(),
            throw_origins: Vec::new(),
            escape_lanes: Vec::new(),
        },
        resolved_call_targets: BTreeMap::new(),
    }
}

fn no_effects() -> CallableMayEffects {
    CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_pending: false,
        pending_effect_categories: Vec::new(),
        inout_path_effects: Vec::new(),
    }
}
