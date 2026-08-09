use std::collections::BTreeMap;

use skiff_artifact_model::PackageCallableId;
use skiff_runtime_linked_bytecode::{
    FunctionIndex, LinkedCallableSignature, LinkedFunction, LinkedGatewayCallable,
    LinkedGatewayCallableRole, LinkedGatewayEntry, LinkedOperationEntry, LinkedParameterSlot,
    SpecializationKey,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation};

use super::{unsatisfied, DeploymentLinker};

impl DeploymentLinker<'_> {
    pub(super) fn canonical_roots(&self) -> Result<Vec<SpecializationKey>, BytecodeLinkError> {
        let implementation = self.implementation_package()?;
        let mut roots = Vec::new();
        for binding in &self.deployment.deployment().operation_bindings {
            roots.push(self.key_for_callable(
                implementation,
                &binding.package_callable_id,
                self.deployment_location(),
            )?);
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
    ) -> Result<Vec<LinkedOperationEntry>, BytecodeLinkError> {
        let implementation = self.implementation_package()?;
        let mut entries = self
            .deployment
            .deployment()
            .operation_bindings
            .iter()
            .map(|binding| {
                let key = self.key_for_callable(
                    implementation,
                    &binding.package_callable_id,
                    self.deployment_location(),
                )?;
                let function = indices.get(&key).copied().ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::CanonicalRootSet,
                        self.deployment_location(),
                        "operation root is absent from the canonical closure".to_string(),
                    )
                })?;
                Ok(LinkedOperationEntry::new(
                    binding.contract_operation_id.clone(),
                    function,
                    callable_signature(
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
                    )?,
                ))
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
