use std::collections::BTreeMap;

use skiff_artifact_model::{
    FrozenConstantNode, GatewayAdapterKind, PackageCallableId, PackageLocalAbiSymbol,
    ReceiverCallAbi,
};
use skiff_runtime_linked_bytecode::{
    FunctionIndex, LinkedCallableSignature, LinkedFunction, LinkedGatewayCallable,
    LinkedGatewayCallableRole, LinkedGatewayEntry, LinkedOperationEntry, LinkedOperationReceiver,
    LinkedParameterSlot, SpecializationKey,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::bytecode::{
    types::TypeLinker, BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation,
};

use super::constants::{
    validation::{constant_error, constant_location},
    LinkedConstantTables,
};

use super::{unsatisfied, DeploymentLinker};

impl DeploymentLinker<'_> {
    pub(super) fn canonical_roots(
        &self,
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<Vec<SpecializationKey>, BytecodeLinkError> {
        let implementation = self.implementation_package()?;
        let mut roots = Vec::new();
        for binding in &self.deployment.deployment().operation_bindings {
            let receiver_callable = implementation
                .artifact()
                .callable_links
                .get(&binding.package_callable_id)
                .is_some_and(|link| {
                    link.target.callable_kind
                        == skiff_artifact_model::OperationCallableKind::ImplMethod
                });
            if receiver_callable {
                roots.push(self.key_for_receiver_callable(
                    implementation,
                    &binding.package_callable_id,
                    type_linker,
                )?);
            } else {
                roots.push(self.key_for_callable(
                    implementation,
                    &binding.package_callable_id,
                    self.deployment_location(),
                )?);
            }
        }
        for entry in self.deployment.deployment().gateway_entries.values() {
            for callable in [
                entry.handler.as_ref(),
                entry.pre.as_ref(),
                entry.guard.as_ref(),
                entry.close_handler.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                roots.push(self.key_for_callable(
                    implementation,
                    callable,
                    self.deployment_location(),
                )?);
            }
        }
        roots.extend(self.actor_roots(type_linker)?);
        roots.extend(self.frozen_behavior_roots(type_linker)?);
        Ok(roots)
    }

    fn frozen_behavior_roots(
        &self,
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<Vec<SpecializationKey>, BytecodeLinkError> {
        let mut roots = Vec::new();
        for package in self
            .deployment
            .packages()
            .values()
            .filter(|package| package.has_bytecode())
        {
            let bytecode = package.bytecode().ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    self.package_location(package),
                    "frozen behavior owner is type-only".to_string(),
                )
            })?;
            let nodes = bytecode.view().frozen_constant_graph().nodes.as_slice();
            for (position, node) in nodes.iter().enumerate() {
                let FrozenConstantNode::Implementation { behaviors, .. } = node else {
                    continue;
                };
                let location = constant_location(package, position, nodes.len())?;
                for behavior in behaviors {
                    roots.push(
                        self.key_for_receiver_function(
                            package,
                            &behavior.function_key,
                            type_linker,
                        )
                        .map_err(|error| constant_error(location.clone(), error.to_string()))?,
                    );
                }
            }
        }
        Ok(roots)
    }

    fn actor_roots(
        &self,
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<Vec<SpecializationKey>, BytecodeLinkError> {
        let mut roots = Vec::new();
        for package in self.deployment.packages().values() {
            for actor in &package.artifact().actor_implementations {
                let actor_abi = self.exact_actor_abi(package, &actor.actor)?;
                if let Some(create) = &actor.create {
                    roots.push(self.key_for_receiver_callable(
                        package,
                        &create.package_callable_id,
                        type_linker,
                    )?);
                }
                for (method_identity, _) in &actor.methods {
                    roots.push(self.key_for_actor_method(
                        package,
                        &actor.actor,
                        &actor_abi.actor_abi_identity,
                        &actor.actor_implementation_identity,
                        method_identity,
                        type_linker,
                    )?);
                }
            }
        }
        Ok(roots)
    }

    pub(super) fn key_for_callable(
        &self,
        package: &HydratedBytecodePackage,
        callable: &PackageCallableId,
        location: BytecodeLinkLocation,
    ) -> Result<SpecializationKey, BytecodeLinkError> {
        let function_key = package.function_key_for_callable(callable).ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::CanonicalRootSet,
                location.clone(),
                format!("callable {callable} has no admitted bytecode function"),
            )
        })?;
        let canonical = package
            .canonical_implementation_callable_for_function_key(function_key)
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::CanonicalRootSet,
                    location.clone(),
                    format!("function {function_key:?} has no canonical implementation callable"),
                )
            })?;
        if canonical != callable {
            return Err(unsatisfied(
                BytecodeLinkObligation::CanonicalRootSet,
                location,
                format!(
                    "entry callable {callable} aliases canonical implementation {canonical}; the linked entry boundary cannot retain both identities"
                ),
            ));
        }
        let function = super::closure::find_function(package, function_key).ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::CanonicalRootSet,
                location.clone(),
                format!("admitted function {function_key:?} is absent"),
            )
        })?;
        self.require_narrow_template(function, None, location.clone())?;
        super::relocations::specialization_key(package, function_key, canonical.clone(), location)
    }

    pub(super) fn link_operation_entries(
        &self,
        indices: &BTreeMap<SpecializationKey, FunctionIndex>,
        functions: &[LinkedFunction],
        constant_tables: &LinkedConstantTables,
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<Vec<LinkedOperationEntry>, BytecodeLinkError> {
        let implementation = self.implementation_package()?;
        let mut entries = self
            .deployment
            .deployment()
            .operation_bindings
            .iter()
            .map(|binding| {
                let receiver = self.operation_receiver(
                    implementation,
                    &binding.package_callable_id,
                    constant_tables,
                )?;
                let key = if receiver.is_some() {
                    self.key_for_receiver_callable(
                        implementation,
                        &binding.package_callable_id,
                        type_linker,
                    )?
                } else {
                    self.key_for_callable(
                        implementation,
                        &binding.package_callable_id,
                        self.deployment_location(),
                    )?
                };
                let function = indices.get(&key).copied().ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::CanonicalRootSet,
                        self.deployment_location(),
                        "operation root is absent from the canonical closure".to_string(),
                    )
                })?;
                let signature = callable_signature(
                    functions.get(function.get() as usize).ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::CanonicalRootSet,
                            self.deployment_location(),
                            format!(
                                "operation root function {} is out of bounds",
                                function.get()
                            ),
                        )
                    })?,
                    self.deployment_location(),
                )?;
                Ok(match receiver {
                    Some(receiver) => LinkedOperationEntry::new_with_receiver(
                        binding.contract_operation_id.clone(),
                        function,
                        signature,
                        receiver,
                    ),
                    None => LinkedOperationEntry::new(
                        binding.contract_operation_id.clone(),
                        function,
                        signature,
                    ),
                })
            })
            .collect::<Result<Vec<_>, BytecodeLinkError>>()?;
        entries.sort_by(|left, right| {
            left.contract_operation_id()
                .cmp(right.contract_operation_id())
        });
        Ok(entries)
    }

    pub(super) fn link_gateway_entries(
        &self,
        indices: &BTreeMap<SpecializationKey, FunctionIndex>,
        functions: &[LinkedFunction],
    ) -> Result<Vec<LinkedGatewayEntry>, BytecodeLinkError> {
        let implementation = self.implementation_package()?;
        self.deployment
            .deployment()
            .gateway_entries
            .iter()
            .map(|(entry_key, entry)| {
                let mut callables = Vec::new();
                for (role, callable) in [
                    (LinkedGatewayCallableRole::Handler, entry.handler.as_ref()),
                    (LinkedGatewayCallableRole::Pre, entry.pre.as_ref()),
                    (LinkedGatewayCallableRole::Guard, entry.guard.as_ref()),
                    (
                        LinkedGatewayCallableRole::CloseHandler,
                        entry.close_handler.as_ref(),
                    ),
                ] {
                    let Some(callable) = callable else {
                        continue;
                    };
                    let key = self.key_for_callable(
                        implementation,
                        callable,
                        self.deployment_location(),
                    )?;
                    let function = indices.get(&key).copied().ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::CanonicalRootSet,
                            self.deployment_location(),
                            "gateway root is absent from the canonical closure".to_string(),
                        )
                    })?;
                    callables.push(LinkedGatewayCallable::new(
                        role,
                        callable.clone(),
                        function,
                        callable_signature(
                            functions.get(function.get() as usize).ok_or_else(|| {
                                unsatisfied(
                                    BytecodeLinkObligation::CanonicalRootSet,
                                    self.deployment_location(),
                                    format!(
                                        "gateway root function {} is out of bounds",
                                        function.get()
                                    ),
                                )
                            })?,
                            self.deployment_location(),
                        )?,
                    ));
                }
                if entry.adapter_plan.kind == GatewayAdapterKind::RawHttp {
                    let handler = callables
                        .iter()
                        .find(|callable| callable.role() == LinkedGatewayCallableRole::Handler)
                        .ok_or_else(|| {
                            unsatisfied(
                                BytecodeLinkObligation::CanonicalRootSet,
                                self.deployment_location(),
                                "rawHttp gateway lacks an exact linked handler".to_string(),
                            )
                        })?;
                    let handler_function = functions
                        .get(handler.function().get() as usize)
                        .ok_or_else(|| {
                            unsatisfied(
                                BytecodeLinkObligation::CanonicalRootSet,
                                self.deployment_location(),
                                format!(
                                    "rawHttp handler function {} is out of bounds",
                                    handler.function().get()
                                ),
                            )
                        })?;
                    let [parameter] = handler_function.frame().parameters() else {
                        return Err(unsatisfied(
                            BytecodeLinkObligation::FrameAndValueTransferPlan,
                            self.deployment_location(),
                            "rawHttp handler must retain one exact frame parameter".to_string(),
                        ));
                    };
                    if parameter.dense_record_shape().is_none() {
                        return Err(unsatisfied(
                            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                            self.deployment_location(),
                            "rawHttp handler parameter lacks compiler-owned dense materialization"
                                .to_string(),
                        ));
                    }
                }
                LinkedGatewayEntry::try_new(
                    entry_key.clone(),
                    entry.gateway_entry_identity.clone(),
                    entry.protocol_surface.clone(),
                    callables.into_boxed_slice(),
                    entry.adapter_plan.clone(),
                    entry.close_adapter_plan.clone(),
                )
                .map_err(|error| {
                    unsatisfied(
                        BytecodeLinkObligation::CanonicalRootSet,
                        self.deployment_location(),
                        error.to_string(),
                    )
                })
            })
            .collect()
    }

    fn operation_receiver(
        &self,
        implementation: &HydratedBytecodePackage,
        callable_id: &PackageCallableId,
        constant_tables: &LinkedConstantTables,
    ) -> Result<Option<LinkedOperationReceiver>, BytecodeLinkError> {
        let link_fact = implementation
            .artifact()
            .callable_links
            .get(callable_id)
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::CanonicalRootSet,
                    self.package_location(implementation),
                    format!("operation callable {callable_id} has no exact link fact"),
                )
            })?;
        if link_fact.target.callable_kind != skiff_artifact_model::OperationCallableKind::ImplMethod
        {
            return Ok(None);
        }
        let operation_function = implementation
            .function_key_for_callable(callable_id)
            .map(str::to_owned);
        let mut public_instances = implementation
            .artifact()
            .package_local_abi
            .public_symbols
            .iter()
            .filter_map(|(public_path, symbol)| match symbol {
                PackageLocalAbiSymbol::PublicInstance { methods, .. }
                    if operation_function.as_deref().is_some_and(|function| {
                        methods.values().any(|candidate| {
                            implementation.function_key_for_callable(candidate) == Some(function)
                        })
                    }) =>
                {
                    Some(public_path)
                }
                _ => None,
            });
        let Some(public_path) = public_instances.next() else {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                self.package_location(implementation),
                format!(
                    "receiver operation callable {callable_id} has no exact public instance method row"
                ),
            ));
        };
        if public_instances.next().is_some() {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                self.package_location(implementation),
                format!(
                    "receiver operation callable {callable_id} is ambiguous across public instances"
                ),
            ));
        }
        let receiver_link = implementation
            .artifact()
            .implementation_links
            .constants
            .get(public_path)
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConstantInitializationPlan,
                    self.package_location(implementation),
                    format!("public instance {public_path} has no exact receiver constant link"),
                )
            })?;
        let root = format!(
            "{}.{}",
            receiver_link.file.module_path, receiver_link.symbol
        );
        let artifact_constant = implementation
            .bytecode()
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConstantInitializationPlan,
                    self.package_location(implementation),
                    format!("provider package has no admitted bytecode for receiver {root}"),
                )
            })?
            .view()
            .constant_roots()
            .get(&root)
            .copied()
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConstantInitializationPlan,
                    self.package_location(implementation),
                    format!("provider receiver constant root {root} is absent from bytecode"),
                )
            })?;
        let constant = constant_tables.resolve(
            implementation,
            artifact_constant,
            self.package_location(implementation),
        )?;
        Ok(Some(LinkedOperationReceiver::new(
            constant,
            ReceiverCallAbi::ExplicitSelfFirst,
        )))
    }
}

fn callable_signature(
    function: &LinkedFunction,
    location: BytecodeLinkLocation,
) -> Result<LinkedCallableSignature, BytecodeLinkError> {
    let frame = function.frame();
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
            .map(LinkedParameterSlot::mode)
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
        function.declarative_effect_summary().clone(),
    )
    .map_err(|error| {
        unsatisfied(
            BytecodeLinkObligation::FrameAndValueTransferPlan,
            location,
            error.to_string(),
        )
    })
}
