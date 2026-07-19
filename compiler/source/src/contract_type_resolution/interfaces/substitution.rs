use std::collections::BTreeMap;

use skiff_artifact_model::{
    FunctionTypeParamIr, PackageCallableParameter, PackageTypeRef, TypeRefIr,
};

use crate::SourceExecutableReceiver;

use super::super::SourceInterfaceRequirementSignature;

pub(super) fn substitute_requirement(
    requirement: &SourceInterfaceRequirementSignature,
    substitutions: &BTreeMap<String, PackageTypeRef>,
) -> Result<SourceInterfaceRequirementSignature, String> {
    Ok(SourceInterfaceRequirementSignature {
        parameters: requirement
            .parameters
            .iter()
            .map(|parameter| {
                Ok(PackageCallableParameter {
                    name: parameter.name.clone(),
                    ty: substitute_package_type(&parameter.ty, substitutions)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
        return_type: substitute_package_type(&requirement.return_type, substitutions)?,
        receiver: match &requirement.receiver {
            SourceExecutableReceiver::Implicit { ty } => SourceExecutableReceiver::Implicit {
                ty: substitute_package_type(ty, substitutions)?,
            },
            receiver => receiver.clone(),
        },
        interface_type_params: requirement.interface_type_params.clone(),
        method_type_params: requirement.method_type_params.clone(),
        is_native: requirement.is_native,
        is_provider: requirement.is_provider,
        is_static: requirement.is_static,
    })
}

pub(crate) fn substitute_package_type(
    ty: &PackageTypeRef,
    substitutions: &BTreeMap<String, PackageTypeRef>,
) -> Result<PackageTypeRef, String> {
    match ty {
        PackageTypeRef::Local {
            local_type: TypeRefIr::TypeParam { name },
        } => Ok(substitutions
            .get(name)
            .cloned()
            .unwrap_or_else(|| ty.clone())),
        PackageTypeRef::Local { local_type } => {
            let substituted = substitute_local_type(local_type, substitutions)?;
            Ok(PackageTypeRef::Local {
                local_type: substituted,
            })
        }
        PackageTypeRef::Contract { .. } => Ok(ty.clone()),
        PackageTypeRef::Container { name, arguments } => Ok(PackageTypeRef::Container {
            name: name.clone(),
            arguments: arguments
                .iter()
                .map(|argument| substitute_package_type(argument, substitutions))
                .collect::<Result<Vec<_>, _>>()?,
        }),
        PackageTypeRef::Nullable { inner } => Ok(PackageTypeRef::Nullable {
            inner: Box::new(substitute_package_type(inner, substitutions)?),
        }),
    }
}

fn substitute_local_type(
    ty: &TypeRefIr,
    substitutions: &BTreeMap<String, PackageTypeRef>,
) -> Result<TypeRefIr, String> {
    match ty {
        TypeRefIr::TypeParam { name } => match substitutions.get(name) {
            Some(PackageTypeRef::Local { local_type }) => Ok(local_type.clone()),
            Some(non_local) => Err(format!(
                "exact substitution of `{name}` into a local inline type would erase {non_local:?}"
            )),
            None => Ok(ty.clone()),
        },
        TypeRefIr::Native { name, args } => Ok(TypeRefIr::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| substitute_local_type(arg, substitutions))
                .collect::<Result<Vec<_>, _>>()?,
        }),
        TypeRefIr::Record { fields } => Ok(TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, field)| {
                    Ok((name.clone(), substitute_local_type(field, substitutions)?))
                })
                .collect::<Result<BTreeMap<_, _>, String>>()?,
        }),
        TypeRefIr::Union { items } => Ok(TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| substitute_local_type(item, substitutions))
                .collect::<Result<Vec<_>, _>>()?,
        }),
        TypeRefIr::Nullable { inner } => Ok(TypeRefIr::Nullable {
            inner: Box::new(substitute_local_type(inner, substitutions)?),
        }),
        TypeRefIr::Function {
            params,
            return_type,
        } => Ok(TypeRefIr::Function {
            params: params
                .iter()
                .map(|parameter| {
                    Ok(FunctionTypeParamIr {
                        name: parameter.name.clone(),
                        ty: substitute_local_type(&parameter.ty, substitutions)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
            return_type: Box::new(substitute_local_type(return_type, substitutions)?),
        }),
        TypeRefIr::AnyInterface { interface } => {
            let mut canonical_type_args = Vec::with_capacity(interface.canonical_type_args.len());
            for argument in &interface.canonical_type_args {
                canonical_type_args.push(substitute_local_type(argument, substitutions)?);
            }
            Ok(TypeRefIr::AnyInterface {
                interface: skiff_artifact_model::InterfaceInstantiationRef {
                    interface_abi_id: interface.interface_abi_id.clone(),
                    canonical_type_args,
                },
            })
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. } => Ok(ty.clone()),
    }
}
