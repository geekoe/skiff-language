use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    validate_bytecode_schema_records, BytecodePoolEntry, BytecodeRelocation,
    BytecodeSpecialization, ContractTypeDescriptor, ContractTypeRef, HostEffectSignature,
    InterfaceInstantiationRef, InterfaceMethodSlotSignatureIr, NominalTypeRefBaseIr,
    PackageBuildId, PackageSchemaTypeId, TypeRefIr, ValueTransferPlan, MAX_BYTECODE_SCHEMA_DEPTH,
};

use super::{manifest_error, manifest_mismatch};
use crate::deployment_bytecode::{
    DeploymentBytecodeHydrationError, DeploymentBytecodeManifestKind, HydratedBytecodePackage,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SchemaReference {
    package_id: String,
    stable_schema_key: String,
    type_id: PackageSchemaTypeId,
}

pub(super) fn validate_bytecode_schema_closure(
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let owners = packages
        .values()
        .map(|package| (package.reference().package_id.as_str(), package))
        .collect::<BTreeMap<_, _>>();
    for package in packages.values() {
        validate_bytecode_schema_records(
            &package.artifact().package_id,
            &package.artifact().bytecode_schema_records,
        )
        .map_err(|error| {
            manifest_mismatch(
                package.reference(),
                DeploymentBytecodeManifestKind::SchemaDescriptor,
                format!("package bytecode schema descriptor graph is invalid: {error}"),
            )
        })?;
    }

    let mut roots = BTreeSet::new();
    for package in packages.values() {
        collect_package_roots(package, &mut roots)?;
    }
    let mut visiting = BTreeSet::new();
    let mut complete = BTreeSet::new();
    let mut visited_by_owner = BTreeMap::<String, BTreeSet<PackageSchemaTypeId>>::new();
    for root in &roots {
        visit_schema_reference(
            root,
            &owners,
            &mut visiting,
            &mut complete,
            &mut visited_by_owner,
            1,
        )?;
    }

    for package in packages.values() {
        let expected = visited_by_owner
            .remove(&package.reference().package_id)
            .unwrap_or_default();
        let actual = package
            .artifact()
            .bytecode_schema_records
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        if actual != expected {
            return manifest_error(
                package.reference(),
                DeploymentBytecodeManifestKind::SchemaDescriptor,
                format!(
                    "bytecodeSchemaRecords keys {actual:?} do not exact-cover reachable descriptor keys {expected:?}"
                ),
            );
        }
    }
    Ok(())
}

fn collect_package_roots(
    package: &HydratedBytecodePackage,
    roots: &mut BTreeSet<SchemaReference>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let Some(bytecode) = package.bytecode() else {
        return Ok(());
    };
    let view = bytecode.view();
    for entry in &view.pools().types {
        if let BytecodePoolEntry::TypeRef { ty, .. } = entry {
            collect_type_ref(package, ty, roots)?;
        }
    }
    for entry in &view.pools().constants {
        if let BytecodePoolEntry::ConstantRef { plan, .. } = entry {
            collect_plan(package, plan, roots)?;
        }
    }
    for entry in &view.pools().shapes {
        if let BytecodePoolEntry::ShapeRef { shape } = entry {
            for field in &shape.fields {
                collect_plan(package, &field.plan, roots)?;
            }
        }
    }
    for entry in &view.pools().resume {
        if let BytecodePoolEntry::ResumeDescriptor(descriptor) = entry {
            for plan in &descriptor.result_plans {
                collect_plan(package, plan, roots)?;
            }
        }
    }
    for entry in &view.pools().callback_capture {
        if let BytecodePoolEntry::CallbackCaptureLayout(layout) = entry {
            for capture in &layout.captures {
                collect_plan(package, &capture.plan, roots)?;
            }
        }
    }
    for entry in &view.pools().effects {
        if let BytecodePoolEntry::HostEffectRef(effect) = entry {
            collect_host_signature(package, &effect.signature, roots)?;
        }
    }
    for function in view.functions() {
        for parameter in &function.frame_layout.parameter_slots {
            collect_plan(package, &parameter.plan, roots)?;
        }
        for plan in function
            .frame_layout
            .result_plans
            .iter()
            .chain(&function.frame_layout.slot_plans)
        {
            collect_plan(package, plan, roots)?;
        }
        for relocation in &function.relocations {
            collect_relocation(package, relocation, roots)?;
        }
    }
    Ok(())
}

fn collect_relocation(
    package: &HydratedBytecodePackage,
    relocation: &BytecodeRelocation,
    roots: &mut BTreeSet<SchemaReference>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    match relocation {
        BytecodeRelocation::LocalExecutableRef { specialization, .. }
        | BytecodeRelocation::PackageCallableRef { specialization, .. } => {
            collect_specialization(package, specialization, roots)?;
        }
        BytecodeRelocation::InterfaceRequirementRef { interface } => {
            collect_interface(package, interface, roots)?;
        }
        BytecodeRelocation::LocalInterfaceRef { interface } => {
            collect_interface(package, &interface.interface, roots)?;
            collect_type_ref(package, &interface.concrete_type, roots)?;
            for method in &interface.methods {
                collect_interface_signature(package, &method.signature, roots)?;
            }
        }
        BytecodeRelocation::RemoteInterfaceRef { interface } => {
            collect_interface(package, &interface.interface, roots)?;
            for method in &interface.methods {
                collect_interface_signature(package, &method.signature, roots)?;
            }
        }
        BytecodeRelocation::HostEffectRef(effect) => {
            collect_host_signature(package, &effect.signature, roots)?;
        }
        BytecodeRelocation::IntrinsicRef { intrinsic } => {
            collect_host_signature(package, &intrinsic.signature, roots)?;
        }
        BytecodeRelocation::TypeRef { ty } => collect_type_ref(package, ty, roots)?,
        BytecodeRelocation::ServiceOperationRef { .. }
        | BytecodeRelocation::ActorMethodRef { .. }
        | BytecodeRelocation::SyntheticCallbackRef { .. }
        | BytecodeRelocation::ShapeRef { .. }
        | BytecodeRelocation::FrozenConstantRef { .. } => {}
    }
    Ok(())
}

fn collect_specialization(
    package: &HydratedBytecodePackage,
    specialization: &BytecodeSpecialization,
    roots: &mut BTreeSet<SchemaReference>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    for ty in &specialization.type_arguments {
        collect_type_ref(package, ty, roots)?;
    }
    if let Some(receiver) = &specialization.concrete_receiver {
        collect_type_ref(package, receiver, roots)?;
    }
    Ok(())
}

fn collect_host_signature(
    package: &HydratedBytecodePackage,
    signature: &HostEffectSignature,
    roots: &mut BTreeSet<SchemaReference>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    for ty in signature
        .parameter_types
        .iter()
        .chain(&signature.result_types)
    {
        collect_type_ref(package, ty, roots)?;
    }
    for plan in signature
        .parameter_plans
        .iter()
        .chain(&signature.result_plans)
    {
        collect_plan(package, plan, roots)?;
    }
    Ok(())
}

fn collect_interface_signature(
    package: &HydratedBytecodePackage,
    signature: &InterfaceMethodSlotSignatureIr,
    roots: &mut BTreeSet<SchemaReference>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    for parameter in &signature.params {
        collect_type_ref(package, &parameter.ty, roots)?;
    }
    collect_type_ref(package, &signature.return_type, roots)
}

fn collect_interface(
    package: &HydratedBytecodePackage,
    interface: &InterfaceInstantiationRef,
    roots: &mut BTreeSet<SchemaReference>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let identity =
        serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).map_err(|error| {
            manifest_mismatch(
                package.reference(),
                DeploymentBytecodeManifestKind::SchemaDescriptor,
                format!("interface ABI identity is not an exact TypeRefIr: {error}"),
            )
        })?;
    let canonical = skiff_canonical_json::canonical_json_bytes(&identity).map_err(|error| {
        manifest_mismatch(
            package.reference(),
            DeploymentBytecodeManifestKind::SchemaDescriptor,
            format!("interface ABI identity cannot be canonicalized: {error}"),
        )
    })?;
    if canonical != interface.interface_abi_id.as_bytes() {
        return manifest_error(
            package.reference(),
            DeploymentBytecodeManifestKind::SchemaDescriptor,
            "interface ABI identity is not canonical TypeRefIr JSON".to_string(),
        );
    }
    collect_type_ref(package, &identity, roots)?;
    for argument in &interface.canonical_type_args {
        collect_type_ref(package, argument, roots)?;
    }
    Ok(())
}

fn collect_plan(
    package: &HydratedBytecodePackage,
    plan: &ValueTransferPlan,
    roots: &mut BTreeSet<SchemaReference>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    if let ValueTransferPlan::FromType { ty } = plan {
        collect_type_ref(package, ty, roots)?;
    }
    Ok(())
}

fn collect_type_ref(
    package: &HydratedBytecodePackage,
    ty: &TypeRefIr,
    roots: &mut BTreeSet<SchemaReference>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    match ty {
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            roots.insert(SchemaReference {
                package_id: package_id.clone(),
                stable_schema_key: stable_schema_key.clone(),
                type_id: package_schema_type_id.clone(),
            });
        }
        TypeRefIr::Builtin { args, .. } => {
            for argument in args {
                collect_type_ref(package, argument, roots)?;
            }
        }
        TypeRefIr::AppliedNominal { base, arguments } => {
            collect_nominal_base(base, roots);
            for argument in arguments {
                collect_type_ref(package, argument, roots)?;
            }
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values() {
                collect_type_ref(package, field, roots)?;
            }
        }
        TypeRefIr::Union { items } => {
            for item in items {
                collect_type_ref(package, item, roots)?;
            }
        }
        TypeRefIr::Nullable { inner } => collect_type_ref(package, inner, roots)?,
        TypeRefIr::AnyInterface { interface } => collect_interface(package, interface, roots)?,
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for parameter in params {
                collect_type_ref(package, &parameter.ty, roots)?;
            }
            collect_type_ref(package, return_type, roots)?;
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => {}
    }
    Ok(())
}

fn collect_nominal_base(base: &NominalTypeRefBaseIr, roots: &mut BTreeSet<SchemaReference>) {
    if let NominalTypeRefBaseIr::PackageSchema {
        package_id,
        stable_schema_key,
        package_schema_type_id,
    } = base
    {
        roots.insert(SchemaReference {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            type_id: package_schema_type_id.clone(),
        });
    }
}

fn visit_schema_reference(
    reference: &SchemaReference,
    owners: &BTreeMap<&str, &HydratedBytecodePackage>,
    visiting: &mut BTreeSet<(String, PackageSchemaTypeId)>,
    complete: &mut BTreeSet<(String, PackageSchemaTypeId)>,
    visited_by_owner: &mut BTreeMap<String, BTreeSet<PackageSchemaTypeId>>,
    depth: u32,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let owner = owners.get(reference.package_id.as_str()).ok_or_else(|| {
        crate::deployment_bytecode::DeploymentBytecodeHydrationError::MissingSchemaPackageOwner {
            package_id: reference.package_id.clone(),
            stable_schema_key: reference.stable_schema_key.clone(),
            type_id: reference.type_id.clone(),
        }
    })?;
    if depth > MAX_BYTECODE_SCHEMA_DEPTH {
        return manifest_error(
            owner.reference(),
            DeploymentBytecodeManifestKind::SchemaDescriptor,
            format!(
                "cross-package bytecode schema closure exceeds depth {MAX_BYTECODE_SCHEMA_DEPTH}"
            ),
        );
    }
    let key = (reference.package_id.clone(), reference.type_id.clone());
    if complete.contains(&key) {
        return Ok(());
    }
    if !visiting.insert(key.clone()) {
        return manifest_error(
            owner.reference(),
            DeploymentBytecodeManifestKind::SchemaDescriptor,
            format!("cross-package bytecode schema cycle reaches {key:?}"),
        );
    }
    let record = owner
        .artifact()
        .bytecode_schema_records
        .get(&reference.type_id)
        .ok_or_else(|| {
            manifest_mismatch(
                owner.reference(),
                DeploymentBytecodeManifestKind::SchemaDescriptor,
                format!(
                    "reachable PackageSchema record {}:{}:{} is missing",
                    reference.package_id, reference.stable_schema_key, reference.type_id
                ),
            )
        })?;
    if record.package_id != reference.package_id
        || record.stable_schema_key != reference.stable_schema_key
        || record.package_schema_type_id != reference.type_id
    {
        return manifest_error(
            owner.reference(),
            DeploymentBytecodeManifestKind::SchemaDescriptor,
            format!(
                "PackageSchema record {} disagrees with reference {}:{}:{}",
                record.package_schema_type_id,
                reference.package_id,
                reference.stable_schema_key,
                reference.type_id
            ),
        );
    }
    visited_by_owner
        .entry(reference.package_id.clone())
        .or_default()
        .insert(reference.type_id.clone());
    let mut children = Vec::new();
    collect_descriptor_references(&record.canonical_descriptor.descriptor, &mut children);
    for child in children {
        visit_schema_reference(
            &child,
            owners,
            visiting,
            complete,
            visited_by_owner,
            depth + 1,
        )?;
    }
    visiting.remove(&key);
    complete.insert(key);
    Ok(())
}

fn collect_descriptor_references(
    descriptor: &ContractTypeDescriptor,
    output: &mut Vec<SchemaReference>,
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

fn collect_contract_type_references(ty: &ContractTypeRef, output: &mut Vec<SchemaReference>) {
    match ty {
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => output.push(SchemaReference {
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
