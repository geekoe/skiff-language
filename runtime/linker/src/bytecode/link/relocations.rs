use std::collections::BTreeMap;

use skiff_artifact_model::{
    contract_for_opcode, decode_branch_target, BytecodeRelocation, BytecodeSpecialization,
    DecodedInstruction, LinkedOperandKind, OperandKind, OperandSpec, PackageCallableId,
    PackageRefIr, TypeRefIr, ValidatedFunction,
};
use skiff_runtime_linked_bytecode::{
    ArtifactFunctionKey, FrameSlotIndex, FunctionIndex, LinkedInstruction, LinkedInstructionTarget,
    LinkedResolvedOperand, SpecializationKey, SwitchTableIndex,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::bytecode::{
    types::TypeLinker, BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation,
};

use super::{tables::instruction_index, unsatisfied, DeploymentLinker};

impl<'a> DeploymentLinker<'a> {
    pub(super) fn resolve_direct_target(
        &self,
        caller: &HydratedBytecodePackage,
        relocation: &BytecodeRelocation,
        owner: &ValidatedFunction,
    ) -> Result<SpecializationKey, BytecodeLinkError> {
        let location = BytecodeLinkLocation::Function {
            package: caller.reference().clone(),
            function_key: owner.function_key.clone(),
        };
        match relocation {
            BytecodeRelocation::LocalExecutableRef {
                function_key,
                specialization,
            } => {
                let target =
                    super::closure::find_function(caller, function_key).ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::RelocationResolution,
                            location.clone(),
                            format!(
                                "local target {function_key:?} is absent from the admitted view"
                            ),
                        )
                    })?;
                self.require_narrow_template(target, Some(specialization), location.clone())?;
                let canonical = caller
                    .canonical_implementation_callable_for_function_key(function_key)
                    .ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::RelocationResolution,
                            location.clone(),
                            format!("local target {function_key:?} has no canonical callable"),
                        )
                    })?;
                specialization_key(caller, function_key, canonical.clone(), location)
            }
            BytecodeRelocation::PackageCallableRef {
                package_ref,
                package_callable_id,
                specialization,
            } => {
                let target_package =
                    self.resolve_package_ref(caller, package_ref, location.clone())?;
                let function_key = target_package
                    .function_key_for_callable(package_callable_id)
                    .ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::RelocationResolution,
                            location.clone(),
                            format!(
                                "package callable {package_callable_id} is absent from exact target {}",
                                target_package.reference().package_build_id
                            ),
                        )
                    })?;
                let target = super::closure::find_function(target_package, function_key)
                    .ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::RelocationResolution,
                            location.clone(),
                            format!("package target function {function_key:?} is absent"),
                        )
                    })?;
                self.require_narrow_template(target, Some(specialization), location.clone())?;
                let canonical = target_package
                    .canonical_implementation_callable_for_function_key(function_key)
                    .ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::RelocationResolution,
                            location.clone(),
                            format!("package target {function_key:?} has no canonical callable"),
                        )
                    })?;
                if canonical != package_callable_id {
                    return Err(unsatisfied(
                        BytecodeLinkObligation::RelocationResolution,
                        location,
                        format!(
                            "package callable {package_callable_id} aliases canonical implementation {canonical}; the narrow exact-local target cannot retain both identities"
                        ),
                    ));
                }
                specialization_key(target_package, function_key, canonical.clone(), location)
            }
            _ => Err(BytecodeLinkError::ImplementationUnavailable {
                obligation: BytecodeLinkObligation::RelocationResolution,
                location,
            }),
        }
    }

    pub(super) fn require_narrow_template(
        &self,
        function: &ValidatedFunction,
        specialization: Option<&BytecodeSpecialization>,
        location: BytecodeLinkLocation,
    ) -> Result<(), BytecodeLinkError> {
        if !function.type_parameters.is_empty() || function.self_type_ref.is_some() {
            return Err(BytecodeLinkError::ImplementationUnavailable {
                obligation: BytecodeLinkObligation::ConcreteSpecialization,
                location,
            });
        }
        if specialization.is_some_and(|specialization| {
            !specialization.type_arguments.is_empty() || specialization.concrete_receiver.is_some()
        }) {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteSpecialization,
                location,
                "non-generic target carries generic or receiver specialization facts".to_string(),
            ));
        }
        Ok(())
    }

    pub(super) fn resolve_package_ref(
        &self,
        caller: &HydratedBytecodePackage,
        package_ref: &PackageRefIr,
        location: BytecodeLinkLocation,
    ) -> Result<&'a HydratedBytecodePackage, BytecodeLinkError> {
        match package_ref {
            PackageRefIr::PackageId { package_id } => {
                let mut matches = self
                    .deployment
                    .packages()
                    .values()
                    .filter(|package| package.reference().package_id == *package_id);
                let selected = matches.next().ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::ExactPackageClosure,
                        location.clone(),
                        format!("package id {package_id:?} is absent from the exact closure"),
                    )
                })?;
                if matches.next().is_some() {
                    return Err(unsatisfied(
                        BytecodeLinkObligation::ExactPackageClosure,
                        location,
                        format!("package id {package_id:?} is ambiguous in the exact closure"),
                    ));
                }
                Ok(selected)
            }
            PackageRefIr::Dependency { dependency_ref } => {
                let key = skiff_artifact_model::PackageRequirementKey {
                    caller_package_build_id: caller.reference().package_build_id.clone(),
                    package_requirement_alias: dependency_ref.clone(),
                };
                let binding = self
                    .deployment
                    .deployment()
                    .package_bindings
                    .iter()
                    .find(|binding| binding.key == key)
                    .ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::ExactPackageClosure,
                            location.clone(),
                            format!("dependency alias {dependency_ref:?} has no exact binding"),
                        )
                    })?;
                self.deployment
                    .packages()
                    .get(&binding.package.package_build_id)
                    .filter(|package| package.reference() == &binding.package)
                    .ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::ExactPackageClosure,
                            location,
                            format!(
                                "dependency alias {dependency_ref:?} target is absent from the exact closure"
                            ),
                        )
                    })
            }
        }
    }
}

pub(super) struct RelocationSource<'a> {
    package: &'a HydratedBytecodePackage,
    function: &'a ValidatedFunction,
    specialization: &'a SpecializationKey,
    substitutions: &'a BTreeMap<String, TypeRefIr>,
}

impl<'a> RelocationSource<'a> {
    pub(super) fn new(
        package: &'a HydratedBytecodePackage,
        function: &'a ValidatedFunction,
        specialization: &'a SpecializationKey,
        substitutions: &'a BTreeMap<String, TypeRefIr>,
    ) -> Self {
        Self {
            package,
            function,
            specialization,
            substitutions,
        }
    }
}

pub(super) struct RelocationContext<'a, 'deployment, 'limits> {
    linker: &'a DeploymentLinker<'deployment>,
    source: RelocationSource<'a>,
    function_indices: &'a BTreeMap<SpecializationKey, FunctionIndex>,
    type_linker: &'a mut TypeLinker<'limits>,
}

impl<'a, 'deployment, 'limits> RelocationContext<'a, 'deployment, 'limits> {
    pub(super) fn new(
        linker: &'a DeploymentLinker<'deployment>,
        source: RelocationSource<'a>,
        function_indices: &'a BTreeMap<SpecializationKey, FunctionIndex>,
        type_linker: &'a mut TypeLinker<'limits>,
    ) -> Self {
        Self {
            linker,
            source,
            function_indices,
            type_linker,
        }
    }

    pub(super) fn link(
        &mut self,
        instruction: &DecodedInstruction,
    ) -> Result<LinkedInstruction, BytecodeLinkError> {
        let location = self.linker.instruction_location(
            self.source.package,
            self.source.function,
            instruction.pc,
        );
        let contract = contract_for_opcode(instruction.descriptor.kind);
        let mut resolved = Vec::new();
        for (ordinal, operand) in contract.operands.iter().enumerate() {
            if operand.linked_kind == LinkedOperandKind::Immediate {
                continue;
            }
            let raw = *instruction.operand_words.get(ordinal).ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::RelocationResolution,
                    location.clone(),
                    format!("decoded operand ordinal {ordinal} is absent"),
                )
            })?;
            let target = self.resolve_operand(
                instruction,
                *operand,
                contract.operand_word_count(),
                raw,
                location.clone(),
            )?;
            let ordinal = u32::try_from(ordinal).map_err(|_| {
                unsatisfied(
                    BytecodeLinkObligation::RelocationResolution,
                    location.clone(),
                    "operand ordinal does not fit u32".to_string(),
                )
            })?;
            resolved.push(LinkedResolvedOperand::new(ordinal, target));
        }
        LinkedInstruction::new(
            instruction.descriptor.kind,
            instruction.operand_words.clone().into_boxed_slice(),
            resolved.into_boxed_slice(),
            instruction.pc,
        )
        .map_err(|error| {
            unsatisfied(
                BytecodeLinkObligation::RelocationResolution,
                location,
                error.to_string(),
            )
        })
    }

    fn resolve_operand(
        &mut self,
        instruction: &DecodedInstruction,
        operand: OperandSpec,
        operand_word_count: u32,
        raw: u32,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedInstructionTarget, BytecodeLinkError> {
        match operand.kind {
            OperandKind::Immediate => Err(unsatisfied(
                BytecodeLinkObligation::RelocationResolution,
                location,
                "non-immediate operand has immediate encoding".to_string(),
            )),
            OperandKind::Slot => Ok(LinkedInstructionTarget::FrameSlot(FrameSlotIndex::new(raw))),
            OperandKind::Branch => branch_target(
                self.source.function,
                instruction,
                operand_word_count,
                raw,
                location,
            ),
            OperandKind::Table => table_target(operand.linked_kind, raw, location),
            OperandKind::Pool => self.pool_target(operand.linked_kind, raw, location),
            OperandKind::Reloc => self.relocation_target(raw, location),
        }
    }

    fn pool_target(
        &mut self,
        kind: LinkedOperandKind,
        raw: u32,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedInstructionTarget, BytecodeLinkError> {
        match kind {
            LinkedOperandKind::Type => Ok(LinkedInstructionTarget::Type(
                self.type_linker.intern_pool_type(
                    self.source.package,
                    self.source.specialization,
                    raw,
                    self.source.substitutions,
                    location,
                )?,
            )),
            LinkedOperandKind::Constant => Err(BytecodeLinkError::ImplementationUnavailable {
                obligation: BytecodeLinkObligation::ConstantInitializationPlan,
                location,
            }),
            LinkedOperandKind::Shape => Err(BytecodeLinkError::ImplementationUnavailable {
                obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location,
            }),
            LinkedOperandKind::WritablePath | LinkedOperandKind::CallbackCaptureLayout => {
                Err(BytecodeLinkError::ImplementationUnavailable {
                    obligation: BytecodeLinkObligation::CallbackCapturePlan,
                    location,
                })
            }
            LinkedOperandKind::ResumeSite => Err(BytecodeLinkError::ImplementationUnavailable {
                obligation: BytecodeLinkObligation::ExceptionAndResumePlan,
                location,
            }),
            _ => Err(unsatisfied(
                BytecodeLinkObligation::RelocationResolution,
                location,
                "pool operand has a non-pool linked target kind".to_string(),
            )),
        }
    }

    fn relocation_target(
        &self,
        raw: u32,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedInstructionTarget, BytecodeLinkError> {
        let relocation = self
            .source
            .function
            .relocations
            .get(raw as usize)
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::RelocationResolution,
                    location.clone(),
                    format!("validated relocation row {raw} is absent"),
                )
            })?;
        match relocation {
            BytecodeRelocation::LocalExecutableRef { .. }
            | BytecodeRelocation::PackageCallableRef { .. } => {
                let key = self.linker.resolve_direct_target(
                    self.source.package,
                    relocation,
                    self.source.function,
                )?;
                self.function_indices
                    .get(&key)
                    .copied()
                    .map(LinkedInstructionTarget::Function)
                    .ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::RelocationResolution,
                            location,
                            "resolved local target is outside the discovered closure".to_string(),
                        )
                    })
            }
            _ => Err(BytecodeLinkError::ImplementationUnavailable {
                obligation: BytecodeLinkObligation::RelocationResolution,
                location,
            }),
        }
    }
}

fn branch_target(
    function: &ValidatedFunction,
    instruction: &DecodedInstruction,
    operand_word_count: u32,
    raw: u32,
    location: BytecodeLinkLocation,
) -> Result<LinkedInstructionTarget, BytecodeLinkError> {
    let target_pc =
        decode_branch_target(instruction.pc, operand_word_count, raw).ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location.clone(),
                "branch target arithmetic overflowed".to_string(),
            )
        })?;
    instruction_index(function, target_pc, location).map(LinkedInstructionTarget::Branch)
}

fn table_target(
    kind: LinkedOperandKind,
    raw: u32,
    location: BytecodeLinkLocation,
) -> Result<LinkedInstructionTarget, BytecodeLinkError> {
    match kind {
        LinkedOperandKind::SwitchTable => Ok(LinkedInstructionTarget::SwitchTable(
            SwitchTableIndex::new(raw),
        )),
        _ => Err(BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::ConcreteTargetTables,
            location,
        }),
    }
}

pub(super) fn specialization_key(
    package: &HydratedBytecodePackage,
    function_key: &str,
    callable: PackageCallableId,
    location: BytecodeLinkLocation,
) -> Result<SpecializationKey, BytecodeLinkError> {
    let artifact_function_key = ArtifactFunctionKey::parse(function_key).map_err(|error| {
        unsatisfied(
            BytecodeLinkObligation::ConcreteSpecialization,
            location,
            error.to_string(),
        )
    })?;
    Ok(SpecializationKey::new(
        package.reference().package_build_id.clone(),
        artifact_function_key,
        callable,
        Box::new([]),
        None,
    ))
}
