use std::collections::BTreeMap;

use skiff_artifact_model::package_unit::InterfaceMethodSignature;
use skiff_artifact_model::{
    type_ref_abi_key, ExecutableIr, ExecutableSignatureIr, FileIrUnit, FunctionTypeParamIr,
    InterfaceInstantiationRef, ParamIr, ServiceSymbolRef, TypeDescriptorIr, TypeRefIr,
};

pub(crate) type PublicationVisibleTypeNames = BTreeMap<(String, u32), String>;

pub(crate) fn publication_type_names_from_file_units<'a>(
    file_ir_units: impl IntoIterator<Item = (&'a str, &'a FileIrUnit)>,
) -> PublicationVisibleTypeNames {
    file_ir_units
        .into_iter()
        .flat_map(|(module_path, unit)| {
            unit.type_table
                .iter()
                .enumerate()
                .map(move |(index, ty)| ((module_path.to_string(), index as u32), ty.name.clone()))
        })
        .collect()
}

pub(crate) fn projection_visible_executable_signature(
    executable: &ExecutableIr,
    publication_type_names: &PublicationVisibleTypeNames,
) -> ExecutableSignatureIr {
    ExecutableSignatureIr {
        params: executable
            .params
            .iter()
            .map(|param| ParamIr {
                name: param.name.clone(),
                slot: param.slot,
                ty: projection_visible_type_ref(&param.ty, publication_type_names),
            })
            .collect(),
        return_type: projection_visible_type_ref(&executable.return_type, publication_type_names),
        self_type: executable
            .self_type
            .as_ref()
            .map(|ty| projection_visible_type_ref(ty, publication_type_names)),
        may_suspend: executable.may_suspend,
    }
}

pub(crate) fn projection_visible_interface_method_signature(
    method: &InterfaceMethodSignature,
    publication_type_names: &PublicationVisibleTypeNames,
) -> InterfaceMethodSignature {
    InterfaceMethodSignature {
        name: method.name.clone(),
        type_params: method.type_params.clone(),
        params: method
            .params
            .iter()
            .map(|param| FunctionTypeParamIr {
                name: param.name.clone(),
                ty: projection_visible_type_ref(&param.ty, publication_type_names),
            })
            .collect(),
        return_type: projection_visible_type_ref(&method.return_type, publication_type_names),
        is_native: method.is_native,
        is_provider: method.is_provider,
        is_static: method.is_static,
        implicit_self: method
            .implicit_self
            .as_ref()
            .map(|ty| projection_visible_type_ref(ty, publication_type_names)),
    }
}

pub(crate) fn projection_visible_type_descriptor(
    descriptor: &TypeDescriptorIr,
    publication_type_names: &PublicationVisibleTypeNames,
) -> TypeDescriptorIr {
    match descriptor {
        TypeDescriptorIr::Record { fields } => TypeDescriptorIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| {
                    (
                        name.clone(),
                        projection_visible_type_ref(ty, publication_type_names),
                    )
                })
                .collect(),
        },
        TypeDescriptorIr::Alias { target } => TypeDescriptorIr::Alias {
            target: projection_visible_type_ref(target, publication_type_names),
        },
        TypeDescriptorIr::Union { variants } => TypeDescriptorIr::Union {
            variants: variants
                .iter()
                .map(|variant| projection_visible_type_ref(variant, publication_type_names))
                .collect(),
        },
        TypeDescriptorIr::Native { symbol } => TypeDescriptorIr::Native {
            symbol: symbol.clone(),
        },
    }
}

pub(crate) fn projection_visible_type_ref(
    ty: &TypeRefIr,
    publication_type_names: &PublicationVisibleTypeNames,
) -> TypeRefIr {
    match ty {
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => publication_type_names
            .get(&(module_path.clone(), *type_index))
            .map(|symbol| TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: module_path.clone(),
                    symbol: symbol.clone(),
                },
            })
            .unwrap_or_else(|| ty.clone()),
        TypeRefIr::Native { name, args } => TypeRefIr::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| projection_visible_type_ref(arg, publication_type_names))
                .collect(),
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| {
                    (
                        name.clone(),
                        projection_visible_type_ref(ty, publication_type_names),
                    )
                })
                .collect(),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| projection_visible_type_ref(item, publication_type_names))
                .collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(projection_visible_type_ref(inner, publication_type_names)),
        },
        TypeRefIr::AnyInterface { interface } => {
            let interface_abi_id = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
                .map(|identity| {
                    type_ref_abi_key(&projection_visible_type_ref(
                        &identity,
                        publication_type_names,
                    ))
                })
                .unwrap_or_else(|_| interface.interface_abi_id.clone());
            TypeRefIr::AnyInterface {
                interface: InterfaceInstantiationRef {
                    interface_abi_id,
                    canonical_type_args: interface
                        .canonical_type_args
                        .iter()
                        .map(|arg| projection_visible_type_ref(arg, publication_type_names))
                        .collect(),
                },
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: projection_visible_type_ref(&param.ty, publication_type_names),
                })
                .collect(),
            return_type: Box::new(projection_visible_type_ref(
                return_type,
                publication_type_names,
            )),
        },
        TypeRefIr::LocalType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => ty.clone(),
    }
}
