use super::*;
use crate::{
    SourceLocalInterfaceConformance, SourceLocalInterfaceConformanceFacts,
    SourceLocalInterfaceConformanceFactsError as Error,
};

impl TypeResolutionModel {
    /// Builds the canonical, source-authoritative handoff for every local
    /// `implements` declaration.
    ///
    /// This is fallible by design: pool-local type references, package aliases
    /// without a selected exact ABI, and non-exact implementation slots never
    /// become projection input.
    pub fn local_interface_conformance_facts(
        &self,
    ) -> Result<SourceLocalInterfaceConformanceFacts, Error> {
        let entries = self
            .interface_conformances
            .iter()
            .enumerate()
            .map(|(index, conformance)| self.local_interface_conformance_entry(index, conformance))
            .collect::<Result<Vec<_>, _>>()?;
        SourceLocalInterfaceConformanceFacts::try_from_entries(entries)
    }

    fn local_interface_conformance_entry(
        &self,
        index: usize,
        conformance: &InterfaceConformanceResolution,
    ) -> Result<SourceLocalInterfaceConformance, Error> {
        let receiver = conformance.receiver.clone();
        let receiver_type = TypeRefIr::ServiceSymbol {
            symbol: service_symbol_ref_from_source_key(&receiver),
        };
        let declaration_arguments = conformance
            .receiver_type_params
            .iter()
            .map(|name| TypeRefIr::TypeParam { name: name.clone() })
            .collect::<Vec<_>>();

        let slots = match &conformance.interface.identity {
            TypeRefIr::ServiceSymbol { symbol } => {
                let interface = InterfaceInstantiation {
                    symbol: SourceSymbolKey::new(
                        symbol
                            .module_path
                            .strip_prefix("root.")
                            .unwrap_or(&symbol.module_path),
                        &symbol.symbol,
                    ),
                    args: conformance.interface.args.clone(),
                };
                let semantic_conformance = crate::semantic::interface::InterfaceConformanceFact {
                    receiver_type_params: conformance.receiver_type_params.clone(),
                    receiver: TypeInstantiationPattern {
                        symbol: receiver.clone(),
                        args: declaration_arguments.clone(),
                    },
                    interface,
                };
                self.interface_semantics
                    .method_slots_for_local_conformance(&semantic_conformance)
                    .map_err(|source| Error::SourceInterfaceSlots {
                        receiver: receiver.clone(),
                        message: source.to_string(),
                    })?
            }
            TypeRefIr::PackageSymbol { .. } => self
                .package_method_slots_for_local_conformance(
                    &receiver,
                    &declaration_arguments,
                    &conformance.interface,
                    conformance,
                )
                .map_err(|message| Error::ImportedInterfaceSlots {
                    receiver: receiver.clone(),
                    message,
                })?
                .ok_or_else(|| Error::ImportedInterfaceImplementationMismatch {
                    receiver: receiver.clone(),
                    interface_abi_id: type_ref_abi_key(&conformance.interface.identity),
                })?,
            _ => {
                return Err(Error::SourceInterfaceSlots {
                    receiver,
                    message: format!(
                        "implements identity {} is not a source or package interface",
                        debug_text(&conformance.interface.identity)
                    ),
                });
            }
        };

        let mut method_names = BTreeSet::new();
        let implementations = slots
            .iter()
            .enumerate()
            .map(|(expected, slot)| {
                let expected = u32::try_from(expected).unwrap_or(u32::MAX);
                if slot.slot != expected {
                    return Err(Error::NonContiguousInterfaceSlot {
                        receiver: receiver.clone(),
                        expected,
                        actual: slot.slot,
                    });
                }
                if !method_names.insert(slot.name.as_str()) {
                    return Err(Error::DuplicateInterfaceMethodSlot {
                        receiver: receiver.clone(),
                        method: slot.name.clone(),
                    });
                }
                self.local_impl_methods
                    .get(&receiver)
                    .and_then(|methods| methods.get(&slot.name))
                    .map(|method| method.source_callable.clone())
                    .ok_or_else(|| Error::MissingImplementationMethod {
                        receiver: receiver.clone(),
                        slot: slot.slot,
                        method: slot.name.clone(),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let identity = self.owner_stable_conformance_type_ref(
            receiver.module_path(),
            &conformance.interface.identity,
            "interface identity",
        )?;
        let arguments = conformance
            .interface
            .args
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                self.owner_stable_conformance_type_ref(
                    receiver.module_path(),
                    argument,
                    &format!("interface type argument {index}"),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let interface = interface_instantiation_ref(identity, arguments);

        SourceLocalInterfaceConformance::try_new(
            conformance.receiver_type_params.clone(),
            receiver,
            receiver_type,
            interface,
            implementations,
        )
        .map_err(|source| Error::InvalidEntry { index, source })
    }

    fn owner_stable_conformance_type_ref(
        &self,
        owner_module: &str,
        ty: &TypeRefIr,
        location: &str,
    ) -> Result<TypeRefIr, Error> {
        let recurse = |ty: &TypeRefIr, child: &str| {
            self.owner_stable_conformance_type_ref(owner_module, ty, child)
        };
        match ty {
            TypeRefIr::Builtin { name, args } => Ok(TypeRefIr::Builtin {
                name: name.clone(),
                args: args
                    .iter()
                    .enumerate()
                    .map(|(index, argument)| {
                        recurse(argument, &format!("{location} builtin argument {index}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            TypeRefIr::LocalType { type_index } => {
                let symbol = self
                    .local_type_name_for_index(owner_module, *type_index)
                    .ok_or_else(|| Error::UnresolvedLocalType {
                        location: location.to_string(),
                        type_index: *type_index,
                    })?;
                Ok(TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: owner_module.to_string(),
                        symbol: symbol.to_string(),
                    },
                })
            }
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => {
                let symbol = self
                    .local_type_name_for_index(module_path, *type_index)
                    .ok_or_else(|| Error::UnresolvedPublicationType {
                        location: location.to_string(),
                        module_path: module_path.clone(),
                        type_index: *type_index,
                    })?;
                Ok(TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: module_path.clone(),
                        symbol: symbol.to_string(),
                    },
                })
            }
            TypeRefIr::ServiceSymbol { symbol } => Ok(TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: symbol
                        .module_path
                        .strip_prefix("root.")
                        .unwrap_or(&symbol.module_path)
                        .to_string(),
                    symbol: symbol.symbol.clone(),
                },
            }),
            TypeRefIr::PackageSymbol { symbol } => Ok(TypeRefIr::PackageSymbol {
                symbol: self.owner_stable_package_symbol(symbol, location)?,
            }),
            TypeRefIr::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => Ok(TypeRefIr::PackageSchema {
                package_id: package_id.clone(),
                stable_schema_key: stable_schema_key.clone(),
                package_schema_type_id: package_schema_type_id.clone(),
            }),
            TypeRefIr::AppliedNominal { base, arguments } => {
                let normalized_base = recurse(
                    &nominal_base_type_ref(base),
                    &format!("{location} nominal base"),
                )?;
                let base = nominal_base_from_type_ref(normalized_base).map_err(|_| {
                    Error::InvalidAppliedNominalBase {
                        location: location.to_string(),
                    }
                })?;
                Ok(TypeRefIr::AppliedNominal {
                    base,
                    arguments: arguments
                        .iter()
                        .enumerate()
                        .map(|(index, argument)| {
                            recurse(argument, &format!("{location} nominal argument {index}"))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                })
            }
            TypeRefIr::DbObjectSymbol { symbol } => Ok(TypeRefIr::DbObjectSymbol {
                symbol: symbol.clone(),
            }),
            TypeRefIr::Record { fields } => Ok(TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| {
                        Ok((
                            name.clone(),
                            recurse(ty, &format!("{location} record field {name}"))?,
                        ))
                    })
                    .collect::<Result<BTreeMap<_, _>, Error>>()?,
            }),
            TypeRefIr::Union { items } => Ok(TypeRefIr::Union {
                items: items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| recurse(item, &format!("{location} union item {index}")))
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            TypeRefIr::Nullable { inner } => Ok(TypeRefIr::Nullable {
                inner: Box::new(recurse(inner, &format!("{location} nullable inner"))?),
            }),
            TypeRefIr::Literal { value } => Ok(TypeRefIr::Literal {
                value: value.clone(),
            }),
            TypeRefIr::TypeParam { name } => Ok(TypeRefIr::TypeParam { name: name.clone() }),
            TypeRefIr::AnyInterface { interface } => {
                let identity = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
                    .map_err(|source| Error::InvalidNestedInterfaceIdentity {
                        location: location.to_string(),
                        message: source.to_string(),
                    })?;
                let identity =
                    recurse(&identity, &format!("{location} nested interface identity"))?;
                let arguments = interface
                    .canonical_type_args
                    .iter()
                    .enumerate()
                    .map(|(index, argument)| {
                        recurse(
                            argument,
                            &format!("{location} nested interface argument {index}"),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(TypeRefIr::AnyInterface {
                    interface: interface_instantiation_ref(identity, arguments),
                })
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => Ok(TypeRefIr::Function {
                params: params
                    .iter()
                    .enumerate()
                    .map(|(index, parameter)| {
                        Ok(FunctionTypeParamIr {
                            name: parameter.name.clone(),
                            ty: recurse(
                                &parameter.ty,
                                &format!("{location} function parameter {index}"),
                            )?,
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()?,
                return_type: Box::new(recurse(
                    return_type,
                    &format!("{location} function return"),
                )?),
            }),
        }
    }

    fn owner_stable_package_symbol(
        &self,
        symbol: &PackageSymbolRef,
        location: &str,
    ) -> Result<PackageSymbolRef, Error> {
        let (package_id, selected_abi) = match &symbol.package {
            PackageRefIr::Dependency { dependency_ref } => {
                let canonical_ref = self.canonical_package_dependency_ref(dependency_ref);
                let package_id = self
                    .package_dependencies
                    .get(canonical_ref)
                    .or_else(|| self.package_dependencies.get(dependency_ref))
                    .cloned()
                    .ok_or_else(|| Error::MissingPackageOwner {
                        location: location.to_string(),
                        symbol_path: symbol.symbol_path.clone(),
                    })?;
                let (abi, _) = self
                    .package_artifact_identities
                    .get(canonical_ref)
                    .or_else(|| self.package_artifact_identities.get(dependency_ref))
                    .ok_or_else(|| Error::MissingPackageOwner {
                        location: location.to_string(),
                        symbol_path: symbol.symbol_path.clone(),
                    })?;
                (package_id, abi.as_str().to_string())
            }
            PackageRefIr::PackageId { package_id } => {
                let abi_identities = self
                    .package_dependencies
                    .iter()
                    .filter(|(_, selected_package_id)| *selected_package_id == package_id)
                    .filter_map(|(dependency_ref, _)| {
                        self.package_artifact_identities
                            .get(dependency_ref)
                            .map(|(abi, _)| abi.as_str().to_string())
                    })
                    .collect::<BTreeSet<_>>();
                match abi_identities.len() {
                    0 => {
                        return Err(Error::MissingPackageOwner {
                            location: location.to_string(),
                            symbol_path: symbol.symbol_path.clone(),
                        });
                    }
                    1 => (
                        package_id.clone(),
                        abi_identities
                            .into_iter()
                            .next()
                            .expect("one ABI identity was selected"),
                    ),
                    _ => {
                        return Err(Error::AmbiguousPackageOwner {
                            location: location.to_string(),
                            symbol_path: symbol.symbol_path.clone(),
                            abi_identities: abi_identities.into_iter().collect(),
                        });
                    }
                }
            }
        };
        if let Some(actual) = symbol.abi_expectation.as_deref() {
            if actual != selected_abi {
                return Err(Error::PackageAbiMismatch {
                    location: location.to_string(),
                    symbol_path: symbol.symbol_path.clone(),
                    expected: selected_abi,
                    actual: actual.to_string(),
                });
            }
        }
        Ok(PackageSymbolRef {
            package: PackageRefIr::PackageId { package_id },
            symbol_path: symbol.symbol_path.clone(),
            abi_expectation: Some(selected_abi),
        })
    }
}
