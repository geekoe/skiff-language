use skiff_artifact_model::{
    contract_for_opcode, decode_branch_target, BytecodeIntrinsicRef, BytecodeRelocation,
    BytecodeSpecialization, OperandKind, PackageRefIr, ValidatedFunction,
};
use skiff_runtime_linked_bytecode::{
    LinkedBytecodeCandidate, LinkedFunction, LinkedInstructionTarget, LinkedInterfaceTableKind,
    LinkedIntrinsicKind,
};
use skiff_runtime_loader::{HydratedBytecodePackage, HydratedDeploymentBytecode};

use crate::{VerificationError, VerificationLocation};

use super::{instruction_index_for_pc, source_function};
use crate::admission::binding::{
    semantic_violation, type_origins::prove_inline_type_relocation, TargetCoverage,
};

pub(super) fn prove_instructions(
    hydrated: &HydratedDeploymentBytecode,
    package: &HydratedBytecodePackage,
    function: &LinkedFunction,
    source: &ValidatedFunction,
    candidate: &LinkedBytecodeCandidate,
    coverage: &mut TargetCoverage,
) -> Result<(), VerificationError> {
    let function_location = VerificationLocation::Function {
        function: function.index(),
    };
    if function.instructions().len() != source.instructions.len() {
        return Err(semantic_violation(
            function_location,
            "linked raw-instruction coverage differs from the admitted function",
        ));
    }
    for (position, (linked, artifact)) in function
        .instructions()
        .iter()
        .zip(&source.instructions)
        .enumerate()
    {
        let instruction = u32::try_from(position).map_err(|_| {
            semantic_violation(function_location, "instruction index does not fit u32")
        })?;
        let location = VerificationLocation::Instruction {
            function: function.index(),
            instruction: skiff_runtime_linked_bytecode::InstructionIndex::new(instruction),
        };
        if linked.opcode() != artifact.descriptor.kind
            || linked.artifact_pc() != artifact.pc
            || linked.operands() != artifact.operand_words
        {
            return Err(semantic_violation(
                location,
                "linked opcode, pc, or raw operand words differ from the admitted instruction",
            ));
        }
        prove_typed_operands(
            hydrated, package, function, source, candidate, linked, artifact, coverage, location,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn prove_typed_operands(
    hydrated: &HydratedDeploymentBytecode,
    package: &HydratedBytecodePackage,
    function: &LinkedFunction,
    source: &ValidatedFunction,
    candidate: &LinkedBytecodeCandidate,
    linked: &skiff_runtime_linked_bytecode::LinkedInstruction,
    artifact: &skiff_artifact_model::DecodedInstruction,
    coverage: &mut TargetCoverage,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let contract = contract_for_opcode(linked.opcode());
    if contract.kind != artifact.descriptor.kind
        || contract.operands.len() != artifact.operand_words.len()
    {
        return Err(semantic_violation(
            location,
            "admitted descriptor and canonical opcode contract disagree",
        ));
    }
    for (ordinal, specification) in contract.operands.iter().enumerate() {
        let ordinal_u32 = u32::try_from(ordinal)
            .map_err(|_| semantic_violation(location, "operand ordinal does not fit u32"))?;
        let resolved = linked
            .resolved_operands()
            .iter()
            .find(|resolved| resolved.operand_ordinal() == ordinal_u32);
        if specification.linked_kind == skiff_artifact_model::LinkedOperandKind::Immediate {
            if resolved.is_some() {
                return Err(semantic_violation(
                    location,
                    "immediate operand unexpectedly carries a typed target",
                ));
            }
            continue;
        }
        let target = resolved
            .ok_or_else(|| semantic_violation(location, "typed operand target is missing"))?
            .target();
        if target.kind() != specification.linked_kind {
            return Err(semantic_violation(
                location,
                "typed operand target kind differs from the canonical opcode contract",
            ));
        }
        coverage.record(target);
        let raw = artifact.operand_words[ordinal];
        match specification.kind {
            OperandKind::Immediate => {
                return Err(semantic_violation(
                    location,
                    "canonical immediate operand requires no typed target",
                ));
            }
            OperandKind::Slot | OperandKind::Table => {
                prove_direct_index_target(target, raw, location)?;
            }
            OperandKind::Branch => {
                prove_branch_target(target, raw, source, artifact, location)?;
            }
            OperandKind::Pool => {
                prove_pool_target(package, function, candidate, target, raw, location)?;
            }
            OperandKind::Reloc => {
                let relocation = source.relocations.get(raw as usize).ok_or_else(|| {
                    semantic_violation(location, "raw relocation index is out of bounds")
                })?;
                if !specification
                    .allowed_relocations
                    .contains(&relocation.kind())
                {
                    return Err(semantic_violation(
                        location,
                        "relocation kind is not allowed by the canonical operand contract",
                    ));
                }
                prove_relocation_target(
                    hydrated, package, function, candidate, relocation, target, location,
                )?;
            }
        }
    }
    if linked.resolved_operands().len()
        != contract
            .operands
            .iter()
            .filter(|operand| {
                operand.linked_kind != skiff_artifact_model::LinkedOperandKind::Immediate
            })
            .count()
    {
        return Err(semantic_violation(
            location,
            "linked instruction has an extra typed operand sidecar",
        ));
    }
    Ok(())
}

fn prove_direct_index_target(
    target: LinkedInstructionTarget,
    raw: u32,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let actual = match target {
        LinkedInstructionTarget::FrameSlot(index) => index.get(),
        LinkedInstructionTarget::SwitchTable(index) => index.get(),
        LinkedInstructionTarget::ActiveRegion(index) => index.get(),
        LinkedInstructionTarget::CallLoanLayout(index) => index.get(),
        _ => {
            return Err(semantic_violation(
                location,
                "slot/table operand resolved to a non-local target",
            ));
        }
    };
    if actual != raw {
        return Err(semantic_violation(
            location,
            "linked local target index differs from its raw artifact operand",
        ));
    }
    Ok(())
}

fn prove_branch_target(
    target: LinkedInstructionTarget,
    raw: u32,
    source: &ValidatedFunction,
    artifact: &skiff_artifact_model::DecodedInstruction,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let LinkedInstructionTarget::Branch(target) = target else {
        return Err(semantic_violation(
            location,
            "branch operand resolved to a non-instruction target",
        ));
    };
    let target_pc =
        decode_branch_target(artifact.pc, artifact.descriptor.operand_word_count(), raw)
            .ok_or_else(|| semantic_violation(location, "branch target arithmetic overflowed"))?;
    if target != instruction_index_for_pc(source, target_pc, location)? {
        return Err(semantic_violation(
            location,
            "linked branch target differs from the decoded artifact target",
        ));
    }
    Ok(())
}

fn prove_pool_target(
    package: &HydratedBytecodePackage,
    function: &LinkedFunction,
    candidate: &LinkedBytecodeCandidate,
    target: LinkedInstructionTarget,
    raw: u32,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    match target {
        LinkedInstructionTarget::Constant(index) => prove_pool_origin(
            candidate
                .constants()
                .get(index.get() as usize)
                .map(|row| row.origin()),
            package,
            function,
            raw,
            location,
        ),
        LinkedInstructionTarget::Type(index) => prove_pool_origin(
            candidate
                .types()
                .get(index.get() as usize)
                .map(|row| row.origin()),
            package,
            function,
            raw,
            location,
        ),
        LinkedInstructionTarget::Shape(index) => prove_pool_origin(
            candidate
                .shapes()
                .get(index.get() as usize)
                .map(|row| row.origin()),
            package,
            function,
            raw,
            location,
        ),
        LinkedInstructionTarget::WritablePath(index) => prove_pool_origin(
            candidate
                .writable_paths()
                .get(index.get() as usize)
                .map(|row| row.origin()),
            package,
            function,
            raw,
            location,
        ),
        LinkedInstructionTarget::CallbackCaptureLayout(index) => prove_pool_origin(
            candidate
                .callback_capture_layouts()
                .get(index.get() as usize)
                .map(|row| row.origin()),
            package,
            function,
            raw,
            location,
        ),
        LinkedInstructionTarget::ResumeSite(index) => {
            let resume = candidate
                .resume_sites()
                .get(index.get() as usize)
                .ok_or_else(|| semantic_violation(location, "resume target is out of bounds"))?;
            let exact = resume.function() == function.index()
                && package
                    .bytecode()
                    .view()
                    .resume_sites()
                    .iter()
                    .any(|source| {
                        source.descriptor_index == raw
                            && source.function_key
                                == function.key().artifact_function_key().as_str()
                    });
            if !exact {
                return Err(semantic_violation(
                    location,
                    "resume target does not bind the exact artifact descriptor and specialization",
                ));
            }
            Ok(())
        }
        _ => Err(semantic_violation(
            location,
            "pool operand resolved to a non-pool target",
        )),
    }
}

fn prove_pool_origin<I>(
    origin: Option<&skiff_runtime_linked_bytecode::LinkedArtifactPoolOrigin<I>>,
    package: &HydratedBytecodePackage,
    function: &LinkedFunction,
    artifact_index: u32,
    location: VerificationLocation,
) -> Result<(), VerificationError>
where
    I: ArtifactIndex,
{
    let origin =
        origin.ok_or_else(|| semantic_violation(location, "pool target is out of bounds"))?;
    let exact_specialization = origin
        .specialization()
        .is_none_or(|specialization| specialization == function.key());
    if origin.package_build_id() != &package.reference().package_build_id
        || origin.artifact_index().get_index() != artifact_index
        || !exact_specialization
    {
        return Err(semantic_violation(
            location,
            "linked pool target does not carry the exact artifact row and specialization origin",
        ));
    }
    Ok(())
}

trait ArtifactIndex {
    fn get_index(&self) -> u32;
}

macro_rules! artifact_indices {
    ($($index:ty),+ $(,)?) => {
        $(
            impl ArtifactIndex for $index {
                fn get_index(&self) -> u32 {
                    self.get()
                }
            }
        )+
    };
}

artifact_indices!(
    skiff_runtime_linked_bytecode::ArtifactConstantIndex,
    skiff_runtime_linked_bytecode::ArtifactTypeIndex,
    skiff_runtime_linked_bytecode::ArtifactShapeIndex,
    skiff_runtime_linked_bytecode::ArtifactWritablePathIndex,
    skiff_runtime_linked_bytecode::ArtifactCallbackCaptureIndex,
);

fn prove_relocation_target(
    hydrated: &HydratedDeploymentBytecode,
    package: &HydratedBytecodePackage,
    function: &LinkedFunction,
    candidate: &LinkedBytecodeCandidate,
    relocation: &BytecodeRelocation,
    target: LinkedInstructionTarget,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    match (relocation, target) {
        (
            BytecodeRelocation::LocalExecutableRef {
                function_key,
                specialization,
            },
            LinkedInstructionTarget::Function(index),
        ) => prove_direct_function(
            candidate,
            index,
            package,
            function_key,
            specialization,
            location,
        ),
        (
            BytecodeRelocation::PackageCallableRef {
                package_ref,
                package_callable_id,
                specialization,
            },
            LinkedInstructionTarget::Function(index),
        ) => prove_package_function(
            hydrated,
            package,
            candidate,
            index,
            package_ref,
            package_callable_id,
            specialization,
            location,
        ),
        (
            BytecodeRelocation::ServiceOperationRef { service_call },
            LinkedInstructionTarget::ServiceOperation(index),
        ) => {
            let linked = candidate
                .service_operations()
                .get(index.get() as usize)
                .ok_or_else(|| semantic_violation(location, "service target is out of bounds"))?;
            let exact = linked.service_requirement_key().caller_package_build_id
                == package.reference().package_build_id
                && linked.service_requirement_key().service_requirement_slot
                    == service_call.service_requirement_slot
                && linked.contract_operation_id() == &service_call.contract_operation_id
                && linked.expected_protocol_identity() == &service_call.expected_protocol_identity;
            exact_or_error(exact, location, "service relocation target")
        }
        (
            BytecodeRelocation::ActorMethodRef { .. },
            LinkedInstructionTarget::ActorMethod(index),
        ) => prove_actor_target(package, candidate, index.get(), relocation, location),
        (
            BytecodeRelocation::InterfaceRequirementRef { .. }
            | BytecodeRelocation::LocalInterfaceRef { .. }
            | BytecodeRelocation::RemoteInterfaceRef { .. },
            LinkedInstructionTarget::InterfaceTable(index),
        ) => prove_interface_target(package, candidate, index.get(), relocation, location),
        (
            BytecodeRelocation::SyntheticCallbackRef { function_key },
            LinkedInstructionTarget::SyntheticCallback(index),
        ) => prove_callback_target(package, candidate, index.get(), function_key, location),
        _ => prove_value_relocation_target(
            package, function, candidate, relocation, target, location,
        ),
    }
}

fn prove_value_relocation_target(
    package: &HydratedBytecodePackage,
    function: &LinkedFunction,
    candidate: &LinkedBytecodeCandidate,
    relocation: &BytecodeRelocation,
    target: LinkedInstructionTarget,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    match (relocation, target) {
        (
            BytecodeRelocation::HostEffectRef(effect),
            LinkedInstructionTarget::HostEffectAdapter(index),
        ) => {
            let linked = candidate
                .host_effect_adapters()
                .get(index.get() as usize)
                .ok_or_else(|| semantic_violation(location, "host target is out of bounds"))?;
            // Proves the exact linked typed ID: the canonical namespace,
            // symbol, binding ID and metadata must match the artifact's
            // relocation facts. Binding semantics (arity, types, plans,
            // required context) are never re-derived here; the pinned linked
            // entry is their only authority.
            let exact = linked.namespace() == effect.target.namespace
                && linked.symbol() == effect.target.symbol
                && effect.target.binding_key.as_deref() == Some(linked.binding_key().as_str())
                && linked.metadata() == &effect.target.metadata;
            exact_or_error(exact, location, "host-effect relocation target")?;
            Ok(())
        }
        (
            BytecodeRelocation::IntrinsicRef { intrinsic },
            LinkedInstructionTarget::Intrinsic(index),
        ) => prove_intrinsic_target(candidate, index.get(), intrinsic, location),
        (BytecodeRelocation::TypeRef { ty }, LinkedInstructionTarget::Type(index)) => {
            let linked = candidate
                .types()
                .get(index.get() as usize)
                .ok_or_else(|| semantic_violation(location, "type target is out of bounds"))?;
            prove_inline_type_relocation(package, function, linked, ty, location)
        }
        (BytecodeRelocation::ShapeRef { shape_index }, LinkedInstructionTarget::Shape(index)) => {
            let linked = candidate
                .shapes()
                .get(index.get() as usize)
                .ok_or_else(|| semantic_violation(location, "shape target is out of bounds"))?;
            let exact = linked.origin().package_build_id() == &package.reference().package_build_id
                && linked.origin().artifact_index().get() == *shape_index
                && linked
                    .origin()
                    .specialization()
                    .is_none_or(|specialization| specialization == function.key());
            exact_or_error(exact, location, "shape relocation target")
        }
        (
            BytecodeRelocation::FrozenConstantRef { node_index },
            LinkedInstructionTarget::Constant(index),
        ) => prove_frozen_constant_target(
            package,
            candidate,
            index.get(),
            *node_index,
            function,
            location,
        ),
        _ => Err(semantic_violation(
            location,
            "typed relocation target variant differs from the exact artifact relocation",
        )),
    }
}

fn prove_direct_function(
    candidate: &LinkedBytecodeCandidate,
    index: skiff_runtime_linked_bytecode::FunctionIndex,
    package: &HydratedBytecodePackage,
    function_key: &str,
    specialization: &BytecodeSpecialization,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let linked = candidate
        .functions()
        .get(index.get() as usize)
        .ok_or_else(|| semantic_violation(location, "function target is out of bounds"))?;
    let canonical = package.canonical_implementation_callable_for_function_key(function_key);
    let exact = linked.key().package_build_id() == &package.reference().package_build_id
        && linked.key().artifact_function_key().as_str() == function_key
        && canonical == Some(linked.key().template_function_key())
        && specialization_shape_matches(linked, specialization);
    exact_or_error(exact, location, "local executable relocation target")
}

#[allow(clippy::too_many_arguments)]
fn prove_package_function(
    hydrated: &HydratedDeploymentBytecode,
    caller: &HydratedBytecodePackage,
    candidate: &LinkedBytecodeCandidate,
    index: skiff_runtime_linked_bytecode::FunctionIndex,
    package_ref: &PackageRefIr,
    callable: &skiff_artifact_model::PackageCallableId,
    specialization: &BytecodeSpecialization,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let linked = candidate
        .functions()
        .get(index.get() as usize)
        .ok_or_else(|| semantic_violation(location, "function target is out of bounds"))?;
    let target = resolve_package_ref(hydrated, caller, package_ref).ok_or_else(|| {
        semantic_violation(
            location,
            "package callable reference has no exact hydrated target",
        )
    })?;
    let exact_function_key = target.function_key_for_callable(callable);
    let canonical = exact_function_key.and_then(|function_key| {
        target.canonical_implementation_callable_for_function_key(function_key)
    });
    let exact = linked.key().package_build_id() == &target.reference().package_build_id
        && exact_function_key == Some(linked.key().artifact_function_key().as_str())
        && linked.key().template_function_key() == callable
        && canonical.and_then(|canonical| {
            target.function_key_for_canonical_implementation_callable(canonical)
        }) == exact_function_key
        && specialization_shape_matches(linked, specialization);
    exact_or_error(exact, location, "package callable relocation target")
}

fn specialization_shape_matches(
    function: &LinkedFunction,
    specialization: &BytecodeSpecialization,
) -> bool {
    function.key().concrete_type_arguments().len() == specialization.type_arguments.len()
        && function.key().concrete_receiver().is_some()
            == specialization.concrete_receiver.is_some()
}

fn resolve_package_ref<'a>(
    hydrated: &'a HydratedDeploymentBytecode,
    caller: &HydratedBytecodePackage,
    reference: &PackageRefIr,
) -> Option<&'a HydratedBytecodePackage> {
    match reference {
        PackageRefIr::PackageId { package_id } => {
            let mut matches = hydrated
                .packages()
                .values()
                .filter(|package| package.reference().package_id == *package_id);
            let selected = matches.next()?;
            matches.next().is_none().then_some(selected)
        }
        PackageRefIr::Dependency { dependency_ref } => {
            let key = skiff_artifact_model::PackageRequirementKey {
                caller_package_build_id: caller.reference().package_build_id.clone(),
                package_requirement_alias: dependency_ref.clone(),
            };
            let binding = hydrated
                .deployment()
                .package_bindings
                .iter()
                .find(|binding| binding.key == key)?;
            hydrated
                .packages()
                .get(&binding.package.package_build_id)
                .filter(|package| package.reference() == &binding.package)
        }
    }
}

fn prove_actor_target(
    package: &HydratedBytecodePackage,
    candidate: &LinkedBytecodeCandidate,
    index: u32,
    relocation: &BytecodeRelocation,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let BytecodeRelocation::ActorMethodRef {
        actor,
        actor_abi_identity,
        actor_implementation_identity,
        method_identity,
    } = relocation
    else {
        return Err(semantic_violation(location, "expected actor relocation"));
    };
    let linked = candidate
        .actor_methods()
        .get(index as usize)
        .ok_or_else(|| semantic_violation(location, "actor target is out of bounds"))?;
    let exact = linked.owner_package_build_id() == &package.reference().package_build_id
        && linked.actor() == actor
        && linked.actor_abi_identity() == actor_abi_identity
        && linked.actor_implementation_identity() == actor_implementation_identity
        && linked.method_identity() == method_identity;
    exact_or_error(exact, location, "actor relocation target")
}

fn prove_interface_target(
    package: &HydratedBytecodePackage,
    candidate: &LinkedBytecodeCandidate,
    index: u32,
    relocation: &BytecodeRelocation,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let linked = candidate
        .interface_tables()
        .get(index as usize)
        .ok_or_else(|| semantic_violation(location, "interface target is out of bounds"))?;
    let exact = match (relocation, linked.kind()) {
        (
            BytecodeRelocation::InterfaceRequirementRef { interface },
            LinkedInterfaceTableKind::Requirement(_) | LinkedInterfaceTableKind::Callback(_),
        ) => linked.interface().artifact() == interface,
        (
            BytecodeRelocation::LocalInterfaceRef { interface },
            LinkedInterfaceTableKind::Local(table),
        ) => {
            linked.interface().artifact() == &interface.interface
                && local_interface_rows_match(package, candidate, table, interface)
        }
        (
            BytecodeRelocation::RemoteInterfaceRef { interface },
            LinkedInterfaceTableKind::Remote(table),
        ) => {
            linked.interface().artifact() == &interface.interface
                && table.service_requirement_key().caller_package_build_id
                    == package.reference().package_build_id
                && table.service_requirement_key().service_requirement_slot
                    == interface.service_requirement_slot
                && table.public_instance_key().as_str() == interface.public_instance_key
                && table.callee_protocol_identity() == &interface.callee_protocol_identity
                && table.methods().len() == interface.methods.len()
                && table
                    .methods()
                    .iter()
                    .zip(&interface.methods)
                    .all(|(linked, source)| {
                        linked.method_slot() == source.slot
                            && linked.method_abi_id().as_str() == source.method_abi_id
                            && linked.contract_operation_id() == &source.contract_operation_id
                    })
        }
        _ => false,
    };
    exact_or_error(exact, location, "interface relocation target")
}

fn local_interface_rows_match(
    package: &HydratedBytecodePackage,
    candidate: &LinkedBytecodeCandidate,
    linked: &skiff_runtime_linked_bytecode::LinkedLocalInterfaceTable,
    source: &skiff_artifact_model::LocalInterfaceRef,
) -> bool {
    linked.methods().len() == source.methods.len()
        && linked
            .methods()
            .iter()
            .zip(&source.methods)
            .all(|(linked, source)| {
                let function = candidate.functions().get(linked.function().get() as usize);
                linked.method_slot() == source.slot
                    && linked.method_name() == source.method_name
                    && linked.method_abi_id().as_str() == source.method_abi_id
                    && linked.receiver_call_abi() == source.receiver_call_abi
                    && function.is_some_and(|function| {
                        function.key().package_build_id() == &package.reference().package_build_id
                            && function.key().artifact_function_key().as_str()
                                == source.function_key
                    })
            })
}

fn prove_callback_target(
    package: &HydratedBytecodePackage,
    candidate: &LinkedBytecodeCandidate,
    index: u32,
    function_key: &str,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let linked = candidate
        .synthetic_callbacks()
        .get(index as usize)
        .ok_or_else(|| semantic_violation(location, "callback target is out of bounds"))?;
    let function = candidate
        .functions()
        .get(linked.function().get() as usize)
        .and_then(|function| {
            (function.key().package_build_id() == &package.reference().package_build_id)
                .then_some(function)
        });
    let exact = linked.artifact_function_key().as_str() == function_key
        && function.is_some_and(|function| {
            function.key().artifact_function_key().as_str() == function_key
                && source_function(package, function.key()).is_some()
        });
    exact_or_error(exact, location, "synthetic callback relocation target")
}

fn prove_intrinsic_target(
    candidate: &LinkedBytecodeCandidate,
    index: u32,
    source: &skiff_artifact_model::IntrinsicReference,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let linked = candidate
        .intrinsics()
        .get(index as usize)
        .ok_or_else(|| semantic_violation(location, "intrinsic target is out of bounds"))?;
    let exact = match (linked.kind(), &source.target) {
        (
            LinkedIntrinsicKind::Static(linked),
            BytecodeIntrinsicRef::Static {
                canonical_key,
                signature_version,
            },
        ) => {
            linked.canonical_key().as_str() == canonical_key
                && linked.signature_version() == *signature_version
        }
        (LinkedIntrinsicKind::Receiver(linked), BytecodeIntrinsicRef::Receiver { op }) => {
            linked == op
        }
        _ => false,
    };
    exact_or_error(exact, location, "intrinsic relocation target")?;
    if !skiff_artifact_model::intrinsic_registry()
        .entries()
        .iter()
        .any(|entry| entry.target == source.target)
    {
        return Err(semantic_violation(
            location,
            "intrinsic target is absent from the frozen intrinsic registry",
        ));
    }
    Ok(())
}

fn prove_frozen_constant_target(
    package: &HydratedBytecodePackage,
    candidate: &LinkedBytecodeCandidate,
    constant_index: u32,
    artifact_node_index: u32,
    function: &LinkedFunction,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let constant = candidate
        .constants()
        .get(constant_index as usize)
        .ok_or_else(|| semantic_violation(location, "constant target is out of bounds"))?;
    let node = candidate
        .frozen_constant_nodes()
        .get(constant.reference().node().get() as usize)
        .ok_or_else(|| semantic_violation(location, "constant node is out of bounds"))?;
    let exact = node.origin().package_build_id() == &package.reference().package_build_id
        && node.origin().artifact_index().get() == artifact_node_index
        && node
            .origin()
            .specialization()
            .is_none_or(|specialization| specialization == function.key());
    exact_or_error(exact, location, "frozen constant relocation target")
}

fn exact_or_error(
    exact: bool,
    location: VerificationLocation,
    kind: &'static str,
) -> Result<(), VerificationError> {
    if !exact {
        return Err(semantic_violation(
            location,
            format!("linked {kind} differs from its exact artifact authority"),
        ));
    }
    Ok(())
}
