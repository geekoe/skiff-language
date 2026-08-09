use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    FunctionTypeParamIr, InterfaceInstantiationRef, NominalTypeRefBaseIr, TypeRefIr,
};

use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation};

pub(in crate::bytecode) fn substitute_type(
    ty: &TypeRefIr,
    substitutions: &BTreeMap<String, TypeRefIr>,
    location: &BytecodeLinkLocation,
) -> Result<TypeRefIr, BytecodeLinkError> {
    substitute_type_inner(ty, substitutions, location, &mut BTreeSet::new())
}

fn substitute_type_inner(
    ty: &TypeRefIr,
    substitutions: &BTreeMap<String, TypeRefIr>,
    location: &BytecodeLinkLocation,
    resolving: &mut BTreeSet<String>,
) -> Result<TypeRefIr, BytecodeLinkError> {
    Ok(match ty {
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name: name.clone(),
            args: substitute_types(args, substitutions, location, resolving)?,
        },
        TypeRefIr::LocalType { type_index } => TypeRefIr::LocalType {
            type_index: *type_index,
        },
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => TypeRefIr::PublicationType {
            module_path: module_path.clone(),
            type_index: *type_index,
        },
        TypeRefIr::ServiceSymbol { symbol } => TypeRefIr::ServiceSymbol {
            symbol: symbol.clone(),
        },
        TypeRefIr::PackageSymbol { symbol } => TypeRefIr::PackageSymbol {
            symbol: symbol.clone(),
        },
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => TypeRefIr::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.clone(),
        },
        TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
            base: substitute_nominal_base(base),
            arguments: substitute_types(arguments, substitutions, location, resolving)?,
        },
        TypeRefIr::DbObjectSymbol { symbol } => TypeRefIr::DbObjectSymbol {
            symbol: symbol.clone(),
        },
        TypeRefIr::Record { fields } => {
            substitute_record(fields, substitutions, location, resolving)?
        }
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: substitute_types(items, substitutions, location, resolving)?,
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(substitute_type_inner(
                inner,
                substitutions,
                location,
                resolving,
            )?),
        },
        TypeRefIr::Literal { value } => TypeRefIr::Literal {
            value: value.clone(),
        },
        TypeRefIr::TypeParam { name } => {
            substitute_parameter(name, substitutions, location, resolving)?
        }
        TypeRefIr::AnyInterface { interface } => {
            substitute_interface(interface, substitutions, location, resolving)?
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => substitute_function(params, return_type, substitutions, location, resolving)?,
    })
}

fn substitute_record(
    fields: &BTreeMap<String, TypeRefIr>,
    substitutions: &BTreeMap<String, TypeRefIr>,
    location: &BytecodeLinkLocation,
    resolving: &mut BTreeSet<String>,
) -> Result<TypeRefIr, BytecodeLinkError> {
    let fields = fields
        .iter()
        .map(|(name, field)| {
            Ok((
                name.clone(),
                substitute_type_inner(field, substitutions, location, resolving)?,
            ))
        })
        .collect::<Result<_, BytecodeLinkError>>()?;
    Ok(TypeRefIr::Record { fields })
}

fn substitute_parameter(
    name: &str,
    substitutions: &BTreeMap<String, TypeRefIr>,
    location: &BytecodeLinkLocation,
    resolving: &mut BTreeSet<String>,
) -> Result<TypeRefIr, BytecodeLinkError> {
    if !resolving.insert(name.to_string()) {
        return Err(obligation_error(
            location.clone(),
            format!("type substitution cycle includes parameter {name:?}"),
        ));
    }
    let replacement = substitutions.get(name).ok_or_else(|| {
        obligation_error(
            location.clone(),
            format!("type parameter {name:?} remains unresolved"),
        )
    })?;
    let concrete = substitute_type_inner(replacement, substitutions, location, resolving)?;
    resolving.remove(name);
    Ok(concrete)
}

fn substitute_interface(
    interface: &InterfaceInstantiationRef,
    substitutions: &BTreeMap<String, TypeRefIr>,
    location: &BytecodeLinkLocation,
    resolving: &mut BTreeSet<String>,
) -> Result<TypeRefIr, BytecodeLinkError> {
    Ok(TypeRefIr::AnyInterface {
        interface: InterfaceInstantiationRef {
            interface_abi_id: interface.interface_abi_id.clone(),
            canonical_type_args: substitute_types(
                &interface.canonical_type_args,
                substitutions,
                location,
                resolving,
            )?,
        },
    })
}

fn substitute_function(
    params: &[FunctionTypeParamIr],
    return_type: &TypeRefIr,
    substitutions: &BTreeMap<String, TypeRefIr>,
    location: &BytecodeLinkLocation,
    resolving: &mut BTreeSet<String>,
) -> Result<TypeRefIr, BytecodeLinkError> {
    let params = params
        .iter()
        .map(|parameter| {
            Ok(FunctionTypeParamIr {
                name: parameter.name.clone(),
                ty: substitute_type_inner(&parameter.ty, substitutions, location, resolving)?,
            })
        })
        .collect::<Result<_, BytecodeLinkError>>()?;
    Ok(TypeRefIr::Function {
        params,
        return_type: Box::new(substitute_type_inner(
            return_type,
            substitutions,
            location,
            resolving,
        )?),
    })
}

fn substitute_nominal_base(base: &NominalTypeRefBaseIr) -> NominalTypeRefBaseIr {
    base.clone()
}

fn substitute_types(
    types: &[TypeRefIr],
    substitutions: &BTreeMap<String, TypeRefIr>,
    location: &BytecodeLinkLocation,
    resolving: &mut BTreeSet<String>,
) -> Result<Vec<TypeRefIr>, BytecodeLinkError> {
    types
        .iter()
        .map(|ty| substitute_type_inner(ty, substitutions, location, resolving))
        .collect()
}

fn obligation_error(location: BytecodeLinkLocation, detail: String) -> BytecodeLinkError {
    BytecodeLinkError::UnsatisfiedObligation {
        obligation: BytecodeLinkObligation::ConcreteSpecialization,
        location,
        detail,
    }
}
