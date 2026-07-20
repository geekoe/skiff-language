use std::collections::{BTreeMap, HashSet};

use skiff_artifact_model::{
    ContractLiteral, ContractSchemaType, ContractTypeDescriptor, ContractTypeId, ContractTypeRef,
};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    value::{HeapHandle, HeapNode, RuntimeValue},
};

use crate::{
    service_linkable::ServiceLinkableMaterializationError, service_linkable_detached::model_error,
};

pub(crate) fn validate_schema_closure(
    ty: &ContractTypeRef,
    schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
) -> Result<(), ServiceLinkableMaterializationError> {
    validate_schema_closure_inner(ty, schema, &mut HashSet::new())
}

fn validate_schema_closure_inner(
    ty: &ContractTypeRef,
    schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
    active: &mut HashSet<ContractTypeId>,
) -> Result<(), ServiceLinkableMaterializationError> {
    match ty {
        ContractTypeRef::Builtin { arguments, .. } => {
            for argument in arguments {
                validate_schema_closure_inner(argument, schema, active)?;
            }
        }
        ContractTypeRef::Contract { contract_type_id } => {
            let descriptor = schema.get(contract_type_id).ok_or_else(|| {
                ServiceLinkableMaterializationError::MissingSchema {
                    contract_type_id: contract_type_id.clone(),
                }
            })?;
            if descriptor.contract_type_id != *contract_type_id {
                return Err(
                    ServiceLinkableMaterializationError::SchemaIdentityMismatch {
                        requested: contract_type_id.clone(),
                        actual: descriptor.contract_type_id.clone(),
                    },
                );
            }
            if !active.insert(contract_type_id.clone()) {
                return Err(ServiceLinkableMaterializationError::CyclicSchema {
                    contract_type_id: contract_type_id.clone(),
                });
            }
            validate_descriptor_closure(&descriptor.shape.descriptor, schema, active)?;
            active.remove(contract_type_id);
        }
        ContractTypeRef::Record { fields } => {
            for field in fields.values() {
                validate_schema_closure_inner(field, schema, active)?;
            }
        }
        ContractTypeRef::StructuralUnion { variants } => {
            for variant in variants {
                validate_schema_closure_inner(variant, schema, active)?;
            }
        }
        ContractTypeRef::Nullable { inner } => {
            validate_schema_closure_inner(inner, schema, active)?;
        }
        ContractTypeRef::Literal { .. } => {}
    }
    Ok(())
}

fn validate_descriptor_closure(
    descriptor: &ContractTypeDescriptor,
    schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
    active: &mut HashSet<ContractTypeId>,
) -> Result<(), ServiceLinkableMaterializationError> {
    match descriptor {
        ContractTypeDescriptor::Record { fields } => {
            for field in fields.values() {
                validate_schema_closure_inner(field, schema, active)?;
            }
        }
        ContractTypeDescriptor::StructuralUnion { variants } => {
            for variant in variants {
                validate_schema_closure_inner(variant, schema, active)?;
            }
        }
        ContractTypeDescriptor::DiscriminatedUnion { branches, .. } => {
            for branch in branches {
                validate_schema_closure_inner(&branch.branch_type, schema, active)?;
            }
        }
        ContractTypeDescriptor::Representation { target }
        | ContractTypeDescriptor::Alias { target } => {
            validate_schema_closure_inner(target, schema, active)?;
        }
        ContractTypeDescriptor::CallbackInterface { operations } => {
            for operation in operations.values() {
                for parameter in &operation.parameters {
                    validate_schema_closure_inner(parameter, schema, active)?;
                }
                validate_schema_closure_inner(&operation.return_type, schema, active)?;
            }
        }
        ContractTypeDescriptor::Enumeration { .. } => {}
    }
    Ok(())
}

pub(crate) fn contract_type_is_callback_interface(
    ty: &ContractTypeRef,
    schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
) -> Result<bool, ServiceLinkableMaterializationError> {
    contract_type_is_callback_interface_inner(ty, schema, &mut HashSet::new())
}

fn contract_type_is_callback_interface_inner(
    ty: &ContractTypeRef,
    schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
    active: &mut HashSet<ContractTypeId>,
) -> Result<bool, ServiceLinkableMaterializationError> {
    let ContractTypeRef::Contract { contract_type_id } = ty else {
        return Ok(false);
    };
    if !active.insert(contract_type_id.clone()) {
        return Err(ServiceLinkableMaterializationError::CyclicSchema {
            contract_type_id: contract_type_id.clone(),
        });
    }
    let schema_type = schema.get(contract_type_id).ok_or_else(|| {
        ServiceLinkableMaterializationError::MissingSchema {
            contract_type_id: contract_type_id.clone(),
        }
    })?;
    let result = match &schema_type.shape.descriptor {
        ContractTypeDescriptor::CallbackInterface { .. } => true,
        ContractTypeDescriptor::Alias { target }
        | ContractTypeDescriptor::Representation { target } => {
            contract_type_is_callback_interface_inner(target, schema, active)?
        }
        _ => false,
    };
    active.remove(contract_type_id);
    Ok(result)
}

pub(crate) fn value_matches_contract_type(
    value: &RuntimeValue,
    heap: &RequestHeap,
    ty: &ContractTypeRef,
    schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
) -> Result<bool, ServiceLinkableMaterializationError> {
    value_matches_contract_type_inner(value, heap, ty, schema, &mut HashSet::new())
}

fn value_matches_contract_type_inner(
    value: &RuntimeValue,
    heap: &RequestHeap,
    ty: &ContractTypeRef,
    schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
    active: &mut HashSet<ContractTypeId>,
) -> Result<bool, ServiceLinkableMaterializationError> {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => {
            builtin_value_matches(value, heap, name, arguments, schema, active)
        }
        ContractTypeRef::Contract { contract_type_id } => {
            if !active.insert(contract_type_id.clone()) {
                return Err(ServiceLinkableMaterializationError::CyclicSchema {
                    contract_type_id: contract_type_id.clone(),
                });
            }
            let schema_type = schema.get(contract_type_id).ok_or_else(|| {
                ServiceLinkableMaterializationError::MissingSchema {
                    contract_type_id: contract_type_id.clone(),
                }
            })?;
            let result = descriptor_value_matches(
                value,
                heap,
                &schema_type.shape.descriptor,
                schema,
                active,
            )?;
            active.remove(contract_type_id);
            Ok(result)
        }
        ContractTypeRef::Record { fields } => {
            record_value_matches(value, heap, fields, schema, active)
        }
        ContractTypeRef::StructuralUnion { variants } => {
            union_value_matches(value, heap, variants, schema, active)
        }
        ContractTypeRef::Nullable { inner } => Ok(matches!(value, RuntimeValue::Null)
            || value_matches_contract_type_inner(value, heap, inner, schema, active)?),
        ContractTypeRef::Literal {
            value: ContractLiteral::String { value: literal },
        } => Ok(matches!(value, RuntimeValue::String(actual) if actual == literal)),
    }
}

fn builtin_value_matches(
    value: &RuntimeValue,
    heap: &RequestHeap,
    name: &str,
    arguments: &[ContractTypeRef],
    schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
    active: &mut HashSet<ContractTypeId>,
) -> Result<bool, ServiceLinkableMaterializationError> {
    let no_arguments = arguments.is_empty();
    Ok(match name {
        "void" | "null" if no_arguments => matches!(value, RuntimeValue::Null),
        "bool" if no_arguments => matches!(value, RuntimeValue::Bool(_)),
        "number" if no_arguments => {
            matches!(value, RuntimeValue::Number(number) if number.is_finite())
        }
        "integer" if no_arguments => {
            matches!(value, RuntimeValue::Number(number) if number.is_finite() && number.fract() == 0.0)
        }
        "string" if no_arguments => matches!(value, RuntimeValue::String(_)),
        "Date" if no_arguments => matches!(value, RuntimeValue::Date(_)),
        "bytes" if no_arguments => {
            matches_heap_node(value, heap, |node| matches!(node, HeapNode::Bytes(_)))?
        }
        "Array" if arguments.len() == 1 => {
            let RuntimeValue::Heap(handle) = value else {
                return Ok(false);
            };
            let HeapNode::Array(items) = heap.get(*handle).map_err(model_error)? else {
                return Ok(false);
            };
            let mut matches = true;
            for item in items {
                matches &=
                    value_matches_contract_type_inner(item, heap, &arguments[0], schema, active)?;
            }
            matches
        }
        "Map" if arguments.len() == 2 => {
            let RuntimeValue::Heap(handle) = value else {
                return Ok(false);
            };
            let HeapNode::Map(map) = heap.get(*handle).map_err(model_error)? else {
                return Ok(false);
            };
            if !matches!(&arguments[0], ContractTypeRef::Builtin { name, arguments } if name == "string" && arguments.is_empty())
            {
                return Ok(false);
            }
            let mut matches = true;
            for item in map.values() {
                matches &=
                    value_matches_contract_type_inner(item, heap, &arguments[1], schema, active)?;
            }
            matches
        }
        "Json" if no_arguments => json_value_matches(value, heap, &mut HashSet::new())?,
        _ => false,
    })
}

fn descriptor_value_matches(
    value: &RuntimeValue,
    heap: &RequestHeap,
    descriptor: &ContractTypeDescriptor,
    schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
    active: &mut HashSet<ContractTypeId>,
) -> Result<bool, ServiceLinkableMaterializationError> {
    match descriptor {
        ContractTypeDescriptor::Record { fields } => {
            record_value_matches(value, heap, fields, schema, active)
        }
        ContractTypeDescriptor::StructuralUnion { variants } => {
            union_value_matches(value, heap, variants, schema, active)
        }
        ContractTypeDescriptor::DiscriminatedUnion {
            discriminator_field,
            branches,
        } => {
            let RuntimeValue::Heap(handle) = value else {
                return Ok(false);
            };
            let HeapNode::Object(object) = heap.get(*handle).map_err(model_error)? else {
                return Ok(false);
            };
            let Some(RuntimeValue::String(tag)) = object.fields().get(discriminator_field) else {
                return Ok(false);
            };
            let Some(branch) = branches.iter().find(|branch| branch.tag == *tag) else {
                return Ok(false);
            };
            value_matches_contract_type_inner(value, heap, &branch.branch_type, schema, active)
        }
        ContractTypeDescriptor::Representation { target }
        | ContractTypeDescriptor::Alias { target } => {
            value_matches_contract_type_inner(value, heap, target, schema, active)
        }
        ContractTypeDescriptor::Enumeration { variants } => {
            Ok(matches!(value, RuntimeValue::String(actual) if variants.contains(actual)))
        }
        ContractTypeDescriptor::CallbackInterface { .. } => Ok(false),
    }
}

fn record_value_matches(
    value: &RuntimeValue,
    heap: &RequestHeap,
    fields: &BTreeMap<String, ContractTypeRef>,
    schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
    active: &mut HashSet<ContractTypeId>,
) -> Result<bool, ServiceLinkableMaterializationError> {
    let RuntimeValue::Heap(handle) = value else {
        return Ok(false);
    };
    let HeapNode::Object(object) = heap.get(*handle).map_err(model_error)? else {
        return Ok(false);
    };
    if object.fields().len() != fields.len() {
        return Ok(false);
    }
    for (name, ty) in fields {
        let Some(field) = object.fields().get(name) else {
            return Ok(false);
        };
        if !value_matches_contract_type_inner(field, heap, ty, schema, active)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn union_value_matches(
    value: &RuntimeValue,
    heap: &RequestHeap,
    variants: &[ContractTypeRef],
    schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
    active: &mut HashSet<ContractTypeId>,
) -> Result<bool, ServiceLinkableMaterializationError> {
    let mut matched = 0usize;
    for variant in variants {
        if value_matches_contract_type_inner(value, heap, variant, schema, active)? {
            matched += 1;
        }
    }
    if matched > 1 {
        return Err(ServiceLinkableMaterializationError::AmbiguousStructuralUnion);
    }
    Ok(matched == 1)
}

fn json_value_matches(
    value: &RuntimeValue,
    heap: &RequestHeap,
    active: &mut HashSet<HeapHandle>,
) -> Result<bool, ServiceLinkableMaterializationError> {
    let RuntimeValue::Heap(handle) = value else {
        return Ok(match value {
            RuntimeValue::Null | RuntimeValue::Bool(_) | RuntimeValue::String(_) => true,
            RuntimeValue::Number(number) => number.is_finite(),
            RuntimeValue::Date(_) | RuntimeValue::ActorRef(_) | RuntimeValue::Heap(_) => false,
        });
    };
    if !active.insert(*handle) {
        return Ok(false);
    }
    let result = match heap.get(*handle).map_err(model_error)? {
        HeapNode::Array(items) => {
            let mut matches = true;
            for item in items {
                matches &= json_value_matches(item, heap, active)?;
            }
            matches
        }
        HeapNode::Object(object) => {
            let mut matches = true;
            for item in object.fields().values() {
                matches &= json_value_matches(item, heap, active)?;
            }
            matches
        }
        HeapNode::Map(map) => {
            let mut matches = true;
            for item in map.values() {
                matches &= json_value_matches(item, heap, active)?;
            }
            matches
        }
        HeapNode::Bytes(_) | HeapNode::Interface(_) => false,
    };
    active.remove(handle);
    Ok(result)
}

fn matches_heap_node(
    value: &RuntimeValue,
    heap: &RequestHeap,
    predicate: impl FnOnce(&HeapNode) -> bool,
) -> Result<bool, ServiceLinkableMaterializationError> {
    let RuntimeValue::Heap(handle) = value else {
        return Ok(false);
    };
    Ok(predicate(heap.get(*handle).map_err(model_error)?))
}
