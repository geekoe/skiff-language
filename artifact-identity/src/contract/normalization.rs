use skiff_artifact_model::{
    BoundaryOperationContract, BoundaryStreamContract, ContractDiscriminatedUnionBranch,
    ContractTypeDescriptor, ContractTypeRef, ContractTypeShape,
};

use crate::{ArtifactIdentityError, Result};

/// Producer-side canonicalization for code-free contract definitions.
///
/// Artifact readers must validate an already materialized operation instead of
/// replacing it with this function's output.
pub fn normalize_contract_operation_contract(
    mut operation: BoundaryOperationContract,
    path: &str,
) -> Result<BoundaryOperationContract> {
    for (index, parameter) in operation.parameters.iter_mut().enumerate() {
        parameter.ty = normalize_contract_type_ref(
            parameter.ty.clone(),
            &format!("{path}.parameters[{index}].ty"),
        )?;
    }
    operation.return_value.ty =
        normalize_contract_type_ref(operation.return_value.ty, &format!("{path}.returnValue.ty"))?;
    if let BoundaryStreamContract::ServerStream { item_type, .. } = &mut operation.stream {
        *item_type =
            normalize_contract_type_ref(item_type.clone(), &format!("{path}.stream.itemType"))?;
    }
    Ok(operation)
}

/// Producer-side canonicalization for a code-free contract schema entry.
///
/// The ServiceContract validator uses this only to compare canonical output
/// with the loaded shape; it never writes the result back into an artifact.
pub fn normalize_contract_type_shape(
    mut shape: ContractTypeShape,
    path: &str,
) -> Result<ContractTypeShape> {
    shape.descriptor = normalize_descriptor(shape.descriptor, &format!("{path}.descriptor"))?;
    Ok(shape)
}

pub(super) fn normalize_contract_type_ref(
    ty: ContractTypeRef,
    path: &str,
) -> Result<ContractTypeRef> {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => normalize_builtin(name, arguments, path),
        ContractTypeRef::PackageSchema { .. }
        | ContractTypeRef::TypeParam { .. }
        | ContractTypeRef::Literal { .. } => Ok(ty),
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            if !matches!(interface.as_ref(), ContractTypeRef::PackageSchema { .. }) {
                return Err(ArtifactIdentityError::InvalidServiceContract {
                    message: format!(
                        "{path}.interface must be an exact PackageSchema interface nominal"
                    ),
                });
            }
            Ok(ContractTypeRef::AnyInterface {
                interface: Box::new(normalize_contract_type_ref(
                    *interface,
                    &format!("{path}.interface"),
                )?),
                arguments: arguments
                    .into_iter()
                    .enumerate()
                    .map(|(index, argument)| {
                        normalize_contract_type_ref(argument, &format!("{path}.arguments[{index}]"))
                    })
                    .collect::<Result<_>>()?,
            })
        }
        ContractTypeRef::Record { fields } => Ok(ContractTypeRef::Record {
            fields: fields
                .into_iter()
                .map(|(name, field)| {
                    let field =
                        normalize_contract_type_ref(field, &format!("{path}.fields[{name}]"))?;
                    Ok((name, field))
                })
                .collect::<Result<_>>()?,
        }),
        ContractTypeRef::StructuralUnion { variants } => {
            normalize_inline_union(variants, false, path)
        }
        ContractTypeRef::Nullable { inner } => normalize_inline_union(vec![*inner], true, path),
    }
}

fn normalize_descriptor(
    descriptor: ContractTypeDescriptor,
    path: &str,
) -> Result<ContractTypeDescriptor> {
    match descriptor {
        ContractTypeDescriptor::Record { fields } => Ok(ContractTypeDescriptor::Record {
            fields: fields
                .into_iter()
                .map(|(name, field)| {
                    let field =
                        normalize_contract_type_ref(field, &format!("{path}.fields[{name}]"))?;
                    Ok((name, field))
                })
                .collect::<Result<_>>()?,
        }),
        ContractTypeDescriptor::StructuralUnion { variants } => {
            Ok(ContractTypeDescriptor::StructuralUnion {
                variants: normalize_named_union_variants(variants, path)?,
            })
        }
        ContractTypeDescriptor::DiscriminatedUnion {
            discriminator_field,
            branches,
        } => Ok(ContractTypeDescriptor::DiscriminatedUnion {
            discriminator_field,
            branches: normalize_discriminated_branches(branches, path)?,
        }),
        ContractTypeDescriptor::Alias { target } => Ok(ContractTypeDescriptor::Alias {
            target: normalize_contract_type_ref(target, &format!("{path}.target"))?,
        }),
        ContractTypeDescriptor::Representation { target } => {
            Ok(ContractTypeDescriptor::Representation {
                target: normalize_contract_type_ref(target, &format!("{path}.target"))?,
            })
        }
        ContractTypeDescriptor::Enumeration { .. } => Ok(descriptor),
        ContractTypeDescriptor::CallbackInterface { operations } => {
            Ok(ContractTypeDescriptor::CallbackInterface {
                operations: operations
                    .into_iter()
                    .map(|(name, mut operation)| {
                        for (index, parameter) in operation.parameters.iter_mut().enumerate() {
                            *parameter = normalize_contract_type_ref(
                                parameter.clone(),
                                &format!("{path}.operations[{name}].parameters[{index}]"),
                            )?;
                        }
                        operation.return_type = normalize_contract_type_ref(
                            operation.return_type,
                            &format!("{path}.operations[{name}].returnType"),
                        )?;
                        Ok((name, operation))
                    })
                    .collect::<Result<_>>()?,
            })
        }
    }
}

fn normalize_builtin(
    name: String,
    arguments: Vec<ContractTypeRef>,
    path: &str,
) -> Result<ContractTypeRef> {
    let canonical_name = match name.as_str() {
        "boolean" => "bool",
        "String" => "string",
        "Bytes" | "std.bytes.bytes" => "bytes",
        "std.collection.Array" => "Array",
        "std.collection.Map" => "Map",
        "std.date.Date" => "Date",
        "std.time.Duration" => "Duration",
        "string" | "number" | "integer" | "bool" | "null" | "void" | "bytes" | "Date"
        | "Duration" | "Json" | "JsonObject" | "Array" | "Map" => name.as_str(),
        _ => return invalid_contract(format!("{path}: unknown contract builtin `{name}`")),
    };
    let expected_arity = match canonical_name {
        "Array" => 1,
        "Map" => 2,
        _ => 0,
    };
    if arguments.len() != expected_arity {
        return invalid_contract(format!(
            "{path}: builtin {canonical_name} expects {expected_arity} arguments, got {}",
            arguments.len()
        ));
    }
    let arguments = arguments
        .into_iter()
        .enumerate()
        .map(|(index, argument)| {
            normalize_contract_type_ref(argument, &format!("{path}.arguments[{index}]"))
        })
        .collect::<Result<_>>()?;
    Ok(ContractTypeRef::Builtin {
        name: canonical_name.to_string(),
        arguments,
    })
}

fn normalize_inline_union(
    variants: Vec<ContractTypeRef>,
    force_nullable: bool,
    path: &str,
) -> Result<ContractTypeRef> {
    let (variants, has_null) = normalize_union_members(variants, force_nullable, path)?;
    let base = match variants.as_slice() {
        [] if has_null => return Ok(ContractTypeRef::builtin("null")),
        [] => return invalid_contract(format!("{path}: structural union must not be empty")),
        [only] => only.clone(),
        _ => ContractTypeRef::StructuralUnion { variants },
    };
    if has_null {
        Ok(ContractTypeRef::Nullable {
            inner: Box::new(base),
        })
    } else {
        Ok(base)
    }
}

fn normalize_named_union_variants(
    variants: Vec<ContractTypeRef>,
    path: &str,
) -> Result<Vec<ContractTypeRef>> {
    let (mut variants, has_null) = normalize_union_members(variants, false, path)?;
    if has_null {
        variants.push(ContractTypeRef::builtin("null"));
        sort_and_deduplicate(&mut variants, path)?;
    }
    if variants.len() < 2 {
        return invalid_contract(format!(
            "{path}: named structural union must contain at least two distinct variants"
        ));
    }
    Ok(variants)
}

fn normalize_union_members(
    variants: Vec<ContractTypeRef>,
    force_nullable: bool,
    path: &str,
) -> Result<(Vec<ContractTypeRef>, bool)> {
    let mut flattened = Vec::new();
    let mut has_null = force_nullable;
    for (index, variant) in variants.into_iter().enumerate() {
        let normalized =
            normalize_contract_type_ref(variant, &format!("{path}.variants[{index}]"))?;
        collect_union_member(normalized, &mut flattened, &mut has_null);
    }
    sort_and_deduplicate(&mut flattened, path)?;
    Ok((flattened, has_null))
}

fn collect_union_member(
    variant: ContractTypeRef,
    flattened: &mut Vec<ContractTypeRef>,
    has_null: &mut bool,
) {
    match variant {
        ContractTypeRef::StructuralUnion { variants } => {
            for variant in variants {
                collect_union_member(variant, flattened, has_null);
            }
        }
        ContractTypeRef::Nullable { inner } => {
            *has_null = true;
            collect_union_member(*inner, flattened, has_null);
        }
        ContractTypeRef::Builtin { name, arguments } if name == "null" && arguments.is_empty() => {
            *has_null = true;
        }
        variant => flattened.push(variant),
    }
}

fn normalize_discriminated_branches(
    mut branches: Vec<ContractDiscriminatedUnionBranch>,
    path: &str,
) -> Result<Vec<ContractDiscriminatedUnionBranch>> {
    for (index, branch) in branches.iter_mut().enumerate() {
        if branch.tag.is_empty() {
            return invalid_contract(format!(
                "{path}.branches[{index}].tag: discriminator branch tag must not be empty"
            ));
        }
        branch.branch_type = normalize_contract_type_ref(
            branch.branch_type.clone(),
            &format!("{path}.branches[{}].branchType", branch.tag),
        )?;
    }
    branches.sort_by(|left, right| left.tag.cmp(&right.tag));
    for pair in branches.windows(2) {
        if pair[0].tag == pair[1].tag {
            return invalid_contract(format!(
                "{path}.branches[{}]: duplicate discriminator branch tag",
                pair[0].tag
            ));
        }
    }
    if branches.is_empty() {
        return invalid_contract(format!(
            "{path}.branches: discriminated union must contain at least one branch"
        ));
    }
    Ok(branches)
}

fn sort_and_deduplicate(values: &mut Vec<ContractTypeRef>, path: &str) -> Result<()> {
    let mut keyed = values
        .drain(..)
        .map(|value| {
            let key = skiff_canonical_json::canonical_json_bytes(&value).map_err(|error| {
                ArtifactIdentityError::InvalidServiceContract {
                    message: format!("{path}: failed to canonicalize union variant: {error}"),
                }
            })?;
            Ok((key, value))
        })
        .collect::<Result<Vec<_>>>()?;
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    keyed.dedup_by(|left, right| left.0 == right.0);
    values.extend(keyed.into_iter().map(|(_, value)| value));
    Ok(())
}

fn invalid_contract<T>(message: impl Into<String>) -> Result<T> {
    Err(ArtifactIdentityError::InvalidServiceContract {
        message: message.into(),
    })
}
