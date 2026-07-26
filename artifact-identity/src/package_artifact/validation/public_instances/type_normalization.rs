use std::collections::BTreeMap;

use skiff_artifact_model::{
    FunctionTypeParamIr, InterfaceMethodSignature, NominalTypeRefBaseIr, PackageArtifact,
    PackageLocalAbiSymbol, PackageRefIr, PackageSymbolRef, PackageTypeRef, TypeRefIr,
};

use crate::Result;

use super::super::invalid_artifact;

pub(super) fn normalized_implementation_type(
    artifact: &PackageArtifact,
    ty: &TypeRefIr,
    self_type: Option<&serde_json::Value>,
) -> Result<serde_json::Value> {
    let mut value = serde_json::to_value(ty).map_err(|error| {
        crate::ArtifactIdentityError::InvalidPackageArtifact {
            message: format!("failed to inspect public instance type: {error}"),
        }
    })?;
    normalize_implementation_type_value(artifact, &mut value, self_type)?;
    Ok(value)
}

pub(super) fn normalized_implementation_type_ref(
    artifact: &PackageArtifact,
    ty: &TypeRefIr,
    self_type: Option<&serde_json::Value>,
) -> Result<TypeRefIr> {
    serde_json::from_value(normalized_implementation_type(artifact, ty, self_type)?).map_err(
        |error| crate::ArtifactIdentityError::InvalidPackageArtifact {
            message: format!("failed to materialize normalized public instance type: {error}"),
        },
    )
}

pub(super) fn package_type_matches_implementation(
    artifact: &PackageArtifact,
    public_type: &PackageTypeRef,
    implementation_type: &TypeRefIr,
    self_type: Option<&serde_json::Value>,
) -> Result<bool> {
    let implementation_type =
        normalized_implementation_type_ref(artifact, implementation_type, self_type)?;
    package_type_matches_normalized(artifact, public_type, &implementation_type)
}

pub(super) fn instantiate_interface_methods(
    methods: &[InterfaceMethodSignature],
    interface_type_params: &[String],
    interface_arguments: &[TypeRefIr],
) -> Result<Vec<InterfaceMethodSignature>> {
    if interface_type_params.len() != interface_arguments.len() {
        return super::super::invalid_artifact(
            "public instance interface type argument count is not exact",
        );
    }
    let substitutions = interface_type_params
        .iter()
        .cloned()
        .zip(interface_arguments.iter().cloned())
        .collect::<BTreeMap<_, _>>();
    methods
        .iter()
        .map(|method| {
            let mut method_substitutions = substitutions.clone();
            for type_param in &method.type_params {
                method_substitutions.remove(type_param);
            }
            Ok(InterfaceMethodSignature {
                name: method.name.clone(),
                type_params: method.type_params.clone(),
                params: method
                    .params
                    .iter()
                    .map(|parameter| {
                        Ok(FunctionTypeParamIr {
                            name: parameter.name.clone(),
                            ty: substitute_type_params(&parameter.ty, &method_substitutions)?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
                return_type: substitute_type_params(&method.return_type, &method_substitutions)?,
                may_suspend: method.may_suspend,
                is_native: method.is_native,
                is_provider: method.is_provider,
                is_static: method.is_static,
                implicit_self: method
                    .implicit_self
                    .as_ref()
                    .map(|ty| substitute_type_params(ty, &method_substitutions))
                    .transpose()?,
            })
        })
        .collect()
}

fn substitute_type_params(
    ty: &TypeRefIr,
    substitutions: &BTreeMap<String, TypeRefIr>,
) -> Result<TypeRefIr> {
    let substitute = |ty: &TypeRefIr| substitute_type_params(ty, substitutions);
    Ok(match ty {
        TypeRefIr::TypeParam { name } => substitutions
            .get(name)
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name: name.clone(),
            args: args.iter().map(substitute).collect::<Result<Vec<_>>>()?,
        },
        TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
            base: base.clone(),
            arguments: arguments
                .iter()
                .map(substitute)
                .collect::<Result<Vec<_>>>()?,
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, field)| Ok((name.clone(), substitute(field)?)))
                .collect::<Result<BTreeMap<_, _>>>()?,
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items.iter().map(substitute).collect::<Result<Vec<_>>>()?,
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(substitute(inner)?),
        },
        TypeRefIr::AnyInterface { interface } => {
            let identity = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).map_err(
                |error| crate::ArtifactIdentityError::InvalidPackageArtifact {
                    message: format!(
                        "public instance interface identity is not canonical TypeRefIr JSON: {error}"
                    ),
                },
            )?;
            TypeRefIr::AnyInterface {
                interface: crate::interface_instantiation_ref(
                    substitute(&identity)?,
                    interface
                        .canonical_type_args
                        .iter()
                        .map(substitute)
                        .collect::<Result<Vec<_>>>()?,
                ),
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|parameter| {
                    Ok(FunctionTypeParamIr {
                        name: parameter.name.clone(),
                        ty: substitute(&parameter.ty)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            return_type: Box::new(substitute(return_type)?),
        },
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. } => ty.clone(),
    })
}

fn package_type_matches_normalized(
    artifact: &PackageArtifact,
    public_type: &PackageTypeRef,
    implementation_type: &TypeRefIr,
) -> Result<bool> {
    match public_type {
        PackageTypeRef::Local { local_type } => {
            let local_type = normalized_implementation_type_ref(artifact, local_type, None)?;
            local_type_matches(artifact, &local_type, implementation_type)
        }
        PackageTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => Ok(match implementation_type {
            TypeRefIr::PackageSchema {
                package_id: implementation_package,
                stable_schema_key: implementation_key,
                package_schema_type_id: implementation_type_id,
            } => {
                package_id == implementation_package
                    && stable_schema_key == implementation_key
                    && package_schema_type_id == implementation_type_id
            }
            TypeRefIr::PackageSymbol { symbol } => package_schema_matches_symbol(
                artifact,
                package_id,
                stable_schema_key,
                package_schema_type_id,
                symbol,
            ),
            _ => false,
        }),
        PackageTypeRef::Container { name, arguments } => {
            let TypeRefIr::Builtin {
                name: implementation_name,
                args,
            } = implementation_type
            else {
                return Ok(false);
            };
            if name != implementation_name || arguments.len() != args.len() {
                return Ok(false);
            }
            for (argument, implementation_argument) in arguments.iter().zip(args) {
                if !package_type_matches_normalized(artifact, argument, implementation_argument)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        PackageTypeRef::Nullable { inner } => {
            let TypeRefIr::Nullable {
                inner: implementation_inner,
            } = implementation_type
            else {
                return Ok(false);
            };
            package_type_matches_normalized(artifact, inner, implementation_inner)
        }
        PackageTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            let TypeRefIr::AnyInterface {
                interface: implementation_interface,
            } = implementation_type
            else {
                return Ok(false);
            };
            let implementation_identity =
                serde_json::from_str::<TypeRefIr>(&implementation_interface.interface_abi_id)
                    .map_err(|error| {
                        crate::ArtifactIdentityError::InvalidPackageArtifact {
                            message: format!(
                                "public instance interface identity is not canonical TypeRefIr JSON: {error}"
                            ),
                        }
                    })?;
            if !package_type_matches_normalized(artifact, interface, &implementation_identity)?
                || arguments.len() != implementation_interface.canonical_type_args.len()
            {
                return Ok(false);
            }
            for (argument, implementation_argument) in arguments
                .iter()
                .zip(&implementation_interface.canonical_type_args)
            {
                if !package_type_matches_normalized(artifact, argument, implementation_argument)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }
}

fn local_type_matches(
    artifact: &PackageArtifact,
    public_type: &TypeRefIr,
    implementation_type: &TypeRefIr,
) -> Result<bool> {
    if public_type == implementation_type {
        return Ok(true);
    }
    match (public_type, implementation_type) {
        (
            TypeRefIr::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            },
            TypeRefIr::PackageSymbol { symbol },
        ) => Ok(package_schema_matches_symbol(
            artifact,
            package_id,
            stable_schema_key,
            package_schema_type_id,
            symbol,
        )),
        (
            TypeRefIr::Builtin {
                name: public_name,
                args: public_args,
            },
            TypeRefIr::Builtin {
                name: implementation_name,
                args: implementation_args,
            },
        ) if public_name == implementation_name
            && public_args.len() == implementation_args.len() =>
        {
            local_types_match_all(artifact, public_args, implementation_args)
        }
        (
            TypeRefIr::AppliedNominal {
                base: public_base,
                arguments: public_arguments,
            },
            TypeRefIr::AppliedNominal {
                base: implementation_base,
                arguments: implementation_arguments,
            },
        ) if nominal_bases_match(public_base, implementation_base)
            && public_arguments.len() == implementation_arguments.len() =>
        {
            local_types_match_all(artifact, public_arguments, implementation_arguments)
        }
        (
            TypeRefIr::Record {
                fields: public_fields,
            },
            TypeRefIr::Record {
                fields: implementation_fields,
            },
        ) if public_fields.len() == implementation_fields.len()
            && public_fields.keys().eq(implementation_fields.keys()) =>
        {
            for (public_field, implementation_field) in
                public_fields.values().zip(implementation_fields.values())
            {
                if !local_type_matches(artifact, public_field, implementation_field)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (
            TypeRefIr::Union {
                items: public_items,
            },
            TypeRefIr::Union {
                items: implementation_items,
            },
        ) if public_items.len() == implementation_items.len() => {
            local_types_match_all(artifact, public_items, implementation_items)
        }
        (
            TypeRefIr::Nullable {
                inner: public_inner,
            },
            TypeRefIr::Nullable {
                inner: implementation_inner,
            },
        ) => local_type_matches(artifact, public_inner, implementation_inner),
        (
            TypeRefIr::Function {
                params: public_params,
                return_type: public_return,
            },
            TypeRefIr::Function {
                params: implementation_params,
                return_type: implementation_return,
            },
        ) if public_params.len() == implementation_params.len() => {
            for (public_param, implementation_param) in
                public_params.iter().zip(implementation_params)
            {
                if public_param.name != implementation_param.name
                    || !local_type_matches(artifact, &public_param.ty, &implementation_param.ty)?
                {
                    return Ok(false);
                }
            }
            local_type_matches(artifact, public_return, implementation_return)
        }
        (
            TypeRefIr::AnyInterface {
                interface: public_interface,
            },
            TypeRefIr::AnyInterface {
                interface: implementation_interface,
            },
        ) if public_interface.canonical_type_args.len()
            == implementation_interface.canonical_type_args.len() =>
        {
            let public_identity =
                serde_json::from_str::<TypeRefIr>(&public_interface.interface_abi_id).map_err(
                    |error| crate::ArtifactIdentityError::InvalidPackageArtifact {
                        message: format!(
                            "public instance public interface identity is not canonical TypeRefIr JSON: {error}"
                        ),
                    },
                )?;
            let implementation_identity =
                serde_json::from_str::<TypeRefIr>(&implementation_interface.interface_abi_id)
                    .map_err(|error| {
                        crate::ArtifactIdentityError::InvalidPackageArtifact {
                            message: format!(
                                "public instance implementation interface identity is not canonical TypeRefIr JSON: {error}"
                            ),
                        }
                    })?;
            if !local_type_matches(artifact, &public_identity, &implementation_identity)? {
                return Ok(false);
            }
            local_types_match_all(
                artifact,
                &public_interface.canonical_type_args,
                &implementation_interface.canonical_type_args,
            )
        }
        _ => Ok(false),
    }
}

fn local_types_match_all(
    artifact: &PackageArtifact,
    public_types: &[TypeRefIr],
    implementation_types: &[TypeRefIr],
) -> Result<bool> {
    for (public_type, implementation_type) in public_types.iter().zip(implementation_types) {
        if !local_type_matches(artifact, public_type, implementation_type)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn nominal_bases_match(
    public_base: &NominalTypeRefBaseIr,
    implementation_base: &NominalTypeRefBaseIr,
) -> bool {
    public_base == implementation_base
}

fn package_schema_matches_symbol(
    artifact: &PackageArtifact,
    package_id: &str,
    stable_schema_key: &str,
    package_schema_type_id: &skiff_artifact_model::PackageSchemaTypeId,
    symbol: &PackageSymbolRef,
) -> bool {
    match &symbol.package {
        PackageRefIr::PackageId {
            package_id: symbol_package_id,
        } if symbol_package_id == &artifact.package_id => {
            if package_id != artifact.package_id
                || symbol.abi_expectation.is_some()
                || !artifact
                    .package_schema_type_records
                    .contains_key(package_schema_type_id)
            {
                return false;
            }
            let Some(PackageLocalAbiSymbol::Type {
                is_alias: false, ..
            }) = artifact
                .package_local_abi
                .public_symbols
                .get(stable_schema_key)
            else {
                return false;
            };
            let Some(public_link) = artifact.implementation_links.types.get(stable_schema_key)
            else {
                return false;
            };
            let Some(source_link) = artifact.implementation_links.types.get(&symbol.symbol_path)
            else {
                return false;
            };
            public_link.file.file_ir_identity == source_link.file.file_ir_identity
                && public_link.file.module_path == source_link.file.module_path
                && public_link.type_index == source_link.type_index
        }
        PackageRefIr::PackageId {
            package_id: symbol_package_id,
        } => {
            package_id == symbol_package_id
                && stable_schema_key == symbol.symbol_path
                && artifact
                    .package_requirements
                    .iter()
                    .any(|requirement| requirement.package_id == *symbol_package_id)
        }
        PackageRefIr::Dependency { dependency_ref } => artifact
            .package_requirements
            .iter()
            .find(|requirement| requirement.alias == *dependency_ref)
            .is_some_and(|requirement| {
                package_id == requirement.package_id
                    && stable_schema_key == symbol.symbol_path
                    && symbol.abi_expectation.as_deref().is_none_or(|expectation| {
                        expectation == requirement.expected_local_abi.as_str()
                    })
            }),
    }
}

fn normalize_implementation_type_value(
    artifact: &PackageArtifact,
    value: &mut serde_json::Value,
    self_type: Option<&serde_json::Value>,
) -> Result<()> {
    let serde_json::Value::Object(object) = value else {
        return Ok(());
    };

    if matches!(
        object.get("kind").and_then(serde_json::Value::as_str),
        Some("builtin")
    ) && object.get("name").and_then(serde_json::Value::as_str) == Some("Self")
        && object
            .get("args")
            .and_then(serde_json::Value::as_array)
            .is_none_or(Vec::is_empty)
    {
        let Some(self_type) = self_type else {
            return invalid_artifact("public instance interface type contains unresolved Self");
        };
        *value = self_type.clone();
        return Ok(());
    }

    if let Some(interface_abi_id) = object
        .get_mut("interface")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|interface| interface.get_mut("interfaceAbiId"))
        .and_then(|value| value.as_str())
    {
        let interface_type =
            serde_json::from_str::<TypeRefIr>(interface_abi_id).map_err(|error| {
                crate::ArtifactIdentityError::InvalidPackageArtifact {
                    message: format!(
                    "public instance interface identity is not canonical TypeRefIr JSON: {error}"
                ),
                }
            })?;
        let mut interface_type = serde_json::to_value(interface_type).map_err(|error| {
            crate::ArtifactIdentityError::InvalidPackageArtifact {
                message: format!("failed to inspect public instance interface identity: {error}"),
            }
        })?;
        normalize_implementation_type_value(artifact, &mut interface_type, self_type)?;
        let encoded = serde_json::to_string(&interface_type).map_err(|error| {
            crate::ArtifactIdentityError::InvalidPackageArtifact {
                message: format!("failed to normalize public instance interface identity: {error}"),
            }
        })?;
        object
            .get_mut("interface")
            .and_then(serde_json::Value::as_object_mut)
            .expect("interface object was just inspected")
            .insert(
                "interfaceAbiId".to_string(),
                serde_json::Value::String(encoded),
            );
    }

    for nested in object.values_mut() {
        match nested {
            serde_json::Value::Array(items) => {
                for item in items {
                    normalize_implementation_type_value(artifact, item, self_type)?;
                }
            }
            serde_json::Value::Object(_) => {
                normalize_implementation_type_value(artifact, nested, self_type)?;
            }
            _ => {}
        }
    }

    let kind = object
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if matches!(kind, "serviceSymbol" | "dbObjectSymbol") {
        let source_symbol = object.get("symbol").and_then(serde_json::Value::as_object);
        let module_path = source_symbol
            .and_then(|symbol| symbol.get("modulePath"))
            .and_then(serde_json::Value::as_str);
        let symbol = source_symbol
            .and_then(|symbol| symbol.get("symbol"))
            .and_then(serde_json::Value::as_str);
        if let (Some(module_path), Some(symbol)) = (module_path, symbol) {
            let source_path = format!("{module_path}.{symbol}");
            if matches!(
                artifact
                    .package_local_abi
                    .implementation_symbols
                    .get(&source_path),
                Some(PackageLocalAbiSymbol::Type { .. })
            ) {
                *value = serde_json::to_value(TypeRefIr::PackageSymbol {
                    symbol: skiff_artifact_model::PackageSymbolRef {
                        package: PackageRefIr::PackageId {
                            package_id: artifact.package_id.clone(),
                        },
                        symbol_path: source_path,
                        abi_expectation: None,
                    },
                })
                .expect("PackageSymbol TypeRefIr must serialize");
                return Ok(());
            }
        }
    }
    if kind == "packageSymbol" {
        let Some(symbol) = object
            .get_mut("symbol")
            .and_then(serde_json::Value::as_object_mut)
        else {
            return Ok(());
        };
        let is_current_package = symbol
            .get("package")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|package| {
                package.get("kind").and_then(serde_json::Value::as_str) == Some("packageId")
                    && package.get("packageId").and_then(serde_json::Value::as_str)
                        == Some(artifact.package_id.as_str())
            });
        if is_current_package {
            symbol.remove("abiExpectation");
        }
    }
    Ok(())
}
