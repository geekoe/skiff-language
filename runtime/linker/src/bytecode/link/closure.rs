use std::collections::BTreeSet;

use skiff_artifact_model::{
    contract_for_opcode, BytecodeRelocation, OperandKind, ValidatedFunction, BYTECODE_MAGIC,
};
use skiff_runtime_linked_bytecode::{
    BytecodePackageIndex, LinkedBytecodeAuthorityPins, LinkedPackageBytecodeProvenance,
    SpecializationKey,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::bytecode::{
    types::TypeLinker, worklist::CanonicalWorklist, BytecodeLinkError, BytecodeLinkLimit,
    BytecodeLinkLocation, BytecodeLinkObligation,
};

use super::{unsatisfied, DeploymentLinker};

#[derive(Debug, Clone)]
pub(super) struct ReachableRelocation {
    pub(super) specialization: SpecializationKey,
    pub(super) pc: u32,
    pub(super) relocation: BytecodeRelocation,
}

impl<'a> DeploymentLinker<'a> {
    pub(super) fn validate_exact_package_closure(&self) -> Result<(), BytecodeLinkError> {
        let location = self.deployment_location();
        let deployment_registry = self.deployment.platform_error_projection_registry();
        for package in self.deployment.packages().values() {
            if package.platform_error_projection_registry() != deployment_registry {
                return Err(unsatisfied(
                    BytecodeLinkObligation::ExactPackageClosure,
                    self.package_location(package),
                    "hydrated package platform error projection registry differs from the deployment closure receipt"
                        .to_string(),
                ));
            }
        }
        self.tracker
            .check_packages(self.deployment.packages().len() as u64, location.clone())?;
        let implementation = &self.deployment.deployment().implementation;
        let implementation_is_exact = self
            .deployment
            .packages()
            .get(&implementation.package_build_id)
            .is_some_and(|package| package.reference() == implementation);
        if !implementation_is_exact {
            return Err(unsatisfied(
                BytecodeLinkObligation::ExactPackageClosure,
                location,
                format!(
                    "implementation package {} is absent or differs from its exact hydrated reference",
                    implementation.package_build_id
                ),
            ));
        }

        let mut expected = BTreeSet::from([implementation.package_build_id.clone()]);
        for binding in &self.deployment.deployment().package_bindings {
            if !self
                .deployment
                .packages()
                .contains_key(&binding.key.caller_package_build_id)
            {
                return Err(unsatisfied(
                    BytecodeLinkObligation::ExactPackageClosure,
                    location,
                    format!(
                        "package binding caller {} is absent from the hydrated closure",
                        binding.key.caller_package_build_id
                    ),
                ));
            }
            let exact_target = self
                .deployment
                .packages()
                .get(&binding.package.package_build_id)
                .is_some_and(|package| package.reference() == &binding.package);
            if !exact_target {
                return Err(unsatisfied(
                    BytecodeLinkObligation::ExactPackageClosure,
                    location,
                    format!(
                        "package binding target {} is absent or differs from its exact hydrated reference",
                        binding.package.package_build_id
                    ),
                ));
            }
            expected.insert(binding.package.package_build_id.clone());
        }
        let actual = self
            .deployment
            .packages()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        if actual != expected {
            return Err(unsatisfied(
                BytecodeLinkObligation::ExactPackageClosure,
                location,
                format!("hydrated build closure is {actual:?}, expected {expected:?}"),
            ));
        }

        let mut package_ids = BTreeSet::new();
        for (build, package) in self.deployment.packages() {
            if build != &package.reference().package_build_id {
                return Err(unsatisfied(
                    BytecodeLinkObligation::ExactPackageClosure,
                    self.package_location(package),
                    format!(
                        "hydrated package map key {build} disagrees with exact reference {}",
                        package.reference().package_build_id
                    ),
                ));
            }
            if !package_ids.insert(package.reference().package_id.as_str()) {
                return Err(unsatisfied(
                    BytecodeLinkObligation::ExactPackageClosure,
                    self.package_location(package),
                    format!(
                        "package id {:?} has more than one owner in the exact closure",
                        package.reference().package_id
                    ),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn link_package_provenance(
        &self,
    ) -> Result<Vec<LinkedPackageBytecodeProvenance>, BytecodeLinkError> {
        self.deployment
            .packages()
            .values()
            .enumerate()
            .map(|(index, package)| {
                let index = u32::try_from(index).map_err(|_| BytecodeLinkError::LimitExceeded {
                    limit: BytecodeLinkLimit::Packages,
                    actual: self.deployment.packages().len() as u64,
                    max: u32::MAX as u64,
                    location: self.deployment_location(),
                })?;
                let view = package.bytecode().view();
                let authorities = LinkedBytecodeAuthorityPins::new(
                    view.native_value_lifecycle_registry().clone(),
                    view.value_lifecycle_policy().clone(),
                    view.host_effect_registry().clone(),
                    view.intrinsic_registry().clone(),
                    package.platform_error_projection_registry().clone(),
                )
                .map_err(|error| {
                    unsatisfied(
                        BytecodeLinkObligation::ExactPackageClosure,
                        self.package_location(package),
                        error.to_string(),
                    )
                })?;
                LinkedPackageBytecodeProvenance::new(
                    BytecodePackageIndex::new(index),
                    package.reference().package_build_id.clone(),
                    package.bytecode().reference().clone(),
                    view.bytecode_identity(),
                    BYTECODE_MAGIC,
                    view.schema_version(),
                    view.isa_version(),
                    view.opcode_table_fingerprint(),
                    authorities,
                )
                .map_err(|error| {
                    unsatisfied(
                        BytecodeLinkObligation::ExactPackageClosure,
                        self.package_location(package),
                        error.to_string(),
                    )
                })
            })
            .collect()
    }

    pub(super) fn discover_closure(
        &self,
        roots: Vec<SpecializationKey>,
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<Vec<SpecializationKey>, BytecodeLinkError> {
        let location = self.deployment_location();
        let mut worklist = CanonicalWorklist::try_from_roots(roots, self.limits, location.clone())?;
        while let Some((_, key)) = worklist.pop_next() {
            let mut discoveries = Vec::new();
            for reference in self.relocations_for_specialization(&key)? {
                let (package, function) = self.source_function(&reference.specialization)?;
                match &reference.relocation {
                    relocation @ (BytecodeRelocation::LocalExecutableRef { .. }
                    | BytecodeRelocation::PackageCallableRef { .. }) => discoveries.push((
                        reference.pc,
                        self.resolve_direct_target(package, relocation, function)?,
                    )),
                    BytecodeRelocation::ActorMethodRef {
                        actor,
                        actor_abi_identity,
                        actor_implementation_identity,
                        method_identity,
                    } => discoveries.push((
                        reference.pc,
                        self.key_for_actor_method(
                            package,
                            actor,
                            actor_abi_identity,
                            actor_implementation_identity,
                            method_identity,
                            type_linker,
                        )?,
                    )),
                    BytecodeRelocation::LocalInterfaceRef { interface } => {
                        for method in &interface.methods {
                            discoveries.push((
                                reference.pc,
                                self.key_for_receiver_function(
                                    package,
                                    &method.function_key,
                                    type_linker,
                                )?,
                            ));
                        }
                    }
                    BytecodeRelocation::SyntheticCallbackRef { function_key } => {
                        discoveries.push((
                            reference.pc,
                            self.key_for_synthetic_callback(package, function_key)?,
                        ))
                    }
                    _ => {}
                }
            }
            worklist.enqueue_discovered(discoveries, self.limits, location.clone())?;
        }
        Ok(worklist.canonical_keys().cloned().collect())
    }

    pub(super) fn reachable_relocations(
        &self,
        keys: &[SpecializationKey],
    ) -> Result<Vec<ReachableRelocation>, BytecodeLinkError> {
        let mut result = Vec::new();
        for key in keys {
            result.extend(self.relocations_for_specialization(key)?);
        }
        Ok(result)
    }

    fn relocations_for_specialization(
        &self,
        key: &SpecializationKey,
    ) -> Result<Vec<ReachableRelocation>, BytecodeLinkError> {
        let (package, function) = self.source_function(key)?;
        let mut result = Vec::new();
        for instruction in &function.instructions {
            let contract = contract_for_opcode(instruction.descriptor.kind);
            for (ordinal, operand) in contract.operands.iter().enumerate() {
                if operand.kind != OperandKind::Reloc {
                    continue;
                }
                let raw = *instruction.operand_words.get(ordinal).ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::RelocationResolution,
                        self.instruction_location(package, function, instruction.pc),
                        format!("decoded operand ordinal {ordinal} is absent"),
                    )
                })?;
                let relocation = function.relocations.get(raw as usize).ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::RelocationResolution,
                        self.instruction_location(package, function, instruction.pc),
                        format!("validated relocation row {raw} is absent"),
                    )
                })?;
                result.push(ReachableRelocation {
                    specialization: key.clone(),
                    pc: instruction.pc,
                    relocation: relocation.clone(),
                });
            }
        }
        Ok(result)
    }

    pub(super) fn source_function(
        &self,
        key: &SpecializationKey,
    ) -> Result<(&'a HydratedBytecodePackage, &'a ValidatedFunction), BytecodeLinkError> {
        let package = self
            .deployment
            .packages()
            .get(key.package_build_id())
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ExactPackageClosure,
                    self.deployment_location(),
                    format!("specialization owner {} is absent", key.package_build_id()),
                )
            })?;
        let function =
            find_function(package, key.artifact_function_key().as_str()).ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteSpecialization,
                    BytecodeLinkLocation::Package {
                        package: Box::new(package.reference().clone()),
                    },
                    format!(
                        "artifact function {:?} is absent",
                        key.artifact_function_key().as_str()
                    ),
                )
            })?;
        let canonical = package
            .canonical_implementation_callable_for_function_key(&function.function_key)
            .or_else(|| package.canonical_effect_callable_for_function_key(&function.function_key));
        if canonical != Some(key.template_function_key())
            || !key.concrete_type_arguments().is_empty()
            || key.concrete_receiver().is_some() != function.self_type_ref.is_some()
        {
            return Err(unsatisfied(
                BytecodeLinkObligation::ConcreteSpecialization,
                self.function_location(package, function),
                "specialization key differs from canonical admitted template facts".to_string(),
            ));
        }
        Ok((package, function))
    }

    pub(super) fn implementation_package(
        &self,
    ) -> Result<&'a HydratedBytecodePackage, BytecodeLinkError> {
        let implementation = &self.deployment.deployment().implementation;
        self.deployment
            .packages()
            .get(&implementation.package_build_id)
            .filter(|package| package.reference() == implementation)
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ExactPackageClosure,
                    self.deployment_location(),
                    "exact implementation package is absent".to_string(),
                )
            })
    }

    pub(super) fn deployment_location(&self) -> BytecodeLinkLocation {
        BytecodeLinkLocation::Deployment {
            deployment: Box::new(self.deployment.reference().clone()),
        }
    }

    pub(super) fn package_location(
        &self,
        package: &HydratedBytecodePackage,
    ) -> BytecodeLinkLocation {
        BytecodeLinkLocation::Package {
            package: Box::new(package.reference().clone()),
        }
    }

    pub(super) fn function_location(
        &self,
        package: &HydratedBytecodePackage,
        function: &ValidatedFunction,
    ) -> BytecodeLinkLocation {
        BytecodeLinkLocation::Function {
            package: Box::new(package.reference().clone()),
            function_key: function.function_key.clone(),
        }
    }

    pub(super) fn instruction_location(
        &self,
        package: &HydratedBytecodePackage,
        function: &ValidatedFunction,
        artifact_pc: u32,
    ) -> BytecodeLinkLocation {
        BytecodeLinkLocation::Instruction {
            package: Box::new(package.reference().clone()),
            function_key: function.function_key.clone(),
            artifact_pc,
        }
    }
}

pub(super) fn find_function<'a>(
    package: &'a HydratedBytecodePackage,
    function_key: &str,
) -> Option<&'a ValidatedFunction> {
    package
        .bytecode()
        .view()
        .functions()
        .iter()
        .find(|function| function.function_key == function_key)
}
