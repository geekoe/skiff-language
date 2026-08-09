use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ContractLiteral, ContractTypeDescriptor, ContractTypeRef, PackageSchemaCanonicalDescriptor,
    PackageSchemaTypeId, PackageSchemaTypeRecord,
};

use super::super::authority::PackageBuildAuthorityValidationError;
use super::{
    MAX_BYTECODE_SCHEMA_DEPTH, MAX_BYTECODE_SCHEMA_STRING_BYTES, MAX_BYTECODE_SCHEMA_TYPE_NODES,
};

pub(super) fn validate_identity_inputs(
    package_id: &str,
    stable_schema_key: &str,
    _descriptor: &PackageSchemaCanonicalDescriptor,
) -> Result<(), PackageBuildAuthorityValidationError> {
    if package_id.trim().is_empty() || stable_schema_key.trim().is_empty() {
        return invalid("PackageSchema packageId and stableSchemaKey must be non-empty");
    }
    Ok(())
}

pub(super) fn validate_graph(
    expected_package_id: &str,
    records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> Result<(), PackageBuildAuthorityValidationError> {
    validate_string("PackageArtifact.packageId", expected_package_id)?;
    let mut owner_keys = BTreeSet::new();
    let mut budget = GraphBudget::default();
    for (type_id, record) in records {
        validate_string("packageSchemaTypeId", type_id.as_str())?;
        validate_identity_inputs(
            &record.package_id,
            &record.stable_schema_key,
            &record.canonical_descriptor,
        )?;
        validate_type_parameters(&record.canonical_descriptor.type_params)?;
        if record.package_id != expected_package_id {
            return invalid(format!(
                "bytecode schema record {} is owned by {}, expected PackageArtifact owner {expected_package_id}",
                record.stable_schema_key, record.package_id
            ));
        }
        if !owner_keys.insert((
            record.package_id.as_str(),
            record.stable_schema_key.as_str(),
        )) {
            return invalid(format!(
                "bytecodeSchemaRecords repeats owner/stable key {}:{}",
                record.package_id, record.stable_schema_key
            ));
        }
        visit_descriptor_shape(
            &record.canonical_descriptor.descriptor,
            &record.canonical_descriptor.type_params,
            &mut budget,
            1,
        )?;
    }

    let mut visiting = BTreeSet::new();
    let mut complete = BTreeSet::new();
    for type_id in records.keys() {
        visit_record(
            type_id,
            expected_package_id,
            records,
            &mut visiting,
            &mut complete,
            1,
        )?;
    }
    Ok(())
}

pub(super) fn validate_single_descriptor(
    descriptor: &PackageSchemaCanonicalDescriptor,
) -> Result<(), PackageBuildAuthorityValidationError> {
    validate_type_parameters(&descriptor.type_params)?;
    visit_descriptor_shape(
        &descriptor.descriptor,
        &descriptor.type_params,
        &mut GraphBudget::default(),
        1,
    )
}

#[derive(Default)]
struct GraphBudget {
    nodes: u64,
}

impl GraphBudget {
    fn charge(&mut self, depth: u32) -> Result<(), PackageBuildAuthorityValidationError> {
        if depth > MAX_BYTECODE_SCHEMA_DEPTH {
            return invalid(format!(
                "bytecode schema nesting exceeds {MAX_BYTECODE_SCHEMA_DEPTH}"
            ));
        }
        self.nodes = self.nodes.checked_add(1).ok_or_else(|| {
            PackageBuildAuthorityValidationError::new(
                "bytecode schema type node count overflows u64",
            )
        })?;
        if self.nodes > MAX_BYTECODE_SCHEMA_TYPE_NODES {
            return invalid(format!(
                "bytecode schema type nodes exceed {MAX_BYTECODE_SCHEMA_TYPE_NODES}"
            ));
        }
        Ok(())
    }
}

fn visit_descriptor_shape(
    descriptor: &ContractTypeDescriptor,
    scope: &[String],
    budget: &mut GraphBudget,
    depth: u32,
) -> Result<(), PackageBuildAuthorityValidationError> {
    budget.charge(depth)?;
    match descriptor {
        ContractTypeDescriptor::Record { fields } => {
            for (name, ty) in fields {
                validate_string("record field", name)?;
                visit_type_shape(ty, scope, budget, depth + 1)?;
            }
        }
        ContractTypeDescriptor::StructuralUnion { variants } => {
            for variant in variants {
                visit_type_shape(variant, scope, budget, depth + 1)?;
            }
        }
        ContractTypeDescriptor::DiscriminatedUnion {
            discriminator_field,
            branches,
        } => {
            validate_string("discriminatorField", discriminator_field)?;
            for branch in branches {
                validate_string("discriminator tag", &branch.tag)?;
                visit_type_shape(&branch.branch_type, scope, budget, depth + 1)?;
            }
        }
        ContractTypeDescriptor::Representation { target }
        | ContractTypeDescriptor::Alias { target } => {
            visit_type_shape(target, scope, budget, depth + 1)?;
        }
        ContractTypeDescriptor::Enumeration { variants } => {
            for variant in variants {
                validate_string("enumeration variant", variant)?;
            }
        }
        ContractTypeDescriptor::CallbackInterface { operations } => {
            for (name, operation) in operations {
                validate_string("callback operation", name)?;
                for parameter in &operation.parameters {
                    visit_type_shape(parameter, scope, budget, depth + 1)?;
                }
                visit_type_shape(&operation.return_type, scope, budget, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn visit_type_shape(
    ty: &ContractTypeRef,
    scope: &[String],
    budget: &mut GraphBudget,
    depth: u32,
) -> Result<(), PackageBuildAuthorityValidationError> {
    budget.charge(depth)?;
    match ty {
        ContractTypeRef::Builtin { name, arguments } => {
            validate_string("builtin name", name)?;
            for argument in arguments {
                visit_type_shape(argument, scope, budget, depth + 1)?;
            }
        }
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            validate_string("child packageId", package_id)?;
            validate_string("child stableSchemaKey", stable_schema_key)?;
            validate_string("child packageSchemaTypeId", package_schema_type_id.as_str())?;
        }
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            visit_type_shape(interface, scope, budget, depth + 1)?;
            for argument in arguments {
                visit_type_shape(argument, scope, budget, depth + 1)?;
            }
        }
        ContractTypeRef::TypeParam { name } => {
            validate_string("type parameter reference", name)?;
            if !scope.iter().any(|parameter| parameter == name) {
                return invalid(format!(
                    "PackageSchema descriptor references undeclared type parameter {name}"
                ));
            }
        }
        ContractTypeRef::Record { fields } => {
            for (name, field) in fields {
                validate_string("inline record field", name)?;
                visit_type_shape(field, scope, budget, depth + 1)?;
            }
        }
        ContractTypeRef::StructuralUnion { variants } => {
            for variant in variants {
                visit_type_shape(variant, scope, budget, depth + 1)?;
            }
        }
        ContractTypeRef::Nullable { inner } => {
            visit_type_shape(inner, scope, budget, depth + 1)?;
        }
        ContractTypeRef::Literal { value } => match value {
            ContractLiteral::String { value } => validate_string("literal string", value)?,
        },
    }
    Ok(())
}

fn visit_record(
    type_id: &PackageSchemaTypeId,
    owner_package_id: &str,
    records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    visiting: &mut BTreeSet<PackageSchemaTypeId>,
    complete: &mut BTreeSet<PackageSchemaTypeId>,
    depth: u32,
) -> Result<(), PackageBuildAuthorityValidationError> {
    if depth > MAX_BYTECODE_SCHEMA_DEPTH {
        return invalid(format!(
            "bytecode schema dependency depth exceeds {MAX_BYTECODE_SCHEMA_DEPTH}"
        ));
    }
    if complete.contains(type_id) {
        return Ok(());
    }
    if !visiting.insert(type_id.clone()) {
        return invalid(format!(
            "bytecode schema descriptor cycle reaches {type_id}"
        ));
    }
    let record = records.get(type_id).ok_or_else(|| {
        PackageBuildAuthorityValidationError::new(format!(
            "bytecode schema closure is missing record {type_id}"
        ))
    })?;
    let mut children = Vec::new();
    collect_descriptor_refs(&record.canonical_descriptor.descriptor, &mut children);
    for (package_id, stable_schema_key, child_id) in children {
        if package_id != owner_package_id {
            continue;
        }
        let child = records.get(child_id).ok_or_else(|| {
            PackageBuildAuthorityValidationError::new(format!(
                "bytecode schema closure is missing {package_id}:{stable_schema_key}:{child_id}"
            ))
        })?;
        if child.package_id != package_id || child.stable_schema_key != stable_schema_key {
            return invalid(format!(
                "bytecode schema child {child_id} owner/stable key disagrees with its reference"
            ));
        }
        visit_record(
            child_id,
            owner_package_id,
            records,
            visiting,
            complete,
            depth + 1,
        )?;
    }
    visiting.remove(type_id);
    complete.insert(type_id.clone());
    Ok(())
}

fn collect_descriptor_refs<'a>(
    descriptor: &'a ContractTypeDescriptor,
    out: &mut Vec<(&'a str, &'a str, &'a PackageSchemaTypeId)>,
) {
    match descriptor {
        ContractTypeDescriptor::Record { fields } => {
            fields.values().for_each(|ty| collect_type_refs(ty, out));
        }
        ContractTypeDescriptor::StructuralUnion { variants } => {
            variants.iter().for_each(|ty| collect_type_refs(ty, out));
        }
        ContractTypeDescriptor::DiscriminatedUnion { branches, .. } => branches
            .iter()
            .for_each(|branch| collect_type_refs(&branch.branch_type, out)),
        ContractTypeDescriptor::Representation { target }
        | ContractTypeDescriptor::Alias { target } => collect_type_refs(target, out),
        ContractTypeDescriptor::CallbackInterface { operations } => {
            for operation in operations.values() {
                operation
                    .parameters
                    .iter()
                    .for_each(|ty| collect_type_refs(ty, out));
                collect_type_refs(&operation.return_type, out);
            }
        }
        ContractTypeDescriptor::Enumeration { .. } => {}
    }
}

fn collect_type_refs<'a>(
    ty: &'a ContractTypeRef,
    out: &mut Vec<(&'a str, &'a str, &'a PackageSchemaTypeId)>,
) {
    match ty {
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => out.push((package_id, stable_schema_key, package_schema_type_id)),
        ContractTypeRef::Builtin { arguments, .. }
        | ContractTypeRef::StructuralUnion {
            variants: arguments,
        } => arguments
            .iter()
            .for_each(|child| collect_type_refs(child, out)),
        ContractTypeRef::Record { fields } => fields
            .values()
            .for_each(|child| collect_type_refs(child, out)),
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            collect_type_refs(interface, out);
            arguments
                .iter()
                .for_each(|child| collect_type_refs(child, out));
        }
        ContractTypeRef::Nullable { inner } => collect_type_refs(inner, out),
        ContractTypeRef::TypeParam { .. } | ContractTypeRef::Literal { .. } => {}
    }
}

fn validate_type_parameters(
    parameters: &[String],
) -> Result<(), PackageBuildAuthorityValidationError> {
    let mut seen = BTreeSet::new();
    for parameter in parameters {
        validate_string("type parameter", parameter)?;
        if !seen.insert(parameter.as_str()) {
            return invalid(format!(
                "PackageSchema descriptor repeats type parameter {parameter}"
            ));
        }
    }
    Ok(())
}

fn validate_string(label: &str, value: &str) -> Result<(), PackageBuildAuthorityValidationError> {
    if value.is_empty() {
        return invalid(format!("{label} must be non-empty"));
    }
    if value.len() as u64 > MAX_BYTECODE_SCHEMA_STRING_BYTES {
        return invalid(format!(
            "{label} exceeds {MAX_BYTECODE_SCHEMA_STRING_BYTES} UTF-8 bytes"
        ));
    }
    if value.chars().any(char::is_control) {
        return invalid(format!("{label} contains a control character"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, PackageBuildAuthorityValidationError> {
    Err(PackageBuildAuthorityValidationError::new(message))
}
