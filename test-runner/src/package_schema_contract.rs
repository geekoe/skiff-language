use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryErrorContract, BoundaryOperationContract,
    BoundaryStreamContract, ContractTypeDescriptor, ContractTypeRef, PackageSchemaTypeId,
    PackageSchemaTypeRecord, PackageTypeRequirement,
};

pub(crate) fn schema_closure(
    operations: &BTreeMap<String, BoundaryOperationContract>,
    available_records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> Result<
    (
        Vec<PackageTypeRequirement>,
        BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    ),
    String,
> {
    let mut pending = Vec::new();
    for operation in operations.values() {
        collect_operation_refs(operation, &mut pending);
    }
    let mut ids = BTreeSet::new();
    while let Some((owner, key, id)) = pending.pop() {
        let record = available_records
            .get(&id)
            .ok_or_else(|| format!("missing Package schema record {owner}:{key}:{id}"))?;
        if record.package_id != owner || record.stable_schema_key != key {
            return Err(format!(
                "Package schema record {id} owner/key does not match {owner}:{key}"
            ));
        }
        if ids.insert(id) {
            collect_descriptor_refs(&record.canonical_descriptor.descriptor, &mut pending);
        }
    }

    let records = ids
        .iter()
        .map(|id| (id.clone(), available_records[id].clone()))
        .collect::<BTreeMap<_, _>>();
    let mut grouped = BTreeMap::<String, Vec<PackageSchemaTypeId>>::new();
    for (id, record) in &records {
        grouped
            .entry(record.package_id.clone())
            .or_default()
            .push(id.clone());
    }
    let requirements = grouped
        .into_iter()
        .map(|(package_id, required_type_ids)| PackageTypeRequirement {
            package_id,
            required_type_ids,
        })
        .collect();
    Ok((requirements, records))
}

fn collect_operation_refs(
    operation: &BoundaryOperationContract,
    out: &mut Vec<(String, String, PackageSchemaTypeId)>,
) {
    operation
        .parameters
        .iter()
        .for_each(|parameter| collect_type_refs(&parameter.ty, out));
    collect_type_refs(&operation.return_value.ty, out);
    if let BoundaryErrorContract::Typed { payload_type, .. } = &operation.errors {
        collect_type_refs(payload_type, out);
    }
    if let BoundaryStreamContract::ServerStream { item_type, .. } = &operation.stream {
        collect_type_refs(item_type, out);
    }
    if let BoundaryCallbackContract::RequestScoped {
        interface_types, ..
    } = &operation.callbacks
    {
        out.extend(interface_types.iter().map(|reference| {
            (
                reference.package_id.clone(),
                reference.stable_schema_key.clone(),
                reference.package_schema_type_id.clone(),
            )
        }));
    }
}

fn collect_descriptor_refs(
    descriptor: &ContractTypeDescriptor,
    out: &mut Vec<(String, String, PackageSchemaTypeId)>,
) {
    match descriptor {
        ContractTypeDescriptor::Record { fields } => {
            fields.values().for_each(|ty| collect_type_refs(ty, out))
        }
        ContractTypeDescriptor::StructuralUnion { variants } => {
            variants.iter().for_each(|ty| collect_type_refs(ty, out))
        }
        ContractTypeDescriptor::DiscriminatedUnion { branches, .. } => branches
            .iter()
            .for_each(|branch| collect_type_refs(&branch.branch_type, out)),
        ContractTypeDescriptor::Representation { target }
        | ContractTypeDescriptor::Alias { target } => collect_type_refs(target, out),
        ContractTypeDescriptor::CallbackInterface { operations } => {
            operations.values().for_each(|operation| {
                operation
                    .parameters
                    .iter()
                    .for_each(|ty| collect_type_refs(ty, out));
                collect_type_refs(&operation.return_type, out);
            })
        }
        ContractTypeDescriptor::Enumeration { .. } => {}
    }
}

fn collect_type_refs(ty: &ContractTypeRef, out: &mut Vec<(String, String, PackageSchemaTypeId)>) {
    match ty {
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => out.push((
            package_id.clone(),
            stable_schema_key.clone(),
            package_schema_type_id.clone(),
        )),
        ContractTypeRef::Builtin { arguments, .. } => {
            arguments.iter().for_each(|ty| collect_type_refs(ty, out))
        }
        ContractTypeRef::Record { fields } => {
            fields.values().for_each(|ty| collect_type_refs(ty, out))
        }
        ContractTypeRef::StructuralUnion { variants } => {
            variants.iter().for_each(|ty| collect_type_refs(ty, out))
        }
        ContractTypeRef::Nullable { inner } => collect_type_refs(inner, out),
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            collect_type_refs(interface, out);
            arguments.iter().for_each(|ty| collect_type_refs(ty, out));
        }
        ContractTypeRef::TypeParam { .. } | ContractTypeRef::Literal { .. } => {}
    }
}
