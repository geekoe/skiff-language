use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    self, BoundaryOperationContract, BytecodeIntrinsicRef, BytecodeRelocation,
    CallableEffectSummary, CallableMayEffects, CallableRegistryTypeExpression,
    ContractOperationId, ContractTypeRef, HostEffectRegistryEntry,
    InterfaceInstantiationRef, InterfaceMethodSlotSignatureIr, LiteralIr, PackageBuildId,
    PackageLocalAbiSymbol, PackageRefIr, PackageSchemaTypeId, PackageSymbolRef, ParamModeIr,
    PendingEffectCategory, ResolvedPackageValueType, ServiceRequirementKey, TypeRefIr, ValueLifecycleFactResolver, ValueLifecycleResolverError,
    ValueLifecyclePolicyBudget,
};
use skiff_runtime_linked_bytecode::{
    ArtifactFunctionKey, FunctionIndex, LinkedActorCreateTarget, LinkedActorImplementationRef,
    LinkedActorMethodTarget, LinkedCallableSignature, LinkedFrameLayout,
    LinkedHostEffectAdapterTarget, LinkedInterfaceInstantiation, LinkedInterfaceMethodAbiId,
    LinkedInterfaceRequirementMethod, LinkedInterfaceRequirementTable, LinkedInterfaceTable,
    LinkedInterfaceTableKind, LinkedLocalInterfaceMethod, LinkedLocalInterfaceTable,
    LinkedNativeCallableSignature, LinkedPublicInstanceKey, LinkedRemoteInterfaceMethod,
    LinkedRemoteInterfaceTable, LinkedServiceOperationTarget, LinkedSyntheticCallbackTarget,
    SpecializationKey, TypeIndex,
};
use skiff_runtime_loader::{HydratedBytecodePackage, HydratedDeploymentBytecode};

use crate::bytecode::{
    types::{normalize_type, TypeLinker},
    BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation,
};

use super::{closure::ReachableRelocation, unsatisfied, DeploymentLinker};

pub(in crate::bytecode) struct LinkedDispatchTables {
    pub(in crate::bytecode) service_operations: Vec<LinkedServiceOperationTarget>,
    pub(in crate::bytecode) actor_creates: Vec<LinkedActorCreateTarget>,
    pub(in crate::bytecode) actor_methods: Vec<LinkedActorMethodTarget>,
    pub(in crate::bytecode) interface_tables: Vec<LinkedInterfaceTable>,
    pub(in crate::bytecode) synthetic_callbacks: Vec<LinkedSyntheticCallbackTarget>,
    pub(in crate::bytecode) synthetic_callback_origins: BTreeMap<(PackageBuildId, String), skiff_runtime_linked_bytecode::SyntheticCallbackIndex>,
    pub(in crate::bytecode) host_effect_adapters: Vec<LinkedHostEffectAdapterTarget>,
    pub(in crate::bytecode) intrinsics: Vec<skiff_runtime_linked_bytecode::LinkedIntrinsicTarget>,
}

impl DeploymentLinker<'_> {
    pub(super) fn link_dispatch_tables(
        &self,
        reachable: &[ReachableRelocation],
        indices: &BTreeMap<SpecializationKey, FunctionIndex>,
        frames: &[LinkedFrameLayout],
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<LinkedDispatchTables, BytecodeLinkError> {
        let service_operations = self.link_service_operations(reachable, type_linker)?;
        let (actor_creates, actor_methods) = self.link_actor_targets(reachable, indices, frames, type_linker)?;
        let interface_tables = self.link_interface_tables(reachable, indices, frames, type_linker)?;
        let synthetic_callbacks = self.link_synthetic_callbacks(reachable, indices, frames, type_linker)?;
        let (host_effect_adapters, intrinsics) =
            self.link_host_and_intrinsics(reachable, indices, type_linker)?;

        let synthetic_callback_origins = synthetic_callbacks
            .iter()
            .enumerate()
            .map(|(index, target)| {
                let package_build_id = indices
                    .iter()
                    .find(|(_, function)| function.get() == target.function().get())
                    .map(|(key, _)| key.package_build_id().clone())
                    .ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::ConcreteTargetTables,
                            self.deployment_location(),
                            "synthetic callback function is absent from the closure".to_string(),
                        )
                    })?;
                Ok((
                    (package_build_id, target.artifact_function_key().as_str().to_string()),
                    skiff_runtime_linked_bytecode::SyntheticCallbackIndex::new(index as u32),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, BytecodeLinkError>>()?;

        let mut tables = LinkedDispatchTables {
            service_operations,
            actor_creates,
            actor_methods,
            interface_tables,
            synthetic_callbacks,
            synthetic_callback_origins,
            host_effect_adapters,
            intrinsics,
        };
        tables.sort_and_validate()?;
        Ok(tables)
    }

    fn link_service_operations(
        &self,
        reachable: &[ReachableRelocation],
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<Vec<LinkedServiceOperationTarget>, BytecodeLinkError> {
        let mut rows = Vec::new();
        for (key, dependency) in self.deployment.service_dependencies() {
            let has_reachable_operation = dependency.used_operations().iter().any(|operation| {
                reachable.iter().any(|reference| matches!(
                    &reference.relocation,
                    BytecodeRelocation::ServiceOperationRef { service_call }
                        if reference.specialization.package_build_id() == &key.caller_package_build_id
                            && service_call.service_requirement_slot == key.service_requirement_slot
                            && &service_call.contract_operation_id == operation
                ))
            });
            if !has_reachable_operation {
                continue;
            }
            let contract = self
                .deployment
                .contract_store()
                .get(dependency.contract())
                .ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::ConcreteTargetTables,
                        BytecodeLinkLocation::ServiceDependency { key: key.clone() },
                        "service dependency contract is absent from hydration".to_string(),
                    )
                })?;
            for operation in dependency.used_operations() {
                let referenced = reachable.iter().any(|reference| matches!(
                    &reference.relocation,
                    BytecodeRelocation::ServiceOperationRef { service_call }
                        if reference.specialization.package_build_id() == &key.caller_package_build_id
                            && service_call.service_requirement_slot == key.service_requirement_slot
                            && &service_call.contract_operation_id == operation
                ));
                if !referenced {
                    continue;
                }
                let descriptor = contract.operations.get(operation).ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::ConcreteTargetTables,
                        BytecodeLinkLocation::ServiceDependency { key: key.clone() },
                        format!("service dependency operation {operation} is absent from its contract"),
                    )
                })?;
                let caller = self
                    .deployment
                    .packages()
                    .get(&key.caller_package_build_id)
                    .ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::ConcreteTargetTables,
                            BytecodeLinkLocation::ServiceDependency { key: key.clone() },
                            "service dependency caller package is absent".to_string(),
                        )
                    })?;
                let signature = boundary_signature(
                    &descriptor.contract,
                    caller,
                    type_linker,
                    BytecodeLinkLocation::ServiceDependency { key: key.clone() },
                )?;
                rows.push(LinkedServiceOperationTarget::new(
                    skiff_runtime_linked_bytecode::ServiceOperationIndex::new(rows.len() as u32),
                    key.clone(),
                    operation.clone(),
                    contract.service_protocol_identity.clone(),
                    signature,
                ));
            }
        }
        Ok(rows)
    }

    fn link_actor_targets(
        &self,
        reachable: &[ReachableRelocation],
        indices: &BTreeMap<SpecializationKey, FunctionIndex>,
        frames: &[LinkedFrameLayout],
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<(Vec<LinkedActorCreateTarget>, Vec<LinkedActorMethodTarget>), BytecodeLinkError> {
        let creates = Vec::new();
        let mut methods = Vec::new();
        for package in self.deployment.packages().values() {
            for actor in &package.artifact().actor_implementations {
                let has_reachable_method = reachable.iter().any(|reference| matches!(
                    &reference.relocation,
                    BytecodeRelocation::ActorMethodRef {
                        actor: target_actor,
                        actor_implementation_identity,
                        ..
                    } if reference.specialization.package_build_id()
                        == &package.reference().package_build_id
                        && target_actor == &actor.actor
                        && actor_implementation_identity == &actor.actor_implementation_identity
                ));
                if !has_reachable_method {
                    continue;
                }
                let actor_abi = package
                    .artifact()
                    .implementation_links
                    .types
                    .values()
                    .find(|export| {
                        export.file.module_path == actor.actor.module_path
                            && export.symbol == actor.actor.symbol
                    })
                    .and_then(|export| export.actor.as_ref())
                    .ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::ConcreteTargetTables,
                            self.package_location(package),
                            format!(
                                "actor {} has no package ABI authority",
                                actor.actor.symbol_path()
                            ),
                        )
                    })?;
                let actor_ref = LinkedActorImplementationRef::new(
                    package.reference().package_build_id.clone(),
                    actor.actor.clone(),
                    actor_abi.actor_abi_identity.clone(),
                    actor.actor_implementation_identity.clone(),
                );
                for (method_identity, callable) in &actor.methods {
                    let referenced = reachable.iter().any(|reference| matches!(
                        &reference.relocation,
                        BytecodeRelocation::ActorMethodRef {
                            actor: target_actor,
                            actor_abi_identity,
                            actor_implementation_identity,
                            method_identity: target_method,
                        } if reference.specialization.package_build_id() == &package.reference().package_build_id
                            && target_actor == &actor.actor
                            && actor_abi_identity == &actor_abi.actor_abi_identity
                            && actor_implementation_identity == &actor.actor_implementation_identity
                            && target_method == method_identity
                    ));
                    if !referenced {
                        continue;
                    }
                    let key = self.key_for_receiver_callable(package, callable, type_linker)?;
                    let function = indices.get(&key).copied().ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::ConcreteTargetTables,
                            self.package_location(package),
                            format!("actor method {method_identity:?} is absent from the closure"),
                        )
                    })?;
                    let signature = frame_signature(
                        frames.get(function.get() as usize).ok_or_else(|| {
                            unsatisfied(
                                BytecodeLinkObligation::ConcreteTargetTables,
                                self.package_location(package),
                                "actor method frame is absent".to_string(),
                            )
                        })?,
                        package,
                        package.function_key_for_callable(callable),
                    )?;
                    methods.push(LinkedActorMethodTarget::new(
                        skiff_runtime_linked_bytecode::ActorMethodIndex::new(methods.len() as u32),
                        actor_ref.clone(),
                        method_identity.clone(),
                        function,
                        signature,
                    ));
                }
            }
        }
        Ok((creates, methods))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::bytecode) enum InterfaceKind {
    Requirement,
    Callback,
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq)]
enum InterfaceSource {
    Requirement,
    Local(skiff_artifact_model::LocalInterfaceRef),
    Remote(skiff_artifact_model::RemoteInterfaceRef),
}

#[derive(Debug, Clone, PartialEq)]
struct InterfaceKey {
    package_build_id: PackageBuildId,
    specialization: SpecializationKey,
    interface: InterfaceInstantiationRef,
    kind: InterfaceKind,
    concrete_type: Option<TypeIndex>,
    service_requirement_key: Option<ServiceRequirementKey>,
    public_instance_key: Option<String>,
    source: InterfaceSource,
}

fn boundary_signature(
    contract: &BoundaryOperationContract,
    caller: &HydratedBytecodePackage,
    type_linker: &mut TypeLinker<'_>,
    location: BytecodeLinkLocation,
) -> Result<LinkedCallableSignature, BytecodeLinkError> {
    let specialization = first_specialization(caller, &location)?;
    let parameter_types = contract
        .parameters
        .iter()
        .map(|parameter| {
            intern_contract_type(caller, &specialization, &parameter.ty, type_linker, &location)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let result_types = contract_return_types(&contract.return_value.ty)
        .into_iter()
        .map(|ty| intern_contract_type(caller, &specialization, &ty, type_linker, &location))
        .collect::<Result<Vec<_>, _>>()?;
    let parameter_plans = parameter_types
        .iter()
        .copied()
        .map(|ty| {
            let concrete = type_linker
                .linked_type_ref(ty)
                .cloned()
                .ok_or_else(|| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "service parameter type is absent".to_string()))?;
            type_linker.plan_for_concrete_type(&concrete, location.clone())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let result_plans = result_types
        .iter()
        .copied()
        .map(|ty| {
            let concrete = type_linker
                .linked_type_ref(ty)
                .cloned()
                .ok_or_else(|| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "service result type is absent".to_string()))?;
            type_linker.plan_for_concrete_type(&concrete, location.clone())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let parameter_mode_count = parameter_types.len();
    LinkedCallableSignature::new(
        parameter_types.into_boxed_slice(),
        vec![ParamModeIr::Value; parameter_mode_count].into_boxed_slice(),
        parameter_plans.into_boxed_slice(),
        result_types.into_boxed_slice(),
        result_plans.into_boxed_slice(),
        service_effect_summary(contract),
    )
    .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location, error.to_string()))
}

fn contract_return_types(ty: &ContractTypeRef) -> Vec<ContractTypeRef> {
    if matches!(ty, ContractTypeRef::Builtin { name, .. } if name == "void") {
        Vec::new()
    } else {
        vec![ty.clone()]
    }
}

fn service_effect_summary(contract: &BoundaryOperationContract) -> CallableEffectSummary {
    let guarantee = &contract.effect_guarantee;
    CallableEffectSummary::Analyzed {
        effects: CallableMayEffects {
            escapes_caller_value: !guarantee.detached_parameters
                || !guarantee.detached_return
                || !guarantee.detached_error,
            requires_same_heap_identity: !guarantee.no_same_heap_identity,
            invokes_unknown_target: false,
            may_pending: true,
            pending_effect_categories: vec![PendingEffectCategory::ServiceCall],
            inout_path_effects: Vec::new(),
        },
    }
}

fn intern_contract_type(
    caller: &HydratedBytecodePackage,
    specialization: &SpecializationKey,
    ty: &ContractTypeRef,
    type_linker: &mut TypeLinker<'_>,
    location: &BytecodeLinkLocation,
) -> Result<TypeIndex, BytecodeLinkError> {
    let ir = contract_type_to_type_ref(ty, location)?;
    type_linker.intern_concrete_type(caller, specialization, &ir, &BTreeMap::new(), location.clone())
}

fn contract_type_to_type_ref(
    ty: &ContractTypeRef,
    location: &BytecodeLinkLocation,
) -> Result<TypeRefIr, BytecodeLinkError> {
    Ok(match ty {
        ContractTypeRef::Builtin { name, arguments } => TypeRefIr::Builtin {
            name: name.clone(),
            args: arguments
                .iter()
                .map(|arg| contract_type_to_type_ref(arg, location))
                .collect::<Result<_, _>>()?,
        },
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => TypeRefIr::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.clone(),
        },
        ContractTypeRef::AnyInterface { interface, arguments } => TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: String::from_utf8(
                    skiff_canonical_json::canonical_json_bytes(&contract_type_to_type_ref(interface, location)?)
                        .map_err(|error| {
                            unsatisfied(
                                BytecodeLinkObligation::ConcreteTargetTables,
                                location.clone(),
                                format!("contract interface identity cannot be canonicalized: {error}"),
                            )
                        })?,
                )
                .map_err(|_| {
                    unsatisfied(
                        BytecodeLinkObligation::ConcreteTargetTables,
                        location.clone(),
                        "contract interface identity is not UTF-8".to_string(),
                    )
                })?,
                canonical_type_args: arguments
                    .iter()
                    .map(|arg| contract_type_to_type_ref(arg, location))
                    .collect::<Result<_, _>>()?,
            },
        },
        ContractTypeRef::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, field)| Ok((name.clone(), contract_type_to_type_ref(field, location)?)))
                .collect::<Result<_, _>>()?,
        },
        ContractTypeRef::StructuralUnion { variants } => TypeRefIr::Union {
            items: variants
                .iter()
                .map(|variant| contract_type_to_type_ref(variant, location))
                .collect::<Result<_, _>>()?,
        },
        ContractTypeRef::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(contract_type_to_type_ref(inner, location)?),
        },
        ContractTypeRef::Literal { value } => {
            let value = match value {
                skiff_artifact_model::ContractLiteral::String { value } => value,
            };
            TypeRefIr::Literal {
                value: LiteralIr::String { value: value.clone() },
            }
        }
        ContractTypeRef::TypeParam { name } => {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                format!("service contract retains unresolved type parameter {name:?}"),
            ));
        }
    })
}

fn first_specialization(
    package: &HydratedBytecodePackage,
    location: &BytecodeLinkLocation,
) -> Result<SpecializationKey, BytecodeLinkError> {
    let function = package
        .bytecode()
        .view()
        .functions()
        .iter()
        .next()
        .ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                "caller package has no linked specialization".to_string(),
            )
        })?;
    let canonical = package
        .canonical_implementation_callable_for_function_key(&function.function_key)
        .ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                format!("function {} has no canonical callable", function.function_key),
            )
        })?;
    super::relocations::specialization_key(
        package,
        &function.function_key,
        canonical.clone(),
        location.clone(),
    )
}

fn frame_signature(
    frame: &LinkedFrameLayout,
    _package: &HydratedBytecodePackage,
    _effect_callable: Option<&str>,
) -> Result<LinkedCallableSignature, BytecodeLinkError> {
    let location = BytecodeLinkLocation::Deployment {
        deployment: Box::new(skiff_artifact_model::ServiceDeploymentRef {
            service_id: "service".to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: skiff_artifact_model::DeploymentRevision::new("revision:targets"),
            deployment_artifact_identity: skiff_artifact_model::DeploymentArtifactIdentity::new(
                "deployment:targets",
            ),
        }),
    };
    LinkedCallableSignature::new(
        frame
            .parameters()
            .iter()
            .map(|parameter| {
                frame
                    .slot_types()
                    .get(parameter.slot().get() as usize)
                    .copied()
                    .ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::FrameAndValueTransferPlan,
                            location.clone(),
                            format!("parameter slot {} is out of bounds", parameter.slot().get()),
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice(),
        frame
            .parameters()
            .iter()
            .map(skiff_runtime_linked_bytecode::LinkedParameterSlot::mode)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        frame
            .parameters()
            .iter()
            .map(|parameter| parameter.plan().clone())
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        frame.result_types().to_vec().into_boxed_slice(),
        frame.result_plans().to_vec().into_boxed_slice(),
        CallableEffectSummary::Unknown {
            reason: skiff_artifact_model::CallableEffectUnknownReason::AnalysisPending,
        },
    )
    .map_err(|error| {
        unsatisfied(
            BytecodeLinkObligation::FrameAndValueTransferPlan,
            location,
            error.to_string(),
        )
    })
}

impl LinkedDispatchTables {
    fn sort_and_validate(&mut self) -> Result<(), BytecodeLinkError> {
        let mut seen = BTreeSet::new();
        for target in &self.service_operations {
            let key = (
                target.service_requirement_key().clone(),
                target.contract_operation_id().clone(),
            );
            if !seen.insert(key) {
                return Err(unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    BytecodeLinkLocation::Deployment {
                        deployment: Box::new(skiff_artifact_model::ServiceDeploymentRef {
                            service_id: "service".to_string(),
                            contract_version: "1.0.0".to_string(),
                            deployment_revision:
                                skiff_artifact_model::DeploymentRevision::new("revision:targets"),
                            deployment_artifact_identity:
                                skiff_artifact_model::DeploymentArtifactIdentity::new(
                                    "deployment:targets",
                                ),
                        }),
                    },
                    "duplicate service operation target".to_string(),
                ));
            }
        }
        let mut actor_method_seen = BTreeSet::new();
        for target in &self.actor_methods {
            let key = (
                target.owner_package_build_id().clone(),
                target.actor().module_path.clone(),
                target.actor().symbol.clone(),
                target.actor_implementation_identity().clone(),
                target.method_identity().clone(),
            );
            if !actor_method_seen.insert(key) {
                return Err(unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    BytecodeLinkLocation::Deployment {
                        deployment: Box::new(skiff_artifact_model::ServiceDeploymentRef {
                            service_id: "service".to_string(),
                            contract_version: "1.0.0".to_string(),
                            deployment_revision:
                                skiff_artifact_model::DeploymentRevision::new("revision:targets"),
                            deployment_artifact_identity:
                                skiff_artifact_model::DeploymentArtifactIdentity::new(
                                    "deployment:targets",
                                ),
                        }),
                    },
                    "duplicate actor method target".to_string(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use skiff_artifact_model::{
        HostEffectReference, HostEffectRegistryEntry, HostEffectSignature, NativeTarget,
        PackageArtifactRef, PackageBuildId, PackageLocalAbiIdentity, ParamModeIr,
        PendingEffectCategory, ResolvedPackageValueType, TypeDescriptorIr, TypeRefIr,
        ValueLifecycleFactResolver, ValueLifecyclePolicyBudget, ValueLifecycleResolverError,
        ValueTransferPlan,
    };

    use super::{
        registry_entry_for, registry_type_expression, validate_host_effect_authority,
    };
    use crate::bytecode::BytecodeLinkLocation;

    fn location() -> BytecodeLinkLocation {
        BytecodeLinkLocation::Package {
            package: Box::new(PackageArtifactRef {
                package_id: "example.com/host-authority".to_string(),
                package_version: "0.1.0".to_string(),
                package_build_id: PackageBuildId::new("build"),
                package_local_abi_identity: PackageLocalAbiIdentity::new("abi"),
            }),
        }
    }

    fn budget() -> ValueLifecyclePolicyBudget {
        ValueLifecyclePolicyBudget::new(1_000, 1_000_000, 64).expect("test budget is valid")
    }

    struct StubResolver;

    impl ValueLifecycleFactResolver for StubResolver {
        fn resolve_package_symbol(
            &mut self,
            _symbol: &skiff_artifact_model::PackageSymbolRef,
        ) -> Result<ResolvedPackageValueType, ValueLifecycleResolverError> {
            Ok(ResolvedPackageValueType {
                type_parameters: Vec::new(),
                descriptor: TypeDescriptorIr::Alias {
                    target: TypeRefIr::builtin("number"),
                },
            })
        }

        fn resolve_package_schema(
            &mut self,
            _package_id: &str,
            _stable_schema_key: &str,
            _package_schema_type_id: &skiff_artifact_model::PackageSchemaTypeId,
        ) -> Result<skiff_artifact_model::PackageSchemaTypeRecord, ValueLifecycleResolverError>
        {
            Err(resolver_error("schema resolution is unavailable in this test"))
        }

        fn validate_interface(
            &mut self,
            _interface: &skiff_artifact_model::InterfaceInstantiationRef,
        ) -> Result<(), ValueLifecycleResolverError> {
            Err(resolver_error("interface validation is unavailable in this test"))
        }

        fn validate_contract_interface(
            &mut self,
            _interface: &skiff_artifact_model::ContractTypeRef,
            _arguments: &[skiff_artifact_model::ContractTypeRef],
        ) -> Result<(), ValueLifecycleResolverError> {
            Err(resolver_error(
                "contract interface validation is unavailable in this test",
            ))
        }
    }

    fn resolver_error(message: &str) -> ValueLifecycleResolverError {
        ValueLifecycleResolverError {
            authority: "bytecodeLinker.hostAuthorityTest".to_string(),
            message: message.to_string(),
        }
    }

    fn fill_abi(ty: &mut TypeRefIr) {
        match ty {
            TypeRefIr::PackageSymbol { symbol } => {
                if symbol.abi_expectation.is_none() {
                    symbol.abi_expectation = Some("test-abi".to_string());
                }
            }
            TypeRefIr::Builtin { args, .. } => {
                for argument in args {
                    fill_abi(argument);
                }
            }
            _ => {}
        }
    }

    fn signature_from(entry: &HostEffectRegistryEntry) -> HostEffectSignature {
        let parameter_types = entry
            .signature
            .parameter_types
            .iter()
            .map(|ty| registry_type_expression(ty, &location()).expect("registry type converts"))
            .map(|mut ty| {
                fill_abi(&mut ty);
                ty
            })
            .collect::<Vec<_>>();
        let parameter_plans = parameter_types
            .iter()
            .cloned()
            .map(|ty| ValueTransferPlan::FromType { ty })
            .collect();
        let result_types = entry
            .signature
            .result_types
            .iter()
            .map(|ty| registry_type_expression(ty, &location()).expect("registry type converts"))
            .map(|mut ty| {
                fill_abi(&mut ty);
                ty
            })
            .collect::<Vec<_>>();
        let result_plans = result_types
            .iter()
            .cloned()
            .map(|ty| ValueTransferPlan::FromType { ty })
            .collect();
        HostEffectSignature {
            parameter_types,
            parameter_modes: entry.signature.parameter_modes.clone(),
            parameter_plans,
            result_types,
            result_plans,
            effects: entry.signature.effects.clone(),
        }
    }

    fn reference_for(entry: &HostEffectRegistryEntry) -> HostEffectReference {
        let (namespace, symbol) = entry
            .target
            .split_once('.')
            .expect("registry target is namespace-qualified");
        HostEffectReference {
            target: NativeTarget {
                namespace: namespace.to_string(),
                symbol: symbol.to_string(),
                binding_key: Some(entry.binding_key.clone()),
                metadata: BTreeMap::new(),
            },
            signature: signature_from(entry),
            db_operation: None,
        }
    }

    fn sleep_reference() -> HostEffectReference {
        let entry = skiff_artifact_model::host_effect_registry()
            .entries()
            .iter()
            .find(|entry| entry.binding_key == "std.time.sleep")
            .expect("pinned registry has std.time.sleep");
        reference_for(entry)
    }

    #[test]
    fn pinned_registry_owns_the_sleep_typed_entry() {
        let effect = sleep_reference();
        let entry = registry_entry_for(&effect, &location()).expect("pinned sleep resolves");
        assert_eq!(entry.binding_key, "std.time.sleep");
        assert_eq!(entry.signature.parameter_types.len(), 1);
        assert_eq!(entry.signature.result_types.len(), 0);
        assert!(entry.signature.effects.may_pending());
    }

    #[test]
    fn exact_sleep_arity_type_plan_and_effects_are_admitted() {
        let effect = sleep_reference();
        let mut resolver = StubResolver;
        let mut budget = budget();
        validate_host_effect_authority(
            &effect,
            &effect.signature,
            &mut resolver,
            &mut budget,
            &location(),
        )
        .expect("exact canonical sleep facts validate against the frozen registry");
    }

    #[test]
    fn wrong_sleep_arity_is_rejected() {
        let mut effect = sleep_reference();
        let duplicated = effect.signature.parameter_types[0].clone();
        effect.signature.parameter_types.push(duplicated.clone());
        effect
            .signature
            .parameter_modes
            .push(ParamModeIr::Value);
        effect
            .signature
            .parameter_plans
            .push(ValueTransferPlan::FromType { ty: duplicated });
        let mut resolver = StubResolver;
        let mut budget = budget();
        assert!(
            validate_host_effect_authority(
                &effect,
                &effect.signature,
                &mut resolver,
                &mut budget,
                &location(),
            )
            .is_err(),
            "wrong arity must not be admitted"
        );
    }

    #[test]
    fn wrong_sleep_parameter_type_is_rejected() {
        let mut effect = sleep_reference();
        effect.signature.parameter_types[0] = TypeRefIr::builtin("integer");
        let mut resolver = StubResolver;
        let mut budget = budget();
        assert!(
            validate_host_effect_authority(
                &effect,
                &effect.signature,
                &mut resolver,
                &mut budget,
                &location(),
            )
            .is_err(),
            "wrong parameter type must not be admitted"
        );
    }

    #[test]
    fn wrong_sleep_parameter_plan_is_rejected() {
        let mut effect = sleep_reference();
        effect.signature.parameter_plans[0] = ValueTransferPlan::FromType {
            ty: TypeRefIr::builtin("integer"),
        };
        let mut resolver = StubResolver;
        let mut budget = budget();
        assert!(
            validate_host_effect_authority(
                &effect,
                &effect.signature,
                &mut resolver,
                &mut budget,
                &location(),
            )
            .is_err(),
            "wrong parameter plan must not be admitted"
        );
    }

    #[test]
    fn wrong_sleep_effects_are_rejected() {
        let mut effect = sleep_reference();
        effect.signature.effects.pending_effect_categories =
            vec![PendingEffectCategory::HostEffect];
        let mut resolver = StubResolver;
        let mut budget = budget();
        assert!(
            validate_host_effect_authority(
                &effect,
                &effect.signature,
                &mut resolver,
                &mut budget,
                &location(),
            )
            .is_err(),
            "rewritten effects must not be admitted"
        );
    }

    #[test]
    fn std_binding_mismatch_is_no_longer_swallowed() {
        let entry = skiff_artifact_model::host_effect_registry()
            .entries()
            .iter()
            .find(|entry| entry.binding_key == "std.crypto.uuid")
            .expect("pinned registry has std.crypto.uuid");
        let mut effect = reference_for(entry);
        effect.signature.effects.may_pending = true;
        effect.signature.effects.pending_effect_categories =
            vec![PendingEffectCategory::HostEffect];
        let mut resolver = StubResolver;
        let mut budget = budget();
        assert!(
            validate_host_effect_authority(
                &effect,
                &effect.signature,
                &mut resolver,
                &mut budget,
                &location(),
            )
            .is_err(),
            "a std.* binding mismatch must fail closed instead of being swallowed"
        );
    }

    #[test]
    fn unknown_binding_key_fails_closed() {
        let effect = HostEffectReference {
            target: NativeTarget {
                namespace: "fixture".to_string(),
                symbol: "drift".to_string(),
                binding_key: Some("fixture.drift".to_string()),
                metadata: BTreeMap::new(),
            },
            signature: HostEffectSignature {
                parameter_types: Vec::new(),
                parameter_modes: Vec::new(),
                parameter_plans: Vec::new(),
                result_types: Vec::new(),
                result_plans: Vec::new(),
                effects: skiff_artifact_model::CallableMayEffects {
                    escapes_caller_value: false,
                    requires_same_heap_identity: false,
                    invokes_unknown_target: false,
                    may_pending: false,
                    pending_effect_categories: Vec::new(),
                    inout_path_effects: Vec::new(),
                },
            },
            db_operation: None,
        };
        assert!(registry_entry_for(&effect, &location()).is_err());
    }

    #[test]
    fn missing_binding_key_fails_closed() {
        let mut effect = sleep_reference();
        effect.target.binding_key = None;
        assert!(registry_entry_for(&effect, &location()).is_err());
    }
}

impl DeploymentLinker<'_> {
    fn link_interface_tables(
        &self,
        reachable: &[ReachableRelocation],
        indices: &BTreeMap<SpecializationKey, FunctionIndex>,
        frames: &[LinkedFrameLayout],
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<Vec<LinkedInterfaceTable>, BytecodeLinkError> {
        let mut unique = Vec::<InterfaceKey>::new();
        for package in self.deployment.packages().values() {
            for function in package.bytecode().view().functions() {
                let Some(source_specialization) = reachable.iter().find_map(|reference| {
                    (reference.specialization.package_build_id() == &package.reference().package_build_id
                        && reference.specialization.artifact_function_key().as_str() == function.function_key)
                        .then_some(&reference.specialization)
                }) else { continue; };
                for instruction in &function.instructions {
                    let contract = skiff_artifact_model::contract_for_opcode(instruction.descriptor.kind);
                    for (ordinal, operand) in contract.operands.iter().enumerate() {
                        if operand.kind != skiff_artifact_model::OperandKind::Reloc {
                            continue;
                        }
                        let relocation_index = *instruction.operand_words.get(ordinal).ok_or_else(|| {
                            unsatisfied(
                                BytecodeLinkObligation::RelocationResolution,
                                self.instruction_location(package, function, instruction.pc),
                                "relocation operand is absent".to_string(),
                            )
                        })?;
                        let relocation = function.relocations.get(relocation_index as usize).ok_or_else(|| {
                            unsatisfied(
                                BytecodeLinkObligation::RelocationResolution,
                                self.instruction_location(package, function, instruction.pc),
                                "relocation index is out of bounds".to_string(),
                            )
                        })?;
                        let key = match relocation {
                            BytecodeRelocation::InterfaceRequirementRef { interface } => {
                                let kind = if instruction.descriptor.kind == skiff_artifact_model::Opcode::InvokeCallback {
                                    InterfaceKind::Callback
                                } else {
                                    InterfaceKind::Requirement
                                };
                                Some(InterfaceKey {
                                    package_build_id: package.reference().package_build_id.clone(),
                                    specialization: source_specialization.clone(),
                                    interface: interface.clone(),
                                    kind,
                                    concrete_type: None,
                                    service_requirement_key: None,
                                    public_instance_key: None,
                                    source: InterfaceSource::Requirement,
                                })
                            }
                            BytecodeRelocation::LocalInterfaceRef { interface } => {
                                let specialization = self.specialization_for_function_key(package, &function.function_key, indices)?;
                                let concrete_type = type_linker.intern_concrete_type(
                                    package,
                                    specialization,
                                    &interface.concrete_type,
                                    &BTreeMap::new(),
                                    self.instruction_location(package, function, instruction.pc),
                                )?;
                                Some(InterfaceKey {
                                    package_build_id: package.reference().package_build_id.clone(),
                                    specialization: source_specialization.clone(),
                                    interface: interface.interface.clone(),
                                    kind: InterfaceKind::Local,
                                    concrete_type: Some(concrete_type),
                                    service_requirement_key: None,
                                    public_instance_key: None,
                                    source: InterfaceSource::Local(interface.clone()),
                                })
                            }
                            BytecodeRelocation::RemoteInterfaceRef { interface } => {
                                Some(InterfaceKey {
                                    package_build_id: package.reference().package_build_id.clone(),
                                    specialization: source_specialization.clone(),
                                    interface: interface.interface.clone(),
                                    kind: InterfaceKind::Remote,
                                    concrete_type: None,
                                    service_requirement_key: Some(ServiceRequirementKey {
                                        caller_package_build_id: package.reference().package_build_id.clone(),
                                        service_requirement_slot: interface.service_requirement_slot,
                                    }),
                                    public_instance_key: Some(interface.public_instance_key.clone()),
                                    source: InterfaceSource::Remote(interface.clone()),
                                })
                            }
                            _ => None,
                        };
                        if let Some(key) = key {
                            if !unique.contains(&key) {
                                unique.push(key);
                            }
                        }
                    }
                }
            }
        }

        let mut rows = Vec::new();
        for key in unique {
            let index = skiff_runtime_linked_bytecode::InterfaceTableIndex::new(rows.len() as u32);
            let package = self.deployment.packages().get(&key.package_build_id).ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    self.deployment_location(),
                    "no package is hydrated for interface target".to_string(),
                )
            })?;
            rows.push(self.link_one_interface_table(package, index, &key, indices, frames, type_linker)?);
        }
        Ok(rows)
    }

    fn link_one_interface_table(
        &self,
        package: &HydratedBytecodePackage,
        index: skiff_runtime_linked_bytecode::InterfaceTableIndex,
        key: &InterfaceKey,
        indices: &BTreeMap<SpecializationKey, FunctionIndex>,
        frames: &[LinkedFrameLayout],
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<LinkedInterfaceTable, BytecodeLinkError> {
        let location = self.package_location(package);
        let specialization = &key.specialization;
        let instantiation = linked_instantiation(&key.interface, package, specialization, type_linker, &location)?;
        let kind = match &key.kind {
            InterfaceKind::Requirement | InterfaceKind::Callback => {
                let methods = self.interface_requirement_methods(package, &key.interface, specialization, type_linker, &location)?;
                let table = LinkedInterfaceRequirementTable::new(methods.into_boxed_slice())
                    .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?;
                if key.kind == InterfaceKind::Callback {
                    LinkedInterfaceTableKind::Callback(table)
                } else {
                    LinkedInterfaceTableKind::Requirement(table)
                }
            }
            InterfaceKind::Local => {
                let source = match &key.source {
                    InterfaceSource::Local(source) => source,
                    _ => return Err(unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "local interface source is absent".to_string())),
                };
                let concrete_type = key.concrete_type.ok_or_else(|| {
                    unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "local interface concrete type is absent".to_string())
                })?;
                let mut methods = Vec::new();
                for method in &source.methods {
                    let signature = interface_slot_signature(package, specialization, &method.signature, type_linker, &location)?;
                    let function_key = method.function_key.clone();
                    let specialization = self.specialization_for_function_key(package, &function_key, indices)?;
                    let function = indices.get(specialization).copied().ok_or_else(|| {
                        unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), format!("local interface method {function_key:?} is absent from the closure"))
                    })?;
                    let abi_id = LinkedInterfaceMethodAbiId::parse(method.method_abi_id.clone())
                        .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?;
                    methods.push(
                        LinkedLocalInterfaceMethod::new(
                            method.slot,
                            method.method_name.clone(),
                            abi_id,
                            signature,
                            function,
                            method.receiver_call_abi,
                        )
                        .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?,
                    );
                }
                LinkedInterfaceTableKind::Local(
                    LinkedLocalInterfaceTable::new(concrete_type, methods.into_boxed_slice())
                        .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?,
                )
            }
            InterfaceKind::Remote => {
                let source = match &key.source {
                    InterfaceSource::Remote(source) => source,
                    _ => return Err(unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "remote interface source is absent".to_string())),
                };
                let mut methods = Vec::new();
                for method in &source.methods {
                    let signature = interface_slot_signature(package, specialization, &method.signature, type_linker, &location)?;
                    let abi_id = LinkedInterfaceMethodAbiId::parse(method.method_abi_id.clone())
                        .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?;
                    methods.push(LinkedRemoteInterfaceMethod::new(
                        method.slot,
                        abi_id,
                        signature,
                        method.contract_operation_id.clone(),
                    ));
                }
                let requirement_key = key.service_requirement_key.clone().ok_or_else(|| {
                    unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "remote interface requirement key is absent".to_string())
                })?;
                let public_instance_key = key.public_instance_key.clone().ok_or_else(|| {
                    unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "remote interface public instance is absent".to_string())
                })?;
                LinkedInterfaceTableKind::Remote(
                    LinkedRemoteInterfaceTable::new(
                        requirement_key,
                        LinkedPublicInstanceKey::parse(public_instance_key)
                            .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?,
                        methods.into_boxed_slice(),
                        source.callee_protocol_identity.clone(),
                    )
                    .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?,
                )
            }
        };
        let _ = frames;
        Ok(LinkedInterfaceTable::new(index, instantiation, kind))
    }

    fn interface_requirement_methods(
        &self,
        package: &HydratedBytecodePackage,
        interface: &InterfaceInstantiationRef,
        specialization: &SpecializationKey,
        type_linker: &mut TypeLinker<'_>,
        location: &BytecodeLinkLocation,
    ) -> Result<Vec<LinkedInterfaceRequirementMethod>, BytecodeLinkError> {
        let identity = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
            .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), format!("interface ABI is not TypeRefIr: {error}")))?;
        let mut methods = Vec::new();
        match identity {
            TypeRefIr::PackageSymbol { symbol } => {
                let owner = self.resolve_package_symbol_owner(package, &symbol, location)?;
                let abi_symbol = owner.artifact().package_local_abi.implementation_symbols.get(&symbol.symbol_path)
                    .or_else(|| owner.artifact().package_local_abi.public_symbols.get(&symbol.symbol_path))
                    .ok_or_else(|| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), format!("interface symbol {} is absent", symbol.symbol_path)))?;
                let PackageLocalAbiSymbol::Type { interface_methods, type_params, .. } = abi_symbol else {
                    return Err(unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "interface identity is not a type symbol".to_string()));
                };
                if type_params.len() != interface.canonical_type_args.len() {
                    return Err(unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "interface type parameter arity mismatch".to_string()));
                }
                for (slot, method) in interface_methods.iter().enumerate() {
                    let _ = specialization;
                    let signature = interface_method_signature(package, specialization, method, type_linker, location)?;
                    let abi_id = skiff_artifact_identity::canonical_interface_method_abi_id(interface, &method.name);
                    methods.push(LinkedInterfaceRequirementMethod::new(
                        slot as u32,
                        LinkedInterfaceMethodAbiId::parse(abi_id).map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?,
                        signature,
                    ));
                }
            }
            TypeRefIr::ServiceSymbol { symbol } => {
                let export = package
                    .artifact()
                    .implementation_links
                    .types
                    .values()
                    .find(|export| {
                        export.file.module_path == symbol.module_path
                            && export.symbol == symbol.symbol
                    })
                    .ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::ConcreteTargetTables,
                            location.clone(),
                            format!("interface service symbol {} is absent", symbol.symbol_path()),
                        )
                    })?;
                if export.type_params.len() != interface.canonical_type_args.len() {
                    return Err(unsatisfied(
                        BytecodeLinkObligation::ConcreteTargetTables,
                        location.clone(),
                        "interface type parameter arity mismatch".to_string(),
                    ));
                }
                for (slot, method) in export.interface_methods.iter().enumerate() {
                    let _ = specialization;
                    let signature = interface_method_signature(package, specialization, method, type_linker, location)?;
                    let abi_id = skiff_artifact_identity::canonical_interface_method_abi_id(interface, &method.name);
                    methods.push(LinkedInterfaceRequirementMethod::new(
                        slot as u32,
                        LinkedInterfaceMethodAbiId::parse(abi_id).map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?,
                        signature,
                    ));
                }
            }
            TypeRefIr::PackageSchema { package_id, stable_schema_key, package_schema_type_id } => {
                let owner = self.deployment.packages().values().find(|candidate| candidate.reference().package_id == package_id)
                    .ok_or_else(|| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), format!("interface schema owner {package_id} is absent")))?;
                let record = owner.artifact().bytecode_schema_records.get(&package_schema_type_id)
                    .filter(|record| record.package_id == package_id && record.stable_schema_key == stable_schema_key)
                    .ok_or_else(|| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "interface schema record is absent".to_string()))?;
                let skiff_artifact_model::ContractTypeDescriptor::CallbackInterface { operations } = &record.canonical_descriptor.descriptor else {
                    return Err(unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "interface schema is not a CallbackInterface".to_string()));
                };
                for (slot, (name, operation)) in operations.iter().enumerate() {
                    let _ = specialization;
                    let mut params = Vec::new();
                    for (index, ty) in operation.parameters.iter().enumerate() {
                        params.push(skiff_artifact_model::FunctionTypeParamIr {
                            name: format!("arg{index}"),
                            ty: contract_type_to_type_ref(ty, location)?,
                        });
                    }
                    let return_type = contract_type_to_type_ref(&operation.return_type, location)?;
                    let signature = interface_slot_signature_from_types(
                        package, specialization, &params, &return_type, type_linker, location,
                    )?;
                    let abi_id = skiff_artifact_identity::canonical_interface_method_abi_id(interface, name);
                    methods.push(LinkedInterfaceRequirementMethod::new(
                        slot as u32,
                        LinkedInterfaceMethodAbiId::parse(abi_id).map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?,
                        signature,
                    ));
                }
            }
            _ => return Err(unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "interface ABI identity is not PackageSymbol or PackageSchema".to_string())),
        }
        Ok(methods)
    }

    fn link_synthetic_callbacks(
        &self,
        reachable: &[ReachableRelocation],
        indices: &BTreeMap<SpecializationKey, FunctionIndex>,
        frames: &[LinkedFrameLayout],
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<Vec<LinkedSyntheticCallbackTarget>, BytecodeLinkError> {
        let mut seen = BTreeSet::new();
        let mut rows = Vec::new();
        for package in self.deployment.packages().values() {
            for function in package.bytecode().view().functions() {
                for relocation in &function.relocations {
                    if !reachable.iter().any(|reference| {
                        reference.specialization.package_build_id() == &package.reference().package_build_id
                            && reference.specialization.artifact_function_key().as_str() == function.function_key
                            && &reference.relocation == relocation
                    }) { continue; }
                    let BytecodeRelocation::SyntheticCallbackRef { function_key } = relocation else {
                        continue;
                    };
                    if !seen.insert((package.reference().package_build_id.clone(), function_key.clone())) {
                        continue;
                    }
                    let key = self.key_for_synthetic_callback(package, function_key)?;
                    let function = indices.get(&key).copied().ok_or_else(|| {
                        unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, self.package_location(package), format!("synthetic callback {function_key} is absent from the closure"))
                    })?;
                    let signature = frame_signature(
                        frames.get(function.get() as usize).ok_or_else(|| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, self.package_location(package), "synthetic callback frame is absent".to_string()))?,
                        package,
                        Some(function_key.as_str()),
                    )?;
                    let artifact_function_key = ArtifactFunctionKey::parse(function_key.clone())
                        .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, self.package_location(package), error.to_string()))?;
                    rows.push(LinkedSyntheticCallbackTarget::new(
                        skiff_runtime_linked_bytecode::SyntheticCallbackIndex::new(rows.len() as u32),
                        artifact_function_key,
                        function,
                        None,
                        signature,
                    ));
                }
            }
        }
        let _ = type_linker;
        Ok(rows)
    }

    fn host_signature_with_abi(
        &self,
        signature: &skiff_artifact_model::HostEffectSignature,
    ) -> skiff_artifact_model::HostEffectSignature {
        let mut signature = signature.clone();
        for ty in signature
            .parameter_types
            .iter_mut()
            .chain(signature.result_types.iter_mut())
        {
            self.fill_package_abi(ty);
        }
        for plan in signature
            .parameter_plans
            .iter_mut()
            .chain(signature.result_plans.iter_mut())
        {
            if let skiff_artifact_model::ValueTransferPlan::FromType { ty } = plan {
                self.fill_package_abi(ty);
            }
        }
        signature
    }

    fn fill_package_abi(&self, ty: &mut TypeRefIr) {
        match ty {
            TypeRefIr::PackageSymbol { symbol } => {
                if symbol.abi_expectation.is_none() {
                    if let Some(std) = self.deployment.packages().values().find(|package| {
                        package.reference().package_id == "skiff.run/std"
                    }) {
                        symbol.abi_expectation = Some(
                            std.reference().package_local_abi_identity.as_str().to_string(),
                        );
                    }
                }
            }
            TypeRefIr::Builtin { args, .. } => {
                for arg in args {
                    self.fill_package_abi(arg);
                }
            }
            TypeRefIr::Nullable { inner } => self.fill_package_abi(inner),
            TypeRefIr::Union { items } => {
                for item in items {
                    self.fill_package_abi(item);
                }
            }
            TypeRefIr::AppliedNominal { arguments, .. } => {
                for argument in arguments {
                    self.fill_package_abi(argument);
                }
            }
            _ => {}
        }
    }

    fn link_host_and_intrinsics(
        &self,
        reachable: &[ReachableRelocation],
        indices: &BTreeMap<SpecializationKey, FunctionIndex>,
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<(Vec<LinkedHostEffectAdapterTarget>, Vec<skiff_runtime_linked_bytecode::LinkedIntrinsicTarget>), BytecodeLinkError> {
        let mut host = Vec::new();
        let mut intrinsics = Vec::new();
        let mut seen_host = BTreeSet::new();
        let mut seen_intrinsics = BTreeSet::new();
        for package in self.deployment.packages().values() {
            for function in package.bytecode().view().functions() {
                if !indices.keys().any(|key| {
                    key.package_build_id() == &package.reference().package_build_id
                        && key.artifact_function_key().as_str() == function.function_key
                }) {
                    continue;
                }
                for relocation in &function.relocations {
                    if !reachable.iter().any(|reference| {
                        reference.specialization.package_build_id() == &package.reference().package_build_id
                            && reference.specialization.artifact_function_key().as_str() == function.function_key
                            && &reference.relocation == relocation
                    }) { continue; }
                    let location = self.function_location(package, function);
                    match relocation {
                        BytecodeRelocation::HostEffectRef(effect) => {
                            let specialization = self.specialization_for_function_key(package, &function.function_key, indices)?;
                            // The pinned registry is the only typed signature
                            // authority. The artifact's self-reported signature
                            // is checked against that registry and never
                            // copied into the linked entry.
                            let entry = registry_entry_for(effect, &location)?;
                            let mut resolver = DeploymentLifecycleResolver::new(self.deployment, package);
                            let mut budget = ValueLifecyclePolicyBudget::new(1_000, 1_000_000, 64)
                                .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?;
                            let self_reported = self.host_signature_with_abi(&effect.signature);
                            validate_host_effect_authority(
                                effect,
                                &self_reported,
                                &mut resolver,
                                &mut budget,
                                &location,
                            )?;
                            let signature = self.registry_native_signature(
                                entry,
                                package,
                                specialization,
                                type_linker,
                                &location,
                            )?;
                            let binding_key = entry.binding_key.as_str();
                            if !seen_host.insert(binding_key.to_string()) {
                                continue;
                            }
                            host.push(LinkedHostEffectAdapterTarget::new(
                                skiff_runtime_linked_bytecode::HostEffectAdapterIndex::new(host.len() as u32),
                                effect.target.namespace.clone(),
                                effect.target.symbol.clone(),
                                skiff_runtime_linked_bytecode::LinkedHostBindingKey::parse(binding_key)
                                    .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?,
                                effect.target.metadata.clone(),
                                signature,
                            )
                            .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?);
                        }
                        BytecodeRelocation::IntrinsicRef { intrinsic } => {
                            let specialization = self.specialization_for_function_key(package, &function.function_key, indices)?;
                            let mut resolver = DeploymentLifecycleResolver::new(self.deployment, package);
                            let mut budget = ValueLifecyclePolicyBudget::new(1_000, 1_000_000, 64)
                                .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?;
                            skiff_artifact_model::intrinsic_registry()
                                .match_reference(intrinsic, &mut resolver, &mut budget)
                                .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), format!("intrinsic registry rejected target: {error:?}")))?;
                            let signature = native_signature(package, specialization, &intrinsic.signature, type_linker, &location)?;
                            let kind = match &intrinsic.target {
                                BytecodeIntrinsicRef::Static { canonical_key, signature_version } => skiff_runtime_linked_bytecode::LinkedIntrinsicKind::Static(
                                    skiff_runtime_linked_bytecode::LinkedStaticIntrinsicTarget::new(
                                        skiff_runtime_linked_bytecode::LinkedIntrinsicCanonicalKey::parse(canonical_key.clone())
                                            .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?,
                                        *signature_version,
                                    )
                                    .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?,
                                ),
                                BytecodeIntrinsicRef::Receiver { op } => skiff_runtime_linked_bytecode::LinkedIntrinsicKind::Receiver(*op),
                            };
                            let intrinsic_key = match &kind {
                                skiff_runtime_linked_bytecode::LinkedIntrinsicKind::Static(target) => format!(
                                    "static:{}@{}",
                                    target.canonical_key().as_str(),
                                    target.signature_version()
                                ),
                                skiff_runtime_linked_bytecode::LinkedIntrinsicKind::Receiver(op) => {
                                    format!("receiver:{}", op.canonical_key)
                                }
                            };
                            if !seen_intrinsics.insert(intrinsic_key) {
                                continue;
                            }
                            intrinsics.push(skiff_runtime_linked_bytecode::LinkedIntrinsicTarget::new(
                                skiff_runtime_linked_bytecode::IntrinsicIndex::new(intrinsics.len() as u32),
                                kind,
                                signature,
                            ));
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok((host, intrinsics))
    }

    pub(super) fn key_for_receiver_callable(
        &self,
        package: &HydratedBytecodePackage,
        callable: &skiff_artifact_model::PackageCallableId,
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<SpecializationKey, BytecodeLinkError> {
        let location = self.package_location(package);
        let function_key = package.function_key_for_callable(callable).ok_or_else(|| {
            unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), format!("callable {callable} has no function key"))
        })?;
        let function = package.bytecode().view().functions().iter().find(|function| function.function_key == function_key)
            .ok_or_else(|| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), format!("function {function_key} is absent")))?;
        if !function.type_parameters.is_empty() {
            return Err(unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location, "receiver callable is generic".to_string()));
        }
        let self_ref = function.self_type_ref.ok_or_else(|| {
            unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "receiver callable has no self type".to_string())
        })?;
        let receiver = type_linker.intern_package_global_type(package, self_ref, location.clone())?.0;
        let canonical = package.canonical_implementation_callable_for_function_key(function_key).ok_or_else(|| {
            unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "receiver callable has no canonical implementation".to_string())
        })?;
        if canonical != callable {
            return Err(unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location, "receiver callable aliases a different canonical identity".to_string()));
        }
        let artifact_function_key = ArtifactFunctionKey::parse(function_key)
            .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?;
        Ok(SpecializationKey::new(
            package.reference().package_build_id.clone(),
            artifact_function_key,
            canonical.clone(),
            Box::new([]),
            Some(receiver),
        ))
    }

    pub(super) fn key_for_synthetic_callback(
        &self,
        package: &HydratedBytecodePackage,
        function_key: &str,
    ) -> Result<SpecializationKey, BytecodeLinkError> {
        let location = self.package_location(package);
        let function = package.bytecode().view().functions().iter().find(|function| function.function_key == function_key)
            .ok_or_else(|| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), format!("synthetic callback {function_key} is absent")))?;
        let skiff_artifact_model::BytecodeFunctionOrigin::SyntheticCallback { owner, site_ordinal } = &function.origin else {
            return Err(unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location, "callback target is not synthetic".to_string()));
        };
        let callable = package.synthetic_callback_callable(owner, *site_ordinal).ok_or_else(|| {
            unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), format!("synthetic callback {function_key} has no callable identity"))
        })?;
        super::relocations::specialization_key(package, function_key, callable.clone(), location)
    }

    fn specialization_for_function_key<'b>(
        &self,
        package: &HydratedBytecodePackage,
        function_key: &str,
        indices: &'b BTreeMap<SpecializationKey, FunctionIndex>,
    ) -> Result<&'b SpecializationKey, BytecodeLinkError> {
        indices.keys().find(|key| {
            key.package_build_id() == &package.reference().package_build_id
                && key.artifact_function_key().as_str() == function_key
        }).ok_or_else(|| {
            unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, self.package_location(package), format!("function {function_key} has no linked specialization"))
        })
    }

    fn resolve_package_symbol_owner(
        &self,
        caller: &HydratedBytecodePackage,
        symbol: &PackageSymbolRef,
        location: &BytecodeLinkLocation,
    ) -> Result<&HydratedBytecodePackage, BytecodeLinkError> {
        match &symbol.package {
            PackageRefIr::PackageId { package_id } => self.deployment.packages().values().find(|package| package.reference().package_id == *package_id).ok_or_else(|| {
                unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), format!("package owner {package_id} is absent"))
            }),
            PackageRefIr::Dependency { dependency_ref } => {
                let key = skiff_artifact_model::PackageRequirementKey {
                    caller_package_build_id: caller.reference().package_build_id.clone(),
                    package_requirement_alias: dependency_ref.clone(),
                };
                let binding = self.deployment.deployment().package_bindings.iter().find(|binding| binding.key == key).ok_or_else(|| {
                    unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), format!("dependency alias {dependency_ref} is absent"))
                })?;
                self.deployment.packages().get(&binding.package.package_build_id).filter(|package| package.reference() == &binding.package).ok_or_else(|| {
                    unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "dependency target is absent".to_string())
                })
            }
        }
    }
}

fn linked_instantiation(
    interface: &InterfaceInstantiationRef,
    package: &HydratedBytecodePackage,
    specialization: &SpecializationKey,
    type_linker: &mut TypeLinker<'_>,
    location: &BytecodeLinkLocation,
) -> Result<LinkedInterfaceInstantiation, BytecodeLinkError> {
    let concrete_type_arguments = interface
        .canonical_type_args
        .iter()
        .map(|ty| type_linker.intern_concrete_type(package, specialization, ty, &BTreeMap::new(), location.clone()))
        .collect::<Result<Vec<_>, _>>()?;
    LinkedInterfaceInstantiation::new(interface.clone(), concrete_type_arguments.into_boxed_slice())
        .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))
}

fn interface_slot_signature(
    package: &HydratedBytecodePackage,
    specialization: &SpecializationKey,
    signature: &InterfaceMethodSlotSignatureIr,
    type_linker: &mut TypeLinker<'_>,
    location: &BytecodeLinkLocation,
) -> Result<LinkedCallableSignature, BytecodeLinkError> {
    interface_slot_signature_from_types(
        package,
        specialization,
        &signature.params,
        &signature.return_type,
        type_linker,
        location,
    )
}

fn interface_slot_signature_from_types(
    package: &HydratedBytecodePackage,
    specialization: &SpecializationKey,
    params: &[skiff_artifact_model::FunctionTypeParamIr],
    return_type: &TypeRefIr,
    type_linker: &mut TypeLinker<'_>,
    location: &BytecodeLinkLocation,
) -> Result<LinkedCallableSignature, BytecodeLinkError> {
    let mut parameter_types = Vec::new();
    let mut parameter_plans = Vec::new();
    for parameter in params {
        let ty = type_linker.intern_concrete_type(package, specialization, &parameter.ty, &BTreeMap::new(), location.clone())?;
        let concrete = type_linker.linked_type_ref(ty).cloned().ok_or_else(|| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "interface parameter type is absent".to_string()))?;
        let plan = type_linker.plan_for_concrete_type(&concrete, location.clone())?;
        parameter_types.push(ty);
        parameter_plans.push(plan);
    }
    let result_types = if matches!(return_type, TypeRefIr::Builtin { name, .. } if name == "void") {
        Vec::new()
    } else {
        vec![type_linker.intern_concrete_type(package, specialization, return_type, &BTreeMap::new(), location.clone())?]
    };
    let mut result_plans = Vec::new();
    for ty in &result_types {
        let concrete = type_linker.linked_type_ref(*ty).cloned().ok_or_else(|| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "interface result type is absent".to_string()))?;
        result_plans.push(type_linker.plan_for_concrete_type(&concrete, location.clone())?);
    }
    let parameter_mode_count = parameter_types.len();
    LinkedCallableSignature::new(
        parameter_types.into_boxed_slice(),
        vec![ParamModeIr::Value; parameter_mode_count].into_boxed_slice(),
        parameter_plans.into_boxed_slice(),
        result_types.into_boxed_slice(),
        result_plans.into_boxed_slice(),
        CallableEffectSummary::Unknown {
            reason: skiff_artifact_model::CallableEffectUnknownReason::AnalysisPending,
        },
    )
    .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))
}

fn interface_method_signature(
    package: &HydratedBytecodePackage,
    specialization: &SpecializationKey,
    method: &skiff_artifact_model::InterfaceMethodSignature,
    type_linker: &mut TypeLinker<'_>,
    location: &BytecodeLinkLocation,
) -> Result<LinkedCallableSignature, BytecodeLinkError> {
    interface_slot_signature_from_types(package, specialization, &method.params, &method.return_type, type_linker, location)
}

fn native_signature(
    package: &HydratedBytecodePackage,
    specialization: &SpecializationKey,
    signature: &skiff_artifact_model::HostEffectSignature,
    type_linker: &mut TypeLinker<'_>,
    location: &BytecodeLinkLocation,
) -> Result<LinkedNativeCallableSignature, BytecodeLinkError> {
    let mut parameter_types = Vec::new();
    let mut parameter_plans = Vec::new();
    for (ty, plan) in signature.parameter_types.iter().zip(&signature.parameter_plans) {
        let index = type_linker.intern_concrete_type(package, specialization, ty, &BTreeMap::new(), location.clone())?;
        let concrete = type_linker.linked_type_ref(index).cloned().ok_or_else(|| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "native parameter type is absent".to_string()))?;
        parameter_types.push(index);
        parameter_plans.push(type_linker.link_plan_for_type(plan, &concrete, location.clone())?);
    }
    let mut result_types = Vec::new();
    let mut result_plans = Vec::new();
    for (ty, plan) in signature.result_types.iter().zip(&signature.result_plans) {
        let index = type_linker.intern_concrete_type(package, specialization, ty, &BTreeMap::new(), location.clone())?;
        let concrete = type_linker.linked_type_ref(index).cloned().ok_or_else(|| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), "native result type is absent".to_string()))?;
        result_types.push(index);
        result_plans.push(type_linker.link_plan_for_type(plan, &concrete, location.clone())?);
    }
    LinkedNativeCallableSignature::new(
        parameter_types.into_boxed_slice(),
        signature.parameter_modes.clone().into_boxed_slice(),
        parameter_plans.into_boxed_slice(),
        result_types.into_boxed_slice(),
        result_plans.into_boxed_slice(),
        signature.effects.clone(),
    )
    .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))
}

/// Looks up a host effect relocation in the frozen registry by its canonical
/// binding ID. The registry is the only typed authority: an absent binding ID
/// fails closed, and the returned entry never borrows the artifact's
/// self-reported signature.
fn registry_entry_for(
    effect: &skiff_artifact_model::HostEffectReference,
    location: &BytecodeLinkLocation,
) -> Result<&'static HostEffectRegistryEntry, BytecodeLinkError> {
    let binding_key = effect.target.binding_key.as_deref().ok_or_else(|| {
        unsatisfied(
            BytecodeLinkObligation::ConcreteTargetTables,
            location.clone(),
            "host effect target carries no canonical binding ID".to_string(),
        )
    })?;
    skiff_artifact_model::host_effect_registry()
        .entries()
        .iter()
        .find(|entry| entry.binding_key == binding_key)
        .ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                format!("host effect binding key `{binding_key}` is absent from the registry"),
            )
        })
}

/// Proves the artifact's canonical facts (exact target, metadata, binding ID
/// and self-reported signature) against the frozen registry. Any mismatch is
/// fatal: there is no std-binding bypass.
fn validate_host_effect_authority<R: ValueLifecycleFactResolver>(
    effect: &skiff_artifact_model::HostEffectReference,
    self_reported: &skiff_artifact_model::HostEffectSignature,
    resolver: &mut R,
    budget: &mut ValueLifecyclePolicyBudget,
    location: &BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    skiff_artifact_model::host_effect_registry()
        .match_reference(&effect.target, self_reported, resolver, budget)
        .map(|_| ())
        .map_err(|error| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                format!("host effect registry rejected the artifact's canonical facts: {error:?}"),
            )
        })
}

impl DeploymentLinker<'_> {
    /// Builds the linked native signature exclusively from the frozen
    /// registry entry. The artifact's self-reported signature is never
    /// copied.
    fn registry_native_signature(
        &self,
        entry: &HostEffectRegistryEntry,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        type_linker: &mut TypeLinker<'_>,
        location: &BytecodeLinkLocation,
    ) -> Result<LinkedNativeCallableSignature, BytecodeLinkError> {
        let mut parameter_types = Vec::new();
        let mut parameter_plans = Vec::new();
        for (ty, plan) in entry
            .signature
            .parameter_types
            .iter()
            .zip(&entry.signature.parameter_plans)
        {
            let mut ty = registry_type_expression(ty, location)?;
            self.fill_package_abi(&mut ty);
            let index = type_linker.intern_concrete_type(
                package,
                specialization,
                &ty,
                &BTreeMap::new(),
                location.clone(),
            )?;
            let concrete = type_linker
                .linked_type_ref(index)
                .cloned()
                .ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::ConcreteTargetTables,
                        location.clone(),
                        "registry host parameter type is absent".to_string(),
                    )
                })?;
            parameter_types.push(index);
            parameter_plans.push(type_linker.link_plan_for_type(
                &registry_plan_expression(plan, location)?,
                &concrete,
                location.clone(),
            )?);
        }
        let mut result_types = Vec::new();
        let mut result_plans = Vec::new();
        for (ty, plan) in entry
            .signature
            .result_types
            .iter()
            .zip(&entry.signature.result_plans)
        {
            let mut ty = registry_type_expression(ty, location)?;
            self.fill_package_abi(&mut ty);
            let index = type_linker.intern_concrete_type(
                package,
                specialization,
                &ty,
                &BTreeMap::new(),
                location.clone(),
            )?;
            let concrete = type_linker
                .linked_type_ref(index)
                .cloned()
                .ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::ConcreteTargetTables,
                        location.clone(),
                        "registry host result type is absent".to_string(),
                    )
                })?;
            result_types.push(index);
            result_plans.push(type_linker.link_plan_for_type(
                &registry_plan_expression(plan, location)?,
                &concrete,
                location.clone(),
            )?);
        }
        LinkedNativeCallableSignature::new(
            parameter_types.into_boxed_slice(),
            entry.signature.parameter_modes.clone().into_boxed_slice(),
            parameter_plans.into_boxed_slice(),
            result_types.into_boxed_slice(),
            result_plans.into_boxed_slice(),
            entry.signature.effects.clone(),
        )
        .map_err(|error| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                error.to_string(),
            )
        })
    }
}

fn registry_type_expression(
    ty: &CallableRegistryTypeExpression,
    location: &BytecodeLinkLocation,
) -> Result<TypeRefIr, BytecodeLinkError> {
    Ok(match ty {
        CallableRegistryTypeExpression::TypeParameter { ordinal } => {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                format!(
                    "registry host signature type parameter {ordinal} requires an instantiation and is not admitted"
                ),
            ))
        }
        CallableRegistryTypeExpression::Builtin { name, arguments } => TypeRefIr::Builtin {
            name: name.clone(),
            args: arguments
                .iter()
                .map(|argument| registry_type_expression(argument, location))
                .collect::<Result<_, _>>()?,
        },
        CallableRegistryTypeExpression::PackageSymbol {
            package_id,
            symbol_path,
        } => TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: package_id.clone(),
                },
                symbol_path: symbol_path.clone(),
                abi_expectation: None,
            },
        },
    })
}

fn registry_plan_expression(
    plan: &skiff_artifact_model::CallableRegistryPlanExpression,
    location: &BytecodeLinkLocation,
) -> Result<skiff_artifact_model::ValueTransferPlan, BytecodeLinkError> {
    match plan {
        skiff_artifact_model::CallableRegistryPlanExpression::FromType { ty } => {
            Ok(skiff_artifact_model::ValueTransferPlan::FromType {
                ty: registry_type_expression(ty, location)?,
            })
        }
    }
}

struct DeploymentLifecycleResolver<'a> {
    deployment: &'a HydratedDeploymentBytecode,
    caller: &'a HydratedBytecodePackage,
}

impl<'a> DeploymentLifecycleResolver<'a> {
    fn new(deployment: &'a HydratedDeploymentBytecode, caller: &'a HydratedBytecodePackage) -> Self {
        Self { deployment, caller }
    }
}

impl ValueLifecycleFactResolver for DeploymentLifecycleResolver<'_> {
    fn resolve_package_symbol(
        &mut self,
        symbol: &PackageSymbolRef,
    ) -> Result<ResolvedPackageValueType, ValueLifecycleResolverError> {
        let owner = match &symbol.package {
            PackageRefIr::PackageId { package_id } => self.deployment.packages().values().find(|package| package.reference().package_id == *package_id).ok_or_else(|| resolver_error("package owner absent"))?,
            PackageRefIr::Dependency { dependency_ref } => {
                let key = skiff_artifact_model::PackageRequirementKey {
                    caller_package_build_id: self.caller.reference().package_build_id.clone(),
                    package_requirement_alias: dependency_ref.clone(),
                };
                let binding = self.deployment.deployment().package_bindings.iter().find(|binding| binding.key == key).ok_or_else(|| resolver_error("dependency binding absent"))?;
                self.deployment.packages().get(&binding.package.package_build_id).ok_or_else(|| resolver_error("dependency package absent"))?
            }
        };
        let symbol = owner.artifact().package_local_abi.implementation_symbols.get(&symbol.symbol_path)
            .or_else(|| owner.artifact().package_local_abi.public_symbols.get(&symbol.symbol_path))
            .ok_or_else(|| resolver_error("package symbol absent"))?;
        let PackageLocalAbiSymbol::Type { descriptor, type_params, .. } = symbol else {
            return Err(resolver_error("package symbol is not a type"));
        };
        let location = BytecodeLinkLocation::Package {
            package: Box::new(owner.reference().clone()),
        };
        let descriptor = normalize_resolved_descriptor(
            self.deployment,
            owner,
            descriptor,
            &location,
        )
        .map_err(|error| resolver_error(error.to_string()))?;
        Ok(ResolvedPackageValueType {
            type_parameters: type_params.clone(),
            descriptor,
        })
    }

    fn resolve_package_schema(
        &mut self,
        package_id: &str,
        stable_schema_key: &str,
        package_schema_type_id: &PackageSchemaTypeId,
    ) -> Result<skiff_artifact_model::PackageSchemaTypeRecord, ValueLifecycleResolverError> {
        let owner = self.deployment.packages().values().find(|package| package.reference().package_id == package_id).ok_or_else(|| resolver_error("schema owner absent"))?;
        owner.artifact().bytecode_schema_records.get(package_schema_type_id)
            .filter(|record| record.package_id == package_id && record.stable_schema_key == stable_schema_key)
            .cloned()
            .ok_or_else(|| resolver_error("schema record absent"))
    }

    fn validate_interface(
        &mut self,
        interface: &InterfaceInstantiationRef,
    ) -> Result<(), ValueLifecycleResolverError> {
        let identity: TypeRefIr = serde_json::from_str(&interface.interface_abi_id).map_err(|_| resolver_error("interface identity is not TypeRefIr"))?;
        let TypeRefIr::PackageSymbol { symbol } = identity else {
            return Err(resolver_error("interface identity is not PackageSymbol"));
        };
        self.resolve_package_symbol(&symbol)?;
        Ok(())
    }

    fn validate_contract_interface(
        &mut self,
        interface: &skiff_artifact_model::ContractTypeRef,
        arguments: &[skiff_artifact_model::ContractTypeRef],
    ) -> Result<(), ValueLifecycleResolverError> {
        let skiff_artifact_model::ContractTypeRef::PackageSchema { package_id, stable_schema_key, package_schema_type_id } = interface else {
            return Err(resolver_error("contract interface is not PackageSchema"));
        };
        let record = self.resolve_package_schema(package_id, stable_schema_key, package_schema_type_id)?;
        if !matches!(record.canonical_descriptor.descriptor, skiff_artifact_model::ContractTypeDescriptor::CallbackInterface { .. }) {
            return Err(resolver_error("contract interface is not callback"));
        }
        if record.canonical_descriptor.type_params.len() != arguments.len() {
            return Err(resolver_error("contract interface arity mismatch"));
        }
        Ok(())
    }
}

fn normalize_resolved_descriptor(
    deployment: &HydratedDeploymentBytecode,
    owner: &HydratedBytecodePackage,
    descriptor: &skiff_artifact_model::TypeDescriptorIr,
    location: &BytecodeLinkLocation,
) -> Result<skiff_artifact_model::TypeDescriptorIr, BytecodeLinkError> {
    let normalize_ty = |ty: &TypeRefIr| normalize_type(deployment, owner, ty, location);
    Ok(match descriptor {
        skiff_artifact_model::TypeDescriptorIr::Record { fields } => {
            skiff_artifact_model::TypeDescriptorIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| Ok((name.clone(), normalize_ty(ty)?)))
                    .collect::<Result<_, BytecodeLinkError>>()?,
            }
        }
        skiff_artifact_model::TypeDescriptorIr::Representation { representation } => {
            skiff_artifact_model::TypeDescriptorIr::Representation {
                representation: normalize_ty(representation)?,
            }
        }
        skiff_artifact_model::TypeDescriptorIr::Union { branches } => {
            skiff_artifact_model::TypeDescriptorIr::Union {
                branches: branches
                    .iter()
                    .map(|branch| match branch {
                        skiff_artifact_model::NamedUnionBranchIr::ConcreteNominal {
                            nominal_type,
                        } => Ok(skiff_artifact_model::NamedUnionBranchIr::ConcreteNominal {
                            nominal_type: normalize_ty(nominal_type)?,
                        }),
                        skiff_artifact_model::NamedUnionBranchIr::SyntheticDiscriminator {
                            payload_type,
                            discriminator_field,
                            discriminator_value,
                        } => Ok(
                            skiff_artifact_model::NamedUnionBranchIr::SyntheticDiscriminator {
                                payload_type: normalize_ty(payload_type)?,
                                discriminator_field: discriminator_field.clone(),
                                discriminator_value: discriminator_value.clone(),
                            },
                        ),
                        skiff_artifact_model::NamedUnionBranchIr::Literal { value } => {
                            Ok(skiff_artifact_model::NamedUnionBranchIr::Literal {
                                value: value.clone(),
                            })
                        }
                    })
                    .collect::<Result<_, BytecodeLinkError>>()?,
            }
        }
        skiff_artifact_model::TypeDescriptorIr::Alias { target } => {
            skiff_artifact_model::TypeDescriptorIr::Alias {
                target: normalize_ty(target)?,
            }
        }
        skiff_artifact_model::TypeDescriptorIr::Interface => {
            skiff_artifact_model::TypeDescriptorIr::Interface
        }
    })
}

fn resolver_error(message: impl Into<String>) -> ValueLifecycleResolverError {
    ValueLifecycleResolverError {
        authority: "bytecodeLinker.hydratedValueLifecycle".to_string(),
        message: message.into(),
    }
}

impl DeploymentLinker<'_> {
    pub(super) fn key_for_actor_method(
        &self,
        package: &HydratedBytecodePackage,
        actor: &skiff_artifact_model::ServiceSymbolRef,
        actor_abi_identity: &skiff_artifact_model::ActorAbiIdentity,
        actor_implementation_identity: &skiff_artifact_model::ActorImplementationIdentity,
        method_identity: &skiff_artifact_model::ActorMethodIdentity,
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<SpecializationKey, BytecodeLinkError> {
        let location = self.package_location(package);
        let implementation = package
            .artifact()
            .actor_implementations
            .iter()
            .find(|candidate| {
                candidate.actor == *actor
                    && candidate.actor_implementation_identity == *actor_implementation_identity
            })
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    "actor relocation has no exact implementation authority".to_string(),
                )
            })?;
        let exact_abi = package
            .artifact()
            .implementation_links
            .types
            .values()
            .find(|export| {
                export.file.module_path == actor.module_path && export.symbol == actor.symbol
            })
            .and_then(|export| export.actor.as_ref())
            .is_some_and(|abi| abi.actor_abi_identity == *actor_abi_identity);
        if !exact_abi {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location,
                "actor relocation ABI identity differs from package authority".to_string(),
            ));
        }
        let callable = implementation.methods.get(method_identity).ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                self.package_location(package),
                format!("actor relocation method {method_identity:?} is absent"),
            )
        })?;
        self.key_for_receiver_callable(package, callable, type_linker)
    }

    pub(super) fn key_for_receiver_function(
        &self,
        package: &HydratedBytecodePackage,
        function_key: &str,
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<SpecializationKey, BytecodeLinkError> {
        let location = self.package_location(package);
        let function = package
            .bytecode()
            .view()
            .functions()
            .iter()
            .find(|function| function.function_key == function_key)
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    format!("receiver function {function_key} is absent"),
                )
            })?;
        if !function.type_parameters.is_empty() {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location,
                "receiver function is generic".to_string(),
            ));
        }
        let self_ref = function.self_type_ref.ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                format!("receiver function {function_key} has no self type"),
            )
        })?;
        let receiver = type_linker
            .intern_package_global_type(package, self_ref, location.clone())?
            .0;
        let canonical = package
            .canonical_implementation_callable_for_function_key(function_key)
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    format!("receiver function {function_key} has no canonical callable"),
                )
            })?;
        let artifact_function_key = ArtifactFunctionKey::parse(function_key)
            .map_err(|error| unsatisfied(BytecodeLinkObligation::ConcreteTargetTables, location.clone(), error.to_string()))?;
        Ok(SpecializationKey::new(
            package.reference().package_build_id.clone(),
            artifact_function_key,
            canonical.clone(),
            Box::new([]),
            Some(receiver),
        ))
    }
}

impl LinkedDispatchTables {
    pub(in crate::bytecode) fn service_operation_index(
        &self,
        caller_package_build_id: &PackageBuildId,
        slot: u32,
        operation: &ContractOperationId,
    ) -> Option<skiff_runtime_linked_bytecode::ServiceOperationIndex> {
        self.service_operations.iter().find(|target| {
            target.service_requirement_key().caller_package_build_id == *caller_package_build_id
                && target.service_requirement_key().service_requirement_slot == slot
                && target.contract_operation_id() == operation
        }).map(|target| target.index())
    }

    pub(in crate::bytecode) fn actor_method_index(
        &self,
        package: &HydratedBytecodePackage,
        actor: &skiff_artifact_model::ServiceSymbolRef,
        abi: &skiff_artifact_model::ActorAbiIdentity,
        implementation: &skiff_artifact_model::ActorImplementationIdentity,
        method: &skiff_artifact_model::ActorMethodIdentity,
    ) -> Option<skiff_runtime_linked_bytecode::ActorMethodIndex> {
        self.actor_methods.iter().find(|target| {
            target.owner_package_build_id() == &package.reference().package_build_id
                && target.actor() == actor
                && target.actor_abi_identity() == abi
                && target.actor_implementation_identity() == implementation
                && target.method_identity() == method
        }).map(|target| target.index())
    }

    pub(in crate::bytecode) fn interface_index(
        &self,
        interface: &InterfaceInstantiationRef,
        kind: &InterfaceKind,
    ) -> Option<skiff_runtime_linked_bytecode::InterfaceTableIndex> {
        self.interface_tables.iter().find(|table| {
            table.interface().artifact() == interface
                && matches!(
                    (kind, table.kind()),
                    (InterfaceKind::Requirement, LinkedInterfaceTableKind::Requirement(_))
                        | (InterfaceKind::Callback, LinkedInterfaceTableKind::Callback(_))
                        | (InterfaceKind::Local, LinkedInterfaceTableKind::Local(_))
                        | (InterfaceKind::Remote, LinkedInterfaceTableKind::Remote(_))
                )
        }).map(|table| table.index())
    }

    pub(in crate::bytecode) fn synthetic_callback_index(
        &self,
        package: &HydratedBytecodePackage,
        function_key: &str,
    ) -> Option<skiff_runtime_linked_bytecode::SyntheticCallbackIndex> {
        self.synthetic_callback_origins
            .get(&(package.reference().package_build_id.clone(), function_key.to_string()))
            .copied()
    }

    pub(in crate::bytecode) fn host_index(
        &self,
        namespace: &str,
        symbol: &str,
        binding_key: &str,
        metadata: &BTreeMap<String, skiff_artifact_model::MetadataValue>,
    ) -> Option<skiff_runtime_linked_bytecode::HostEffectAdapterIndex> {
        self.host_effect_adapters.iter().find(|target| {
            target.namespace() == namespace
                && target.symbol() == symbol
                && target.binding_key().as_str() == binding_key
                && target.metadata() == metadata
        }).map(|target| target.index())
    }

    pub(in crate::bytecode) fn intrinsic_index(
        &self,
        reference: &BytecodeIntrinsicRef,
    ) -> Option<skiff_runtime_linked_bytecode::IntrinsicIndex> {
        self.intrinsics.iter().find(|target| match (target.kind(), reference) {
            (
                skiff_runtime_linked_bytecode::LinkedIntrinsicKind::Static(linked),
                BytecodeIntrinsicRef::Static { canonical_key, signature_version },
            ) => linked.canonical_key().as_str() == canonical_key && linked.signature_version() == *signature_version,
            (
                skiff_runtime_linked_bytecode::LinkedIntrinsicKind::Receiver(linked),
                BytecodeIntrinsicRef::Receiver { op },
            ) => linked == op,
            _ => false,
        }).map(|target| target.index())
    }
}
