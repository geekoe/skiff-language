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
    context_module: &str,
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
                ty: projection_visible_type_ref(context_module, &param.ty, publication_type_names),
            })
            .collect(),
        return_type: projection_visible_type_ref(
            context_module,
            &executable.return_type,
            publication_type_names,
        ),
        self_type: executable
            .self_type
            .as_ref()
            .map(|ty| projection_visible_type_ref(context_module, ty, publication_type_names)),
        may_suspend: executable.may_suspend,
    }
}

pub(crate) fn projection_visible_interface_method_signature(
    context_module: &str,
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
                ty: projection_visible_type_ref(context_module, &param.ty, publication_type_names),
            })
            .collect(),
        return_type: projection_visible_type_ref(
            context_module,
            &method.return_type,
            publication_type_names,
        ),
        is_native: method.is_native,
        is_provider: method.is_provider,
        is_static: method.is_static,
        implicit_self: method
            .implicit_self
            .as_ref()
            .map(|ty| projection_visible_type_ref(context_module, ty, publication_type_names)),
    }
}

pub(crate) fn projection_visible_type_descriptor(
    context_module: &str,
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
                        projection_visible_type_ref(context_module, ty, publication_type_names),
                    )
                })
                .collect(),
        },
        TypeDescriptorIr::Alias { target } => TypeDescriptorIr::Alias {
            target: projection_visible_type_ref(context_module, target, publication_type_names),
        },
        TypeDescriptorIr::Union { variants } => TypeDescriptorIr::Union {
            variants: variants
                .iter()
                .map(|variant| {
                    projection_visible_type_ref(context_module, variant, publication_type_names)
                })
                .collect(),
        },
        TypeDescriptorIr::Native { symbol } => TypeDescriptorIr::Native {
            symbol: symbol.clone(),
        },
    }
}

/// Normalize a lowered `TypeRefIr` back into the symbolic form required by the
/// publication-visible ABI surface.
///
/// The `publication-local direct refs` lowering pass rewrites cross-module
/// references inside publication File IR into direct addresses
/// (`PublicationType { module_path, type_index }` across modules,
/// `LocalType { type_index }` within a module). The publication ABI/public
/// signature — the bytes fed into `operationAbiId` and
/// `remoteBoxProvenance.interfaceAbiId` — must stay in symbolic
/// `ServiceSymbol { module_path, symbol }` form so producers and consumers hash
/// identically. Both the direct `PublicationType` form and the same-module
/// `LocalType` form are resolved here (the latter against `context_module`).
pub(crate) fn projection_visible_type_ref(
    context_module: &str,
    ty: &TypeRefIr,
    publication_type_names: &PublicationVisibleTypeNames,
) -> TypeRefIr {
    match ty {
        TypeRefIr::LocalType { type_index } => publication_type_names
            .get(&(context_module.to_string(), *type_index))
            .map(|symbol| TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: context_module.to_string(),
                    symbol: symbol.clone(),
                },
            })
            .unwrap_or_else(|| ty.clone()),
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
                .map(|arg| projection_visible_type_ref(context_module, arg, publication_type_names))
                .collect(),
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| {
                    (
                        name.clone(),
                        projection_visible_type_ref(context_module, ty, publication_type_names),
                    )
                })
                .collect(),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| {
                    projection_visible_type_ref(context_module, item, publication_type_names)
                })
                .collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(projection_visible_type_ref(
                context_module,
                inner,
                publication_type_names,
            )),
        },
        TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
            interface: projection_visible_interface_instantiation_ref(
                context_module,
                interface,
                publication_type_names,
            ),
        },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: projection_visible_type_ref(
                        context_module,
                        &param.ty,
                        publication_type_names,
                    ),
                })
                .collect(),
            return_type: Box::new(projection_visible_type_ref(
                context_module,
                return_type,
                publication_type_names,
            )),
        },
        TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => ty.clone(),
    }
}

/// Normalize an interface instantiation reference (its `interfaceAbiId`
/// identity payload plus canonical type args) back to symbolic form, mirroring
/// [`projection_visible_type_ref`]. Used for `any I` interfaces and for
/// `remoteBoxProvenance` interface identities.
pub(crate) fn projection_visible_interface_instantiation_ref(
    context_module: &str,
    interface: &InterfaceInstantiationRef,
    publication_type_names: &PublicationVisibleTypeNames,
) -> InterfaceInstantiationRef {
    let interface_abi_id = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
        .map(|identity| {
            type_ref_abi_key(&projection_visible_type_ref(
                context_module,
                &identity,
                publication_type_names,
            ))
        })
        .unwrap_or_else(|_| interface.interface_abi_id.clone());
    InterfaceInstantiationRef {
        interface_abi_id,
        canonical_type_args: interface
            .canonical_type_args
            .iter()
            .map(|arg| projection_visible_type_ref(context_module, arg, publication_type_names))
            .collect(),
    }
}
