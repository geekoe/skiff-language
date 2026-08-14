mod artifact;
mod constants;
mod package;
mod records;

use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_identity::{
    assign_service_deployment_identity, gateway_entry_identity, service_deployment_ref,
    ValidatedBytecodeArtifact,
};
use skiff_artifact_model::{
    BoundaryOperationDescriptor, BytecodeArtifact, BytecodeArtifactRef, BytecodePoolEntry,
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary, CallableSemanticFacts,
    ContractOperationId, ContractTypeDescriptor, DeploymentGatewayEntry, DeploymentIngressBinding,
    DeploymentOperationBinding, GatewayAdapterArg, GatewayAdapterKind, GatewayAdapterPlan,
    GatewayAdapterSource, GatewayDispatchMode, GatewayEntryKey, GatewayEntryProtocolSurface,
    GatewayExternalErrorProjection, GatewayExternalSchema, GatewayHttpProtocolSurface,
    GatewayProtocolSurface, IngressProtocol, IngressSelector, PackageArtifact, PackageArtifactRef,
    PackageBinding, PackageBuildId, PackageCallableId, PackageRequirement, PackageRequirementKey,
    PackageSchemaCanonicalDescriptor, PackageSchemaTypeRecord, ServiceContract, ServiceContractRef,
    ServiceDeployment, ServiceDeploymentRef, TypeRefIr,
};
use skiff_runtime_loader::{
    DeploymentBytecodeContentResolver, DeploymentBytecodeHydrationError, DeploymentBytecodeLoader,
    HydratedDeploymentBytecode,
};

use artifact::bytecode_artifact;
pub(super) use constants::ConstantProgram;

pub(super) const ROOT_CALLABLE: &str = "pkg-callable:example.bytecode-link:top-level:fixture.root";
pub(super) const HELPER_CALLABLE: &str =
    "pkg-callable:example.bytecode-link:top-level:fixture.helper";
pub(super) const ENTRY_ALIAS: &str = "pkg-callable:example.bytecode-link:fixture.public_root";
pub(super) const ROOT_FUNCTION: &str = "fixture::root";
pub(super) const HELPER_FUNCTION: &str = "fixture::helper";
pub(super) const CALLBACK_FUNCTION: &str = "fixture::root$callback0";
pub(super) const OWNER_IMPLEMENTATION_PATH: &str = "fixture.Owner";
pub(super) const OWNER_PUBLIC_PATH: &str = "Owner";
pub(super) const PRIVATE_IMPLEMENTATION_PATH: &str = "fixture.Private";
pub(super) const DEPENDENCY_ALIAS: &str = "dependency";
pub(super) const DEPENDENCY_PACKAGE_ID: &str = "example.bytecode-link-dependency";
pub(super) const SCHEMA_STABLE_KEY: &str = "fixture.Schema";

#[derive(Debug, Clone, Copy)]
pub(super) enum DependencyTypeSurfaceConflict {
    Descriptor,
    TypeParameters,
    InterfaceFlag,
}

#[derive(Debug, Clone, Copy)]
enum DependencyBuildPin {
    Exact,
    Unpinned,
    Wrong,
}

#[derive(Debug, Clone, Copy)]
struct NormalizationDependency {
    pin: DependencyBuildPin,
    conflict: Option<DependencyTypeSurfaceConflict>,
    constant: Option<ConstantProgram>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RootProgram {
    LocalCall,
    SyntheticTarget,
    UnreachableCallback,
    ServiceDependency,
    Interface,
    UnreachableInterface,
    Host,
    Intrinsic,
    FromType,
    RecordShape,
    ArraysMaps,
    StreamNext,
    StreamNextLoop,
    StreamProducer,
    ReorderedStreamNextResumePool,
    ReorderedStreamResumePool,
    Constant(ConstantProgram),
}

impl RootProgram {
    pub(super) const fn root_has_parameter(self) -> bool {
        matches!(
            self,
            Self::Interface
                | Self::RecordShape
                | Self::StreamNext
                | Self::StreamNextLoop
                | Self::StreamProducer
                | Self::ReorderedStreamNextResumePool
                | Self::ReorderedStreamResumePool
        )
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

    pub(super) fn synthetic_target() -> Self {
        Self::new(RootProgram::SyntheticTarget, false)
    }

    pub(super) fn interface() -> Self {
        Self::new(RootProgram::Interface, false)
    }

    pub(super) fn unreachable_interface() -> Self {
        Self::new(RootProgram::UnreachableInterface, false)
    }

    pub(super) fn unreachable_callback() -> Self {
        Self::new(RootProgram::UnreachableCallback, false)
    }

    pub(super) fn host() -> Self {
        Self::new(RootProgram::Host, false)
    }

    pub(super) fn intrinsic() -> Self {
        Self::new(RootProgram::Intrinsic, false)
    }

    pub(super) fn record_shape() -> Self {
        Self::new(RootProgram::RecordShape, false)
    }

    pub(super) fn arrays_maps() -> Self {
        Self::new(RootProgram::ArraysMaps, false)
    }

    pub(super) fn stream_next() -> Self {
        Self::new(RootProgram::StreamNext, false)
    }

    pub(super) fn stream_next_loop() -> Self {
        Self::new(RootProgram::StreamNextLoop, false)
    }

    pub(super) fn stream_producer() -> Self {
        Self::new(RootProgram::StreamProducer, false)
    }

    pub(super) fn reordered_stream_next_resume_pool() -> Self {
        Self::new(RootProgram::ReorderedStreamNextResumePool, false)
    }

    pub(super) fn reordered_stream_resume_pool() -> Self {
        Self::new(RootProgram::ReorderedStreamResumePool, false)
    }

    pub(super) fn from_type() -> Self {
        Self::new(RootProgram::FromType, false)
    }

    pub(super) fn service_dependency() -> Self {
        Self::new(RootProgram::ServiceDependency, false)
    }

    pub(super) fn gateway_server_stream() -> Self {
        Self::new_gateway(GatewayDispatchMode::ServerStream, false, false)
    }

    pub(super) fn gateway_guard() -> Self {
        Self::new_gateway(GatewayDispatchMode::Unary, true, false)
    }

    pub(super) fn gateway_pre() -> Self {
        Self::new_gateway(GatewayDispatchMode::Unary, false, true)
    }

    pub(super) fn raw_http_dense_parameter(present: bool) -> Self {
        let mut artifact = artifact::bytecode_artifact(RootProgram::RecordShape);
        let root = artifact.image.functions.get_mut(ROOT_FUNCTION).unwrap();
        root.words = vec![0x25];
        root.frame_layout.slot_type_refs = vec![1];
        let dense_plan = skiff_artifact_model::ValueTransferPlan::SnapshotShare {
            drop: skiff_artifact_model::ValueDropPlan::SnapshotRelease,
        };
        root.frame_layout.slot_plans[0] = dense_plan.clone();
        root.frame_layout.parameter_slots[0].plan = dense_plan.clone();
        root.frame_layout.parameter_slots[0].dense_record_shape_ref = present.then_some(0);
        root.frame_layout.result_count = 0;
        root.frame_layout.result_type_refs.clear();
        root.frame_layout.result_plans.clear();
        root.max_operand_depth = 0;
        root.statement_entries.clear();
        root.source_map.clear();
        let BytecodePoolEntry::ShapeRef { shape } = &mut artifact.image.pools.shapes[0] else {
            unreachable!()
        };
        shape.fields[0].plan = dense_plan;
        skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
        let bytecode = Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap());
        let package = package::package(&bytecode, RootProgram::RecordShape, None, false, false);
        let package_reference = records::package_reference(&package);
        let (contract, contract_reference, operation) =
            records::contract("example.bytecode-link-service", "run", true);
        let (deployment, _) = records::deployment(
            package_reference.clone(),
            contract_reference.clone(),
            operation.clone(),
            PackageCallableId::new(ROOT_CALLABLE),
            None,
            Vec::new(),
        );
        let mut deployment = deployment.as_ref().clone();
        let key = GatewayEntryKey::parse("raw").unwrap();
        let protocol_surface = GatewayEntryProtocolSurface {
            protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
                adapter_kind: GatewayAdapterKind::RawHttp,
                dispatch_mode: GatewayDispatchMode::Unary,
                external_sources: vec![GatewayAdapterSource::HttpRequest],
                request_body_schema: None,
                response_schema: None,
                stream_item_schema: None,
            }),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        };
        deployment.gateway_entries.insert(
            key.clone(),
            DeploymentGatewayEntry {
                gateway_entry_identity: gateway_entry_identity(&protocol_surface).unwrap(),
                protocol_surface,
                handler: Some(PackageCallableId::new(ROOT_CALLABLE)),
                pre: None,
                guard: None,
                adapter_plan: GatewayAdapterPlan {
                    kind: GatewayAdapterKind::RawHttp,
                    args: vec![GatewayAdapterArg {
                        param: "request".to_string(),
                        source: GatewayAdapterSource::HttpRequest,
                    }],
                },
                close_handler: None,
                close_adapter_plan: None,
            },
        );
        deployment.ingress.push(DeploymentIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::Http,
                method: Some("POST".to_string()),
                path: "/raw".to_string(),
            },
            gateway_entry_key: key,
        });
        assign_service_deployment_identity(&mut deployment).unwrap();
        let deployment = Arc::new(deployment);
        let deployment_reference = service_deployment_ref(&deployment);
        let bytecode_reference = bytecode.reference().clone();
        Self {
            resolver: InMemoryResolver {
                deployment_reference: deployment_reference.clone(),
                deployment,
                contracts: BTreeMap::from([(contract_reference, contract)]),
                packages: BTreeMap::from([(package_reference.clone(), Arc::new(package))]),
                bytecodes: BTreeMap::from([(
                    (
                        package_reference.clone(),
                        bytecode_reference.bytecode_identity.clone(),
                    ),
                    bytecode,
                )]),
            },
            deployment_reference,
            package_reference,
            bytecode_reference,
            operation,
        }
    }

    pub(super) fn normalization() -> Self {
        Self::normalization_with(DependencyBuildPin::Exact, None, false)
    }

    pub(super) fn unpinned_normalization() -> Self {
        Self::normalization_with(DependencyBuildPin::Unpinned, None, false)
    }

    pub(super) fn wrong_pinned_normalization() -> Self {
        Self::normalization_with(DependencyBuildPin::Wrong, None, false)
    }

    pub(super) fn conflicting_normalization() -> Self {
        Self::normalization_with(DependencyBuildPin::Exact, None, true)
    }

    pub(super) fn conflicting_dependency_normalization(
        conflict: DependencyTypeSurfaceConflict,
    ) -> Self {
        Self::normalization_with(DependencyBuildPin::Exact, Some(conflict), false)
    }

    pub(super) fn hydrate(&self) -> HydratedDeploymentBytecode {
        self.try_hydrate().unwrap()
    }

    pub(super) fn exact_two_operations() -> (Self, ContractOperationId) {
        let mut fixture = Self::exact_local();
        let original_contract_ref = fixture.resolver.deployment.contract.clone();
        let original_contract = fixture
            .resolver
            .contracts
            .get(&original_contract_ref)
            .unwrap();
        let mut contract = original_contract.as_ref().clone();
        let operation_b = skiff_artifact_identity::contract_operation_id(
            &contract.service_id,
            &contract.contract_version,
            "helper",
        )
        .unwrap();
        contract.operations.insert(
            operation_b.clone(),
            BoundaryOperationDescriptor {
                operation_id: operation_b.clone(),
                stable_key: "helper".to_string(),
                contract: records::operation_contract(false),
            },
        );
        contract
            .diagnostic_text
            .operations
            .insert(operation_b.clone(), "helper".to_string());
        skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
        let contract = Arc::new(contract);
        let contract_ref = skiff_artifact_identity::service_contract_ref(&contract).unwrap();

        let mut deployment = fixture.resolver.deployment.as_ref().clone();
        deployment.contract = contract_ref.clone();
        deployment
            .operation_bindings
            .push(DeploymentOperationBinding {
                contract_operation_id: operation_b.clone(),
                package_callable_id: PackageCallableId::new(HELPER_CALLABLE),
            });
        assign_service_deployment_identity(&mut deployment).unwrap();
        let deployment = Arc::new(deployment);
        let deployment_reference = service_deployment_ref(&deployment);

        fixture.resolver.contracts.remove(&original_contract_ref);
        fixture.resolver.contracts.insert(contract_ref, contract);
        fixture.resolver.deployment_reference = deployment_reference.clone();
        fixture.resolver.deployment = deployment;
        fixture.deployment_reference = deployment_reference;
        (fixture, operation_b)
    }

    pub(super) fn try_hydrate(
        &self,
    ) -> Result<HydratedDeploymentBytecode, DeploymentBytecodeHydrationError> {
        DeploymentBytecodeLoader::new(&self.resolver).load(&self.deployment_reference)
    }

    fn new(program: RootProgram, entry_alias: bool) -> Self {
        Self::new_with_options(program, entry_alias, false, None, false)
    }

    fn new_gateway(dispatch_mode: GatewayDispatchMode, guard: bool, pre: bool) -> Self {
        let mut fixture = Self::new(RootProgram::LocalCall, false);
        let mut deployment = fixture.resolver.deployment.as_ref().clone();
        let key = GatewayEntryKey::parse("phase-1").unwrap();
        let protocol_surface = GatewayEntryProtocolSurface {
            protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
                adapter_kind: GatewayAdapterKind::TypedJson,
                dispatch_mode,
                external_sources: vec![GatewayAdapterSource::HttpBody],
                request_body_schema: Some(GatewayExternalSchema::Number),
                response_schema: (dispatch_mode == GatewayDispatchMode::Unary)
                    .then_some(GatewayExternalSchema::Number),
                stream_item_schema: (dispatch_mode == GatewayDispatchMode::ServerStream)
                    .then_some(GatewayExternalSchema::Number),
            }),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        };
        let callable = PackageCallableId::new(ROOT_CALLABLE);
        deployment.gateway_entries.insert(
            key.clone(),
            DeploymentGatewayEntry {
                gateway_entry_identity: gateway_entry_identity(&protocol_surface).unwrap(),
                protocol_surface,
                handler: Some(callable.clone()),
                pre: pre.then_some(callable.clone()),
                guard: guard.then_some(callable),
                adapter_plan: GatewayAdapterPlan {
                    kind: GatewayAdapterKind::TypedJson,
                    args: vec![GatewayAdapterArg {
                        param: "carrier".to_string(),
                        source: GatewayAdapterSource::HttpBody,
                    }],
                },
                close_handler: None,
                close_adapter_plan: None,
            },
        );
        deployment.ingress.push(DeploymentIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::Http,
                method: Some("POST".to_string()),
                path: "/phase-1".to_string(),
            },
            gateway_entry_key: key,
        });
        assign_service_deployment_identity(&mut deployment).unwrap();
        let deployment = Arc::new(deployment);
        fixture.deployment_reference = service_deployment_ref(&deployment);
        fixture.resolver.deployment_reference = fixture.deployment_reference.clone();
        fixture.resolver.deployment = deployment;
        fixture
    }

    fn normalization_with(
        pin: DependencyBuildPin,
        conflict: Option<DependencyTypeSurfaceConflict>,
        conflicting_type_surfaces: bool,
    ) -> Self {
        Self::new_with_options(
            RootProgram::LocalCall,
            false,
            true,
            Some(NormalizationDependency {
                pin,
                conflict,
                constant: None,
            }),
            conflicting_type_surfaces,
        )
    }

    fn new_with_options(
        program: RootProgram,
        entry_alias: bool,
        include_normalization_surface: bool,
        normalization_dependency: Option<NormalizationDependency>,
        conflicting_type_surfaces: bool,
    ) -> Self {
        let bytecode = if include_normalization_surface {
            normalization_bytecode(program)
        } else {
            artifact::admitted_bytecode(program)
        };
        let alias = entry_alias.then(|| PackageCallableId::new(ENTRY_ALIAS));
        let mut package = package::package(
            &bytecode,
            program,
            alias.as_ref(),
            include_normalization_surface,
            conflicting_type_surfaces,
        );
        let dependency = normalization_dependency.map(|dependency| {
            let bytecode = dependency.constant.map_or_else(
                artifact::empty_admitted_bytecode,
                artifact::constant_only_admitted_bytecode,
            );
            let package = package::dependency_type_owner_package(&bytecode, dependency.conflict);
            let reference = records::package_reference(&package);
            (dependency.pin, reference, Arc::new(package), bytecode)
        });
        if let Some((pin, dependency_reference, _, _)) = &dependency {
            package.package_requirements = vec![PackageRequirement {
                alias: DEPENDENCY_ALIAS.to_string(),
                package_id: dependency_reference.package_id.clone(),
                exact_version: dependency_reference.package_version.clone(),
                expected_local_abi: dependency_reference.package_local_abi_identity.clone(),
                expected_package_build: match pin {
                    DependencyBuildPin::Exact => {
                        Some(dependency_reference.package_build_id.clone())
                    }
                    DependencyBuildPin::Unpinned => None,
                    DependencyBuildPin::Wrong => Some(PackageBuildId::new("build:wrong")),
                },
            }];
            skiff_artifact_identity::assign_package_artifact_identities(&mut package).unwrap();
        }

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
        let package_bindings = dependency
            .as_ref()
            .map(|(_, dependency_reference, _, _)| PackageBinding {
                key: PackageRequirementKey {
                    caller_package_build_id: package_reference.package_build_id.clone(),
                    package_requirement_alias: DEPENDENCY_ALIAS.to_string(),
                },
                package: dependency_reference.clone(),
            })
            .into_iter()
            .collect();
        let (deployment, deployment_reference) = records::deployment(
            package_reference.clone(),
            own_contract_reference,
            operation.clone(),
            entry_callable,
            service_selector,
            package_bindings,
        );
        let bytecode_reference = bytecode.reference().clone();
        let mut packages = BTreeMap::from([(package_reference.clone(), Arc::new(package))]);
        let mut bytecodes = BTreeMap::from([(
            (
                package_reference.clone(),
                bytecode_reference.bytecode_identity.clone(),
            ),
            bytecode,
        )]);
        if let Some((_, dependency_reference, dependency_package, dependency_bytecode)) = dependency
        {
            bytecodes.insert(
                (
                    dependency_reference.clone(),
                    dependency_bytecode.reference().bytecode_identity.clone(),
                ),
                dependency_bytecode,
            );
            packages.insert(dependency_reference, dependency_package);
        }
        let resolver = InMemoryResolver {
            deployment_reference: deployment_reference.clone(),
            deployment,
            contracts,
            packages,
            bytecodes,
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

pub(super) fn schema_record() -> PackageSchemaTypeRecord {
    let descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Enumeration {
            variants: vec!["ready".to_string()],
        },
    };
    let package_schema_type_id = skiff_artifact_model::derive_package_schema_type_id(
        "example.bytecode-link",
        SCHEMA_STABLE_KEY,
        &descriptor,
    )
    .unwrap();
    PackageSchemaTypeRecord {
        package_id: "example.bytecode-link".to_string(),
        stable_schema_key: SCHEMA_STABLE_KEY.to_string(),
        package_schema_type_id,
        canonical_descriptor: descriptor,
    }
}

pub(super) fn schema_type() -> TypeRefIr {
    let record = schema_record();
    TypeRefIr::PackageSchema {
        package_id: record.package_id,
        stable_schema_key: record.stable_schema_key,
        package_schema_type_id: record.package_schema_type_id,
    }
}

fn normalization_bytecode(program: RootProgram) -> Arc<ValidatedBytecodeArtifact> {
    let mut artifact = artifact::bytecode_artifact(program);
    artifact.image.pools.types.push(BytecodePoolEntry::TypeRef {
        ty: schema_type(),
        plan: skiff_artifact_model::ValueTransferPlan::SnapshotShare {
            drop: skiff_artifact_model::ValueDropPlan::SnapshotRelease,
        },
    });
    artifact.image.pools.types.push(BytecodePoolEntry::TypeRef {
        ty: TypeRefIr::PublicationType {
            module_path: "fixture".to_string(),
            type_index: 0,
        },
        plan: skiff_artifact_model::ValueTransferPlan::SnapshotShare {
            drop: skiff_artifact_model::ValueDropPlan::SnapshotRelease,
        },
    });
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
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
            db_operation: None,
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

pub(super) fn synthetic_callback_callable_for(owner: &str) -> PackageCallableId {
    skiff_artifact_model::derive_synthetic_callback_callable_id(
        "example.bytecode-link",
        &PackageCallableId::new(owner),
        0,
    )
    .unwrap()
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
