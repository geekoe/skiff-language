use std::collections::BTreeMap;

use skiff_artifact_model::{
    FunctionTypeParamIr, InterfaceDeclIr, InterfaceOperationIr, PackageRefIr, PackageSymbolRef,
    TypeRefIr,
};

use crate::type_ref::substitute_type_params_in_type_ref;

pub use skiff_artifact_model::package_unit::InterfaceMethodSignature;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterfaceMethodInstantiationError {
    pub expected_type_args: usize,
    pub actual_type_args: usize,
}

pub fn instantiate_interface_method_signatures(
    methods: impl IntoIterator<Item = InterfaceMethodSignature>,
    interface_type_params: &[String],
    canonical_type_args: &[TypeRefIr],
) -> Result<Vec<InterfaceMethodSignature>, InterfaceMethodInstantiationError> {
    if interface_type_params.len() != canonical_type_args.len() {
        return Err(InterfaceMethodInstantiationError {
            expected_type_args: interface_type_params.len(),
            actual_type_args: canonical_type_args.len(),
        });
    }

    let substitutions = interface_type_params
        .iter()
        .cloned()
        .zip(canonical_type_args.iter().cloned())
        .collect::<BTreeMap<_, _>>();
    Ok(methods
        .into_iter()
        .map(|method| instantiate_interface_method_signature(method, &substitutions))
        .collect())
}

fn instantiate_interface_method_signature(
    method: InterfaceMethodSignature,
    substitutions: &BTreeMap<String, TypeRefIr>,
) -> InterfaceMethodSignature {
    let InterfaceMethodSignature {
        name,
        type_params,
        params,
        return_type,
        is_native,
        is_provider,
        is_static,
        implicit_self,
    } = method;
    let mut method_substitutions = substitutions.clone();
    for type_param in &type_params {
        method_substitutions.remove(type_param);
    }
    InterfaceMethodSignature {
        name,
        type_params,
        params: params
            .into_iter()
            .map(|param| FunctionTypeParamIr {
                name: param.name,
                ty: substitute_type_params_in_type_ref(param.ty, &method_substitutions),
            })
            .collect(),
        return_type: substitute_type_params_in_type_ref(return_type, &method_substitutions),
        is_native,
        is_provider,
        is_static,
        implicit_self: implicit_self
            .map(|ty| substitute_type_params_in_type_ref(ty, &method_substitutions)),
    }
}

#[derive(Clone, Debug, Default)]
pub struct PackageTypeSymbolIndex {
    local_types: BTreeMap<(String, u32), String>,
    service_symbols: BTreeMap<(String, String), String>,
    package_dependencies: BTreeMap<String, String>,
}

impl PackageTypeSymbolIndex {
    pub fn insert_type(
        &mut self,
        module_path: impl Into<String>,
        type_index: u32,
        source_symbol: impl Into<String>,
        public_symbol: impl Into<String>,
    ) {
        let module_path = module_path.into();
        let source_symbol = source_symbol.into();
        let public_symbol = public_symbol.into();
        self.local_types
            .entry((module_path.clone(), type_index))
            .or_insert_with(|| public_symbol.clone());
        self.service_symbols
            .entry((module_path, source_symbol))
            .or_insert(public_symbol);
    }

    pub fn insert_dependency(
        &mut self,
        dependency_ref: impl Into<String>,
        package_id: impl Into<String>,
    ) {
        self.package_dependencies
            .insert(dependency_ref.into(), package_id.into());
    }
}

pub fn package_interface_method_signatures(
    package_id: &str,
    type_symbols: &PackageTypeSymbolIndex,
    module_path: &str,
    interface: &InterfaceDeclIr,
) -> Result<Vec<InterfaceMethodSignature>, String> {
    interface
        .operations
        .iter()
        .map(|operation| {
            package_interface_method_signature(
                package_id,
                type_symbols,
                module_path,
                &interface.name,
                operation,
            )
        })
        .collect()
}

pub fn normalize_package_interface_method_signatures(
    package_id: &str,
    type_symbols: &PackageTypeSymbolIndex,
    module_path: &str,
    interface_name: &str,
    methods: &[InterfaceMethodSignature],
) -> Result<Vec<InterfaceMethodSignature>, String> {
    methods
        .iter()
        .map(|method| {
            normalize_package_interface_method_signature(
                package_id,
                type_symbols,
                module_path,
                interface_name,
                method,
            )
        })
        .collect()
}

pub fn normalize_package_interface_type_ref(
    package_id: &str,
    type_symbols: &PackageTypeSymbolIndex,
    module_path: &str,
    ty: &TypeRefIr,
    context: &str,
) -> Result<TypeRefIr, String> {
    package_interface_type_ref(package_id, type_symbols, module_path, ty, context)
}

fn package_interface_method_signature(
    package_id: &str,
    type_symbols: &PackageTypeSymbolIndex,
    module_path: &str,
    interface_name: &str,
    operation: &InterfaceOperationIr,
) -> Result<InterfaceMethodSignature, String> {
    let context = format!("{module_path}.{interface_name}.{}", operation.name);
    Ok(InterfaceMethodSignature {
        name: operation.name.clone(),
        type_params: operation.type_params.clone(),
        params: operation
            .params
            .iter()
            .map(|param| {
                package_interface_method_param(
                    package_id,
                    type_symbols,
                    module_path,
                    param,
                    &context,
                )
            })
            .collect::<Result<Vec<_>, _>>()?,
        return_type: package_interface_type_ref(
            package_id,
            type_symbols,
            module_path,
            &operation.return_type,
            &context,
        )?,
        is_native: operation.is_native,
        is_provider: operation.is_provider,
        is_static: operation.is_static,
        implicit_self: operation
            .implicit_self
            .as_ref()
            .map(|ty| {
                package_interface_type_ref(package_id, type_symbols, module_path, ty, &context)
            })
            .transpose()?,
    })
}

fn normalize_package_interface_method_signature(
    package_id: &str,
    type_symbols: &PackageTypeSymbolIndex,
    module_path: &str,
    interface_name: &str,
    method: &InterfaceMethodSignature,
) -> Result<InterfaceMethodSignature, String> {
    let context = format!("{module_path}.{interface_name}.{}", method.name);
    Ok(InterfaceMethodSignature {
        name: method.name.clone(),
        type_params: method.type_params.clone(),
        params: method
            .params
            .iter()
            .map(|param| {
                package_interface_method_param(
                    package_id,
                    type_symbols,
                    module_path,
                    param,
                    &context,
                )
            })
            .collect::<Result<Vec<_>, _>>()?,
        return_type: package_interface_type_ref(
            package_id,
            type_symbols,
            module_path,
            &method.return_type,
            &context,
        )?,
        is_native: method.is_native,
        is_provider: method.is_provider,
        is_static: method.is_static,
        implicit_self: method
            .implicit_self
            .as_ref()
            .map(|ty| {
                package_interface_type_ref(package_id, type_symbols, module_path, ty, &context)
            })
            .transpose()?,
    })
}

fn package_interface_method_param(
    package_id: &str,
    type_symbols: &PackageTypeSymbolIndex,
    module_path: &str,
    param: &FunctionTypeParamIr,
    context: &str,
) -> Result<FunctionTypeParamIr, String> {
    Ok(FunctionTypeParamIr {
        name: param.name.clone(),
        ty: package_interface_type_ref(package_id, type_symbols, module_path, &param.ty, context)?,
    })
}

fn package_interface_type_ref(
    package_id: &str,
    type_symbols: &PackageTypeSymbolIndex,
    module_path: &str,
    ty: &TypeRefIr,
    context: &str,
) -> Result<TypeRefIr, String> {
    match ty {
        TypeRefIr::Native { name, args } => Ok(TypeRefIr::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| {
                    package_interface_type_ref(package_id, type_symbols, module_path, arg, context)
                })
                .collect::<Result<Vec<_>, _>>()?,
        }),
        TypeRefIr::LocalType { type_index } => {
            let symbol_path = type_symbols
                .local_types
                .get(&(module_path.to_string(), *type_index))
                .ok_or_else(|| {
                    format!(
                        "package {package_id} exported interface method {context} references non-exported local type index {type_index} in {module_path}"
                    )
                })?;
            Ok(package_symbol_type_ref(package_id, symbol_path, None))
        }
        TypeRefIr::ServiceSymbol { symbol } => {
            if let Some(symbol_path) = type_symbols
                .service_symbols
                .get(&(symbol.module_path.clone(), symbol.symbol.clone()))
            {
                Ok(package_symbol_type_ref(package_id, symbol_path, None))
            } else {
                Ok(ty.clone())
            }
        }
        TypeRefIr::PackageSymbol { symbol } => {
            let package = match &symbol.package {
                PackageRefIr::Dependency { dependency_ref } => type_symbols
                    .package_dependencies
                    .get(dependency_ref)
                    .map(|package_id| PackageRefIr::PackageId {
                        package_id: package_id.clone(),
                    })
                    .unwrap_or_else(|| symbol.package.clone()),
                PackageRefIr::PackageId { .. } => symbol.package.clone(),
            };
            Ok(TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package,
                    symbol_path: symbol.symbol_path.clone(),
                    abi_expectation: symbol.abi_expectation.clone(),
                },
            })
        }
        TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => Ok(ty.clone()),
        TypeRefIr::Record { fields } => Ok(TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, field_ty)| {
                    Ok((
                        name.clone(),
                        package_interface_type_ref(
                            package_id,
                            type_symbols,
                            module_path,
                            field_ty,
                            context,
                        )?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>, String>>()?,
        }),
        TypeRefIr::Union { items } => Ok(TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| {
                    package_interface_type_ref(package_id, type_symbols, module_path, item, context)
                })
                .collect::<Result<Vec<_>, _>>()?,
        }),
        TypeRefIr::Nullable { inner } => Ok(TypeRefIr::Nullable {
            inner: Box::new(package_interface_type_ref(
                package_id,
                type_symbols,
                module_path,
                inner,
                context,
            )?),
        }),
        TypeRefIr::AnyInterface { interface } => Ok(TypeRefIr::AnyInterface {
            interface: skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| {
                        package_interface_type_ref(
                            package_id,
                            type_symbols,
                            module_path,
                            arg,
                            context,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
        }),
        TypeRefIr::Function {
            params,
            return_type,
        } => Ok(TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| {
                    package_interface_method_param(
                        package_id,
                        type_symbols,
                        module_path,
                        param,
                        context,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
            return_type: Box::new(package_interface_type_ref(
                package_id,
                type_symbols,
                module_path,
                return_type,
                context,
            )?),
        }),
    }
}

fn package_symbol_type_ref(
    package_id: &str,
    symbol_path: &str,
    abi_expectation: Option<String>,
) -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: package_id.to_string(),
            },
            symbol_path: symbol_path.to_string(),
            abi_expectation,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_param(name: &str) -> TypeRefIr {
        TypeRefIr::TypeParam {
            name: name.to_string(),
        }
    }

    fn native(name: &str, args: Vec<TypeRefIr>) -> TypeRefIr {
        TypeRefIr::Native {
            name: name.to_string(),
            args,
        }
    }

    fn param(name: &str, ty: TypeRefIr) -> FunctionTypeParamIr {
        FunctionTypeParamIr {
            name: name.to_string(),
            ty,
        }
    }

    fn method(
        type_params: Vec<&str>,
        params: Vec<FunctionTypeParamIr>,
        return_type: TypeRefIr,
        implicit_self: Option<TypeRefIr>,
    ) -> InterfaceMethodSignature {
        InterfaceMethodSignature {
            name: "call".to_string(),
            type_params: type_params.into_iter().map(str::to_string).collect(),
            params,
            return_type,
            is_native: false,
            is_provider: false,
            is_static: false,
            implicit_self,
        }
    }

    #[test]
    fn instantiates_implicit_self_type_params() {
        let instantiated = instantiate_interface_method_signatures(
            vec![method(
                vec![],
                vec![],
                type_param("T"),
                Some(type_param("T")),
            )],
            &["T".to_string()],
            &[native("String", Vec::new())],
        )
        .unwrap();

        assert_eq!(
            instantiated[0].implicit_self,
            Some(native("String", Vec::new()))
        );
    }

    #[test]
    fn instantiates_nested_params_and_return_type() {
        let mut fields = BTreeMap::new();
        fields.insert("value".to_string(), type_param("T"));
        let instantiated = instantiate_interface_method_signatures(
            vec![method(
                vec![],
                vec![param(
                    "items",
                    native(
                        "Array",
                        vec![TypeRefIr::Nullable {
                            inner: Box::new(type_param("T")),
                        }],
                    ),
                )],
                TypeRefIr::Record { fields },
                None,
            )],
            &["T".to_string()],
            &[native("Number", Vec::new())],
        )
        .unwrap();

        assert_eq!(
            instantiated[0].params[0].ty,
            native(
                "Array",
                vec![TypeRefIr::Nullable {
                    inner: Box::new(native("Number", Vec::new())),
                }],
            )
        );
        let TypeRefIr::Record { fields } = &instantiated[0].return_type else {
            panic!("return type should stay a record");
        };
        assert_eq!(fields.get("value"), Some(&native("Number", Vec::new())));
    }

    #[test]
    fn method_type_params_shadow_interface_substitutions() {
        let instantiated = instantiate_interface_method_signatures(
            vec![method(
                vec!["T"],
                vec![param("value", type_param("T"))],
                type_param("T"),
                Some(type_param("T")),
            )],
            &["T".to_string()],
            &[native("String", Vec::new())],
        )
        .unwrap();

        assert_eq!(instantiated[0].params[0].ty, type_param("T"));
        assert_eq!(instantiated[0].return_type, type_param("T"));
        assert_eq!(instantiated[0].implicit_self, Some(type_param("T")));
    }

    #[test]
    fn mismatched_type_arg_count_returns_error() {
        let error = instantiate_interface_method_signatures(
            vec![method(vec![], vec![], type_param("T"), None)],
            &["T".to_string(), "U".to_string()],
            &[native("String", Vec::new())],
        )
        .unwrap_err();

        assert_eq!(
            error,
            InterfaceMethodInstantiationError {
                expected_type_args: 2,
                actual_type_args: 1,
            }
        );
    }
}
