use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    BytecodeArtifact, BytecodePoolEntry, BytecodeRelocation, BytecodeSpecialization,
    ContractTypeDescriptor, ContractTypeRef, HostEffectSignature, InterfaceInstantiationRef,
    InterfaceMethodSlotSignatureIr, NominalTypeRefBaseIr, PackageSchemaTypeId,
    PackageSchemaTypeRecord, TypeRefIr, ValueTransferPlan,
};

/// One exact PackageSchema owner/key/identity reference retained from emitted
/// bytecode. The closure helper keeps this typed so consumers never infer a
/// package owner from a symbol or deployment selector.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BytecodeSchemaReference {
    pub package_id: String,
    pub stable_schema_key: String,
    pub type_id: PackageSchemaTypeId,
}

/// Exact schema closure facts derived from one admitted bytecode artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BytecodeSchemaFacts {
    /// Every package owner referenced by the artifact, including foreign
    /// owners whose records belong in their own package closure.
    pub referenced_package_ids: BTreeSet<String>,
    /// Records owned by the given package and reachable from its bytecode.
    pub records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
}

pub fn collect_bytecode_schema_facts(
    package_id: &str,
    artifact: &BytecodeArtifact,
    available: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> Result<BytecodeSchemaFacts, String> {
    let mut roots = BTreeSet::new();
    collect_artifact_roots(artifact, &mut roots)?;
    let mut records = BTreeMap::new();
    let mut visited = BTreeSet::new();
    for root in &roots {
        if root.package_id == package_id {
            visit_record(package_id, root, available, &mut records, &mut visited, 1)?;
        }
    }
    Ok(BytecodeSchemaFacts {
        referenced_package_ids: roots
            .into_iter()
            .map(|reference| reference.package_id)
            .collect(),
        records,
    })
}

fn visit_record(
    package_id: &str,
    reference: &BytecodeSchemaReference,
    available: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    records: &mut BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    visited: &mut BTreeSet<(String, PackageSchemaTypeId)>,
    depth: u32,
) -> Result<(), String> {
    if depth > skiff_artifact_model::MAX_BYTECODE_SCHEMA_DEPTH {
        return Err(format!(
            "bytecode schema closure for package {package_id} exceeds depth {depth}"
        ));
    }
    let key = (reference.package_id.clone(), reference.type_id.clone());
    if records.contains_key(&reference.type_id) || !visited.insert(key) {
        return Ok(());
    }
    let record = available.get(&reference.type_id).ok_or_else(|| {
        format!(
            "bytecode schema closure is missing record {}:{}:{}",
            reference.package_id, reference.stable_schema_key, reference.type_id
        )
    })?;
    if record.package_id != reference.package_id
        || record.stable_schema_key != reference.stable_schema_key
        || record.package_schema_type_id != reference.type_id
    {
        return Err(format!(
            "bytecode schema closure disagrees with reference {}:{}:{}",
            reference.package_id, reference.stable_schema_key, reference.type_id
        ));
    }
    if record.package_id != package_id {
        return Err(format!(
            "bytecode schema closure for package {package_id} cannot own foreign record {}:{}",
            record.package_id, record.stable_schema_key
        ));
    }
    records.insert(record.package_schema_type_id.clone(), record.clone());
    let mut children = Vec::new();
    collect_descriptor_references(&record.canonical_descriptor.descriptor, &mut children);
    for child in children {
        if child.package_id == package_id {
            visit_record(package_id, &child, available, records, visited, depth + 1)?;
        }
    }
    Ok(())
}

fn collect_artifact_roots(
    artifact: &BytecodeArtifact,
    roots: &mut BTreeSet<BytecodeSchemaReference>,
) -> Result<(), String> {
    let pools = &artifact.image.pools;
    for entry in &pools.types {
        if let BytecodePoolEntry::TypeRef { ty, .. } = entry {
            collect_type_ref(ty, roots);
        }
    }
    for entry in &pools.constants {
        if let BytecodePoolEntry::ConstantRef { plan, .. } = entry {
            collect_plan(plan, roots);
        }
    }
    for entry in &pools.shapes {
        if let BytecodePoolEntry::ShapeRef { shape } = entry {
            for field in &shape.fields {
                collect_plan(&field.plan, roots);
            }
        }
    }
    for entry in &pools.resume {
        if let BytecodePoolEntry::ResumeDescriptor(descriptor) = entry {
            for plan in &descriptor.result_plans {
                collect_plan(plan, roots);
            }
        }
    }
    for entry in &pools.callback_capture {
        if let BytecodePoolEntry::CallbackCaptureLayout(layout) = entry {
            for capture in &layout.captures {
                collect_plan(&capture.plan, roots);
            }
        }
    }
    for entry in &pools.effects {
        if let BytecodePoolEntry::HostEffectRef(effect) = entry {
            collect_host_signature(&effect.signature, roots)?;
        }
    }
    for function in artifact.image.functions.values() {
        for parameter in &function.frame_layout.parameter_slots {
            collect_plan(&parameter.plan, roots);
        }
        for plan in function
            .frame_layout
            .result_plans
            .iter()
            .chain(&function.frame_layout.slot_plans)
        {
            collect_plan(plan, roots);
        }
        for relocation in &function.relocations {
            collect_relocation(relocation, roots)?;
        }
    }
    Ok(())
}

fn collect_relocation(
    relocation: &BytecodeRelocation,
    roots: &mut BTreeSet<BytecodeSchemaReference>,
) -> Result<(), String> {
    match relocation {
        BytecodeRelocation::LocalExecutableRef { specialization, .. }
        | BytecodeRelocation::PackageCallableRef { specialization, .. } => {
            collect_specialization(specialization, roots);
        }
        BytecodeRelocation::InterfaceRequirementRef { interface, methods } => {
            collect_interface(interface, roots)?;
            for method in methods {
                collect_interface_signature(&method.signature, roots)?;
            }
        }
        BytecodeRelocation::LocalInterfaceRef { interface } => {
            collect_interface(&interface.interface, roots)?;
            collect_type_ref(&interface.concrete_type, roots);
            for method in &interface.methods {
                collect_interface_signature(&method.signature, roots)?;
            }
        }
        BytecodeRelocation::RemoteInterfaceRef { interface } => {
            collect_interface(&interface.interface, roots)?;
            for method in &interface.methods {
                collect_interface_signature(&method.signature, roots)?;
            }
        }
        BytecodeRelocation::ServiceOperationRef { service_call } => {
            collect_service_boundary_plan(service_call.boundary_plan(), roots);
        }
        BytecodeRelocation::HostEffectRef(effect) => {
            collect_host_signature(&effect.signature, roots)?;
        }
        BytecodeRelocation::IntrinsicRef { intrinsic } => {
            collect_host_signature(&intrinsic.signature, roots)?;
            if let Some(operation) = &intrinsic.db_operation {
                collect_type_ref(&operation.target.type_ref, roots);
                collect_type_ref(&operation.result_type, roots);
                for plan in &operation.result_plans {
                    collect_plan(plan, roots);
                }
            }
        }
        BytecodeRelocation::TypeRef { ty } => collect_type_ref(ty, roots),
        BytecodeRelocation::ActorMethodRef { .. }
        | BytecodeRelocation::SyntheticCallbackRef { .. }
        | BytecodeRelocation::TaskSubmitRef { .. }
        | BytecodeRelocation::ShapeRef { .. }
        | BytecodeRelocation::FrozenConstantRef { .. } => {}
    }
    Ok(())
}

fn collect_specialization(
    specialization: &BytecodeSpecialization,
    roots: &mut BTreeSet<BytecodeSchemaReference>,
) {
    for ty in &specialization.type_arguments {
        collect_type_ref(ty, roots);
    }
    if let Some(receiver) = &specialization.concrete_receiver {
        collect_type_ref(receiver, roots);
    }
}

fn collect_host_signature(
    signature: &HostEffectSignature,
    roots: &mut BTreeSet<BytecodeSchemaReference>,
) -> Result<(), String> {
    for ty in signature
        .parameter_types
        .iter()
        .chain(&signature.result_types)
    {
        collect_type_ref(ty, roots);
    }
    for plan in signature
        .parameter_plans
        .iter()
        .chain(&signature.result_plans)
    {
        collect_plan(plan, roots);
    }
    Ok(())
}

fn collect_interface_signature(
    signature: &InterfaceMethodSlotSignatureIr,
    roots: &mut BTreeSet<BytecodeSchemaReference>,
) -> Result<(), String> {
    for parameter in &signature.params {
        collect_type_ref(&parameter.ty, roots);
    }
    collect_type_ref(&signature.return_type, roots);
    Ok(())
}

fn collect_interface(
    interface: &InterfaceInstantiationRef,
    roots: &mut BTreeSet<BytecodeSchemaReference>,
) -> Result<(), String> {
    let identity = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
        .map_err(|error| format!("interface ABI identity is not an exact TypeRefIr: {error}"))?;
    collect_type_ref(&identity, roots);
    for argument in &interface.canonical_type_args {
        collect_type_ref(argument, roots);
    }
    Ok(())
}

fn collect_service_boundary_plan(
    plan: &skiff_artifact_model::ServiceBoundaryPlan,
    roots: &mut BTreeSet<BytecodeSchemaReference>,
) {
    let mut references = Vec::new();
    for value in plan.arguments.iter().chain(&plan.results) {
        collect_contract_type_references(&value.contract_type, &mut references);
    }
    collect_contract_type_references(&plan.error.fallback_contract_type, &mut references);
    if let Some(value) = &plan.stream_item {
        collect_contract_type_references(&value.contract_type, &mut references);
    }
    roots.extend(references);
}

fn collect_plan(plan: &ValueTransferPlan, roots: &mut BTreeSet<BytecodeSchemaReference>) {
    if let ValueTransferPlan::FromType { ty } = plan {
        collect_type_ref(ty, roots);
    }
}

fn collect_type_ref(ty: &TypeRefIr, roots: &mut BTreeSet<BytecodeSchemaReference>) {
    match ty {
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            roots.insert(BytecodeSchemaReference {
                package_id: package_id.clone(),
                stable_schema_key: stable_schema_key.clone(),
                type_id: package_schema_type_id.clone(),
            });
        }
        TypeRefIr::Builtin { args, .. } => {
            for argument in args {
                collect_type_ref(argument, roots);
            }
        }
        TypeRefIr::AppliedNominal { base, arguments } => {
            collect_nominal_base(base, roots);
            for argument in arguments {
                collect_type_ref(argument, roots);
            }
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values() {
                collect_type_ref(field, roots);
            }
        }
        TypeRefIr::Union { items } => {
            for item in items {
                collect_type_ref(item, roots);
            }
        }
        TypeRefIr::Nullable { inner } => collect_type_ref(inner, roots),
        TypeRefIr::AnyInterface { interface } => {
            let _ = collect_interface(interface, roots);
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for parameter in params {
                collect_type_ref(&parameter.ty, roots);
            }
            collect_type_ref(return_type, roots);
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => {}
    }
}

fn collect_nominal_base(
    base: &NominalTypeRefBaseIr,
    roots: &mut BTreeSet<BytecodeSchemaReference>,
) {
    if let NominalTypeRefBaseIr::PackageSchema {
        package_id,
        stable_schema_key,
        package_schema_type_id,
    } = base
    {
        roots.insert(BytecodeSchemaReference {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            type_id: package_schema_type_id.clone(),
        });
    }
}

fn collect_descriptor_references(
    descriptor: &ContractTypeDescriptor,
    output: &mut Vec<BytecodeSchemaReference>,
) {
    match descriptor {
        ContractTypeDescriptor::Record { fields } => {
            fields
                .values()
                .for_each(|field| collect_contract_type_references(field, output));
        }
        ContractTypeDescriptor::StructuralUnion { variants } => {
            variants
                .iter()
                .for_each(|variant| collect_contract_type_references(variant, output));
        }
        ContractTypeDescriptor::DiscriminatedUnion { branches, .. } => {
            branches.iter().for_each(|branch| {
                collect_contract_type_references(&branch.branch_type, output);
            });
        }
        ContractTypeDescriptor::Representation { target }
        | ContractTypeDescriptor::Alias { target } => {
            collect_contract_type_references(target, output);
        }
        ContractTypeDescriptor::CallbackInterface { operations } => {
            for operation in operations.values() {
                operation
                    .parameters
                    .iter()
                    .for_each(|parameter| collect_contract_type_references(parameter, output));
                collect_contract_type_references(&operation.return_type, output);
            }
        }
        ContractTypeDescriptor::Enumeration { .. } => {}
    }
}

fn collect_contract_type_references(
    ty: &ContractTypeRef,
    output: &mut Vec<BytecodeSchemaReference>,
) {
    match ty {
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => output.push(BytecodeSchemaReference {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            type_id: package_schema_type_id.clone(),
        }),
        ContractTypeRef::Builtin { arguments, .. }
        | ContractTypeRef::StructuralUnion {
            variants: arguments,
        } => arguments
            .iter()
            .for_each(|argument| collect_contract_type_references(argument, output)),
        ContractTypeRef::Record { fields } => fields
            .values()
            .for_each(|field| collect_contract_type_references(field, output)),
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            collect_contract_type_references(interface, output);
            arguments
                .iter()
                .for_each(|argument| collect_contract_type_references(argument, output));
        }
        ContractTypeRef::Nullable { inner } => collect_contract_type_references(inner, output),
        ContractTypeRef::TypeParam { .. } | ContractTypeRef::Literal { .. } => {}
    }
}
