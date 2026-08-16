use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::bytecode::dto::DbOperationReference;
use skiff_artifact_model::{
    self, BoundaryDropPlan, BoundaryTransfer, BoundaryValueFact, BoundaryValuePlan,
    BytecodeIntrinsicRef, BytecodeRelocation, CallableEffectSummary, CallableMayEffects,
    CallableRegistryTypeExpression, ContractLiteral, ContractOperationId, ContractTypeRef,
    HostEffectExecutorIdentity, HostEffectRegistryEntry, InterfaceInstantiationRef, LiteralIr,
    Opcode, PackageBuildId, PackageLocalAbiSymbol, PackageRefIr, PackageSchemaTypeId,
    PackageSymbolRef, ParamModeIr, PendingEffectCategory, ResolvedPackageValueType,
    ServiceBoundaryPlan, ServiceCallbackPlan, ServiceRequirementKey, TypeRefIr,
    ValueLifecycleFactResolver, ValueLifecyclePolicyBudget, ValueLifecycleResolverError,
    ValueProvenance,
};
use skiff_runtime_linked_bytecode::{
    ArtifactFunctionKey, FunctionIndex, InterfaceTableIndex, LinkedActorCreateTarget,
    LinkedActorImplementationRef, LinkedActorMethodTarget, LinkedCallableSignature,
    LinkedFrameLayout, LinkedHostEffectAdapterTarget, LinkedInterfaceInstantiation,
    LinkedInterfaceMethodAbiId, LinkedInterfaceRequirementMethod, LinkedInterfaceRequirementTable,
    LinkedInterfaceTable, LinkedInterfaceTableKind, LinkedLocalInterfaceMethod,
    LinkedLocalInterfaceTable, LinkedNativeCallableSignature, LinkedPublicInstanceKey,
    LinkedRemoteInterfaceMethod, LinkedRemoteInterfaceTable, LinkedServiceBoundaryErrorPlan,
    LinkedServiceBoundaryPlan, LinkedServiceBoundaryValue, LinkedServiceCallbackPlan,
    LinkedServiceOperationTarget, LinkedSyntheticCallbackTarget, LinkedTaskTarget,
    LinkedTaskTiming, ServiceOperationIndex, SpecializationKey, TaskTargetIndex,
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
    pub(in crate::bytecode) requirement_keys: BTreeMap<String, InterfaceTableIndex>,
    pub(in crate::bytecode) synthetic_callbacks: Vec<LinkedSyntheticCallbackTarget>,
    pub(in crate::bytecode) synthetic_callback_origins:
        BTreeMap<(PackageBuildId, String), skiff_runtime_linked_bytecode::SyntheticCallbackIndex>,
    pub(in crate::bytecode) task_submit_indices:
        BTreeMap<(PackageBuildId, String), skiff_runtime_linked_bytecode::IntrinsicIndex>,
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
        let (actor_creates, actor_methods) =
            self.link_actor_targets(reachable, indices, frames, type_linker)?;
        let (interface_tables, requirement_keys) = self.link_interface_tables(
            reachable,
            indices,
            frames,
            type_linker,
            &service_operations,
        )?;
        let synthetic_callbacks =
            self.link_synthetic_callbacks(reachable, indices, frames, type_linker)?;
        let (host_effect_adapters, intrinsics, task_submit_indices) =
            self.link_host_and_intrinsics(reachable, indices, frames, type_linker)?;

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
                    (
                        package_build_id,
                        target.artifact_function_key().as_str().to_string(),
                    ),
                    skiff_runtime_linked_bytecode::SyntheticCallbackIndex::new(index as u32),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, BytecodeLinkError>>()?;

        let mut tables = LinkedDispatchTables {
            service_operations,
            actor_creates,
            actor_methods,
            interface_tables,
            requirement_keys,
            synthetic_callbacks,
            synthetic_callback_origins,
            task_submit_indices,
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
        let mut targets = Vec::new();
        let mut seen = BTreeSet::new();
        for reference in reachable {
            let BytecodeRelocation::ServiceOperationRef { service_call } = &reference.relocation
            else {
                continue;
            };
            let caller_package_build_id = reference.specialization.package_build_id().clone();
            let key = ServiceRequirementKey {
                caller_package_build_id: caller_package_build_id.clone(),
                service_requirement_slot: service_call.service_requirement_slot,
            };
            let location = BytecodeLinkLocation::ServiceDependency { key: key.clone() };
            if !seen.insert((key.clone(), service_call.contract_operation_id.clone())) {
                return Err(unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location,
                    "duplicate service operation relocation".to_string(),
                ));
            }
            let dependency = self
                .deployment
                .service_dependencies()
                .get(&key)
                .ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::ConcreteTargetTables,
                        location.clone(),
                        "compiler-emitted service call has no hydrated dependency slot".to_string(),
                    )
                })?;
            if !dependency
                .used_operations()
                .contains(&service_call.contract_operation_id)
            {
                return Err(unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    "compiler-emitted service operation is absent from the dependency slot"
                        .to_string(),
                ));
            }
            let contract = self
                .deployment
                .contract_store()
                .get(dependency.contract())
                .ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::ConcreteTargetTables,
                        location.clone(),
                        "hydrated dependency contract is absent".to_string(),
                    )
                })?;
            if &service_call.expected_protocol_identity != &contract.service_protocol_identity {
                return Err(unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    "service call protocol identity drifts from the hydrated contract".to_string(),
                ));
            }
            let operation = contract
                .operations
                .get(&service_call.contract_operation_id)
                .ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::ConcreteTargetTables,
                        location.clone(),
                        "service call operation is absent from the hydrated contract".to_string(),
                    )
                })?;
            let plan = service_call.boundary_plan();
            validate_service_plan_against_contract(plan, &operation.contract, &location)?;
            let caller_package = self
                .deployment
                .packages()
                .get(&caller_package_build_id)
                .ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::ExactPackageClosure,
                        location.clone(),
                        "service call caller package is absent from the closure".to_string(),
                    )
                })?;
            let linked_plan = self.link_service_boundary_plan(
                plan,
                caller_package,
                &reference.specialization,
                type_linker,
                location.clone(),
            )?;
            let signature = link_service_signature(plan, &linked_plan, type_linker, &location)?;
            let index = u32::try_from(targets.len()).map_err(|_| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    "service operation table exceeds u32::MAX".to_string(),
                )
            })?;
            targets.push(LinkedServiceOperationTarget::new(
                ServiceOperationIndex::new(index),
                key,
                service_call.contract_operation_id.clone(),
                service_call.expected_protocol_identity.clone(),
                signature,
                linked_plan,
            ));
        }
        Ok(targets)
    }

    fn link_service_boundary_plan(
        &self,
        plan: &ServiceBoundaryPlan,
        caller: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        type_linker: &mut TypeLinker<'_>,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedServiceBoundaryPlan, BytecodeLinkError> {
        let arguments = plan
            .arguments
            .iter()
            .map(|value| {
                self.link_service_boundary_value(
                    value,
                    caller,
                    specialization,
                    type_linker,
                    location.clone(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let results = plan
            .results
            .iter()
            .map(|value| {
                self.link_service_boundary_value(
                    value,
                    caller,
                    specialization,
                    type_linker,
                    location.clone(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let fallback = self.link_service_boundary_value_from_contract_type(
            &plan.error.fallback_contract_type,
            &plan.error.fallback,
            plan.error.transfer,
            &plan.error.drop,
            &plan.error.source,
            caller,
            specialization,
            type_linker,
            location.clone(),
        )?;
        let linked_error = LinkedServiceBoundaryErrorPlan::new(plan.error.clone(), fallback);
        let stream_item = plan
            .stream_item
            .as_deref()
            .map(|value| {
                self.link_service_boundary_value(
                    value,
                    caller,
                    specialization,
                    type_linker,
                    location.clone(),
                )
            })
            .transpose()?;
        let callbacks = match &plan.callbacks {
            ServiceCallbackPlan::None => LinkedServiceCallbackPlan::None,
            ServiceCallbackPlan::RequestScoped {
                interface_types,
                lifetime,
                expiration_error,
            } => LinkedServiceCallbackPlan::RequestScoped {
                interface_types: interface_types.clone().into_boxed_slice(),
                lifetime: *lifetime,
                expiration_error: *expiration_error,
            },
            ServiceCallbackPlan::Unsupported { .. } => {
                return Err(unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    "unsupported callback service boundary surface".to_string(),
                ))
            }
        };
        Ok(LinkedServiceBoundaryPlan::new(
            arguments,
            results,
            linked_error,
            stream_item,
            callbacks,
        ))
    }

    fn link_service_boundary_value(
        &self,
        value: &BoundaryValueFact,
        caller: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        type_linker: &mut TypeLinker<'_>,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedServiceBoundaryValue, BytecodeLinkError> {
        self.link_service_boundary_value_from_contract_type(
            &value.contract_type,
            &value.value_plan,
            value.transfer,
            &value.drop,
            &value.source,
            caller,
            specialization,
            type_linker,
            location,
        )
    }

    fn link_service_boundary_value_from_contract_type(
        &self,
        contract_type: &ContractTypeRef,
        value_plan: &BoundaryValuePlan,
        transfer: BoundaryTransfer,
        drop: &BoundaryDropPlan,
        source: &ValueProvenance,
        caller: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        type_linker: &mut TypeLinker<'_>,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedServiceBoundaryValue, BytecodeLinkError> {
        let type_ref = contract_type_ref_to_ir(contract_type, &location)?;
        let caller_type = type_linker.intern_concrete_type(
            caller,
            specialization,
            &type_ref,
            &BTreeMap::new(),
            location.clone(),
        )?;
        let linked_type_ref = type_linker
            .linked_type_ref(caller_type)
            .cloned()
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    format!(
                        "service boundary contract type {contract_type:?} has no linked type row"
                    ),
                )
            })?;
        Ok(LinkedServiceBoundaryValue::new(
            contract_type.clone(),
            value_plan.clone(),
            transfer,
            drop.clone(),
            source.clone(),
            caller_type,
            linked_type_ref,
        ))
    }

    fn link_actor_targets(
        &self,
        reachable: &[ReachableRelocation],
        indices: &BTreeMap<SpecializationKey, FunctionIndex>,
        frames: &[LinkedFrameLayout],
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<(Vec<LinkedActorCreateTarget>, Vec<LinkedActorMethodTarget>), BytecodeLinkError>
    {
        let mut creates = Vec::new();
        let mut methods = Vec::new();
        for package in self.deployment.packages().values() {
            for actor in &package.artifact().actor_implementations {
                let actor_abi = package
                    .artifact()
                    .implementation_links
                    .types
                    .values()
                    .find(|export| {
                        export.file.module_path == actor.actor.module_path
                            && export
                                .actor
                                .as_ref()
                                .is_some_and(|abi| abi.abi.actor_name == actor.actor.symbol)
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
                if let Some(create) = &actor.create {
                    let callable = &create.package_callable_id;
                    let key = self.key_for_receiver_callable(package, callable, type_linker)?;
                    let function = indices.get(&key).copied().ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::ConcreteTargetTables,
                            self.package_location(package),
                            format!(
                                "actor create {:?} is absent from the closure",
                                create.method_identity
                            ),
                        )
                    })?;
                    let location = BytecodeLinkLocation::Function {
                        package: Box::new(package.reference().clone()),
                        function_key: key.artifact_function_key().as_str().to_string(),
                    };
                    let frame = frames.get(function.get() as usize).ok_or_else(|| {
                        BytecodeLinkError::ImplementationUnavailable {
                            obligation: BytecodeLinkObligation::FrameAndValueTransferPlan,
                            location: location.clone(),
                        }
                    })?;
                    let effects = exact_actor_effects(
                        package.artifact().callable_semantic_facts.get(callable),
                        location.clone(),
                    )?;
                    let signature = frame_signature(frame, effects, location)?;
                    creates.push(LinkedActorCreateTarget::new(
                        skiff_runtime_linked_bytecode::ActorCreateIndex::new(creates.len() as u32),
                        actor_ref.clone(),
                        create.method_identity.clone(),
                        function,
                        signature,
                    ));
                }
                let has_reachable_method = reachable.iter().any(|reference| {
                    matches!(
                        &reference.relocation,
                        BytecodeRelocation::ActorMethodRef {
                            actor: target_actor,
                            actor_implementation_identity,
                            ..
                        } if reference.specialization.package_build_id()
                            == &package.reference().package_build_id
                            && target_actor == &actor.actor
                            && actor_implementation_identity == &actor.actor_implementation_identity
                    )
                });
                if !has_reachable_method {
                    continue;
                }
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
                    let location = BytecodeLinkLocation::Function {
                        package: Box::new(package.reference().clone()),
                        function_key: key.artifact_function_key().as_str().to_string(),
                    };
                    let frame = frames.get(function.get() as usize).ok_or_else(|| {
                        BytecodeLinkError::ImplementationUnavailable {
                            obligation: BytecodeLinkObligation::FrameAndValueTransferPlan,
                            location: location.clone(),
                        }
                    })?;
                    let effects = exact_actor_effects(
                        package.artifact().callable_semantic_facts.get(callable),
                        location.clone(),
                    )?;
                    let signature = frame_signature(frame, effects, location)?;
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

fn validate_service_plan_against_contract(
    plan: &ServiceBoundaryPlan,
    contract: &skiff_artifact_model::BoundaryOperationContract,
    location: &BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    if plan.arguments.len() != contract.parameters.len() {
        return Err(unsatisfied(
            BytecodeLinkObligation::ConcreteTargetTables,
            location.clone(),
            format!(
                "service boundary argument count {} differs from contract {}",
                plan.arguments.len(),
                contract.parameters.len()
            ),
        ));
    }
    for (index, (plan_value, parameter)) in plan
        .arguments
        .iter()
        .zip(contract.parameters.iter())
        .enumerate()
    {
        if plan_value.contract_type != parameter.ty || plan_value.value_plan != parameter.value_plan
        {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                format!("service boundary argument #{index} drifts from the hydrated contract"),
            ));
        }
    }
    let expected_result_count = if contract.return_value.ty == ContractTypeRef::builtin("void") {
        0
    } else {
        1
    };
    if plan.results.len() != expected_result_count {
        return Err(unsatisfied(
            BytecodeLinkObligation::ConcreteTargetTables,
            location.clone(),
            "service boundary result count drifts from the hydrated contract".to_string(),
        ));
    }
    if let Some(result) = plan.results.first() {
        if result.contract_type != contract.return_value.ty
            || result.value_plan != contract.return_value.value_plan
        {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                "service boundary result drifts from the hydrated contract".to_string(),
            ));
        }
    }
    if !matches!(
        contract.stream,
        skiff_artifact_model::BoundaryStreamContract::Unary
    ) || plan.stream_item.is_some()
    {
        return Err(unsatisfied(
            BytecodeLinkObligation::ConcreteTargetTables,
            location.clone(),
            "unsupported stream or callback service boundary surface".to_string(),
        ));
    }
    match (&plan.callbacks, &contract.callbacks) {
        (ServiceCallbackPlan::None, skiff_artifact_model::BoundaryCallbackContract::None) => {}
        (
            ServiceCallbackPlan::RequestScoped {
                interface_types,
                lifetime,
                expiration_error,
            },
            skiff_artifact_model::BoundaryCallbackContract::RequestScoped {
                interface_types: contract_interface_types,
                lifetime: contract_lifetime,
                expiration_error: contract_expiration_error,
            },
        ) => {
            if interface_types != contract_interface_types
                || lifetime != contract_lifetime
                || expiration_error != contract_expiration_error
            {
                return Err(unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    "service callback plan drifts from the hydrated contract".to_string(),
                ));
            }
        }
        _ => {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                "service callback plan drifts from the hydrated contract".to_string(),
            ))
        }
    }
    if plan.effects.effects_for_boundary().is_err() {
        return Err(unsatisfied(
            BytecodeLinkObligation::CallableEffectPlan,
            location.clone(),
            "service boundary plan has an unknown effect summary".to_string(),
        ));
    }
    if !matches!(
        plan.error.policy,
        skiff_artifact_model::BoundaryErrorPolicy::DynamicPublicSchema {
            admission: skiff_artifact_model::BoundaryErrorAdmission::PublicNameableSchemaClosed,
            fallback_identity:
                skiff_artifact_model::BoundaryErrorFallbackIdentity::StdServiceInternalError,
        }
    ) {
        return Err(unsatisfied(
            BytecodeLinkObligation::ConcreteTargetTables,
            location.clone(),
            "service boundary error policy drifts from the canonical open channel".to_string(),
        ));
    }
    Ok(())
}

fn link_service_signature(
    plan: &ServiceBoundaryPlan,
    linked_plan: &LinkedServiceBoundaryPlan,
    type_linker: &mut TypeLinker<'_>,
    location: &BytecodeLinkLocation,
) -> Result<LinkedCallableSignature, BytecodeLinkError> {
    let parameter_types = linked_plan
        .arguments()
        .iter()
        .map(LinkedServiceBoundaryValue::caller_type)
        .collect::<Vec<_>>();
    let parameter_modes = vec![ParamModeIr::Value; parameter_types.len()];
    let parameter_plans = linked_plan
        .arguments()
        .iter()
        .map(|value| {
            type_linker
                .linked_type_plan(value.caller_type())
                .cloned()
                .ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::FrameAndValueTransferPlan,
                        location.clone(),
                        format!(
                            "service boundary caller type {} has no linked transfer plan",
                            value.caller_type().get()
                        ),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let result_types = linked_plan
        .results()
        .iter()
        .map(LinkedServiceBoundaryValue::caller_type)
        .collect::<Vec<_>>();
    let result_plans = linked_plan
        .results()
        .iter()
        .map(|value| {
            type_linker
                .linked_type_plan(value.caller_type())
                .cloned()
                .ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::FrameAndValueTransferPlan,
                        location.clone(),
                        format!(
                            "service boundary result type {} has no linked transfer plan",
                            value.caller_type().get()
                        ),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    LinkedCallableSignature::new(
        parameter_types.into_boxed_slice(),
        parameter_modes.into_boxed_slice(),
        parameter_plans.into_boxed_slice(),
        result_types.into_boxed_slice(),
        result_plans.into_boxed_slice(),
        plan.effects.clone(),
    )
    .map_err(|error| {
        unsatisfied(
            BytecodeLinkObligation::FrameAndValueTransferPlan,
            location.clone(),
            error.to_string(),
        )
    })
}

fn contract_type_ref_to_ir(
    ty: &ContractTypeRef,
    location: &BytecodeLinkLocation,
) -> Result<TypeRefIr, BytecodeLinkError> {
    let fail = |kind: &str| {
        unsatisfied(
            BytecodeLinkObligation::ConcreteTargetTables,
            location.clone(),
            format!("service boundary contract type {kind} is unsupported"),
        )
    };
    match ty {
        ContractTypeRef::Builtin { name, arguments } => {
            let args = arguments
                .iter()
                .map(|argument| contract_type_ref_to_ir(argument, location))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(TypeRefIr::Builtin {
                name: name.clone(),
                args,
            })
        }
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => Ok(TypeRefIr::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.clone(),
        }),
        ContractTypeRef::Record { fields } => {
            let fields = fields
                .iter()
                .map(|(name, field)| Ok((name.clone(), contract_type_ref_to_ir(field, location)?)))
                .collect::<Result<BTreeMap<_, _>, BytecodeLinkError>>()?;
            Ok(TypeRefIr::Record { fields })
        }
        ContractTypeRef::StructuralUnion { variants } => {
            let items = variants
                .iter()
                .map(|variant| contract_type_ref_to_ir(variant, location))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(TypeRefIr::Union { items })
        }
        ContractTypeRef::Nullable { inner } => Ok(TypeRefIr::Nullable {
            inner: Box::new(contract_type_ref_to_ir(inner, location)?),
        }),
        ContractTypeRef::Literal { value } => {
            let ContractLiteral::String { value } = value;
            Ok(TypeRefIr::Literal {
                value: LiteralIr::String {
                    value: value.clone(),
                },
            })
        }
        ContractTypeRef::TypeParam { .. } => Err(fail("type parameter")),
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            let interface_ir = match interface.as_ref() {
                ContractTypeRef::PackageSchema {
                    package_id,
                    stable_schema_key,
                    ..
                } => TypeRefIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::PackageId {
                            package_id: package_id.clone(),
                        },
                        symbol_path: stable_schema_key.clone(),
                        abi_expectation: None,
                    },
                },
                other => contract_type_ref_to_ir(other, location)?,
            };
            let interface_abi_id =
                skiff_canonical_json::canonical_json_bytes(&interface_ir).map_err(|error| {
                    unsatisfied(
                        BytecodeLinkObligation::ConcreteTargetTables,
                        location.clone(),
                        format!(
                            "service boundary callback interface identity cannot be canonicalized: {error}"
                        ),
                    )
                })?;
            let interface_abi_id = String::from_utf8(interface_abi_id).map_err(|error| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    format!("service boundary callback interface identity is not UTF-8: {error}"),
                )
            })?;
            let canonical_type_args = arguments
                .iter()
                .map(|argument| contract_type_ref_to_ir(argument, location))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(TypeRefIr::AnyInterface {
                interface: InterfaceInstantiationRef {
                    interface_abi_id,
                    canonical_type_args,
                },
            })
        }
    }
}

fn remote_method_signature_matches_operation(
    method: &LinkedCallableSignature,
    operation: &LinkedCallableSignature,
) -> bool {
    method.parameter_types().len() == operation.parameter_types().len().saturating_add(1)
        && method.result_types() == operation.result_types()
        && method.parameter_types().get(1..) == Some(operation.parameter_types())
        && method.parameter_modes().get(1..) == Some(operation.parameter_modes())
        && method.parameter_plans().get(1..) == Some(operation.parameter_plans())
        && method.result_plans() == operation.result_plans()
        && method.effect_summary() == operation.effect_summary()
}

fn replace_self_type(ty: &TypeRefIr, receiver: &TypeRefIr) -> TypeRefIr {
    match ty {
        TypeRefIr::Builtin { name, args } if name == "Self" && args.is_empty() => receiver.clone(),
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| replace_self_type(arg, receiver))
                .collect(),
        },
        TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
            base: replace_self_nominal_base(base, receiver),
            arguments: arguments
                .iter()
                .map(|arg| replace_self_type(arg, receiver))
                .collect(),
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, field)| (name.clone(), replace_self_type(field, receiver)))
                .collect(),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| replace_self_type(item, receiver))
                .collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(replace_self_type(inner, receiver)),
        },
        TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| replace_self_type(arg, receiver))
                    .collect(),
            },
        },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| skiff_artifact_model::FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: replace_self_type(&param.ty, receiver),
                })
                .collect(),
            return_type: Box::new(replace_self_type(return_type, receiver)),
        },
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => ty.clone(),
    }
}

fn replace_self_nominal_base(
    base: &skiff_artifact_model::NominalTypeRefBaseIr,
    receiver: &TypeRefIr,
) -> skiff_artifact_model::NominalTypeRefBaseIr {
    match base {
        skiff_artifact_model::NominalTypeRefBaseIr::LocalType { .. }
        | skiff_artifact_model::NominalTypeRefBaseIr::PublicationType { .. }
        | skiff_artifact_model::NominalTypeRefBaseIr::ServiceSymbol { .. }
        | skiff_artifact_model::NominalTypeRefBaseIr::PackageSymbol { .. }
        | skiff_artifact_model::NominalTypeRefBaseIr::PackageSchema { .. } => base.clone(),
    }
}

fn frame_signature(
    frame: &LinkedFrameLayout,
    effects: CallableEffectSummary,
    location: BytecodeLinkLocation,
) -> Result<LinkedCallableSignature, BytecodeLinkError> {
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
        effects,
    )
    .map_err(|error| {
        unsatisfied(
            BytecodeLinkObligation::FrameAndValueTransferPlan,
            location,
            error.to_string(),
        )
    })
}

fn exact_actor_effects(
    facts: Option<&skiff_artifact_model::CallableSemanticFacts>,
    location: BytecodeLinkLocation,
) -> Result<CallableEffectSummary, BytecodeLinkError> {
    let Some(facts) = facts else {
        return Err(BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::CallableEffectPlan,
            location,
        });
    };
    if matches!(&facts.effects, CallableEffectSummary::Unknown { .. })
        || matches!(
            &facts.effects,
            CallableEffectSummary::Analyzed { effects } if effects.invokes_unknown_target
        )
        || matches!(
            &facts.provenance,
            skiff_artifact_model::CallableProvenanceSummary::Unknown { .. }
        )
        || facts
            .resolved_call_targets
            .values()
            .any(|target| matches!(target, skiff_artifact_model::CallableTargetFact::Unknown))
    {
        return Err(BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::CallableEffectPlan,
            location,
        });
    }
    Ok(facts.effects.clone())
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
                            deployment_revision: skiff_artifact_model::DeploymentRevision::new(
                                "revision:targets",
                            ),
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
        let mut actor_create_seen = BTreeSet::new();
        for target in &self.actor_creates {
            let key = (
                target.owner_package_build_id().clone(),
                target.actor().module_path.clone(),
                target.actor().symbol.clone(),
                target.actor_implementation_identity().clone(),
                target.create_identity().clone(),
            );
            if !actor_create_seen.insert(key) {
                return Err(unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    BytecodeLinkLocation::Deployment {
                        deployment: Box::new(skiff_artifact_model::ServiceDeploymentRef {
                            service_id: "service".to_string(),
                            contract_version: "1.0.0".to_string(),
                            deployment_revision: skiff_artifact_model::DeploymentRevision::new(
                                "revision:targets",
                            ),
                            deployment_artifact_identity:
                                skiff_artifact_model::DeploymentArtifactIdentity::new(
                                    "deployment:targets",
                                ),
                        }),
                    },
                    "duplicate actor create target".to_string(),
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
                            deployment_revision: skiff_artifact_model::DeploymentRevision::new(
                                "revision:targets",
                            ),
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
        CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary,
        CallableRegistryTypeExpression, CallableSemanticFacts, CallableTargetFact,
        HostEffectReference, HostEffectRegistryEntry, HostEffectSignature, NativeTarget,
        PackageArtifactRef, PackageBuildId, PackageLocalAbiIdentity, PackageRefIr,
        PackageSymbolRef, ParamModeIr, PendingEffectCategory, TypeRefIr, ValueDropPlan,
        ValueTransferPlan,
    };

    use super::{
        exact_actor_effects, executable_identity_for, registry_entry_for,
        validate_host_effect_authority,
    };
    use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation};

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

    fn fixture_type(ty: &CallableRegistryTypeExpression) -> TypeRefIr {
        match ty {
            CallableRegistryTypeExpression::TypeParameter { .. } => TypeRefIr::builtin("number"),
            CallableRegistryTypeExpression::Builtin { name, arguments } => TypeRefIr::Builtin {
                name: name.clone(),
                args: arguments.iter().map(fixture_type).collect(),
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
                    abi_expectation: Some("test-abi".to_string()),
                },
            },
        }
    }

    fn signature_from(entry: &HostEffectRegistryEntry) -> HostEffectSignature {
        let parameter_types = entry
            .signature
            .parameter_types
            .iter()
            .map(fixture_type)
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
            .map(fixture_type)
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
    fn pinned_registry_owns_the_sleep_executable_identity() {
        let effect = sleep_reference();
        let entry = registry_entry_for(&effect, &location()).expect("pinned sleep resolves");
        assert_eq!(entry.binding_key, "std.time.sleep");
        assert_eq!(entry.signature.parameter_types.len(), 1);
        assert_eq!(entry.signature.result_types.len(), 0);
        assert!(entry.signature.effects.may_pending());
        assert_eq!(
            executable_identity_for(entry, &location()).unwrap(),
            skiff_artifact_model::HostEffectExecutorIdentity::Sleep
        );
    }

    #[test]
    fn only_the_three_exact_registry_rows_mint_executor_identity() {
        use skiff_artifact_model::HostEffectExecutorIdentity::{
            HttpClientRequest, HttpClientStream, Sleep,
        };

        for (binding_key, expected) in [
            ("std.time.sleep", Sleep),
            ("std.http.client.request", HttpClientRequest),
            ("std.http.client.stream", HttpClientStream),
        ] {
            let entry = skiff_artifact_model::host_effect_registry()
                .entries()
                .iter()
                .find(|entry| entry.binding_key == binding_key)
                .unwrap();
            assert_eq!(
                executable_identity_for(entry, &location()).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn descriptive_registry_rows_cannot_mint_executor_identity() {
        for binding_key in ["std.http.client.sse", "core.date.now"] {
            let entry = skiff_artifact_model::host_effect_registry()
                .entries()
                .iter()
                .find(|entry| entry.binding_key == binding_key)
                .unwrap();
            assert!(
                executable_identity_for(entry, &location()).is_err(),
                "{binding_key} must not mint bytecode execution authority"
            );
        }
    }

    #[test]
    fn registry_alias_is_not_an_executable_target_identity() {
        let entry = skiff_artifact_model::host_effect_registry()
            .entries()
            .iter()
            .find(|entry| !entry.aliases.is_empty() && entry.metadata.fields.is_empty())
            .expect("pinned registry retains a descriptive alias negative");
        let mut effect = reference_for(entry);
        let (namespace, symbol) = entry.aliases[0]
            .split_once('.')
            .expect("test alias is namespace-qualified");
        effect.target.namespace = namespace.to_string();
        effect.target.symbol = symbol.to_string();
        assert!(
            validate_host_effect_authority(&effect, &location()).is_err(),
            "registry aliases are descriptive lookup facts, not exact executable target identity"
        );
    }

    #[test]
    fn exact_host_target_and_signature_abi_are_admitted() {
        let effect = sleep_reference();
        validate_host_effect_authority(&effect, &location())
            .expect("exact canonical sleep ABI validates against the frozen registry");
    }

    #[test]
    fn wrong_host_arity_is_rejected() {
        let mut effect = sleep_reference();
        let duplicated = effect.signature.parameter_types[0].clone();
        effect.signature.parameter_types.push(duplicated.clone());
        effect.signature.parameter_modes.push(ParamModeIr::Value);
        effect
            .signature
            .parameter_plans
            .push(ValueTransferPlan::FromType { ty: duplicated });
        assert!(validate_host_effect_authority(&effect, &location()).is_err());
    }

    #[test]
    fn wrong_host_parameter_type_is_rejected() {
        let mut effect = sleep_reference();
        effect.signature.parameter_types[0] = TypeRefIr::builtin("integer");
        assert!(validate_host_effect_authority(&effect, &location()).is_err());
    }

    #[test]
    fn compiler_owned_concrete_plan_is_not_reconstructed_from_registry() {
        let mut effect = sleep_reference();
        effect.signature.parameter_plans[0] = ValueTransferPlan::SnapshotShare {
            drop: ValueDropPlan::Trivial,
        };
        validate_host_effect_authority(&effect, &location())
            .expect("registry ABI matching does not derive compiler-owned transfer plans");
    }

    #[test]
    fn wrong_host_effects_are_rejected() {
        let mut effect = sleep_reference();
        effect.signature.effects.pending_effect_categories =
            vec![PendingEffectCategory::HostEffect];
        assert!(validate_host_effect_authority(&effect, &location()).is_err());
    }

    #[test]
    fn unknown_and_missing_binding_keys_fail_closed() {
        let mut effect = sleep_reference();
        effect.target.binding_key = Some("fixture.drift".to_string());
        assert!(registry_entry_for(&effect, &location()).is_err());
        effect.target.binding_key = None;
        assert!(registry_entry_for(&effect, &location()).is_err());
    }

    fn actor_facts(effects: CallableEffectSummary) -> CallableSemanticFacts {
        CallableSemanticFacts {
            effects,
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

    #[test]
    fn actor_signature_consumes_exact_compiler_effect_facts() {
        let facts = actor_facts(CallableEffectSummary::Analyzed {
            effects: no_effects(),
        });
        assert_eq!(
            exact_actor_effects(Some(&facts), location()).unwrap(),
            facts.effects
        );
    }

    #[test]
    fn actor_signature_never_synthesizes_unknown_effect_facts() {
        assert!(matches!(
            exact_actor_effects(None, location()),
            Err(BytecodeLinkError::ImplementationUnavailable {
                obligation: BytecodeLinkObligation::CallableEffectPlan,
                ..
            })
        ));

        let unknown = actor_facts(CallableEffectSummary::analysis_pending());
        assert!(matches!(
            exact_actor_effects(Some(&unknown), location()),
            Err(BytecodeLinkError::ImplementationUnavailable {
                obligation: BytecodeLinkObligation::CallableEffectPlan,
                ..
            })
        ));

        let mut unknown_target = actor_facts(CallableEffectSummary::Analyzed {
            effects: no_effects(),
        });
        unknown_target
            .resolved_call_targets
            .insert(0, CallableTargetFact::Unknown);
        assert!(matches!(
            exact_actor_effects(Some(&unknown_target), location()),
            Err(BytecodeLinkError::ImplementationUnavailable {
                obligation: BytecodeLinkObligation::CallableEffectPlan,
                ..
            })
        ));

        let mut unknown_effect_target = no_effects();
        unknown_effect_target.invokes_unknown_target = true;
        let unknown_effect_target = actor_facts(CallableEffectSummary::Analyzed {
            effects: unknown_effect_target,
        });
        assert!(matches!(
            exact_actor_effects(Some(&unknown_effect_target), location()),
            Err(BytecodeLinkError::ImplementationUnavailable {
                obligation: BytecodeLinkObligation::CallableEffectPlan,
                ..
            })
        ));
    }
}

impl DeploymentLinker<'_> {
    fn link_interface_tables(
        &self,
        reachable: &[ReachableRelocation],
        indices: &BTreeMap<SpecializationKey, FunctionIndex>,
        _frames: &[LinkedFrameLayout],
        type_linker: &mut TypeLinker<'_>,
        service_operations: &[LinkedServiceOperationTarget],
    ) -> Result<
        (
            Vec<LinkedInterfaceTable>,
            BTreeMap<String, InterfaceTableIndex>,
        ),
        BytecodeLinkError,
    > {
        let mut tables = Vec::<LinkedInterfaceTable>::new();
        let mut requirement_keys = BTreeMap::<String, InterfaceTableIndex>::new();
        for reference in reachable {
            match &reference.relocation {
                BytecodeRelocation::LocalInterfaceRef { interface } => {
                    let package = self
                        .deployment
                        .packages()
                        .get(reference.specialization.package_build_id())
                        .ok_or_else(|| {
                            unsatisfied(
                                BytecodeLinkObligation::ExactPackageClosure,
                                self.deployment_location(),
                                "local interface relocation owner is absent".to_string(),
                            )
                        })?;
                    let location = self.reachable_relocation_location(reference);
                    let linked_interface = self.link_interface_instantiation(
                        package,
                        &reference.specialization,
                        &interface.interface,
                        type_linker,
                        location.clone(),
                    )?;
                    let concrete_type = type_linker.intern_concrete_type(
                        package,
                        &reference.specialization,
                        &interface.concrete_type,
                        &BTreeMap::new(),
                        location.clone(),
                    )?;
                    let mut methods = Vec::with_capacity(interface.methods.len());
                    for method in &interface.methods {
                        let key = self.key_for_local_interface_method(
                            package,
                            method,
                            type_linker,
                            location.clone(),
                        )?;
                        let function = indices.get(&key).copied().ok_or_else(|| {
                            unsatisfied(
                                BytecodeLinkObligation::ConcreteTargetTables,
                                location.clone(),
                                format!(
                                    "local interface method {} targets an absent closure function",
                                    method.function_key
                                ),
                            )
                        })?;
                        let signature = self.link_interface_signature(
                            package,
                            &reference.specialization,
                            &interface.concrete_type,
                            &method.signature,
                            &method.effects,
                            type_linker,
                            location.clone(),
                        )?;
                        let method_abi =
                            LinkedInterfaceMethodAbiId::parse(method.method_abi_id.clone())
                                .map_err(|error| {
                                    unsatisfied(
                                        BytecodeLinkObligation::ConcreteTargetTables,
                                        location.clone(),
                                        error.to_string(),
                                    )
                                })?;
                        methods.push(
                            LinkedLocalInterfaceMethod::new(
                                method.slot,
                                method.method_name.clone(),
                                method_abi,
                                signature,
                                function,
                                method.receiver_call_abi,
                            )
                            .map_err(|error| {
                                unsatisfied(
                                    BytecodeLinkObligation::ConcreteTargetTables,
                                    location.clone(),
                                    error.to_string(),
                                )
                            })?,
                        );
                    }
                    let local =
                        LinkedLocalInterfaceTable::new(concrete_type, methods.into_boxed_slice())
                            .map_err(|error| {
                            unsatisfied(
                                BytecodeLinkObligation::ConcreteTargetTables,
                                location.clone(),
                                error.to_string(),
                            )
                        })?;
                    if let Some(existing) = tables.iter().find(|table| {
                        table.interface().artifact() == &interface.interface
                            && matches!(
                                table.kind(),
                                LinkedInterfaceTableKind::Local(row)
                                    if row.concrete_type() == local.concrete_type()
                            )
                    }) {
                        let LinkedInterfaceTableKind::Local(row) = existing.kind() else {
                            unreachable!("local table predicate only matches Local");
                        };
                        if row != &local {
                            return Err(unsatisfied(
                                BytecodeLinkObligation::ConcreteTargetTables,
                                location,
                                "duplicate local interface table facts drift".to_string(),
                            ));
                        }
                        continue;
                    }
                    tables.push(LinkedInterfaceTable::new(
                        InterfaceTableIndex::new(tables.len() as u32),
                        linked_interface,
                        LinkedInterfaceTableKind::Local(local),
                    ));
                }
                BytecodeRelocation::InterfaceRequirementRef { interface, methods } => {
                    let package = self
                        .deployment
                        .packages()
                        .get(reference.specialization.package_build_id())
                        .ok_or_else(|| {
                            unsatisfied(
                                BytecodeLinkObligation::ExactPackageClosure,
                                self.deployment_location(),
                                "interface requirement relocation owner is absent".to_string(),
                            )
                        })?;
                    let location = self.reachable_relocation_location(reference);
                    let linked_interface = self.link_interface_instantiation(
                        package,
                        &reference.specialization,
                        interface,
                        type_linker,
                        location.clone(),
                    )?;
                    let receiver_type = TypeRefIr::AnyInterface {
                        interface: interface.clone(),
                    };
                    let mut linked_methods = Vec::with_capacity(methods.len());
                    for method in methods {
                        let signature = self.link_interface_signature(
                            package,
                            &reference.specialization,
                            &receiver_type,
                            &method.signature,
                            &method.effects,
                            type_linker,
                            location.clone(),
                        )?;
                        let method_abi =
                            LinkedInterfaceMethodAbiId::parse(method.method_abi_id.clone())
                                .map_err(|error| {
                                    unsatisfied(
                                        BytecodeLinkObligation::ConcreteTargetTables,
                                        location.clone(),
                                        error.to_string(),
                                    )
                                })?;
                        linked_methods.push(LinkedInterfaceRequirementMethod::new(
                            method.slot,
                            method_abi,
                            signature,
                        ));
                    }
                    let requirement =
                        LinkedInterfaceRequirementTable::new(linked_methods.into_boxed_slice())
                            .map_err(|error| {
                                unsatisfied(
                                    BytecodeLinkObligation::ConcreteTargetTables,
                                    location.clone(),
                                    error.to_string(),
                                )
                            })?;
                    let kind = if reference.opcode == Opcode::InvokeCallback {
                        LinkedInterfaceTableKind::Callback(requirement)
                    } else {
                        LinkedInterfaceTableKind::Requirement(requirement)
                    };
                    let key = self.requirement_table_key(interface, methods)?;
                    if let Some(existing) = requirement_keys.get(&key) {
                        if tables
                            .get(existing.get() as usize)
                            .is_none_or(|table| table.interface().artifact() != interface)
                        {
                            return Err(unsatisfied(
                                BytecodeLinkObligation::ConcreteTargetTables,
                                location.clone(),
                                "interface requirement key maps to an absent table".to_string(),
                            ));
                        }
                        continue;
                    }
                    if let Some(existing) = tables.iter().find(|table| {
                        table.interface().artifact() == interface
                            && matches!(
                                (table.kind(), &kind),
                                (LinkedInterfaceTableKind::Requirement(a), LinkedInterfaceTableKind::Requirement(b))
                                    | (LinkedInterfaceTableKind::Callback(a), LinkedInterfaceTableKind::Callback(b))
                                    if a == b
                            )
                    }) {
                        let _ = existing;
                        continue;
                    }
                    tables.push(LinkedInterfaceTable::new(
                        InterfaceTableIndex::new(tables.len() as u32),
                        linked_interface,
                        kind,
                    ));
                    let index = InterfaceTableIndex::new(tables.len() as u32 - 1);
                    requirement_keys.insert(key, index);
                }
                BytecodeRelocation::RemoteInterfaceRef { interface } => {
                    let package = self
                        .deployment
                        .packages()
                        .get(reference.specialization.package_build_id())
                        .ok_or_else(|| {
                            unsatisfied(
                                BytecodeLinkObligation::ExactPackageClosure,
                                self.deployment_location(),
                                "remote interface relocation owner is absent".to_string(),
                            )
                        })?;
                    let location = self.reachable_relocation_location(reference);
                    let key = ServiceRequirementKey {
                        caller_package_build_id: reference
                            .specialization
                            .package_build_id()
                            .clone(),
                        service_requirement_slot: interface.service_requirement_slot,
                    };
                    let dependency = self
                        .deployment
                        .service_dependencies()
                        .get(&key)
                        .ok_or_else(|| {
                            unsatisfied(
                                BytecodeLinkObligation::ConcreteTargetTables,
                                location.clone(),
                                "compiler-emitted remote interface has no hydrated dependency slot"
                                    .to_string(),
                            )
                        })?;
                    if dependency.contract().service_protocol_identity
                        != interface.callee_protocol_identity
                    {
                        return Err(unsatisfied(
                            BytecodeLinkObligation::ConcreteTargetTables,
                            location.clone(),
                            "remote interface protocol drifts from the hydrated contract"
                                .to_string(),
                        ));
                    }
                    let linked_interface = self.link_interface_instantiation(
                        package,
                        &reference.specialization,
                        &interface.interface,
                        type_linker,
                        location.clone(),
                    )?;
                    let mut methods = Vec::with_capacity(interface.methods.len());
                    for method in &interface.methods {
                        if !dependency
                            .used_operations()
                            .contains(&method.contract_operation_id)
                        {
                            return Err(unsatisfied(
                                BytecodeLinkObligation::ConcreteTargetTables,
                                location.clone(),
                                "remote interface operation is absent from the dependency slot"
                                    .to_string(),
                            ));
                        }
                        let operation = service_operations
                            .iter()
                            .find(|operation| {
                                operation.service_requirement_key() == &key
                                    && operation.contract_operation_id()
                                        == &method.contract_operation_id
                                    && operation.expected_protocol_identity()
                                        == &interface.callee_protocol_identity
                            })
                            .ok_or_else(|| {
                                unsatisfied(
                                    BytecodeLinkObligation::ConcreteTargetTables,
                                    location.clone(),
                                    "remote interface method has no exact linked service operation"
                                        .to_string(),
                                )
                            })?;
                        let signature = self.link_interface_signature(
                            package,
                            &reference.specialization,
                            &TypeRefIr::AnyInterface {
                                interface: interface.interface.clone(),
                            },
                            &method.signature,
                            operation.signature().effect_summary(),
                            type_linker,
                            location.clone(),
                        )?;
                        if !remote_method_signature_matches_operation(
                            &signature,
                            operation.signature(),
                        ) {
                            return Err(unsatisfied(
                                BytecodeLinkObligation::ConcreteTargetTables,
                                location.clone(),
                                "remote interface method signature drifts from its service operation"
                                    .to_string(),
                            ));
                        }
                        let method_abi =
                            LinkedInterfaceMethodAbiId::parse(method.method_abi_id.clone())
                                .map_err(|error| {
                                    unsatisfied(
                                        BytecodeLinkObligation::ConcreteTargetTables,
                                        location.clone(),
                                        error.to_string(),
                                    )
                                })?;
                        methods.push(
                            LinkedRemoteInterfaceMethod::new(
                                method.slot,
                                method_abi,
                                signature,
                                method.contract_operation_id.clone(),
                            )
                            .with_service_operation(operation.index()),
                        );
                    }
                    let public_instance_key =
                        LinkedPublicInstanceKey::parse(interface.public_instance_key.clone())
                            .map_err(|error| {
                                unsatisfied(
                                    BytecodeLinkObligation::ConcreteTargetTables,
                                    location.clone(),
                                    error.to_string(),
                                )
                            })?;
                    let remote = LinkedRemoteInterfaceTable::new(
                        key,
                        public_instance_key,
                        methods.into_boxed_slice(),
                        interface.callee_protocol_identity.clone(),
                    )
                    .map_err(|error| {
                        unsatisfied(
                            BytecodeLinkObligation::ConcreteTargetTables,
                            location.clone(),
                            error.to_string(),
                        )
                    })?;
                    if let Some(existing) = tables.iter().find(|table| {
                        table.interface().artifact() == &interface.interface
                            && matches!(
                                table.kind(),
                                LinkedInterfaceTableKind::Remote(row)
                                    if row.service_requirement_key() == remote.service_requirement_key()
                                        && row.public_instance_key() == remote.public_instance_key()
                            )
                    }) {
                        let LinkedInterfaceTableKind::Remote(row) = existing.kind() else {
                            unreachable!("remote table predicate only matches Remote")
                        };
                        if row != &remote {
                            return Err(unsatisfied(
                                BytecodeLinkObligation::ConcreteTargetTables,
                                location,
                                "duplicate remote interface table facts drift".to_string(),
                            ));
                        }
                        continue;
                    }
                    tables.push(LinkedInterfaceTable::new(
                        InterfaceTableIndex::new(tables.len() as u32),
                        linked_interface,
                        LinkedInterfaceTableKind::Remote(remote),
                    ));
                }
                _ => {}
            }
        }
        Ok((tables, requirement_keys))
    }

    fn requirement_table_key(
        &self,
        interface: &InterfaceInstantiationRef,
        methods: &[skiff_artifact_model::InterfaceRequirementMethod],
    ) -> Result<String, BytecodeLinkError> {
        serde_json::to_string(&(interface, methods)).map_err(|error| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                self.deployment_location(),
                format!("interface requirement facts are not canonically serializable: {error}"),
            )
        })
    }

    fn link_interface_instantiation(
        &self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        interface: &InterfaceInstantiationRef,
        type_linker: &mut TypeLinker<'_>,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedInterfaceInstantiation, BytecodeLinkError> {
        let arguments = interface
            .canonical_type_args
            .iter()
            .map(|argument| {
                type_linker.intern_concrete_type(
                    package,
                    specialization,
                    argument,
                    &BTreeMap::new(),
                    location.clone(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        LinkedInterfaceInstantiation::new(interface.clone(), arguments.into_boxed_slice()).map_err(
            |error| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location,
                    error.to_string(),
                )
            },
        )
    }

    fn key_for_local_interface_method(
        &self,
        package: &HydratedBytecodePackage,
        method: &skiff_artifact_model::LocalInterfaceMethod,
        type_linker: &mut TypeLinker<'_>,
        location: BytecodeLinkLocation,
    ) -> Result<SpecializationKey, BytecodeLinkError> {
        let canonical = package
            .canonical_implementation_callable_for_function_key(&method.function_key)
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    format!(
                        "local interface method {} has no canonical implementation callable",
                        method.function_key
                    ),
                )
            })?;
        self.key_for_receiver_callable(package, canonical, type_linker)
            .map_err(|error| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    format!(
                        "local interface method {} receiver join failed: {error}",
                        method.function_key
                    ),
                )
            })
    }

    fn link_interface_signature(
        &self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        receiver_type: &TypeRefIr,
        signature: &skiff_artifact_model::InterfaceMethodSlotSignatureIr,
        effects: &CallableEffectSummary,
        type_linker: &mut TypeLinker<'_>,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedCallableSignature, BytecodeLinkError> {
        let mut parameter_types = Vec::with_capacity(signature.params.len());
        let mut parameter_plans = Vec::with_capacity(signature.params.len());
        for parameter in &signature.params {
            let parameter_type = replace_self_type(&parameter.ty, receiver_type);
            let index = type_linker.intern_concrete_type(
                package,
                specialization,
                &parameter_type,
                &BTreeMap::new(),
                location.clone(),
            )?;
            let plan = type_linker
                .linked_type_plan(index)
                .cloned()
                .ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::FrameAndValueTransferPlan,
                        location.clone(),
                        format!(
                            "interface parameter type {} has no linked transfer plan",
                            index.get()
                        ),
                    )
                })?;
            parameter_types.push(index);
            parameter_plans.push(plan);
        }
        let return_type = replace_self_type(&signature.return_type, receiver_type);
        let result_types = vec![type_linker.intern_concrete_type(
            package,
            specialization,
            &return_type,
            &BTreeMap::new(),
            location.clone(),
        )?];
        let result_plans = result_types
            .iter()
            .map(|index| {
                type_linker
                    .linked_type_plan(*index)
                    .cloned()
                    .ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::FrameAndValueTransferPlan,
                            location.clone(),
                            format!(
                                "interface result type {} has no linked transfer plan",
                                index.get()
                            ),
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        LinkedCallableSignature::new(
            parameter_types.into_boxed_slice(),
            vec![ParamModeIr::Value; signature.params.len()].into_boxed_slice(),
            parameter_plans.into_boxed_slice(),
            result_types.into_boxed_slice(),
            result_plans.into_boxed_slice(),
            effects.clone(),
        )
        .map_err(|error| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location,
                error.to_string(),
            )
        })
    }

    fn link_synthetic_callbacks(
        &self,
        reachable: &[ReachableRelocation],
        indices: &BTreeMap<SpecializationKey, FunctionIndex>,
        frames: &[LinkedFrameLayout],
        _type_linker: &mut TypeLinker<'_>,
    ) -> Result<Vec<LinkedSyntheticCallbackTarget>, BytecodeLinkError> {
        let mut targets = Vec::new();
        let mut seen = BTreeSet::new();
        for reference in reachable {
            let BytecodeRelocation::SyntheticCallbackRef { function_key } = &reference.relocation
            else {
                continue;
            };
            let package = self
                .deployment
                .packages()
                .get(reference.specialization.package_build_id())
                .ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::ExactPackageClosure,
                        self.deployment_location(),
                        "synthetic callback relocation owner is absent".to_string(),
                    )
                })?;
            if !seen.insert(function_key.clone()) {
                continue;
            }
            let key = self.key_for_synthetic_callback(package, function_key)?;
            let function = indices.get(&key).copied().ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    self.package_location(package),
                    format!("synthetic callback {function_key} is absent from the closure"),
                )
            })?;
            let location = BytecodeLinkLocation::Function {
                package: Box::new(package.reference().clone()),
                function_key: key.artifact_function_key().as_str().to_string(),
            };
            let frame = frames.get(function.get() as usize).ok_or_else(|| {
                BytecodeLinkError::ImplementationUnavailable {
                    obligation: BytecodeLinkObligation::FrameAndValueTransferPlan,
                    location: location.clone(),
                }
            })?;
            let effects = exact_actor_effects(
                package
                    .artifact()
                    .callable_semantic_facts
                    .get(key.template_function_key()),
                location.clone(),
            )?;
            let signature = frame_signature(frame, effects, location)?;
            targets.push(LinkedSyntheticCallbackTarget::new(
                skiff_runtime_linked_bytecode::SyntheticCallbackIndex::new(targets.len() as u32),
                key.artifact_function_key().clone(),
                function,
                None,
                signature,
            ));
        }
        Ok(targets)
    }

    fn reachable_relocation_location(
        &self,
        reference: &ReachableRelocation,
    ) -> BytecodeLinkLocation {
        self.deployment
            .packages()
            .get(reference.specialization.package_build_id())
            .map_or_else(
                || self.deployment_location(),
                |package| BytecodeLinkLocation::Instruction {
                    package: Box::new(package.reference().clone()),
                    function_key: reference
                        .specialization
                        .artifact_function_key()
                        .as_str()
                        .to_string(),
                    artifact_pc: reference.pc,
                },
            )
    }

    fn link_host_and_intrinsics(
        &self,
        reachable: &[ReachableRelocation],
        indices: &BTreeMap<SpecializationKey, FunctionIndex>,
        frames: &[LinkedFrameLayout],
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<
        (
            Vec<LinkedHostEffectAdapterTarget>,
            Vec<skiff_runtime_linked_bytecode::LinkedIntrinsicTarget>,
            BTreeMap<(PackageBuildId, String), skiff_runtime_linked_bytecode::IntrinsicIndex>,
        ),
        BytecodeLinkError,
    > {
        let mut host = Vec::new();
        let mut intrinsics = Vec::new();
        let mut task_submit_indices = BTreeMap::new();
        let mut seen_host = BTreeSet::new();
        let mut seen_intrinsics = BTreeSet::new();
        let mut seen_task_submit = BTreeSet::new();
        for package in self
            .deployment
            .packages()
            .values()
            .filter(|package| package.has_bytecode())
        {
            for function in package
                .bytecode()
                .ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::ConcreteTargetTables,
                        self.package_location(package),
                        "type-only package has no functions".to_string(),
                    )
                })?
                .view()
                .functions()
            {
                if !indices.keys().any(|key| {
                    key.package_build_id() == &package.reference().package_build_id
                        && key.artifact_function_key().as_str() == function.function_key
                }) {
                    continue;
                }
                for relocation in &function.relocations {
                    if !reachable.iter().any(|reference| {
                        reference.specialization.package_build_id()
                            == &package.reference().package_build_id
                            && reference.specialization.artifact_function_key().as_str()
                                == function.function_key
                            && &reference.relocation == relocation
                    }) {
                        continue;
                    }
                    let location = self.function_location(package, function);
                    match relocation {
                        BytecodeRelocation::TaskSubmitRef { task } => {
                            let specialization = self.specialization_for_function_key(
                                package,
                                &function.function_key,
                                indices,
                            )?;
                            let key = match &task.target {
                                skiff_artifact_model::bytecode::dto::TaskSubmitTargetRef::Function {
                                    function_key,
                                } => self.key_for_task_function(package, function_key)?,
                                skiff_artifact_model::bytecode::dto::TaskSubmitTargetRef::ActorMethod {
                                    ..
                                } => {
                                    return Err(BytecodeLinkError::ImplementationUnavailable {
                                        obligation: BytecodeLinkObligation::ConcreteTargetTables,
                                        location: location.clone(),
                                    });
                                }
                            };
                            let function_index = indices.get(&key).copied().ok_or_else(|| {
                                unsatisfied(
                                    BytecodeLinkObligation::ConcreteTargetTables,
                                    location.clone(),
                                    "task submit target function is absent from the closure"
                                        .to_string(),
                                )
                            })?;
                            let frame =
                                frames.get(function_index.get() as usize).ok_or_else(|| {
                                    BytecodeLinkError::ImplementationUnavailable {
                                        obligation:
                                            BytecodeLinkObligation::FrameAndValueTransferPlan,
                                        location: location.clone(),
                                    }
                                })?;
                            let effects = exact_actor_effects(
                                package
                                    .artifact()
                                    .callable_semantic_facts
                                    .get(key.template_function_key()),
                                location.clone(),
                            )?;
                            let target_signature =
                                frame_signature(frame, effects, location.clone())?;
                            let task_ref_index = type_linker.intern_concrete_type(
                                package,
                                specialization,
                                &TypeRefIr::builtin("TaskRef"),
                                &BTreeMap::new(),
                                location.clone(),
                            )?;
                            let task_ref_plan = type_linker
                                .linked_type_plan(task_ref_index)
                                .cloned()
                                .ok_or_else(|| {
                                    unsatisfied(
                                        BytecodeLinkObligation::FrameAndValueTransferPlan,
                                        location.clone(),
                                        format!(
                                            "task submit result type {} has no linked plan",
                                            task_ref_index.get()
                                        ),
                                    )
                                })?;
                            let task_effects = CallableMayEffects {
                                escapes_caller_value: false,
                                requires_same_heap_identity: false,
                                invokes_unknown_target: false,
                                may_pending: true,
                                pending_effect_categories: vec![PendingEffectCategory::NativeCall],
                                inout_path_effects: Vec::new(),
                            };
                            let native_signature = LinkedNativeCallableSignature::new(
                                target_signature
                                    .parameter_types()
                                    .to_vec()
                                    .into_boxed_slice(),
                                target_signature
                                    .parameter_modes()
                                    .to_vec()
                                    .into_boxed_slice(),
                                target_signature
                                    .parameter_plans()
                                    .to_vec()
                                    .into_boxed_slice(),
                                vec![task_ref_index].into_boxed_slice(),
                                vec![task_ref_plan].into_boxed_slice(),
                                task_effects,
                            )
                            .map_err(|error| {
                                unsatisfied(
                                    BytecodeLinkObligation::FrameAndValueTransferPlan,
                                    location.clone(),
                                    error.to_string(),
                                )
                            })?;
                            let task_key = (
                                package.reference().package_build_id.clone(),
                                task.target_identity.clone(),
                            );
                            if !seen_task_submit.insert(task_key.clone()) {
                                continue;
                            }
                            let task_target = LinkedTaskTarget::new(
                                TaskTargetIndex::new(task_submit_indices.len() as u32),
                                task.target_identity.clone(),
                                function_index,
                                target_signature,
                                match task.timing {
                                    skiff_artifact_model::bytecode::dto::TaskSubmitTimingRef::Immediate => {
                                        LinkedTaskTiming::Immediate
                                    }
                                    skiff_artifact_model::bytecode::dto::TaskSubmitTimingRef::After {
                                        expression,
                                    } => LinkedTaskTiming::After { expression },
                                    skiff_artifact_model::bytecode::dto::TaskSubmitTimingRef::At {
                                        expression,
                                    } => LinkedTaskTiming::At { expression },
                                },
                            )
                            .map_err(|error| {
                                unsatisfied(
                                    BytecodeLinkObligation::ConcreteTargetTables,
                                    location.clone(),
                                    error.to_string(),
                                )
                            })?;
                            let intrinsic_index =
                                skiff_runtime_linked_bytecode::IntrinsicIndex::new(
                                    intrinsics.len() as u32,
                                );
                            let kind = skiff_runtime_linked_bytecode::LinkedIntrinsicKind::Static(
                                skiff_runtime_linked_bytecode::LinkedStaticIntrinsicTarget::new(
                                    skiff_runtime_linked_bytecode::LinkedIntrinsicCanonicalKey::parse(
                                        "std.task.submit",
                                    )
                                    .map_err(|error| {
                                        unsatisfied(
                                            BytecodeLinkObligation::ConcreteTargetTables,
                                            location.clone(),
                                            error.to_string(),
                                        )
                                    })?,
                                    1,
                                )
                                .map_err(|error| {
                                    unsatisfied(
                                        BytecodeLinkObligation::ConcreteTargetTables,
                                        location.clone(),
                                        error.to_string(),
                                    )
                                })?,
                            );
                            task_submit_indices.insert(task_key, intrinsic_index);
                            intrinsics.push(
                                skiff_runtime_linked_bytecode::LinkedIntrinsicTarget::new(
                                    intrinsic_index,
                                    kind,
                                    native_signature,
                                )
                                .with_task_target(task_target),
                            );
                        }
                        BytecodeRelocation::HostEffectRef(effect) => {
                            let specialization = self.specialization_for_function_key(
                                package,
                                &function.function_key,
                                indices,
                            )?;
                            // The registry authenticates the exact executable ABI;
                            // compiler-owned concrete plans remain in the artifact.
                            let entry = validate_host_effect_authority(effect, &location)?;
                            let executor_identity = executable_identity_for(entry, &location)?;
                            let signature = native_signature(
                                package,
                                specialization,
                                &effect.signature,
                                type_linker,
                                &location,
                            )?;
                            require_executor_representation_carrier(
                                executor_identity,
                                &signature,
                                type_linker,
                                &location,
                            )?;
                            let binding_key = entry.binding_key.as_str();
                            if !seen_host.insert(binding_key.to_string()) {
                                continue;
                            }
                            host.push(
                                LinkedHostEffectAdapterTarget::new(
                                    skiff_runtime_linked_bytecode::HostEffectAdapterIndex::new(
                                        host.len() as u32,
                                    ),
                                    executor_identity,
                                    effect.target.namespace.clone(),
                                    effect.target.symbol.clone(),
                                    skiff_runtime_linked_bytecode::LinkedHostBindingKey::parse(
                                        binding_key,
                                    )
                                    .map_err(|error| {
                                        unsatisfied(
                                            BytecodeLinkObligation::ConcreteTargetTables,
                                            location.clone(),
                                            error.to_string(),
                                        )
                                    })?,
                                    effect.target.metadata.clone(),
                                    signature,
                                )
                                .map_err(|error| {
                                    unsatisfied(
                                        BytecodeLinkObligation::ConcreteTargetTables,
                                        location.clone(),
                                        error.to_string(),
                                    )
                                })?,
                            );
                        }
                        BytecodeRelocation::IntrinsicRef { intrinsic } => {
                            let specialization = self.specialization_for_function_key(
                                package,
                                &function.function_key,
                                indices,
                            )?;
                            let db_operation = intrinsic
                                .db_operation
                                .as_ref()
                                .map(|operation| {
                                    self.link_db_operation(
                                        package,
                                        specialization,
                                        operation,
                                        type_linker,
                                        &location,
                                    )
                                })
                                .transpose()?;
                            // The F6 local-interface lane emits this exact
                            // static string.length row before the shared
                            // intrinsic registry admits receiver string ops.
                            // The VM still fails closed at dispatch.
                            let local_interface_string_length = matches!(
                                &intrinsic.target,
                                BytecodeIntrinsicRef::Static { canonical_key, .. }
                                    if canonical_key == "std.string.length"
                            );
                            if db_operation.is_none() && !local_interface_string_length {
                                let mut resolver =
                                    DeploymentLifecycleResolver::new(self.deployment, package);
                                let mut budget = ValueLifecyclePolicyBudget::new(
                                    1_000, 1_000_000, 64,
                                )
                                .map_err(|error| {
                                    unsatisfied(
                                        BytecodeLinkObligation::ConcreteTargetTables,
                                        location.clone(),
                                        error.to_string(),
                                    )
                                })?;
                                skiff_artifact_model::intrinsic_registry()
                                    .match_reference(intrinsic, &mut resolver, &mut budget)
                                    .map_err(|error| {
                                        unsatisfied(
                                            BytecodeLinkObligation::ConcreteTargetTables,
                                            location.clone(),
                                            format!(
                                                "intrinsic registry rejected target: {error:?}"
                                            ),
                                        )
                                    })?;
                            }
                            let signature = native_signature(
                                package,
                                specialization,
                                &intrinsic.signature,
                                type_linker,
                                &location,
                            )?;
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
                                skiff_runtime_linked_bytecode::LinkedIntrinsicKind::Static(
                                    target,
                                ) => format!(
                                    "static:{}@{}",
                                    target.canonical_key().as_str(),
                                    target.signature_version()
                                ),
                                skiff_runtime_linked_bytecode::LinkedIntrinsicKind::Receiver(
                                    op,
                                ) => {
                                    format!("receiver:{}", op.canonical_key)
                                }
                            };
                            let intrinsic_key = db_operation.as_ref().map_or_else(
                                || intrinsic_key.clone(),
                                |operation| {
                                    format!(
                                        "{intrinsic_key}:{}:{}:{}:{}",
                                        operation.target_id().package_artifact_ref().package_id,
                                        operation.target_id().file_ir_ref().file_ir_identity,
                                        operation.target_id().type_index(),
                                        format!("{:?}", operation.op()),
                                    )
                                },
                            );
                            if !seen_intrinsics.insert(intrinsic_key) {
                                continue;
                            }
                            let mut linked =
                                skiff_runtime_linked_bytecode::LinkedIntrinsicTarget::new(
                                    skiff_runtime_linked_bytecode::IntrinsicIndex::new(
                                        intrinsics.len() as u32,
                                    ),
                                    kind,
                                    signature,
                                );
                            if let Some(db_operation) = db_operation {
                                linked = linked.with_db_operation(db_operation);
                            }
                            intrinsics.push(linked);
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok((host, intrinsics, task_submit_indices))
    }

    fn link_db_operation(
        &self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        operation: &DbOperationReference,
        type_linker: &mut TypeLinker<'_>,
        location: &BytecodeLinkLocation,
    ) -> Result<skiff_runtime_linked_bytecode::LinkedDbOperation, BytecodeLinkError> {
        let target_id = self.resolve_db_object_target(package, operation, location)?;
        let parameter_index = type_linker.intern_concrete_type(
            package,
            specialization,
            &operation.target.type_ref,
            &BTreeMap::new(),
            location.clone(),
        )?;
        let parameter_plan = type_linker
            .linked_type_plan(parameter_index)
            .cloned()
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::FrameAndValueTransferPlan,
                    location.clone(),
                    format!(
                        "db operation target type {} has no linked transfer plan",
                        parameter_index.get()
                    ),
                )
            })?;
        let result_index = type_linker.intern_concrete_type(
            package,
            specialization,
            &operation.result_type,
            &BTreeMap::new(),
            location.clone(),
        )?;
        let result_plan = type_linker
            .linked_type_plan(result_index)
            .cloned()
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::FrameAndValueTransferPlan,
                    location.clone(),
                    format!(
                        "db operation result type {} has no linked transfer plan",
                        result_index.get()
                    ),
                )
            })?;
        skiff_runtime_linked_bytecode::LinkedDbOperation::new(
            target_id,
            operation.target.type_name.clone(),
            operation.op,
            parameter_plan,
            result_index,
            result_plan,
        )
        .map_err(|error| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                error.to_string(),
            )
        })
    }

    fn resolve_db_object_target(
        &self,
        caller: &HydratedBytecodePackage,
        operation: &DbOperationReference,
        location: &BytecodeLinkLocation,
    ) -> Result<skiff_runtime_linked_bytecode::LinkedDbObjectTargetId, BytecodeLinkError> {
        let normalized = normalize_type(
            self.deployment,
            caller,
            &operation.target.type_ref,
            location,
        )
        .map_err(|error| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                format!("db target resolution failed: {error}"),
            )
        })?;
        let TypeRefIr::PackageSymbol { symbol } = normalized else {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                "db target did not normalize to an exact package type".to_string(),
            ));
        };
        let PackageRefIr::PackageId { package_id } = &symbol.package else {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                "db target normalized to a dependency alias instead of an exact package id"
                    .to_string(),
            ));
        };
        let mut owners = self
            .deployment
            .packages()
            .values()
            .filter(|package| package.reference().package_id == *package_id);
        let owner = owners.next().ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                format!("db target package {package_id:?} is absent from the closure"),
            )
        })?;
        if owners.next().is_some() {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                format!("db target package {package_id:?} is ambiguous in the closure"),
            ));
        }
        if owner.reference().package_build_id != caller.reference().package_build_id {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                format!(
                    "db target {package_id:?} is not owner-internal to the caller build {}",
                    caller.reference().package_build_id
                ),
            ));
        }
        let expected = symbol.symbol_path.as_str();
        let mut exports = owner
            .artifact()
            .implementation_links
            .types
            .iter()
            .filter(|(_, export)| canonical_export_path(export) == expected);
        let (_, export) = exports.next().ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                format!("db target type {expected:?} has no exact implementation link"),
            )
        })?;
        if exports.any(|(_, candidate)| candidate != export) {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                format!("db target type {expected:?} has conflicting implementation links"),
            ));
        }
        Ok(skiff_runtime_linked_bytecode::LinkedDbObjectTargetId::new(
            owner.reference().clone(),
            export.file.clone(),
            export.type_index,
        ))
    }

    pub(super) fn key_for_receiver_callable(
        &self,
        package: &HydratedBytecodePackage,
        callable: &skiff_artifact_model::PackageCallableId,
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<SpecializationKey, BytecodeLinkError> {
        let location = self.package_location(package);
        let function_key = package.function_key_for_callable(callable).ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                format!("callable {callable} has no function key"),
            )
        })?;
        let bytecode = package.bytecode().ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                "receiver callable owner is type-only".to_string(),
            )
        })?;
        let function = bytecode
            .view()
            .functions()
            .iter()
            .find(|function| function.function_key == function_key)
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    format!("function {function_key} is absent"),
                )
            })?;
        if !function.type_parameters.is_empty() {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location,
                "receiver callable is generic".to_string(),
            ));
        }
        let self_ref = function.self_type_ref.ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                "receiver callable has no self type".to_string(),
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
                    "receiver callable has no canonical implementation".to_string(),
                )
            })?;
        if canonical != callable {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location,
                "receiver callable aliases a different canonical identity".to_string(),
            ));
        }
        let artifact_function_key = ArtifactFunctionKey::parse(function_key).map_err(|error| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                error.to_string(),
            )
        })?;
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
        let bytecode = package.bytecode().ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                "synthetic callback owner is type-only".to_string(),
            )
        })?;
        let function = bytecode
            .view()
            .functions()
            .iter()
            .find(|function| function.function_key == function_key)
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    format!("synthetic callback {function_key} is absent"),
                )
            })?;
        let skiff_artifact_model::BytecodeFunctionOrigin::SyntheticCallback {
            owner,
            site_ordinal,
        } = &function.origin
        else {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location,
                "callback target is not synthetic".to_string(),
            ));
        };
        let callable = package
            .synthetic_callback_callable(owner, *site_ordinal)
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    format!("synthetic callback {function_key} has no callable identity"),
                )
            })?;
        super::relocations::specialization_key(package, function_key, callable.clone(), location)
    }

    pub(super) fn key_for_task_function(
        &self,
        package: &HydratedBytecodePackage,
        function_key: &str,
    ) -> Result<SpecializationKey, BytecodeLinkError> {
        let location = self.package_location(package);
        let bytecode = package.bytecode().ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                "task function owner is type-only".to_string(),
            )
        })?;
        let function = bytecode
            .view()
            .functions()
            .iter()
            .find(|function| function.function_key == function_key)
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    format!("task function {function_key} is absent"),
                )
            })?;
        if !matches!(
            function.origin,
            skiff_artifact_model::BytecodeFunctionOrigin::Executable { .. }
        ) {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location,
                "task target is not an ordinary executable function".to_string(),
            ));
        }
        let callable = package
            .canonical_implementation_callable_for_function_key(function_key)
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    format!("task function {function_key} has no callable identity"),
                )
            })?;
        super::relocations::specialization_key(package, function_key, callable.clone(), location)
    }

    fn specialization_for_function_key<'b>(
        &self,
        package: &HydratedBytecodePackage,
        function_key: &str,
        indices: &'b BTreeMap<SpecializationKey, FunctionIndex>,
    ) -> Result<&'b SpecializationKey, BytecodeLinkError> {
        indices
            .keys()
            .find(|key| {
                key.package_build_id() == &package.reference().package_build_id
                    && key.artifact_function_key().as_str() == function_key
            })
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    self.package_location(package),
                    format!("function {function_key} has no linked specialization"),
                )
            })
    }
}

fn require_executor_representation_carrier(
    executor: HostEffectExecutorIdentity,
    signature: &LinkedNativeCallableSignature,
    type_linker: &TypeLinker<'_>,
    location: &BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    if executor != HostEffectExecutorIdentity::Sleep {
        return Ok(());
    }
    let [parameter] = signature.parameter_types() else {
        return Err(unsatisfied(
            BytecodeLinkObligation::ConcreteTargetTables,
            location.clone(),
            "Sleep target does not retain exactly one linked parameter type".to_string(),
        ));
    };
    if type_linker
        .linked_representation_carrier(*parameter)
        .is_none()
    {
        return Err(unsatisfied(
            BytecodeLinkObligation::ConcreteTargetTables,
            location.clone(),
            "Sleep parameter lacks its compiler-owned representation carrier fact".to_string(),
        ));
    }
    Ok(())
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
    for (ty, plan) in signature
        .parameter_types
        .iter()
        .zip(&signature.parameter_plans)
    {
        let index = type_linker.intern_concrete_type(
            package,
            specialization,
            ty,
            &BTreeMap::new(),
            location.clone(),
        )?;
        let concrete = type_linker.linked_type_ref(index).cloned().ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                "native parameter type is absent".to_string(),
            )
        })?;
        parameter_types.push(index);
        parameter_plans.push(type_linker.link_plan_for_type_at(
            package,
            specialization,
            &BTreeMap::new(),
            plan,
            &concrete,
            location.clone(),
        )?);
    }
    let mut result_types = Vec::new();
    let mut result_plans = Vec::new();
    for (ty, plan) in signature.result_types.iter().zip(&signature.result_plans) {
        let index = type_linker.intern_concrete_type(
            package,
            specialization,
            ty,
            &BTreeMap::new(),
            location.clone(),
        )?;
        let concrete = type_linker.linked_type_ref(index).cloned().ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                "native result type is absent".to_string(),
            )
        })?;
        result_types.push(index);
        result_plans.push(type_linker.link_plan_for_type_at(
            package,
            specialization,
            &BTreeMap::new(),
            plan,
            &concrete,
            location.clone(),
        )?);
    }
    LinkedNativeCallableSignature::new(
        parameter_types.into_boxed_slice(),
        signature.parameter_modes.clone().into_boxed_slice(),
        parameter_plans.into_boxed_slice(),
        result_types.into_boxed_slice(),
        result_plans.into_boxed_slice(),
        signature.effects.clone(),
    )
    .map_err(|error| {
        unsatisfied(
            BytecodeLinkObligation::ConcreteTargetTables,
            location.clone(),
            error.to_string(),
        )
    })
}

/// Looks up the executable identity and ABI template by canonical binding ID.
/// Concrete types and transfer plans remain compiler-owned artifact facts.
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

/// Extracts the closed execution authority from the exact pinned row. Rows
/// without bytecode execution authority are descriptive native bindings only
/// and must fail before a linked executable target can be minted.
fn executable_identity_for(
    entry: &HostEffectRegistryEntry,
    location: &BytecodeLinkLocation,
) -> Result<HostEffectExecutorIdentity, BytecodeLinkError> {
    entry.executor_identity.ok_or_else(|| {
        unsatisfied(
            BytecodeLinkObligation::ConcreteTargetTables,
            location.clone(),
            format!(
                "host effect registry row `{}` has no bytecode executor identity",
                entry.binding_key
            ),
        )
    })
}

/// Matches exact target, metadata and callable ABI against the frozen registry.
/// This deliberately does not derive or replace compiler-owned transfer plans.
fn validate_host_effect_authority(
    effect: &skiff_artifact_model::HostEffectReference,
    location: &BytecodeLinkLocation,
) -> Result<&'static HostEffectRegistryEntry, BytecodeLinkError> {
    let entry = registry_entry_for(effect, location)?;
    let artifact_target = if effect.target.namespace.is_empty() {
        effect.target.symbol.clone()
    } else {
        format!("{}.{}", effect.target.namespace, effect.target.symbol)
    };
    if entry.target != artifact_target {
        return Err(host_abi_mismatch(
            location,
            format!(
                "host effect target `{artifact_target}` is not exact registry target `{}`",
                entry.target
            ),
        ));
    }
    if !entry.metadata.matches(&effect.target.metadata) {
        return Err(host_abi_mismatch(
            location,
            "host effect metadata does not match the exact registry ABI".to_string(),
        ));
    }
    validate_host_signature_abi(&entry.signature, &effect.signature, location)?;
    Ok(entry)
}

fn validate_host_signature_abi(
    expected: &skiff_artifact_model::CallableRegistrySignature,
    actual: &skiff_artifact_model::HostEffectSignature,
    location: &BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    for (name, expected, actual) in [
        (
            "parameter types",
            expected.parameter_types.len(),
            actual.parameter_types.len(),
        ),
        (
            "parameter modes",
            expected.parameter_modes.len(),
            actual.parameter_modes.len(),
        ),
        (
            "parameter plans",
            expected.parameter_plans.len(),
            actual.parameter_plans.len(),
        ),
        (
            "result types",
            expected.result_types.len(),
            actual.result_types.len(),
        ),
        (
            "result plans",
            expected.result_plans.len(),
            actual.result_plans.len(),
        ),
    ] {
        if expected != actual {
            return Err(host_abi_mismatch(
                location,
                format!(
                    "host effect {name} arity is {actual}, exact registry ABI requires {expected}"
                ),
            ));
        }
    }
    if expected.parameter_modes != actual.parameter_modes {
        return Err(host_abi_mismatch(
            location,
            "host effect parameter modes differ from the exact registry ABI".to_string(),
        ));
    }
    if expected.effects != actual.effects {
        return Err(host_abi_mismatch(
            location,
            "host effect effects differ from the exact registry ABI".to_string(),
        ));
    }

    let mut type_arguments = vec![None; expected.type_parameter_count as usize];
    for (position, templates, types) in [
        (
            "parameter",
            expected.parameter_types.as_slice(),
            actual.parameter_types.as_slice(),
        ),
        (
            "result",
            expected.result_types.as_slice(),
            actual.result_types.as_slice(),
        ),
    ] {
        for (ordinal, (template, ty)) in templates.iter().zip(types).enumerate() {
            match_registry_type_expression(template, ty, &mut type_arguments).map_err(
                |detail| {
                    host_abi_mismatch(
                        location,
                        format!(
                            "host effect {position} {ordinal} differs from registry ABI: {detail}"
                        ),
                    )
                },
            )?;
        }
    }
    if type_arguments.iter().any(Option::is_none) {
        return Err(host_abi_mismatch(
            location,
            "host effect registry type parameter is not bound by the artifact ABI".to_string(),
        ));
    }
    Ok(())
}

fn match_registry_type_expression(
    template: &CallableRegistryTypeExpression,
    actual: &TypeRefIr,
    arguments: &mut [Option<TypeRefIr>],
) -> Result<(), String> {
    match template {
        CallableRegistryTypeExpression::TypeParameter { ordinal } => {
            let slot = arguments
                .get_mut(*ordinal as usize)
                .ok_or_else(|| "type parameter ordinal is outside declared arity".to_string())?;
            if let Some(previous) = slot {
                if previous != actual {
                    return Err("type parameter has inconsistent instantiations".to_string());
                }
            } else {
                *slot = Some(actual.clone());
            }
            Ok(())
        }
        CallableRegistryTypeExpression::Builtin {
            name,
            arguments: expected_arguments,
        } => {
            let (actual_name, actual_arguments): (&str, &[TypeRefIr]) = match actual {
                TypeRefIr::Builtin { name, args } => (name, args),
                TypeRefIr::Nullable { inner } if name == "Nullable" => {
                    ("Nullable", std::slice::from_ref(inner.as_ref()))
                }
                _ => return Err(format!("expected builtin {name}")),
            };
            if actual_name != name || actual_arguments.len() != expected_arguments.len() {
                return Err(format!("expected builtin {name} with exact arity"));
            }
            for (expected, actual) in expected_arguments.iter().zip(actual_arguments) {
                match_registry_type_expression(expected, actual, arguments)?;
            }
            Ok(())
        }
        CallableRegistryTypeExpression::PackageSymbol {
            package_id,
            symbol_path,
        } => {
            let symbol = match actual {
                TypeRefIr::PackageSymbol { symbol } => symbol,
                TypeRefIr::AppliedNominal {
                    base: skiff_artifact_model::NominalTypeRefBaseIr::PackageSymbol { symbol },
                    arguments,
                } if arguments.is_empty() => symbol,
                _ => return Err("expected exact package symbol".to_string()),
            };
            let PackageRefIr::PackageId {
                package_id: actual_package_id,
            } = &symbol.package
            else {
                return Err("package symbol retains an unresolved dependency alias".to_string());
            };
            if actual_package_id != package_id || symbol.symbol_path != *symbol_path {
                return Err("package symbol owner/path mismatch".to_string());
            }
            if symbol.abi_expectation.as_deref().is_none_or(str::is_empty) {
                return Err("package symbol lacks exact ABI identity".to_string());
            }
            Ok(())
        }
    }
}

fn host_abi_mismatch(location: &BytecodeLinkLocation, detail: String) -> BytecodeLinkError {
    unsatisfied(
        BytecodeLinkObligation::ConcreteTargetTables,
        location.clone(),
        detail,
    )
}
struct DeploymentLifecycleResolver<'a> {
    deployment: &'a HydratedDeploymentBytecode,
    caller: &'a HydratedBytecodePackage,
}

impl<'a> DeploymentLifecycleResolver<'a> {
    fn new(
        deployment: &'a HydratedDeploymentBytecode,
        caller: &'a HydratedBytecodePackage,
    ) -> Self {
        Self { deployment, caller }
    }
}

impl ValueLifecycleFactResolver for DeploymentLifecycleResolver<'_> {
    fn resolve_package_symbol(
        &mut self,
        symbol: &PackageSymbolRef,
    ) -> Result<ResolvedPackageValueType, ValueLifecycleResolverError> {
        let owner = match &symbol.package {
            PackageRefIr::PackageId { package_id } => self
                .deployment
                .packages()
                .values()
                .find(|package| package.reference().package_id == *package_id)
                .ok_or_else(|| resolver_error("package owner absent"))?,
            PackageRefIr::Dependency { dependency_ref } => {
                let key = skiff_artifact_model::PackageRequirementKey {
                    caller_package_build_id: self.caller.reference().package_build_id.clone(),
                    package_requirement_alias: dependency_ref.clone(),
                };
                let binding = self
                    .deployment
                    .deployment()
                    .package_bindings
                    .iter()
                    .find(|binding| binding.key == key)
                    .ok_or_else(|| resolver_error("dependency binding absent"))?;
                self.deployment
                    .packages()
                    .get(&binding.package.package_build_id)
                    .ok_or_else(|| resolver_error("dependency package absent"))?
            }
        };
        let symbol = owner
            .artifact()
            .package_local_abi
            .implementation_symbols
            .get(&symbol.symbol_path)
            .or_else(|| {
                owner
                    .artifact()
                    .package_local_abi
                    .public_symbols
                    .get(&symbol.symbol_path)
            })
            .ok_or_else(|| resolver_error("package symbol absent"))?;
        let PackageLocalAbiSymbol::Type {
            descriptor,
            type_params,
            ..
        } = symbol
        else {
            return Err(resolver_error("package symbol is not a type"));
        };
        let location = BytecodeLinkLocation::Package {
            package: Box::new(owner.reference().clone()),
        };
        let descriptor =
            normalize_resolved_descriptor(self.deployment, owner, descriptor, &location)
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
        let owner = self
            .deployment
            .packages()
            .values()
            .find(|package| package.reference().package_id == package_id)
            .ok_or_else(|| resolver_error("schema owner absent"))?;
        owner
            .artifact()
            .bytecode_schema_records
            .get(package_schema_type_id)
            .filter(|record| {
                record.package_id == package_id && record.stable_schema_key == stable_schema_key
            })
            .cloned()
            .ok_or_else(|| resolver_error("schema record absent"))
    }

    fn validate_interface(
        &mut self,
        interface: &InterfaceInstantiationRef,
    ) -> Result<(), ValueLifecycleResolverError> {
        let identity: TypeRefIr = serde_json::from_str(&interface.interface_abi_id)
            .map_err(|_| resolver_error("interface identity is not TypeRefIr"))?;
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
        let skiff_artifact_model::ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } = interface
        else {
            return Err(resolver_error("contract interface is not PackageSchema"));
        };
        let record =
            self.resolve_package_schema(package_id, stable_schema_key, package_schema_type_id)?;
        if !matches!(
            record.canonical_descriptor.descriptor,
            skiff_artifact_model::ContractTypeDescriptor::CallbackInterface { .. }
        ) {
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
                export.file.module_path == actor.module_path
                    && export
                        .actor
                        .as_ref()
                        .is_some_and(|abi| abi.abi.actor_name == actor.symbol)
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
        let bytecode = package.bytecode().ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                "receiver function owner is type-only".to_string(),
            )
        })?;
        let function = bytecode
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
        let artifact_function_key = ArtifactFunctionKey::parse(function_key).map_err(|error| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                error.to_string(),
            )
        })?;
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
        self.service_operations
            .iter()
            .find(|target| {
                target.service_requirement_key().caller_package_build_id == *caller_package_build_id
                    && target.service_requirement_key().service_requirement_slot == slot
                    && target.contract_operation_id() == operation
            })
            .map(|target| target.index())
    }

    pub(in crate::bytecode) fn actor_method_index(
        &self,
        package: &HydratedBytecodePackage,
        actor: &skiff_artifact_model::ServiceSymbolRef,
        abi: &skiff_artifact_model::ActorAbiIdentity,
        implementation: &skiff_artifact_model::ActorImplementationIdentity,
        method: &skiff_artifact_model::ActorMethodIdentity,
    ) -> Option<skiff_runtime_linked_bytecode::ActorMethodIndex> {
        self.actor_methods
            .iter()
            .find(|target| {
                target.owner_package_build_id() == &package.reference().package_build_id
                    && target.actor() == actor
                    && target.actor_abi_identity() == abi
                    && target.actor_implementation_identity() == implementation
                    && target.method_identity() == method
            })
            .map(|target| target.index())
    }

    pub(in crate::bytecode) fn interface_index(
        &self,
        interface: &InterfaceInstantiationRef,
        kind: &InterfaceKind,
    ) -> Option<skiff_runtime_linked_bytecode::InterfaceTableIndex> {
        self.interface_tables
            .iter()
            .find(|table| {
                table.interface().artifact() == interface
                    && matches!(
                        (kind, table.kind()),
                        (
                            InterfaceKind::Requirement,
                            LinkedInterfaceTableKind::Requirement(_)
                        ) | (
                            InterfaceKind::Callback,
                            LinkedInterfaceTableKind::Callback(_)
                        ) | (InterfaceKind::Local, LinkedInterfaceTableKind::Local(_))
                            | (InterfaceKind::Remote, LinkedInterfaceTableKind::Remote(_))
                    )
            })
            .map(|table| table.index())
    }

    pub(in crate::bytecode) fn requirement_index(
        &self,
        key: &str,
        kind: &InterfaceKind,
    ) -> Option<skiff_runtime_linked_bytecode::InterfaceTableIndex> {
        let index = self.requirement_keys.get(key).copied()?;
        let table = self.interface_tables.get(index.get() as usize)?;
        matches!(
            (kind, table.kind()),
            (
                InterfaceKind::Requirement,
                LinkedInterfaceTableKind::Requirement(_)
            ) | (
                InterfaceKind::Callback,
                LinkedInterfaceTableKind::Callback(_)
            )
        )
        .then_some(index)
    }

    pub(in crate::bytecode) fn synthetic_callback_index(
        &self,
        package: &HydratedBytecodePackage,
        function_key: &str,
    ) -> Option<skiff_runtime_linked_bytecode::SyntheticCallbackIndex> {
        self.synthetic_callback_origins
            .get(&(
                package.reference().package_build_id.clone(),
                function_key.to_string(),
            ))
            .copied()
    }

    pub(in crate::bytecode) fn task_submit_index(
        &self,
        package: &HydratedBytecodePackage,
        target_identity: &str,
    ) -> Option<skiff_runtime_linked_bytecode::IntrinsicIndex> {
        self.task_submit_indices
            .get(&(
                package.reference().package_build_id.clone(),
                target_identity.to_string(),
            ))
            .copied()
    }

    pub(in crate::bytecode) fn host_index(
        &self,
        namespace: &str,
        symbol: &str,
        binding_key: &str,
        metadata: &BTreeMap<String, skiff_artifact_model::MetadataValue>,
    ) -> Option<skiff_runtime_linked_bytecode::HostEffectAdapterIndex> {
        self.host_effect_adapters
            .iter()
            .find(|target| {
                target.namespace() == namespace
                    && target.symbol() == symbol
                    && target.binding_key().as_str() == binding_key
                    && target.metadata() == metadata
            })
            .map(|target| target.index())
    }

    pub(in crate::bytecode) fn intrinsic_index(
        &self,
        reference: &BytecodeIntrinsicRef,
    ) -> Option<skiff_runtime_linked_bytecode::IntrinsicIndex> {
        self.intrinsics
            .iter()
            .find(|target| match (target.kind(), reference) {
                (
                    skiff_runtime_linked_bytecode::LinkedIntrinsicKind::Static(linked),
                    BytecodeIntrinsicRef::Static {
                        canonical_key,
                        signature_version,
                    },
                ) => {
                    linked.canonical_key().as_str() == canonical_key
                        && linked.signature_version() == *signature_version
                }
                (
                    skiff_runtime_linked_bytecode::LinkedIntrinsicKind::Receiver(linked),
                    BytecodeIntrinsicRef::Receiver { op },
                ) => linked == op,
                _ => false,
            })
            .map(|target| target.index())
    }
}

fn canonical_export_path(export: &skiff_artifact_model::TypeExport) -> String {
    let prefix = format!("{}.", export.file.module_path);
    if export.symbol.starts_with(&prefix) {
        export.symbol.clone()
    } else {
        format!("{}{}", prefix, export.symbol)
    }
}
