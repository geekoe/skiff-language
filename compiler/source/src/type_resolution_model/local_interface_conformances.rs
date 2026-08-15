use super::*;
use crate::{
    SourceLocalInterfaceConformance, SourceLocalInterfaceConformanceFacts,
    SourceLocalInterfaceConformanceFactsError as Error,
};
use skiff_compiler_core::type_ref::package_type_ref_to_ir_exact;

impl TypeResolutionModel {
    pub(crate) fn public_instance_receiver_instantiation(
        &self,
        resolved: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<(SourceSymbolKey, Vec<TypeRefIr>)>, Error> {
        let Some(receiver) = self.actual_receiver_symbol(resolved, context) else {
            return Ok(None);
        };
        let arguments = match &resolved.ir {
            TypeRefIr::AppliedNominal { arguments, .. } => arguments.clone(),
            _ => Vec::new(),
        }
        .into_iter()
        .enumerate()
        .map(|(index, argument)| {
            self.owner_stable_conformance_type_ref(
                context.module_path,
                &argument,
                &format!("public-instance receiver type argument {index}"),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
        Ok(Some((receiver, arguments)))
    }

    pub(crate) fn public_instance_interface_selector_identities(
        &self,
        selector: &SourceSymbolKey,
    ) -> Result<Vec<TypeRefIr>, String> {
        let mut identities = Vec::new();
        if matches!(
            self.interface_semantics.interface_owner_kind(selector),
            Some(InterfaceOwnerKind::Source | InterfaceOwnerKind::CompilerKnown)
        ) {
            identities.push(interface_symbol_type_ref(selector));
        }

        let selector_text = selector.to_source_symbol();
        if let Some(interface) = self.resolve_package_interface(&selector_text) {
            if !identities.contains(&interface.identity) {
                identities.push(interface.identity);
            }
        }
        if let Some((alias, schema_type)) = self.service_api_type(&selector_text)? {
            if let Some(interface) =
                self.service_api_interface(alias, &schema_type.stable_schema_key)
            {
                if !identities.contains(&interface.identity) {
                    identities.push(interface.identity);
                }
            }
        }
        Ok(identities)
    }

    pub(crate) fn public_instance_interface_method_slots(
        &self,
        receiver: &SourceSymbolKey,
        receiver_type_parameters: &[String],
        receiver_arguments: &[TypeRefIr],
        interface: &InterfaceInstantiationRef,
    ) -> Result<Vec<InterfaceMethodSlotFact>, String> {
        let identity = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
            .map_err(|error| format!("interface ABI id is not a TypeRefIr: {error}"))?;
        if let TypeRefIr::ServiceSymbol { symbol } = identity {
            let conformance = crate::semantic::interface::InterfaceConformanceFact {
                receiver_type_params: receiver_type_parameters.to_vec(),
                receiver: TypeInstantiationPattern {
                    symbol: receiver.clone(),
                    args: receiver_arguments.to_vec(),
                },
                interface: InterfaceInstantiation {
                    symbol: SourceSymbolKey::new(
                        symbol
                            .module_path
                            .strip_prefix("root.")
                            .unwrap_or(&symbol.module_path),
                        symbol.symbol,
                    ),
                    args: interface.canonical_type_args.clone(),
                },
            };
            return self
                .interface_semantics
                .method_slots_for_local_conformance(&conformance)
                .map_err(|error| error.to_string());
        }
        self.interface_method_slots_for_instantiation(interface)
    }

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

    pub(super) fn owner_stable_conformance_type_ref(
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
            TypeRefIr::ServiceSymbol { symbol } => {
                let normalized = ServiceSymbolRef {
                    module_path: symbol
                        .module_path
                        .strip_prefix("root.")
                        .unwrap_or(&symbol.module_path)
                        .to_string(),
                    symbol: symbol.symbol.clone(),
                };
                let qualified_name = source_path(&normalized.module_path, &normalized.symbol);
                let schema = match self.service_api_type(&qualified_name) {
                    Ok(Some((_, schema))) => schema,
                    Ok(None) => return Ok(TypeRefIr::ServiceSymbol { symbol: normalized }),
                    Err(message) => {
                        return Err(Error::ServiceSchemaAuthorityLookup {
                            location: location.to_string(),
                            module_path: normalized.module_path,
                            symbol: normalized.symbol,
                            message,
                        });
                    }
                };
                Ok(package_type_ref_to_ir_exact(
                    &PackageTypeRef::PackageSchema {
                        package_id: schema.package_id.clone(),
                        stable_schema_key: schema.stable_schema_key.clone(),
                        package_schema_type_id: schema.package_schema_type_id.clone(),
                    },
                ))
            }
            TypeRefIr::PackageSymbol { symbol } => {
                if let Some(schema) = self.service_schema_for_package_symbol(symbol, location)? {
                    if matches!(
                        schema.canonical_descriptor.descriptor,
                        skiff_artifact_model::ContractTypeDescriptor::CallbackInterface { .. }
                    ) {
                        return Ok(TypeRefIr::PackageSymbol {
                            symbol: symbol.clone(),
                        });
                    }
                    return Ok(package_type_ref_to_ir_exact(
                        &PackageTypeRef::PackageSchema {
                            package_id: schema.package_id.clone(),
                            stable_schema_key: schema.stable_schema_key.clone(),
                            package_schema_type_id: schema.package_schema_type_id.clone(),
                        },
                    ));
                }
                Ok(TypeRefIr::PackageSymbol {
                    symbol: self.owner_stable_package_symbol(symbol, location)?,
                })
            }
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

    fn service_schema_for_package_symbol(
        &self,
        symbol: &PackageSymbolRef,
        location: &str,
    ) -> Result<Option<&PackageSchemaTypeRecord>, Error> {
        let PackageRefIr::PackageId { package_id } = &symbol.package else {
            return Ok(None);
        };
        if symbol.abi_expectation.is_some() {
            return Ok(None);
        }
        let candidates = self
            .service_api_schemas
            .values()
            .filter_map(|records| records.get(&symbol.symbol_path))
            .filter(|record| &record.package_id == package_id)
            .collect::<Vec<_>>();
        let type_ids = candidates
            .iter()
            .map(|record| record.package_schema_type_id.as_str().to_string())
            .collect::<BTreeSet<_>>();
        if type_ids.len() > 1 {
            return Err(Error::AmbiguousServiceSchemaAuthority {
                location: location.to_string(),
                package_id: package_id.clone(),
                stable_schema_key: symbol.symbol_path.clone(),
                package_schema_type_ids: type_ids.into_iter().collect(),
            });
        }
        Ok(candidates.into_iter().next())
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

#[cfg(test)]
mod tests;
