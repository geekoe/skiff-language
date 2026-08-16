//! Receiver and direct-call ABI facts retained during MIR construction.

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    CallIr, CallTargetIr, ContractOperationId, ExecutableIr, FileIrUnit, PackageCallableId,
    ParamModeIr, ReceiverCallAbi, ServiceProtocolIdentity, SlotKind,
};
use skiff_compiler_source::{ResolvedCallTarget, ResolvedCallTargetFacts};

use crate::mir::{MirBuildError, MirParam, MirParamMode, MirReceiverFacts, MirSlot, MirSlotKind};

use super::mir_param_mode;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PackageDirectAbi {
    parameter_modes: Vec<MirParamMode>,
    has_leading_self: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MirServiceRequirementFacts {
    pub slot: u32,
    pub expected_protocol_identity: ServiceProtocolIdentity,
    pub used_operations: BTreeSet<ContractOperationId>,
}

pub(super) struct MirPackageCatalog<'a> {
    units_by_module: BTreeMap<&'a str, &'a FileIrUnit>,
    package_direct_abis: BTreeMap<PackageCallableId, PackageDirectAbi>,
    service_requirements: BTreeMap<String, MirServiceRequirementFacts>,
}

impl<'a> MirPackageCatalog<'a> {
    pub(super) fn build(
        units: &'a [FileIrUnit],
        resolved_call_targets: &ResolvedCallTargetFacts,
    ) -> Result<Self, MirBuildError> {
        let mut units_by_module = BTreeMap::new();
        let mut file_ir_owners = BTreeMap::new();
        for unit in units {
            if unit.file_ir_identity.trim().is_empty() {
                return Err(MirBuildError::MissingFileIrIdentity {
                    module_path: unit.module_path.clone(),
                });
            }
            if let Some(first_module) =
                file_ir_owners.insert(unit.file_ir_identity.as_str(), unit.module_path.as_str())
            {
                return Err(MirBuildError::DuplicateFileIrIdentity {
                    file_ir_identity: unit.file_ir_identity.clone(),
                    first_module: first_module.to_string(),
                    duplicate_module: unit.module_path.clone(),
                });
            }
            if units_by_module
                .insert(unit.module_path.as_str(), unit)
                .is_some()
            {
                return Err(MirBuildError::DuplicateModulePath {
                    module_path: unit.module_path.clone(),
                });
            }
        }

        let mut package_direct_abis = BTreeMap::new();
        for (_, target) in resolved_call_targets.iter() {
            let ResolvedCallTarget::DependencyPackageFunction {
                package_callable_id,
                exact_signature: Some(signature),
                inout_parameters,
                ..
            } = target
            else {
                continue;
            };
            let parameter_modes = signature
                .parameters
                .iter()
                .map(|parameter| mir_param_mode(parameter.mode))
                .collect::<Vec<_>>();
            let has_leading_self = signature
                .parameters
                .first()
                .is_some_and(|parameter| parameter.name == "self");
            if has_leading_self
                && signature.parameters.first().map(|parameter| parameter.mode)
                    != Some(ParamModeIr::Value)
            {
                return Err(MirBuildError::InvalidPackageCallableAbi {
                    package_callable_id: package_callable_id.clone(),
                    message: "leading self parameter is not Value".to_string(),
                });
            }
            let receiver_offset = usize::from(has_leading_self);
            let exact_inout = signature
                .parameters
                .iter()
                .enumerate()
                .filter(|(_, parameter)| parameter.mode == ParamModeIr::InOut)
                .map(|(index, _)| index.checked_sub(receiver_offset))
                .collect::<Option<BTreeSet<_>>>()
                .ok_or_else(|| MirBuildError::InvalidPackageCallableAbi {
                    package_callable_id: package_callable_id.clone(),
                    message: "receiver parameter is marked inout".to_string(),
                })?;
            let declared_inout = inout_parameters.keys().copied().collect::<BTreeSet<_>>();
            if exact_inout != declared_inout {
                return Err(MirBuildError::InvalidPackageCallableAbi {
                    package_callable_id: package_callable_id.clone(),
                    message: "exact signature modes disagree with source inout positions"
                        .to_string(),
                });
            }
            let abi = PackageDirectAbi {
                parameter_modes,
                has_leading_self,
            };
            if let Some(previous) =
                package_direct_abis.insert(package_callable_id.clone(), abi.clone())
            {
                if previous != abi {
                    return Err(MirBuildError::ConflictingPackageCallableAbi {
                        package_callable_id: package_callable_id.clone(),
                    });
                }
            }
        }
        let mut service_requirements = BTreeMap::new();
        for (_, target) in resolved_call_targets.iter() {
            let (contract_requirement, operations) = match target {
                ResolvedCallTarget::ContractOperation {
                    contract_requirement,
                    contract_operation_id,
                    ..
                } => (contract_requirement, std::slice::from_ref(contract_operation_id)),
                ResolvedCallTarget::RemoteInterface {
                    contract_requirement,
                    operations,
                    ..
                } => (contract_requirement, operations.as_slice()),
                _ => continue,
            };
            let entry = service_requirements
                .entry(contract_requirement.alias.clone())
                .or_insert_with(|| {
                    (
                        contract_requirement.clone(),
                        BTreeSet::<ContractOperationId>::new(),
                    )
                });
            if entry.0 != *contract_requirement {
                return Err(MirBuildError::InvalidServiceRequirementFacts {
                    alias: contract_requirement.alias.clone(),
                    message: "same dependency alias resolves to conflicting contract requirements"
                        .to_string(),
                });
            }
            entry.1.extend(operations.iter().cloned());
        }
        let service_requirement_slots = service_requirements
            .into_iter()
            .enumerate()
            .map(
                |(slot_index, (alias, (contract_requirement, used_operations)))| {
                    let slot = u32::try_from(slot_index).map_err(|_| {
                        MirBuildError::InvalidServiceRequirementFacts {
                            alias: alias.clone(),
                            message: "service requirement slot exceeds u32::MAX".to_string(),
                        }
                    })?;
                    Ok((
                        alias,
                        MirServiceRequirementFacts {
                            slot,
                            expected_protocol_identity: contract_requirement
                                .expected_protocol_identity
                                .clone(),
                            used_operations,
                        },
                    ))
                },
            )
            .collect::<Result<BTreeMap<_, _>, MirBuildError>>()?;
        Ok(Self {
            units_by_module,
            package_direct_abis,
            service_requirements: service_requirement_slots,
        })
    }

    pub(super) fn service_requirement(
        &self,
        dependency_ref: &str,
    ) -> Option<&MirServiceRequirementFacts> {
        self.service_requirements.get(dependency_ref)
    }

    fn unit(&self, module_path: &str) -> Option<&'a FileIrUnit> {
        self.units_by_module.get(module_path).copied()
    }
}

pub(super) fn build_receiver_facts(
    unit: &FileIrUnit,
    executable: &ExecutableIr,
    params: &[MirParam],
    slots: &[MirSlot],
) -> Result<Option<MirReceiverFacts>, MirBuildError> {
    let fail = |message: String| MirBuildError::InvalidReceiverFacts {
        module_path: unit.module_path.clone(),
        symbol: executable.symbol.clone(),
        message,
    };
    let self_slots = slots
        .iter()
        .filter(|slot| slot.kind == MirSlotKind::SelfValue)
        .collect::<Vec<_>>();
    let Some(self_type) = executable.self_type.as_ref() else {
        if !self_slots.is_empty() {
            return Err(fail(
                "selfType is null but the slot layout contains SelfValue".to_string(),
            ));
        }
        return Ok(None);
    };
    let slot = slots
        .first()
        .ok_or_else(|| fail("receiver-bound executable has no slot zero".to_string()))?;
    if slot.slot != 0 || slot.ty.as_ref() != Some(self_type) {
        return Err(fail(
            "receiver slot zero does not carry the exact selfType".to_string(),
        ));
    }
    match slot.kind {
        MirSlotKind::SelfValue => {
            if self_slots.len() != 1 || params.iter().any(|param| param.slot == 0) {
                return Err(fail(
                    "implicit receiver must be the unique SelfValue slot zero".to_string(),
                ));
            }
        }
        MirSlotKind::Param => {
            if !self_slots.is_empty() {
                return Err(fail(
                    "explicit receiver cannot coexist with a SelfValue slot".to_string(),
                ));
            }
            let param = params.first().ok_or_else(|| {
                fail("explicit receiver has no incoming parameter zero".to_string())
            })?;
            if param.name != "self"
                || param.slot != 0
                || param.mode != MirParamMode::Value
                || &param.ty != self_type
            {
                return Err(fail(
                    "explicit receiver must be Value parameter zero with the exact selfType"
                        .to_string(),
                ));
            }
        }
        other => {
            return Err(fail(format!(
                "receiver slot zero has kind {other:?}, expected SelfValue or Param"
            )));
        }
    }
    Ok(Some(MirReceiverFacts {
        ty: self_type.clone(),
        slot: 0,
        parameter_ordinal: 0,
        call_abi: ReceiverCallAbi::ExplicitSelfFirst,
    }))
}

fn executable_parameter_modes(executable: &ExecutableIr) -> Result<Vec<MirParamMode>, String> {
    let mut modes = executable
        .params
        .iter()
        .map(|parameter| mir_param_mode(parameter.mode))
        .collect::<Vec<_>>();
    if executable.self_type.is_some() {
        let slot_zero = executable
            .slots
            .slots
            .first()
            .ok_or_else(|| "receiver-bound target has no slot zero".to_string())?;
        match slot_zero.kind {
            SlotKind::SelfValue => modes.insert(0, MirParamMode::Value),
            SlotKind::Param => {
                if modes.first() != Some(&MirParamMode::Value) {
                    return Err("explicit receiver target parameter zero is not Value".to_string());
                }
            }
            _ => {
                return Err(
                    "receiver-bound target slot zero is neither SelfValue nor Param".to_string(),
                );
            }
        }
    }
    Ok(modes)
}

pub(super) fn direct_call_parameter_modes(
    unit: &FileIrUnit,
    call: &CallIr,
    catalog: &MirPackageCatalog<'_>,
) -> Result<Vec<MirParamMode>, String> {
    match &call.target {
        CallTargetIr::LocalExecutable { executable_index } => {
            let target = unit
                .executables
                .get(*executable_index as usize)
                .ok_or_else(|| {
                    format!("local direct call references missing executable {executable_index}")
                })?;
            validate_direct_receiver_requirement(call, target)?;
            executable_parameter_modes(target)
        }
        CallTargetIr::PublicationExecutable {
            module_path,
            executable_index,
        } => {
            let target_unit = catalog.unit(module_path).ok_or_else(|| {
                format!("publication direct call references missing MIR unit `{module_path}`")
            })?;
            let target = target_unit
                .executables
                .get(*executable_index as usize)
                .ok_or_else(|| {
                    format!(
                        "publication direct call references missing executable {executable_index} in `{module_path}`"
                    )
                })?;
            validate_direct_receiver_requirement(call, target)?;
            executable_parameter_modes(target)
        }
        CallTargetIr::PackageCallable {
            package_callable_id,
            ..
        } => {
            let abi = catalog
                .package_direct_abis
                .get(package_callable_id)
                .ok_or_else(|| {
                    format!(
                        "package-direct target `{package_callable_id}` has no exact source signature"
                    )
                })?;
            if abi.has_leading_self != call.concrete_receiver.is_some() {
                return Err(format!(
                    "package-direct target `{package_callable_id}` receiver requirement disagrees with concreteReceiver"
                ));
            }
            Ok(abi.parameter_modes.clone())
        }
        _ => Err("direct-call mode lookup received a non-direct target".to_string()),
    }
}

fn validate_direct_receiver_requirement(
    call: &CallIr,
    target: &ExecutableIr,
) -> Result<(), String> {
    if target.self_type.is_some() != call.concrete_receiver.is_some() {
        return Err(format!(
            "direct target `{}` receiver requirement disagrees with concreteReceiver",
            target.symbol
        ));
    }
    Ok(())
}
