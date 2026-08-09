use crate::{
    ContractDiscriminatedUnionBranch, ContractTypeDescriptor, ContractTypeRef,
    PackageSchemaCanonicalDescriptor,
};

use super::super::authority::PackageBuildAuthorityValidationError;

pub(super) fn validate_descriptor(
    descriptor: &PackageSchemaCanonicalDescriptor,
) -> Result<(), PackageBuildAuthorityValidationError> {
    let normalized = PackageSchemaCanonicalDescriptor {
        type_params: descriptor.type_params.clone(),
        descriptor: normalize_descriptor(descriptor.descriptor.clone(), "canonicalDescriptor")?,
    };
    if normalized != *descriptor {
        return invalid("PackageSchema canonicalDescriptor is not in canonical form");
    }
    Ok(())
}

fn normalize_descriptor(
    descriptor: ContractTypeDescriptor,
    path: &str,
) -> Result<ContractTypeDescriptor, PackageBuildAuthorityValidationError> {
    match descriptor {
        ContractTypeDescriptor::Record { fields } => Ok(ContractTypeDescriptor::Record {
            fields: fields
                .into_iter()
                .map(|(name, field)| {
                    Ok((
                        name.clone(),
                        normalize_type_ref(field, &format!("{path}.fields[{name}]"))?,
                    ))
                })
                .collect::<Result<_, PackageBuildAuthorityValidationError>>()?,
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
        ContractTypeDescriptor::Representation { target } => {
            Ok(ContractTypeDescriptor::Representation {
                target: normalize_type_ref(target, &format!("{path}.target"))?,
            })
        }
        ContractTypeDescriptor::Alias { target } => Ok(ContractTypeDescriptor::Alias {
            target: normalize_type_ref(target, &format!("{path}.target"))?,
        }),
        ContractTypeDescriptor::Enumeration { .. } => Ok(descriptor),
        ContractTypeDescriptor::CallbackInterface { operations } => {
            Ok(ContractTypeDescriptor::CallbackInterface {
                operations: operations
                    .into_iter()
                    .map(|(name, mut operation)| {
                        for (index, parameter) in operation.parameters.iter_mut().enumerate() {
                            *parameter = normalize_type_ref(
                                parameter.clone(),
                                &format!("{path}.operations[{name}].parameters[{index}]"),
                            )?;
                        }
                        operation.return_type = normalize_type_ref(
                            operation.return_type,
                            &format!("{path}.operations[{name}].returnType"),
                        )?;
                        Ok((name, operation))
                    })
                    .collect::<Result<_, PackageBuildAuthorityValidationError>>()?,
            })
        }
    }
}

fn normalize_type_ref(
    ty: ContractTypeRef,
    path: &str,
) -> Result<ContractTypeRef, PackageBuildAuthorityValidationError> {
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
                return invalid(format!(
                    "{path}.interface must be an exact PackageSchema nominal"
                ));
            }
            Ok(ContractTypeRef::AnyInterface {
                interface: Box::new(normalize_type_ref(
                    *interface,
                    &format!("{path}.interface"),
                )?),
                arguments: arguments
                    .into_iter()
                    .enumerate()
                    .map(|(index, argument)| {
                        normalize_type_ref(argument, &format!("{path}.arguments[{index}]"))
                    })
                    .collect::<Result<_, _>>()?,
            })
        }
        ContractTypeRef::Record { fields } => Ok(ContractTypeRef::Record {
            fields: fields
                .into_iter()
                .map(|(name, field)| {
                    Ok((
                        name.clone(),
                        normalize_type_ref(field, &format!("{path}.fields[{name}]"))?,
                    ))
                })
                .collect::<Result<_, PackageBuildAuthorityValidationError>>()?,
        }),
        ContractTypeRef::StructuralUnion { variants } => {
            normalize_inline_union(variants, false, path)
        }
        ContractTypeRef::Nullable { inner } => normalize_inline_union(vec![*inner], true, path),
    }
}

fn normalize_builtin(
    name: String,
    arguments: Vec<ContractTypeRef>,
    path: &str,
) -> Result<ContractTypeRef, PackageBuildAuthorityValidationError> {
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
        _ => return invalid(format!("{path}: unknown contract builtin `{name}`")),
    };
    let expected_arity = match canonical_name {
        "Array" => 1,
        "Map" => 2,
        _ => 0,
    };
    if arguments.len() != expected_arity {
        return invalid(format!(
            "{path}: builtin {canonical_name} expects {expected_arity} arguments, got {}",
            arguments.len()
        ));
    }
    Ok(ContractTypeRef::Builtin {
        name: canonical_name.to_string(),
        arguments: arguments
            .into_iter()
            .enumerate()
            .map(|(index, argument)| {
                normalize_type_ref(argument, &format!("{path}.arguments[{index}]"))
            })
            .collect::<Result<_, _>>()?,
    })
}

fn normalize_inline_union(
    variants: Vec<ContractTypeRef>,
    force_nullable: bool,
    path: &str,
) -> Result<ContractTypeRef, PackageBuildAuthorityValidationError> {
    let (variants, has_null) = normalize_union_members(variants, force_nullable, path)?;
    let base = match variants.as_slice() {
        [] if has_null => return Ok(ContractTypeRef::builtin("null")),
        [] => return invalid(format!("{path}: structural union must not be empty")),
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
) -> Result<Vec<ContractTypeRef>, PackageBuildAuthorityValidationError> {
    let (mut variants, has_null) = normalize_union_members(variants, false, path)?;
    if has_null {
        variants.push(ContractTypeRef::builtin("null"));
        sort_and_deduplicate(&mut variants, path)?;
    }
    if variants.len() < 2 {
        return invalid(format!(
            "{path}: named structural union must contain at least two distinct variants"
        ));
    }
    Ok(variants)
}

fn normalize_union_members(
    variants: Vec<ContractTypeRef>,
    force_nullable: bool,
    path: &str,
) -> Result<(Vec<ContractTypeRef>, bool), PackageBuildAuthorityValidationError> {
    let mut flattened = Vec::new();
    let mut has_null = force_nullable;
    for (index, variant) in variants.into_iter().enumerate() {
        let normalized = normalize_type_ref(variant, &format!("{path}.variants[{index}]"))?;
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
) -> Result<Vec<ContractDiscriminatedUnionBranch>, PackageBuildAuthorityValidationError> {
    for (index, branch) in branches.iter_mut().enumerate() {
        if branch.tag.is_empty() {
            return invalid(format!("{path}.branches[{index}].tag must be non-empty"));
        }
        branch.branch_type = normalize_type_ref(
            branch.branch_type.clone(),
            &format!("{path}.branches[{}].branchType", branch.tag),
        )?;
    }
    branches.sort_by(|left, right| left.tag.cmp(&right.tag));
    if branches.windows(2).any(|pair| pair[0].tag == pair[1].tag) {
        return invalid(format!("{path}: duplicate discriminated union tag"));
    }
    if branches.is_empty() {
        return invalid(format!(
            "{path}: discriminated union must contain at least one branch"
        ));
    }
    Ok(branches)
}

fn sort_and_deduplicate(
    values: &mut Vec<ContractTypeRef>,
    path: &str,
) -> Result<(), PackageBuildAuthorityValidationError> {
    let mut keyed = values
        .drain(..)
        .map(|value| {
            let key = skiff_canonical_json::canonical_json_bytes(&value).map_err(|error| {
                PackageBuildAuthorityValidationError::new(format!(
                    "{path}: failed to canonicalize union variant: {error}"
                ))
            })?;
            Ok((key, value))
        })
        .collect::<Result<Vec<_>, PackageBuildAuthorityValidationError>>()?;
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    keyed.dedup_by(|left, right| left.0 == right.0);
    values.extend(keyed.into_iter().map(|(_, value)| value));
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, PackageBuildAuthorityValidationError> {
    Err(PackageBuildAuthorityValidationError::new(message))
}
