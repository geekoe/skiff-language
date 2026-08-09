use super::*;

impl TypeResolutionModel {
    pub fn package_dependency_abi_expectations(&self) -> BTreeMap<String, String> {
        self.package_artifact_identities
            .iter()
            .filter_map(|(dependency_ref, (abi, _))| {
                (self.canonical_package_dependency_ref(dependency_ref) == dependency_ref)
                    .then(|| (dependency_ref.clone(), abi.as_str().to_string()))
            })
            .collect()
    }

    /// Returns the exact artifact ABI selected for each package id in this
    /// compilation. File IR occasionally carries a canonical package id rather
    /// than a source dependency alias (notably package-owned interface
    /// identities projected from schema types), but those executable refs still
    /// need the same exact ABI fence as ordinary dependency refs.

    pub fn package_dependency_abi_expectations_by_package_id(&self) -> BTreeMap<String, String> {
        self.package_dependencies
            .iter()
            .filter_map(|(dependency_ref, package_id)| {
                let canonical_ref = self.canonical_package_dependency_ref(dependency_ref);
                (canonical_ref == dependency_ref)
                    .then(|| {
                        self.package_artifact_identities
                            .get(canonical_ref)
                            .map(|(abi, _)| (package_id.clone(), abi.as_str().to_string()))
                    })
                    .flatten()
            })
            .collect()
    }

    /// Adds the published service APIs to the same external nominal-type model
    /// used by ordinary package dependencies. Service operation lowering keeps
    /// its own call target; only public type shapes are shared here.

    pub fn source_interface_conformance(
        &self,
        receiver: &SourceSymbolKey,
        interface_symbol: &ServiceSymbolRef,
    ) -> Option<SourceInterfaceConformanceFact<'_>> {
        self.source_interface_conformance_matching(receiver, |interface_identity| {
            interface_identity_matches_source_symbol(interface_identity, interface_symbol)
        })
    }

    pub fn source_interface_conformance_matching(
        &self,
        receiver: &SourceSymbolKey,
        matches_interface: impl Fn(&TypeRefIr) -> bool,
    ) -> Option<SourceInterfaceConformanceFact<'_>> {
        self.interface_conformances
            .iter()
            .find(|conformance| {
                &conformance.receiver == receiver
                    && matches_interface(&conformance.interface.identity)
            })
            .map(|conformance| SourceInterfaceConformanceFact {
                interface_args: &conformance.interface.args,
            })
    }

    pub fn resolve_type_ref(
        &self,
        ty: &TypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<ResolvedTypeRef, String> {
        self.resolve_type_text(&ty.name, context)
    }

    pub fn resolve_named_type_ref(
        &self,
        name: &str,
        arguments: &[TypeRef],
        context: &TypeResolutionContext<'_>,
    ) -> Result<ResolvedTypeRef, String> {
        let argument_exprs = arguments
            .iter()
            .map(|argument| TypeExpr::parse(&argument.name))
            .collect::<Vec<_>>();
        let ir = self.resolve_named_type(name, &argument_exprs, context)?;
        let ir = self.expand_alias_type_ref(&ir, context)?;
        let source_text = if arguments.is_empty() {
            name.to_string()
        } else {
            format!(
                "{name}<{}>",
                arguments
                    .iter()
                    .map(|argument| argument.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        Ok(ResolvedTypeRef::with_text(ir, source_text))
    }

    pub fn resolve_type_text(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<ResolvedTypeRef, String> {
        let expr = TypeExpr::parse(raw);
        self.reject_any_interface_selector_aliases(&expr, context)?;
        let source_text = self.expand_alias_text(raw, context)?;
        let ir = self.resolve_type_expr(&expr, context)?;
        let ir = self.expand_alias_type_ref(&ir, context)?;
        Ok(ResolvedTypeRef::with_text(ir, source_text))
    }

    /// Produces the exact semantic type represented by `ty`, recursively
    /// replacing every source or package alias with its RHS. Nominal
    /// declarations (records, representations, actors, interfaces, and named
    /// unions) remain named.

    pub fn expand_alias_type_ref_for_module(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
    ) -> Result<TypeRefIr, String> {
        self.expand_alias_type_ref(ty, &TypeResolutionContext::source(module_path))
    }

    pub fn expand_alias_type_ref(
        &self,
        ty: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<TypeRefIr, String> {
        self.expand_alias_type_ref_inner(ty, context, &mut BTreeSet::new())
            .map(normalize_union)
    }

    pub(super) fn expand_alias_type_ref_inner(
        &self,
        ty: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
        visiting: &mut BTreeSet<AliasTypeVisitKey>,
    ) -> Result<TypeRefIr, String> {
        match ty {
            TypeRefIr::Builtin { name, args } => Ok(TypeRefIr::Builtin {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| self.expand_alias_type_ref_inner(arg, context, visiting))
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            TypeRefIr::LocalType { type_index } => {
                let Some(resolution) = self.local_type_resolution(context.module_path, *type_index)
                else {
                    return Err(format!(
                        "alias expansion cannot resolve local type index {type_index} in {}",
                        context.module_path
                    ));
                };
                if !matches!(resolution.kind, SourceTypeKind::Alias { .. }) {
                    return Ok(ty.clone());
                }
                self.expand_source_alias_resolution(resolution, context, visiting)
            }
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => {
                let Some(resolution) = self.local_type_resolution(module_path, *type_index) else {
                    return Err(format!(
                        "alias expansion cannot resolve publication type index {type_index} in {module_path}"
                    ));
                };
                if !matches!(resolution.kind, SourceTypeKind::Alias { .. }) {
                    return Ok(ty.clone());
                }
                self.expand_source_alias_resolution(resolution, context, visiting)
            }
            TypeRefIr::ServiceSymbol { symbol } => {
                let module_path = symbol
                    .module_path
                    .strip_prefix("root.")
                    .unwrap_or(&symbol.module_path);
                let key = SourceSymbolKey::new(module_path, &symbol.symbol);
                let Some(resolution) = self.source_types.get(&key) else {
                    return Ok(ty.clone());
                };
                if !matches!(resolution.kind, SourceTypeKind::Alias { .. }) {
                    return Ok(ty.clone());
                }
                self.expand_source_alias_resolution(resolution, context, visiting)
            }
            TypeRefIr::PackageSymbol { symbol } => {
                let dependency_ref = match &symbol.package {
                    PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
                    PackageRefIr::PackageId { package_id } => package_id.as_str(),
                };
                let canonical_package_id = self
                    .package_dependencies
                    .get(dependency_ref)
                    .map(String::as_str)
                    .unwrap_or(dependency_ref);
                let canonical_symbol = ty.clone();
                let Some(resolution) =
                    self.package_type_resolution(dependency_ref, &symbol.symbol_path)
                else {
                    return Ok(canonical_symbol);
                };
                let SourceTypeKind::Alias {
                    canonical_target, ..
                } = &resolution.kind
                else {
                    return Ok(canonical_symbol);
                };
                let public_path = resolution
                    .public_path
                    .as_deref()
                    .unwrap_or(&symbol.symbol_path);
                let visit_key = AliasTypeVisitKey::Package(PackageSymbolKey {
                    dependency_ref: canonical_package_id.to_string(),
                    symbol_path: public_path.to_string(),
                });
                if !visiting.insert(visit_key.clone()) {
                    return Err(format!(
                        "alias cycle detected while expanding package type {canonical_package_id}/{public_path}"
                    ));
                }
                let result = match canonical_target {
                    Some(target) => self.expand_alias_type_ref_inner(target, context, visiting),
                    None => Err(format!(
                        "package alias {canonical_package_id}/{public_path} has no exact RHS type"
                    )),
                };
                visiting.remove(&visit_key);
                result
            }
            TypeRefIr::AppliedNominal { base, arguments } => {
                let arguments = arguments
                    .iter()
                    .map(|argument| self.expand_alias_type_ref_inner(argument, context, visiting))
                    .collect::<Result<Vec<_>, _>>()?;
                let base_type = nominal_base_type_ref(base);
                let Some(named) = self.resolved_named_type(&base_type, context) else {
                    return Ok(TypeRefIr::AppliedNominal {
                        base: base.clone(),
                        arguments,
                    });
                };
                let SourceTypeKind::Alias {
                    target,
                    canonical_target,
                } = &named.resolution.kind
                else {
                    return Ok(TypeRefIr::AppliedNominal {
                        base: base.clone(),
                        arguments,
                    });
                };
                if named.resolution.type_params.len() != arguments.len() {
                    return Err(format!(
                        "alias {}.{} expects {} type arguments, found {}",
                        named.resolution.module_path,
                        named.resolution.name,
                        named.resolution.type_params.len(),
                        arguments.len()
                    ));
                }
                let visit_key = match &named.visit_key {
                    InterfaceTypeVisitKey::Source(key) => AliasTypeVisitKey::Source(key.clone()),
                    InterfaceTypeVisitKey::Package(key) => AliasTypeVisitKey::Package(key.clone()),
                };
                if !visiting.insert(visit_key.clone()) {
                    return Err(format!(
                        "alias cycle detected while expanding {}.{}",
                        named.resolution.module_path, named.resolution.name
                    ));
                }
                let substitutions = named
                    .resolution
                    .type_params
                    .iter()
                    .cloned()
                    .zip(arguments)
                    .collect::<BTreeMap<_, _>>();
                let target = if let Some(target) = canonical_target {
                    target.clone()
                } else {
                    let alias_context = TypeResolutionContext::with_type_params(
                        &named.resolution.module_path,
                        named.resolution.type_params.iter().cloned().collect(),
                    );
                    let target =
                        self.resolve_type_expr(&TypeExpr::parse(target), &alias_context)?;
                    if named.resolution.module_path == context.module_path {
                        target
                    } else {
                        self.externalize_local_type_ir(&target, &named.resolution.module_path)
                    }
                };
                let target = substitute_type_params_in_type_ref_ref(&target, &substitutions);
                let result = self.expand_alias_type_ref_inner(&target, context, visiting);
                visiting.remove(&visit_key);
                result
            }
            TypeRefIr::PackageSchema { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. } => Ok(ty.clone()),
            TypeRefIr::Record { fields } => Ok(TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, field)| {
                        Ok((
                            name.clone(),
                            self.expand_alias_type_ref_inner(field, context, visiting)?,
                        ))
                    })
                    .collect::<Result<BTreeMap<_, _>, String>>()?,
            }),
            TypeRefIr::Union { items } => Ok(normalize_union(TypeRefIr::Union {
                items: items
                    .iter()
                    .map(|item| self.expand_alias_type_ref_inner(item, context, visiting))
                    .collect::<Result<Vec<_>, _>>()?,
            })),
            TypeRefIr::Nullable { inner } => Ok(TypeRefIr::Nullable {
                inner: Box::new(self.expand_alias_type_ref_inner(inner, context, visiting)?),
            }),
            TypeRefIr::AnyInterface { interface } => {
                let identity: TypeRefIr = serde_json::from_str(&interface.interface_abi_id)
                    .map_err(|error| {
                        format!(
                            "alias expansion found invalid interface ABI identity {}: {error}",
                            interface.interface_abi_id
                        )
                    })?;
                let identity = self.expand_alias_type_ref_inner(&identity, context, visiting)?;
                let args = interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| self.expand_alias_type_ref_inner(arg, context, visiting))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(TypeRefIr::AnyInterface {
                    interface: interface_instantiation_ref(identity, args),
                })
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => Ok(TypeRefIr::Function {
                params: params
                    .iter()
                    .map(|param| {
                        Ok(FunctionTypeParamIr {
                            name: param.name.clone(),
                            ty: self.expand_alias_type_ref_inner(&param.ty, context, visiting)?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?,
                return_type: Box::new(self.expand_alias_type_ref_inner(
                    return_type,
                    context,
                    visiting,
                )?),
            }),
        }
    }

    pub(super) fn expand_source_alias_resolution(
        &self,
        resolution: &SourceTypeResolution,
        caller_context: &TypeResolutionContext<'_>,
        visiting: &mut BTreeSet<AliasTypeVisitKey>,
    ) -> Result<TypeRefIr, String> {
        let SourceTypeKind::Alias {
            target,
            canonical_target,
        } = &resolution.kind
        else {
            return Err(format!(
                "internal alias expansion requested for non-alias {}.{}",
                resolution.module_path, resolution.name
            ));
        };
        let visit_key = AliasTypeVisitKey::Source(SourceSymbolKey::new(
            &resolution.module_path,
            &resolution.name,
        ));
        if !visiting.insert(visit_key.clone()) {
            return Err(format!(
                "alias cycle detected while expanding {}.{}",
                resolution.module_path, resolution.name
            ));
        }
        let result = if let Some(target) = canonical_target {
            self.expand_alias_type_ref_inner(target, caller_context, visiting)
        } else {
            let alias_context = TypeResolutionContext::with_type_params(
                &resolution.module_path,
                caller_context.type_params.clone(),
            );
            let target_ir = self.resolve_type_expr(&TypeExpr::parse(target), &alias_context)?;
            let expanded =
                self.expand_alias_type_ref_inner(&target_ir, &alias_context, visiting)?;
            Ok(if resolution.module_path == caller_context.module_path {
                expanded
            } else {
                self.externalize_local_type_ir(&expanded, &resolution.module_path)
            })
        };
        visiting.remove(&visit_key);
        result
    }

    pub fn resolve_any_interface_type_ref(
        &self,
        interface: &TypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<ResolvedTypeRef, String> {
        let selector = self.resolve_object_safe_interface_selector_type_ref(interface, context)?;
        Ok(ResolvedTypeRef::with_text(
            TypeRefIr::AnyInterface {
                interface: selector.instantiation_ref,
            },
            format!("any {}", selector.source_text),
        ))
    }

    /// Resolves an exact interface declaration selector for nominal
    /// conformance. Marker interfaces are valid here; this does not grant
    /// dynamic `AnyInterface` object safety.
    pub fn resolve_canonical_interface_selector_type_ref(
        &self,
        interface: &TypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<CanonicalInterfaceSelectorResolution, String> {
        let expr = TypeExpr::parse(&interface.name);
        self.resolve_canonical_interface_selector_expr(&expr, context)
    }

    /// Resolves an exact interface selector for dynamic boxing or invocation.
    /// Unlike nominal conformance selection, this enforces object safety.
    pub fn resolve_object_safe_interface_selector_type_ref(
        &self,
        interface: &TypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<CanonicalInterfaceSelectorResolution, String> {
        let selector = self.resolve_canonical_interface_selector_type_ref(interface, context)?;
        self.require_canonical_interface_selector_object_safe(&selector)?;
        Ok(selector)
    }

    pub fn resolve_canonical_interface_selector_resolved_type_ref(
        &self,
        resolved: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<CanonicalInterfaceSelectorResolution, String> {
        let Some(interface) = self.interface_instantiation_from_resolved(resolved, context)? else {
            return Err(format!(
                "resolved type `{}` is not an interface instantiation",
                resolved
            ));
        };
        self.canonical_interface_selector_from_instantiation_resolution(
            resolved.to_string(),
            interface,
        )
    }

    pub fn concrete_nominal_record_symbol(
        &self,
        actual: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Option<SourceSymbolKey> {
        self.actual_receiver_symbol(actual, context)
    }

    pub fn any_interface_method_signature(
        &self,
        receiver: &TypeRefIr,
        method_name: &str,
    ) -> Option<AnyInterfaceMethodResolution> {
        let TypeRefIr::AnyInterface { interface } = receiver else {
            return None;
        };
        let identity: TypeRefIr = serde_json::from_str(&interface.interface_abi_id).ok()?;
        match identity {
            TypeRefIr::ServiceSymbol { symbol } => {
                if let Some(service_interface) =
                    self.service_api_interface(&symbol.module_path, &symbol.symbol)
                {
                    let service_interface = service_interface
                        .instantiate_methods(&interface.canonical_type_args)
                        .ok()?;
                    let (slot, operation) = service_interface
                        .methods
                        .into_iter()
                        .enumerate()
                        .find(|(_, operation)| operation.name == method_name)?;
                    return Some(AnyInterfaceMethodResolution {
                        interface: interface.clone(),
                        slot: slot as u32,
                        method_abi_id: canonical_interface_method_abi_id(
                            interface,
                            &operation.name,
                        ),
                        params: interface_method_signature_params(&operation),
                        return_type: operation.return_type,
                    });
                }
                let key = SourceSymbolKey::new(
                    symbol
                        .module_path
                        .strip_prefix("root.")
                        .unwrap_or(&symbol.module_path),
                    &symbol.symbol,
                );
                if !self.source_type_is_interface(&key) {
                    return None;
                }
                let interface = InterfaceInstantiation {
                    symbol: key,
                    args: interface.canonical_type_args.clone(),
                };
                let canonical = self
                    .interface_semantics
                    .canonical_interface_instantiation_ref(&interface);
                self.interface_semantics
                    .method_slots_for_interface(&interface)
                    .ok()?
                    .into_iter()
                    .find(|slot| slot.name == method_name)
                    .map(|slot| method_slot_resolution(canonical, slot))
            }
            TypeRefIr::PackageSymbol { symbol } => {
                let package_interface = self
                    .package_interface_for_type_ref(&TypeRefIr::PackageSymbol { symbol })?
                    .instantiate_methods(&interface.canonical_type_args)
                    .ok()?;
                let (slot, operation) = package_interface
                    .methods
                    .into_iter()
                    .enumerate()
                    .find(|(_, operation)| operation.name == method_name)?;
                Some(AnyInterfaceMethodResolution {
                    interface: interface.clone(),
                    slot: slot as u32,
                    method_abi_id: canonical_interface_method_abi_id(interface, &operation.name),
                    params: interface_method_signature_params(&operation),
                    return_type: operation.return_type,
                })
            }
            _ => None,
        }
    }

    pub fn interface_method_slots_for_instantiation(
        &self,
        interface: &InterfaceInstantiationRef,
    ) -> Result<Vec<InterfaceMethodSlotFact>, String> {
        let identity: TypeRefIr = serde_json::from_str(&interface.interface_abi_id)
            .map_err(|error| format!("interface ABI id is not a TypeRefIr: {error}"))?;
        match identity {
            TypeRefIr::ServiceSymbol { symbol } => {
                let key = SourceSymbolKey::new(
                    symbol
                        .module_path
                        .strip_prefix("root.")
                        .unwrap_or(&symbol.module_path),
                    &symbol.symbol,
                );
                if !self.source_type_is_interface(&key) {
                    return Err(format!("{key} is not a source interface"));
                }
                let instantiation = InterfaceInstantiation {
                    symbol: key,
                    args: interface.canonical_type_args.clone(),
                };
                self.interface_semantics
                    .method_slots_for_interface(&instantiation)
                    .map_err(|error| error.to_string())
            }
            TypeRefIr::PackageSymbol { symbol } => {
                let package_interface = self
                    .package_interface_for_type_ref(&TypeRefIr::PackageSymbol { symbol })
                    .ok_or_else(|| {
                        "interface ABI id does not resolve to a package interface".to_string()
                    })?
                    .instantiate_methods(&interface.canonical_type_args)?;
                Ok(package_interface
                    .methods
                    .into_iter()
                    .enumerate()
                    .map(|(slot, method)| InterfaceMethodSlotFact {
                        slot: slot as u32,
                        name: method.name.clone(),
                        method_abi_id: canonical_interface_method_abi_id(interface, &method.name),
                        params: interface_method_signature_params(&method),
                        return_type: method.return_type,
                    })
                    .collect())
            }
            other => Err(format!(
                "interface ABI id resolves to non-interface type {}",
                debug_text(&other)
            )),
        }
    }

    pub fn resolve_constructor_target(
        &self,
        type_name: &str,
        type_args: &[TypeRef],
        context: &TypeResolutionContext<'_>,
    ) -> Result<ConstructorTargetResolution, String> {
        let target_text = type_text_with_args(type_name, type_args);
        let target = self.resolve_type_text(&target_text, context)?;
        self.resolve_constructor_target_resolved(&target, context)
    }

    pub fn actor_type_resolution(
        &self,
        ty: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Option<ActorTypeResolution> {
        if let Some(resolution) = self.package_actor_type_resolution(ty) {
            return Some(resolution);
        }
        let key = self.actual_receiver_symbol(ty, context)?;
        let resolution = self.source_types.get(&key)?;
        let SourceTypeKind::Actor {
            key_field,
            fields,
            create,
            ..
        } = &resolution.kind
        else {
            return None;
        };
        let declaration_context = TypeResolutionContext::source(&resolution.module_path);
        let resolve_field = |name: &str, ty: &str| {
            let resolved = self.resolve_type_text(ty, &declaration_context).ok()?;
            let resolved = if resolution.module_path == context.module_path {
                resolved
            } else {
                self.externalize_local_type_refs(&resolved, &resolution.module_path)
            };
            Some((name.to_string(), resolved))
        };
        let key_type = fields.get(key_field)?;
        let id_type = resolve_field(key_field, key_type).map(|(_, ty)| ty)?;
        let fields = fields
            .iter()
            .map(|(name, ty)| resolve_field(name, ty))
            .collect::<Option<BTreeMap<_, _>>>()?;
        let create = match create.as_ref() {
            Some(params) => Some(
                params
                    .iter()
                    .map(|(name, ty)| resolve_field(name, ty))
                    .collect::<Option<Vec<_>>>()?,
            ),
            None => None,
        };
        Some(ActorTypeResolution {
            ty: ty.clone(),
            name: resolution.name.clone(),
            module_path: resolution.module_path.clone(),
            id_type,
            key_field: key_field.clone(),
            fields,
            create,
        })
    }

    /// Resolves an actor declared by a package dependency through either its
    /// public surface or its `topLevelAlias` implementation view. The exact
    /// normalized artifact type references are recovered directly instead of
    /// re-resolving dependency module text, and the returned owner path is the
    /// provider's internal source path so lowering can pin the actor
    /// declaration through a `ServiceSymbol`.
    fn package_actor_type_resolution(&self, ty: &ResolvedTypeRef) -> Option<ActorTypeResolution> {
        let symbol = match &ty.ir {
            TypeRefIr::PackageSymbol { symbol } => symbol,
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::PackageSymbol { symbol },
                arguments,
            } if arguments.is_empty() => symbol,
            _ => return None,
        };
        let dependency_ref = match &symbol.package {
            PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
            PackageRefIr::PackageId { package_id } => package_id.as_str(),
        };
        let resolution = self
            .package_type_resolution_for_view(dependency_ref, &symbol.symbol_path)
            .or_else(|| self.package_type_resolution(dependency_ref, &symbol.symbol_path))?;
        let SourceTypeKind::Actor {
            key_field,
            fields,
            create,
            canonical_id_type,
            canonical_fields,
            canonical_create,
        } = &resolution.kind
        else {
            return None;
        };
        let canonical_id_type = canonical_id_type.as_ref()?;
        let canonical_fields = canonical_fields.as_ref()?;
        if create.is_some() != canonical_create.is_some()
            || fields.len() != canonical_fields.len()
            || !canonical_fields.contains_key(key_field)
        {
            return None;
        }
        let source_symbol_path =
            self.package_receiver_source_symbol_path(dependency_ref, &symbol.symbol_path);
        let Some((module_path, name)) = source_symbol_path.rsplit_once('.') else {
            return None;
        };
        if module_path.is_empty() || name.is_empty() {
            return None;
        }
        let fields = fields
            .iter()
            .filter_map(|(field_name, _)| {
                canonical_fields
                    .get(field_name)
                    .map(|ty| (field_name.clone(), ResolvedTypeRef::new(ty.clone())))
            })
            .collect::<BTreeMap<_, _>>();
        let create = canonical_create.as_ref().map(|params| {
            params
                .iter()
                .map(|(name, ty)| (name.clone(), ResolvedTypeRef::new(ty.clone())))
                .collect()
        });
        Some(ActorTypeResolution {
            ty: ty.clone(),
            name: name.to_string(),
            module_path: module_path.to_string(),
            id_type: ResolvedTypeRef::new(canonical_id_type.clone()),
            key_field: key_field.clone(),
            fields,
            create,
        })
    }

    pub fn actor_method_signature(
        &self,
        ty: &ResolvedTypeRef,
        method_name: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Option<(Vec<FunctionTypeParamIr>, TypeRefIr)> {
        let key = self.actual_receiver_symbol(ty, context)?;
        if !matches!(
            self.source_types.get(&key)?.kind,
            SourceTypeKind::Actor { .. }
        ) {
            return None;
        }
        let method = self.local_impl_methods.get(&key)?.get(method_name)?;
        Some((method.params.clone(), method.return_type.clone()))
    }

    pub fn actor_state_field_type(
        &self,
        ty: &ResolvedTypeRef,
        field: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Option<ResolvedTypeRef> {
        self.actor_type_resolution(ty, context)?
            .fields
            .remove(field)
    }

    pub fn resolve_constructor_target_text(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<ConstructorTargetResolution, String> {
        let expr = TypeExpr::parse(raw);
        let TypeExpr::Named { name, args } = expr else {
            return Err(format!("constructor target `{raw}` is not a named type"));
        };
        let type_args = args
            .iter()
            .map(|arg| TypeRef {
                name: arg.to_type_string(),
            })
            .collect::<Vec<_>>();
        self.resolve_constructor_target(&name, &type_args, context)
    }

    pub fn resolve_constructor_target_resolved(
        &self,
        target: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<ConstructorTargetResolution, String> {
        let (base, arguments) = match &target.ir {
            TypeRefIr::AppliedNominal { base, arguments } => {
                (nominal_base_type_ref(base), arguments.as_slice())
            }
            other => (other.clone(), &[][..]),
        };

        if let TypeRefIr::PackageSymbol { symbol } = &base {
            if let PackageRefIr::PackageId { package_id } = &symbol.package {
                if let Some((alias, schema_type)) =
                    self.service_api_schemas
                        .iter()
                        .find_map(|(alias, records)| {
                            records
                                .get(&symbol.symbol_path)
                                .filter(|record| record.package_id == *package_id)
                                .map(|record| (alias.as_str(), record))
                        })
                {
                    let ContractTypeDescriptor::Record { fields } =
                        &schema_type.canonical_descriptor.descriptor
                    else {
                        return Err(format!(
                            "constructor target `{}` is not a nominal record",
                            target
                        ));
                    };
                    let type_params = &schema_type.canonical_descriptor.type_params;
                    if type_params.len() != arguments.len() {
                        return Err(format!(
                            "constructor `{}` expects {} type arguments, found {}",
                            target,
                            type_params.len(),
                            arguments.len()
                        ));
                    }
                    let substitutions = type_params
                        .iter()
                        .cloned()
                        .zip(arguments.iter().cloned())
                        .collect::<BTreeMap<_, _>>();
                    let fields = fields
                        .iter()
                        .map(|(name, field_ty)| {
                            let field_ty = contract_type_ref_ir(alias, field_ty)?;
                            let field_ty =
                                substitute_type_params_in_type_ref_ref(&field_ty, &substitutions);
                            Ok((name.clone(), ResolvedTypeRef::new(field_ty)))
                        })
                        .collect::<Result<_, String>>()?;
                    return Ok(ConstructorTargetResolution {
                        ty: target.clone(),
                        fields,
                        type_params: type_params.clone(),
                    });
                }
            }
        }

        let prelude_symbol = match &base {
            TypeRefIr::Builtin { name, .. } => Some(name.as_str()),
            TypeRefIr::PackageSymbol { symbol }
                if matches!(
                    &symbol.package,
                    PackageRefIr::PackageId { package_id }
                        if package_id == SKIFF_STD_PUBLICATION_ID
                ) =>
            {
                Some(symbol.symbol_path.as_str())
            }
            _ => None,
        };
        if let Some(shape) = prelude_symbol.and_then(prelude_constructor_shape) {
            return self.instantiate_constructor_shape(target, shape, arguments, context);
        }

        let named = self.resolved_named_type(&base, context).ok_or_else(|| {
            format!(
                "constructor target `{}` is not a resolved nominal type",
                target
            )
        })?;
        if named.resolution.type_params.len() != arguments.len() {
            return Err(format!(
                "constructor `{}` expects {} type arguments, found {}",
                target,
                named.resolution.type_params.len(),
                arguments.len()
            ));
        }
        let (fields, canonical_fields) = match &named.resolution.kind {
            SourceTypeKind::Record {
                fields,
                canonical_fields,
            } => (fields, canonical_fields),
            SourceTypeKind::Actor { .. } => {
                return Err(format!(
                    "actor `{}` is a nominal handle and cannot be constructed directly; use std.actor.get",
                    target
                ));
            }
            _ => {
                return Err(format!(
                    "constructor target `{}` is not a nominal record",
                    target
                ));
            }
        };
        let substitutions = named
            .resolution
            .type_params
            .iter()
            .cloned()
            .zip(arguments.iter().cloned())
            .collect::<BTreeMap<_, _>>();
        let declaration_context = TypeResolutionContext::with_type_params(
            &named.source_module_path,
            named.resolution.type_params.iter().cloned().collect(),
        );
        let fields = fields
            .iter()
            .map(|(name, field_text)| {
                let field_ty = if let Some(field_ty) = canonical_fields
                    .as_ref()
                    .and_then(|canonical| canonical.get(name))
                {
                    field_ty.clone()
                } else {
                    let qualified = named
                        .package_root
                        .as_deref()
                        .map(|package_root| {
                            qualify_package_type_text(
                                field_text,
                                package_root,
                                &named.resolution.local_type_names,
                            )
                        })
                        .unwrap_or_else(|| field_text.clone());
                    self.resolve_type_expr(&TypeExpr::parse(&qualified), &declaration_context)?
                };
                let field_ty = substitute_type_params_in_type_ref_ref(&field_ty, &substitutions);
                let field_ty = self.expand_alias_type_ref(&field_ty, &declaration_context)?;
                let field = ResolvedTypeRef::new(field_ty);
                Ok((
                    name.clone(),
                    if named.source_module_path == context.module_path {
                        field
                    } else {
                        self.externalize_local_type_refs(&field, &named.source_module_path)
                    },
                ))
            })
            .collect::<Result<_, String>>()?;
        Ok(ConstructorTargetResolution {
            ty: target.clone(),
            fields,
            type_params: named.resolution.type_params.clone(),
        })
    }

    pub(super) fn instantiate_constructor_shape(
        &self,
        target: &ResolvedTypeRef,
        shape: ConstructorShape,
        arguments: &[TypeRefIr],
        context: &TypeResolutionContext<'_>,
    ) -> Result<ConstructorTargetResolution, String> {
        if shape.type_params.len() != arguments.len() {
            return Err(format!(
                "constructor `{}` expects {} type arguments, found {}",
                target,
                shape.type_params.len(),
                arguments.len()
            ));
        }
        let substitutions = shape
            .type_params
            .iter()
            .cloned()
            .zip(arguments.iter().cloned())
            .collect::<BTreeMap<_, _>>();
        let declaration_context = TypeResolutionContext::with_type_params(
            &shape.module_path,
            shape.type_params.iter().cloned().collect(),
        );
        let fields = shape
            .fields
            .iter()
            .map(|(name, field_text)| {
                let field_ty = if let Some(field_ty) = shape
                    .canonical_fields
                    .as_ref()
                    .and_then(|canonical| canonical.get(name))
                {
                    field_ty.clone()
                } else {
                    self.resolve_type_expr(&TypeExpr::parse(field_text), &declaration_context)?
                };
                let field_ty = substitute_type_params_in_type_ref_ref(&field_ty, &substitutions);
                let field_ty = self.expand_alias_type_ref(&field_ty, &declaration_context)?;
                let field = ResolvedTypeRef::new(field_ty);
                Ok((
                    name.clone(),
                    if shape.module_path == context.module_path {
                        field
                    } else {
                        self.externalize_local_type_refs(&field, &shape.module_path)
                    },
                ))
            })
            .collect::<Result<_, String>>()?;
        Ok(ConstructorTargetResolution {
            ty: target.clone(),
            fields,
            type_params: shape.type_params,
        })
    }

    pub fn resolve_representation_constructor(
        &self,
        type_name: &str,
        type_args: &[TypeRef],
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<RepresentationConstructorResolution>, String> {
        let Some(shape) = self.representation_shape(type_name, context)? else {
            return Ok(None);
        };
        let target_text = type_text_with_args(type_name, type_args);
        let wrapper = self.resolve_type_text(&target_text, context)?;
        let arguments = match &wrapper.ir {
            TypeRefIr::AppliedNominal { arguments, .. }
            | TypeRefIr::Builtin {
                args: arguments, ..
            } => arguments.as_slice(),
            _ => &[],
        };
        if shape.type_params.len() != arguments.len() {
            return Err(format!(
                "representation constructor `{type_name}` expects {} type arguments, found {}",
                shape.type_params.len(),
                arguments.len()
            ));
        }
        let substitutions = shape
            .type_params
            .iter()
            .cloned()
            .zip(arguments.iter().cloned())
            .collect::<BTreeMap<_, _>>();
        let payload_context = TypeResolutionContext::with_type_params(
            &shape.module_path,
            shape.type_params.iter().cloned().collect(),
        );
        let payload = self.resolve_type_expr(&TypeExpr::parse(&shape.payload), &payload_context)?;
        let payload = substitute_type_params_in_type_ref_ref(&payload, &substitutions);
        let payload = self.expand_alias_type_ref(&payload, &payload_context)?;
        let payload = ResolvedTypeRef::new(payload);
        let payload = if shape.module_path == context.module_path {
            payload
        } else {
            self.externalize_local_type_refs(&payload, &shape.module_path)
        };
        Ok(Some(RepresentationConstructorResolution {
            wrapper,
            payload,
        }))
    }

    pub fn resolve_package_callable(&self, path: &str) -> Option<&PackageCallableResolution> {
        let package_symbol =
            PackageExportResolver::new(&self.package_aliases).resolve_package_symbol_path(path)?;
        self.package_callable_resolution(
            &package_symbol.dependency_ref,
            &package_symbol.symbol_path,
        )
    }

    pub(crate) fn is_top_level_package_dependency_ref(&self, dependency_ref: &str) -> bool {
        self.package_dependency_views.get(dependency_ref) == Some(&PackageDependencyView::TopLevel)
    }

    pub fn package_receiver_method_resolution(
        &self,
        receiver: &TypeRefIr,
        method_name: &str,
    ) -> Option<PackageReceiverMethodResolution> {
        let (symbol, arguments) = match receiver {
            TypeRefIr::PackageSymbol { symbol } => (symbol, Vec::new()),
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::PackageSymbol { symbol },
                arguments,
            } => (symbol, arguments.clone()),
            _ => return None,
        };
        let PackageRefIr::Dependency { dependency_ref } = &symbol.package else {
            return None;
        };
        if !self.is_top_level_package_dependency_ref(dependency_ref) {
            return None;
        }
        let (expected_local_abi, expected_build) =
            self.package_artifact_identities.get(dependency_ref)?;
        if symbol.abi_expectation.as_deref() != Some(expected_local_abi.as_str())
            || expected_build.as_str().is_empty()
        {
            return None;
        }
        let source_symbol_path =
            self.package_receiver_source_symbol_path(dependency_ref, &symbol.symbol_path);
        let key = PackageSymbolKey {
            dependency_ref: dependency_ref.clone(),
            symbol_path: source_symbol_path.clone(),
        };
        let receiver_type = self.package_types.get(&key)?;
        if receiver_type.public_path.as_deref() != Some(source_symbol_path.as_str())
            || self.package_interfaces.contains_key(&key)
            || receiver_type.type_params.len() != arguments.len()
            || arguments.iter().any(contains_type_param)
        {
            return None;
        }
        Some(PackageReceiverMethodResolution {
            dependency_ref: dependency_ref.clone(),
            canonical_dependency_ref: self
                .canonical_package_dependency_ref(dependency_ref)
                .to_string(),
            expected_local_abi: expected_local_abi.clone(),
            expected_package_build: expected_build.clone(),
            source_method_path: format!("{source_symbol_path}.{method_name}"),
            receiver_type_params: receiver_type.type_params.clone(),
            receiver_type_arguments: arguments,
        })
    }

    pub fn local_receiver_method_resolution(
        &self,
        receiver: &TypeRefIr,
        method_name: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Option<LocalReceiverMethodResolution> {
        let resolved = ResolvedTypeRef::new(receiver.clone());
        let owner = self.actual_receiver_symbol(&resolved, context)?;
        let receiver_type = self.source_types.get(&owner)?;
        if !matches!(receiver_type.kind, SourceTypeKind::Record { .. }) {
            return None;
        }
        let receiver_type_arguments = match receiver {
            TypeRefIr::AppliedNominal { arguments, .. } => arguments.clone(),
            _ => Vec::new(),
        };
        if receiver_type.type_params.len() != receiver_type_arguments.len()
            || receiver_type_arguments.iter().any(contains_type_param)
        {
            return None;
        }
        let method = self.local_impl_methods.get(&owner)?.get(method_name)?;
        Some(LocalReceiverMethodResolution {
            source_callable: method.source_callable.clone(),
            receiver_type_arguments,
        })
    }

    pub fn resolve_package_constant(&self, path: &str) -> Option<&PackageConstantResolution> {
        let package_symbol =
            PackageExportResolver::new(&self.package_aliases).resolve_package_symbol_path(path)?;
        if !path.contains('/') {
            return None;
        }
        self.package_constants.get(&PackageSymbolKey {
            dependency_ref: package_symbol.dependency_ref,
            symbol_path: package_symbol.symbol_path,
        })
    }

    pub fn resolve_package_interface(&self, path: &str) -> Option<PackageInterfaceResolution> {
        let package_symbol = self.resolve_package_type_symbol_path(path)?;
        let fact = self.package_interface_fact_for_view(
            &package_symbol.dependency_ref,
            &package_symbol.symbol_path,
        )?;
        let public_path = self
            .package_type_resolution_for_view(
                &package_symbol.dependency_ref,
                &package_symbol.symbol_path,
            )?
            .public_path
            .as_ref()?
            .clone();
        let package_id = self
            .package_dependencies
            .get(&package_symbol.dependency_ref)
            .cloned()
            .unwrap_or_else(|| package_symbol.dependency_ref.clone());
        Some(PackageInterfaceResolution {
            identity: TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId { package_id },
                    symbol_path: public_path,
                    abi_expectation: self
                        .package_artifact_identities
                        .get(&package_symbol.dependency_ref)
                        .map(|(abi, _)| abi.as_str().to_string()),
                },
            },
            type_params: fact.type_params.clone(),
            methods: fact.methods.clone(),
            source_module: fact.source_module.clone(),
        })
    }

    pub(super) fn resolve_package_type_symbol_path(
        &self,
        path: &str,
    ) -> Option<ResolvedPackageSymbol> {
        let resolved =
            PackageExportResolver::new(&self.package_aliases).resolve_package_symbol_path(path)?;
        let view = self
            .package_dependency_views
            .get(&resolved.dependency_ref)
            .copied()
            .unwrap_or(PackageDependencyView::Public);
        let syntax_matches = match view {
            // Public view accepts both `alias.<public-path>` and
            // `alias/<module>.<name>` spellings: the slash is a namespace
            // separator normalized by PackageExportResolver, not a topLevel
            // marker. The view itself is decided by the alias name, so the
            // spellings never collide with the TopLevel view.
            PackageDependencyView::Public => true,
            PackageDependencyView::TopLevel => path.contains('/'),
        };
        if !syntax_matches {
            return None;
        }
        Some(resolved)
    }

    pub fn package_interface_for_type_ref(
        &self,
        ty: &TypeRefIr,
    ) -> Option<PackageInterfaceResolution> {
        let TypeRefIr::PackageSymbol { symbol } = ty else {
            return None;
        };
        let dependency_ref = match &symbol.package {
            PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
            PackageRefIr::PackageId { package_id } => package_id.as_str(),
        };
        let fact = self.package_interface_fact(dependency_ref, &symbol.symbol_path)?;
        let public_path = self
            .package_type_resolution(dependency_ref, &symbol.symbol_path)?
            .public_path
            .as_ref()?
            .clone();
        let package_id = match &symbol.package {
            PackageRefIr::Dependency { dependency_ref } => self
                .package_dependencies
                .get(dependency_ref)
                .cloned()
                .unwrap_or_else(|| dependency_ref.clone()),
            PackageRefIr::PackageId { package_id } => package_id.clone(),
        };
        Some(PackageInterfaceResolution {
            identity: TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId { package_id },
                    symbol_path: public_path,
                    abi_expectation: symbol.abi_expectation.clone(),
                },
            },
            type_params: fact.type_params.clone(),
            methods: fact.methods.clone(),
            source_module: fact.source_module.clone(),
        })
    }

    pub fn package_interface_method_index(&self) -> PackageInterfaceMethodIndex {
        let mut index = PackageInterfaceMethodIndex::default();
        for (key, interface) in &self.package_interfaces {
            index.insert_method_names(
                key.dependency_ref.clone(),
                key.symbol_path.clone(),
                interface.methods.iter().map(|method| method.name.clone()),
            );
        }
        for (alias, package_id) in &self.package_dependencies {
            for (key, interface) in &self.package_interfaces {
                if &key.dependency_ref != package_id {
                    continue;
                }
                index.insert_method_names(
                    alias.clone(),
                    key.symbol_path.clone(),
                    interface.methods.iter().map(|method| method.name.clone()),
                );
            }
        }
        index
    }

    pub fn is_nullable(&self, ty: &ResolvedTypeRef) -> bool {
        matches!(ty.ir, TypeRefIr::Nullable { .. })
            || matches!(&ty.ir, TypeRefIr::Union { items } if items.iter().any(is_null_type))
    }

    pub fn contains_interface_type(
        &self,
        ty: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> bool {
        self.contains_interface_resolved_type(ty, context, &mut BTreeSet::new())
    }

    pub(super) fn contains_interface_resolved_type(
        &self,
        ty: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
        visited: &mut BTreeSet<InterfaceTypeVisitKey>,
    ) -> bool {
        self.contains_interface_type_ref_inner(&ty.ir, context, visited)
    }

    pub(super) fn contains_interface_type_ref_inner(
        &self,
        ty: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
        visited: &mut BTreeSet<InterfaceTypeVisitKey>,
    ) -> bool {
        if self.interface_identity_for_type_ref(ty, context).is_some() {
            return true;
        }
        if self.resolved_named_type(ty, context).is_some_and(|named| {
            let arguments = match ty {
                TypeRefIr::AppliedNominal { arguments, .. } => arguments.as_slice(),
                _ => &[],
            };
            self.contains_interface_named_type(named, arguments, context, visited)
        }) {
            return true;
        }
        match ty {
            TypeRefIr::Builtin { args, .. } => args
                .iter()
                .any(|arg| self.contains_interface_type_ref_inner(arg, context, visited)),
            TypeRefIr::AppliedNominal { arguments, .. } => arguments
                .iter()
                .any(|arg| self.contains_interface_type_ref_inner(arg, context, visited)),
            TypeRefIr::Record { fields } => fields
                .values()
                .any(|field| self.contains_interface_type_ref_inner(field, context, visited)),
            TypeRefIr::Union { items } => items
                .iter()
                .any(|item| self.contains_interface_type_ref_inner(item, context, visited)),
            TypeRefIr::Nullable { inner } => {
                self.contains_interface_type_ref_inner(inner, context, visited)
            }
            TypeRefIr::AnyInterface { interface } => interface
                .canonical_type_args
                .iter()
                .any(|arg| self.contains_interface_type_ref_inner(arg, context, visited)),
            TypeRefIr::Function {
                params,
                return_type,
            } => {
                params.iter().any(|param| {
                    self.contains_interface_type_ref_inner(&param.ty, context, visited)
                }) || self.contains_interface_type_ref_inner(return_type, context, visited)
            }
            TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::PackageSchema { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. } => false,
        }
    }

    pub(super) fn contains_interface_named_type(
        &self,
        named: ResolvedNamedType<'_>,
        arguments: &[TypeRefIr],
        caller_context: &TypeResolutionContext<'_>,
        visited: &mut BTreeSet<InterfaceTypeVisitKey>,
    ) -> bool {
        if !visited.insert(named.visit_key.clone()) {
            return false;
        }

        let substitutions = named
            .resolution
            .type_params
            .iter()
            .cloned()
            .zip(arguments.iter().cloned())
            .collect::<BTreeMap<_, _>>();
        let mut type_params = caller_context.type_params.clone();
        type_params.extend(named.resolution.type_params.iter().cloned());
        let source_context =
            TypeResolutionContext::with_type_params(&named.source_module_path, type_params);

        let contains = match &named.resolution.kind {
            SourceTypeKind::Record {
                fields,
                canonical_fields,
            } => {
                if let Some(fields) = canonical_fields {
                    fields.values().any(|field_ty| {
                        let field_ty =
                            substitute_type_params_in_type_ref_ref(field_ty, &substitutions);
                        self.contains_interface_type_ref_inner(&field_ty, &source_context, visited)
                    })
                } else {
                    fields.values().any(|field_ty| {
                        self.contains_interface_type_text_in_named_type(
                            field_ty,
                            named.package_root.as_deref(),
                            &named.resolution.local_type_names,
                            &substitutions,
                            &source_context,
                            visited,
                        )
                    })
                }
            }
            SourceTypeKind::Alias {
                target,
                canonical_target,
            } => {
                if let Some(target) = canonical_target {
                    let target = substitute_type_params_in_type_ref_ref(target, &substitutions);
                    self.contains_interface_type_ref_inner(&target, &source_context, visited)
                } else {
                    self.contains_interface_type_text_in_named_type(
                        target,
                        named.package_root.as_deref(),
                        &named.resolution.local_type_names,
                        &substitutions,
                        &source_context,
                        visited,
                    )
                }
            }
            SourceTypeKind::Representation { target, .. } => self
                .contains_interface_type_text_in_named_type(
                    target,
                    named.package_root.as_deref(),
                    &named.resolution.local_type_names,
                    &substitutions,
                    &source_context,
                    visited,
                ),
            SourceTypeKind::Actor { .. } | SourceTypeKind::External => false,
        };
        visited.remove(&named.visit_key);
        contains
    }

    pub(super) fn contains_interface_type_text_in_named_type(
        &self,
        raw: &str,
        package_root: Option<&str>,
        local_type_names: &BTreeSet<String>,
        substitutions: &BTreeMap<String, TypeRefIr>,
        context: &TypeResolutionContext<'_>,
        visited: &mut BTreeSet<InterfaceTypeVisitKey>,
    ) -> bool {
        let qualified = package_root
            .map(|package_root| qualify_package_type_text(raw, package_root, local_type_names))
            .unwrap_or_else(|| raw.to_string());
        let resolved = self
            .resolve_type_expr(&TypeExpr::parse(&qualified), context)
            .ok();
        resolved.is_some_and(|resolved| {
            let substituted = substitute_type_params_in_type_ref_ref(&resolved, substitutions);
            self.contains_interface_type_ref_inner(&substituted, context, visited)
        })
    }

    pub fn assignable(&self, actual: &ResolvedTypeRef, expected: &ResolvedTypeRef) -> bool {
        type_assignable(
            &self.canonicalize_type_ref(&actual.ir),
            &self.canonicalize_type_ref(&expected.ir),
        )
    }

    pub(super) fn canonicalize_type_ref(&self, ty: &TypeRefIr) -> TypeRefIr {
        match ty {
            TypeRefIr::PackageSymbol { symbol } => {
                let dependency_ref = match &symbol.package {
                    PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
                    PackageRefIr::PackageId { package_id } => self
                        .package_dependencies
                        .iter()
                        .find_map(|(alias, id)| {
                            (id == package_id
                                && self.canonical_package_dependency_ref(alias) == alias)
                                .then_some(alias.as_str())
                        })
                        .unwrap_or(package_id.as_str()),
                };
                let package_id = self
                    .package_dependencies
                    .get(dependency_ref)
                    .cloned()
                    .unwrap_or_else(|| dependency_ref.to_string());
                let symbol_path = self
                    .package_type_resolution(dependency_ref, &symbol.symbol_path)
                    .map(|resolution| source_path(&resolution.module_path, &resolution.name))
                    .unwrap_or_else(|| symbol.symbol_path.clone());
                TypeRefIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::PackageId { package_id },
                        symbol_path,
                        abi_expectation: symbol.abi_expectation.clone().or_else(|| {
                            self.package_artifact_identities
                                .get(dependency_ref)
                                .map(|(abi, _)| abi.as_str().to_string())
                        }),
                    },
                }
            }
            TypeRefIr::ServiceSymbol { symbol } => {
                let module_path = symbol
                    .module_path
                    .strip_prefix("root.")
                    .unwrap_or(&symbol.module_path);
                canonical_named_symbol(
                    &self.canonical_symbol_path(&format!("{module_path}.{}", symbol.symbol)),
                )
            }
            TypeRefIr::AppliedNominal { base, arguments } => {
                let canonical_base = self.canonicalize_type_ref(&nominal_base_type_ref(base));
                TypeRefIr::AppliedNominal {
                    base: nominal_base_from_type_ref(canonical_base)
                        .expect("canonical nominal base remains nominal"),
                    arguments: arguments
                        .iter()
                        .map(|argument| self.canonicalize_type_ref(argument))
                        .collect(),
                }
            }
            TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| self.canonicalize_type_ref(arg))
                    .collect(),
            },
            TypeRefIr::Nullable { inner } => normalize_union(TypeRefIr::Nullable {
                inner: Box::new(self.canonicalize_type_ref(inner)),
            }),
            TypeRefIr::Union { items } => normalize_union(TypeRefIr::Union {
                items: items
                    .iter()
                    .map(|item| self.canonicalize_type_ref(item))
                    .collect(),
            }),
            TypeRefIr::Record { fields } => TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, field_ty)| (name.clone(), self.canonicalize_type_ref(field_ty)))
                    .collect(),
            },
            TypeRefIr::AnyInterface { interface } => {
                let canonical_type_args = interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| self.canonicalize_type_ref(arg))
                    .collect();
                let Ok(identity) = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
                else {
                    return TypeRefIr::AnyInterface {
                        interface: InterfaceInstantiationRef {
                            interface_abi_id: interface.interface_abi_id.clone(),
                            canonical_type_args,
                        },
                    };
                };
                TypeRefIr::AnyInterface {
                    interface: interface_instantiation_ref(
                        self.canonicalize_type_ref(&identity),
                        canonical_type_args,
                    ),
                }
            }
            other => other.clone(),
        }
    }

    pub fn canonicalize_type_ref_for_module(&self, module_path: &str, ty: &TypeRefIr) -> TypeRefIr {
        match ty {
            TypeRefIr::LocalType { type_index } => self
                .local_type_name_for_index(module_path, *type_index)
                .map(|name| canonical_named_symbol(&source_path(module_path, name)))
                .unwrap_or_else(|| ty.clone()),
            TypeRefIr::PublicationType {
                module_path: owner_module,
                type_index,
            } => self
                .local_type_name_for_index(owner_module, *type_index)
                .map(|name| canonical_named_symbol(&source_path(owner_module, name)))
                .unwrap_or_else(|| ty.clone()),
            TypeRefIr::AppliedNominal { base, arguments } => {
                let canonical_base = self
                    .canonicalize_type_ref_for_module(module_path, &nominal_base_type_ref(base));
                TypeRefIr::AppliedNominal {
                    base: nominal_base_from_type_ref(canonical_base)
                        .expect("canonical nominal base remains nominal"),
                    arguments: arguments
                        .iter()
                        .map(|argument| {
                            self.canonicalize_type_ref_for_module(module_path, argument)
                        })
                        .collect(),
                }
            }
            TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| self.canonicalize_type_ref_for_module(module_path, arg))
                    .collect(),
            },
            TypeRefIr::Nullable { inner } => normalize_union(TypeRefIr::Nullable {
                inner: Box::new(self.canonicalize_type_ref_for_module(module_path, inner)),
            }),
            TypeRefIr::Union { items } => normalize_union(TypeRefIr::Union {
                items: items
                    .iter()
                    .map(|item| self.canonicalize_type_ref_for_module(module_path, item))
                    .collect(),
            }),
            TypeRefIr::Record { fields } => TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, field_ty)| {
                        (
                            name.clone(),
                            self.canonicalize_type_ref_for_module(module_path, field_ty),
                        )
                    })
                    .collect(),
            },
            TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
                interface: InterfaceInstantiationRef {
                    interface_abi_id: interface.interface_abi_id.clone(),
                    canonical_type_args: interface
                        .canonical_type_args
                        .iter()
                        .map(|arg| self.canonicalize_type_ref_for_module(module_path, arg))
                        .collect(),
                },
            },
            other => self.canonicalize_type_ref(other),
        }
    }

    pub(super) fn local_type_name_for_index(
        &self,
        module_path: &str,
        type_index: u32,
    ) -> Option<&str> {
        self.modules
            .get(module_path)?
            .type_indices
            .iter()
            .find_map(|(name, index)| (*index == type_index).then_some(name.as_str()))
    }

    /// Normalize a `<module>.<symbol>` path toward its internal name. A public api
    /// symbol path (e.g. `tools.ToolCall`) is rewritten to its internal source path
    /// (e.g. `agent.tools.ToolCall`); internal paths already map to themselves.
    /// Canonicalizing toward internal names is well-defined because every public
    /// name resolves to exactly one internal name, while internal-only names have
    /// no public name.

    pub(super) fn canonical_symbol_path(&self, symbol_path: &str) -> String {
        let stripped = symbol_path.strip_prefix("root.").unwrap_or(symbol_path);
        self.package_public_to_internal
            .get(stripped)
            .cloned()
            .unwrap_or_else(|| stripped.to_string())
    }

    pub(super) fn representation_shape(
        &self,
        type_name: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<RepresentationShape>, String> {
        let name = strip_generic(type_name.trim());
        if let Some(key) = self.resolve_source_type_key(name, context) {
            let resolved = self
                .source_types
                .get(&key)
                .ok_or_else(|| format!("unresolved representation target `{type_name}`"))?;
            return self.representation_shape_from_resolution(resolved, context);
        } else if let Some(key) = self.external_type_symbols.resolve_source_text(name) {
            let resolved = self
                .source_types
                .get(key)
                .ok_or_else(|| format!("unresolved representation target `{type_name}`"))?;
            return self.representation_shape_from_resolution(resolved, context);
        } else if let Some(package_symbol) = self.resolve_package_type_symbol_path(name) {
            if let Some(resolved) = self.package_type_resolution_for_view(
                &package_symbol.dependency_ref,
                &package_symbol.symbol_path,
            ) {
                return self.representation_shape_from_resolution(resolved, context);
            }
            return Ok(prelude_representation_shape(name));
        }
        Ok(prelude_representation_shape(name))
    }

    pub(super) fn representation_shape_from_resolution(
        &self,
        resolved: &SourceTypeResolution,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<RepresentationShape>, String> {
        match &resolved.kind {
            SourceTypeKind::Representation {
                target,
                named_union_branches,
                ..
            } if named_union_branches.is_none()
                && !matches!(TypeExpr::parse(target), TypeExpr::Union(_)) =>
            {
                Ok(Some(RepresentationShape {
                    module_path: resolved.module_path.clone(),
                    type_params: resolved.type_params.clone(),
                    payload: target.clone(),
                }))
            }
            SourceTypeKind::Representation { .. } => Ok(None),
            SourceTypeKind::Alias { target, .. } => {
                let alias_context = TypeResolutionContext::with_type_params(
                    &resolved.module_path,
                    context.type_params.clone(),
                );
                self.representation_shape(target, &alias_context)
            }
            SourceTypeKind::Record { .. }
            | SourceTypeKind::Actor { .. }
            | SourceTypeKind::External => Ok(None),
        }
    }

    pub(super) fn resolve_any_interface_type_expr(
        &self,
        interface: &TypeExpr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<ResolvedTypeRef, String> {
        let selector = self.resolve_canonical_interface_selector_expr(interface, context)?;
        self.require_canonical_interface_selector_object_safe(&selector)?;
        Ok(ResolvedTypeRef::with_text(
            TypeRefIr::AnyInterface {
                interface: selector.instantiation_ref,
            },
            format!("any {}", selector.source_text),
        ))
    }

    pub(super) fn reject_any_interface_selector_aliases(
        &self,
        expr: &TypeExpr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<(), String> {
        match expr {
            TypeExpr::AnyInterface { interface } => {
                if let TypeExpr::Named { name, .. } = interface.as_ref() {
                    if let Some(key) = self.resolve_source_type_key(name, context) {
                        if self.source_types.get(&key).is_some_and(|resolution| {
                            matches!(resolution.kind, SourceTypeKind::Alias { .. })
                        }) {
                            return Err(format!(
                                "interface selector `{}` targets alias `{name}`, not an interface",
                                interface.to_type_string()
                            ));
                        }
                    }
                    if let Some(package_symbol) = self.resolve_package_type_symbol_path(name) {
                        if self
                            .package_type_resolution_for_view(
                                &package_symbol.dependency_ref,
                                &package_symbol.symbol_path,
                            )
                            .is_some_and(|resolution| {
                                matches!(resolution.kind, SourceTypeKind::Alias { .. })
                            })
                        {
                            return Err(format!(
                                "interface selector `{}` targets alias `{name}`, not an interface",
                                interface.to_type_string()
                            ));
                        }
                    }
                }
                self.reject_any_interface_selector_aliases(interface, context)
            }
            TypeExpr::Named { args, .. } | TypeExpr::Union(args) => {
                for arg in args {
                    self.reject_any_interface_selector_aliases(arg, context)?;
                }
                Ok(())
            }
            TypeExpr::Nullable(inner) => self.reject_any_interface_selector_aliases(inner, context),
            TypeExpr::Record(fields) => {
                for field in fields {
                    self.reject_any_interface_selector_aliases(&field.ty, context)?;
                }
                Ok(())
            }
            TypeExpr::Function {
                params,
                return_type,
            } => {
                for param in params {
                    self.reject_any_interface_selector_aliases(&param.ty, context)?;
                }
                self.reject_any_interface_selector_aliases(return_type, context)
            }
            TypeExpr::EmptyRecord | TypeExpr::StringLiteral(_) => Ok(()),
        }
    }

    pub(super) fn resolve_canonical_interface_selector_expr(
        &self,
        expr: &TypeExpr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<CanonicalInterfaceSelectorResolution, String> {
        let selector_text = expr.to_type_string();
        match expr {
            TypeExpr::AnyInterface { .. } => Err(format!(
                "interface selector `{selector_text}` cannot be nested `any`; use the bare interface selector"
            )),
            TypeExpr::Record(_) | TypeExpr::EmptyRecord => Err(format!(
                "interface selector `{selector_text}` targets an anonymous record, not an interface"
            )),
            TypeExpr::Named { name, args } => {
                self.resolve_canonical_interface_selector_named(
                    name,
                    args,
                    &selector_text,
                    context,
                )
            }
            TypeExpr::StringLiteral(_) => Err(format!(
                "interface selector `{selector_text}` targets a literal type, not an interface"
            )),
            TypeExpr::Nullable(_) | TypeExpr::Union(_) | TypeExpr::Function { .. } => Err(
                format!("interface selector `{selector_text}` must be a named interface type"),
            ),
        }
    }

    pub(super) fn resolve_canonical_interface_selector_named(
        &self,
        name: &str,
        args: &[TypeExpr],
        selector_text: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<CanonicalInterfaceSelectorResolution, String> {
        let name = name.trim();
        let service_name = name.strip_prefix("root.").unwrap_or(name);
        if args.is_empty() && context.type_params.contains(service_name) {
            return Err(format!(
                "interface selector `{selector_text}` targets type parameter `{service_name}`, not an interface"
            ));
        }
        if let Some(canonical_name) = canonical_file_ir_builtin_name(name) {
            return Err(format!(
                "interface selector `{selector_text}` targets primitive/builtin type `{canonical_name}`, not an interface"
            ));
        }
        if prelude_known_type_ref(name, Vec::new()).is_some() {
            return Err(format!(
                "interface selector `{selector_text}` targets primitive/builtin type `{name}`, not an interface"
            ));
        }
        if let Some(key) = self.resolve_source_type_key(name, context) {
            return self.resolve_source_interface_selector_from_key(
                key,
                args,
                selector_text,
                context,
            );
        }
        if let Some(key) = self.external_type_symbols.resolve_source_text(name) {
            return self.resolve_source_interface_selector_from_key(
                key.clone(),
                args,
                selector_text,
                context,
            );
        }
        if let Some((alias, schema_type)) = self.service_api_type(name)? {
            let Some(interface) = self.service_api_interface(alias, &schema_type.stable_schema_key)
            else {
                return Err(format!(
                    "interface selector `{selector_text}` targets a non-interface service API type"
                ));
            };
            let args = self.resolve_interface_selector_args(args, context)?;
            self.require_package_interface_type_args(selector_text, &interface.type_params, &args)?;
            return Ok(CanonicalInterfaceSelectorResolution {
                source_text: selector_text.to_string(),
                identity: interface.identity.clone(),
                instantiation_ref: interface_instantiation_ref(interface.identity, args.clone()),
                args,
            });
        }
        if let Some(package_symbol) = self.resolve_package_type_symbol_path(name) {
            if let Some(interface) = self.resolve_package_interface(name) {
                let args = self.resolve_interface_selector_args(args, context)?;
                self.require_package_interface_type_args(
                    selector_text,
                    &interface.type_params,
                    &args,
                )?;
                return Ok(CanonicalInterfaceSelectorResolution {
                    source_text: selector_text.to_string(),
                    identity: interface.identity.clone(),
                    instantiation_ref: interface_instantiation_ref(
                        interface.identity,
                        args.clone(),
                    ),
                    args,
                });
            }
            let resolution = self.package_type_resolution_for_view(
                &package_symbol.dependency_ref,
                &package_symbol.symbol_path,
            );
            if self
                .package_artifact_identities
                .contains_key(&package_symbol.dependency_ref)
                && resolution.is_none()
            {
                let view = self
                    .package_dependency_views
                    .get(&package_symbol.dependency_ref)
                    .copied()
                    .unwrap_or(PackageDependencyView::Public);
                return Err(format!(
                    "package dependency `{}` has no {} type path `{}`",
                    package_symbol.dependency_ref,
                    match view {
                        PackageDependencyView::Public => "public",
                        PackageDependencyView::TopLevel => "top-level source",
                    },
                    package_symbol.symbol_path
                ));
            }
            if let Some(resolution) = resolution {
                return Err(format!(
                    "interface selector `{selector_text}` targets {}, not an interface",
                    source_type_kind_label(&resolution.kind)
                ));
            }
            return Err(format!(
                "interface selector `{selector_text}` does not resolve to an interface"
            ));
        }
        if let Some(symbol) = self.resolve_db_object_symbol(service_name, context)? {
            return Err(format!(
                "interface selector `{selector_text}` targets db object {}.{}, not an interface",
                symbol.module_path, symbol.symbol
            ));
        }
        if name.contains('.') {
            return Err(format!(
                "interface selector `{selector_text}` does not resolve to a known interface"
            ));
        }
        Err(format!(
            "interface selector `{selector_text}` does not resolve to an interface"
        ))
    }

    pub(super) fn resolve_source_interface_selector_from_key(
        &self,
        key: SourceSymbolKey,
        args: &[TypeExpr],
        selector_text: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<CanonicalInterfaceSelectorResolution, String> {
        let Some(resolution) = self.source_types.get(&key) else {
            return Err(format!(
                "interface selector `{selector_text}` does not resolve to an interface"
            ));
        };
        if !self.source_type_is_interface(&key) {
            return Err(format!(
                "interface selector `{selector_text}` targets {}, not an interface",
                source_type_kind_label(&resolution.kind)
            ));
        }
        if resolution.type_params.len() != args.len() {
            return Err(format!(
                "interface selector `{selector_text}` targets interface {}, which expects {} type arguments, found {}",
                key,
                resolution.type_params.len(),
                args.len()
            ));
        }
        let args = self.resolve_interface_selector_args(args, context)?;
        let interface = InterfaceInstantiation {
            symbol: key,
            args: args.clone(),
        };
        let identity = interface_symbol_type_ref(&interface.symbol);
        Ok(CanonicalInterfaceSelectorResolution {
            source_text: selector_text.to_string(),
            instantiation_ref: self
                .interface_semantics
                .canonical_interface_instantiation_ref(&interface),
            identity,
            args,
        })
    }

    pub(super) fn resolve_interface_selector_args(
        &self,
        args: &[TypeExpr],
        context: &TypeResolutionContext<'_>,
    ) -> Result<Vec<TypeRefIr>, String> {
        args.iter()
            .map(|arg| {
                self.resolve_type_text(&arg.to_type_string(), context)
                    .map(|ty| ty.ir)
            })
            .collect()
    }

    pub(super) fn canonical_interface_selector_from_instantiation_resolution(
        &self,
        source_text: String,
        interface: InterfaceInstantiationResolution,
    ) -> Result<CanonicalInterfaceSelectorResolution, String> {
        if !matches!(
            &interface.identity,
            TypeRefIr::ServiceSymbol { .. } | TypeRefIr::PackageSymbol { .. }
        ) {
            return Err(format!(
                "resolved type `{source_text}` is not an interface instantiation"
            ));
        }
        Ok(CanonicalInterfaceSelectorResolution {
            source_text,
            instantiation_ref: interface_instantiation_ref(
                interface.identity.clone(),
                interface.args.clone(),
            ),
            identity: interface.identity,
            args: interface.args,
        })
    }

    fn require_canonical_interface_selector_object_safe(
        &self,
        selector: &CanonicalInterfaceSelectorResolution,
    ) -> Result<(), String> {
        match &selector.identity {
            TypeRefIr::ServiceSymbol { symbol } => {
                let source_interface = InterfaceInstantiation {
                    symbol: SourceSymbolKey::new(
                        symbol
                            .module_path
                            .strip_prefix("root.")
                            .unwrap_or(&symbol.module_path),
                        &symbol.symbol,
                    ),
                    args: selector.args.clone(),
                };
                let diagnostics = self
                    .interface_semantics
                    .object_safety_diagnostics(&source_interface)
                    .map_err(|error| error.to_string())?;
                if diagnostics.is_empty() {
                    return Ok(());
                }
                Err(format!(
                    "interface selector `{}` is not object-safe: {}",
                    selector.source_text,
                    object_safety_diagnostics_display(&diagnostics)
                ))
            }
            TypeRefIr::PackageSymbol { .. } => {
                let package_interface = self
                    .package_interface_for_type_ref(&selector.identity)
                    .ok_or_else(|| {
                        format!(
                            "interface selector `{}` does not resolve to a package interface",
                            selector.source_text
                        )
                    })?;
                self.require_package_interface_type_args(
                    &selector.source_text,
                    &package_interface.type_params,
                    &selector.args,
                )?;
                self.require_package_interface_object_safe(
                    &selector.source_text,
                    &package_interface.methods,
                )
            }
            _ => Err(format!(
                "resolved type `{}` is not an interface instantiation",
                selector.source_text
            )),
        }
    }

    pub(super) fn require_package_interface_object_safe(
        &self,
        selector_text: &str,
        methods: &[InterfaceMethodSignature],
    ) -> Result<(), String> {
        let mut diagnostics = Vec::new();
        if methods.is_empty() {
            diagnostics.push(InterfaceObjectSafetyDiagnostic::MarkerInterface {
                interface: SourceSymbolKey::new("<package>", selector_text),
            });
        }
        for method in methods {
            if method.is_static {
                diagnostics.push(
                    InterfaceObjectSafetyDiagnostic::UnsupportedMethodRequirement {
                        method_name: method.name.clone(),
                        message: "method requirement cannot be static".to_string(),
                    },
                );
            }
            if method.is_native {
                diagnostics.push(
                    InterfaceObjectSafetyDiagnostic::UnsupportedMethodRequirement {
                        method_name: method.name.clone(),
                        message: "method requirement cannot be native".to_string(),
                    },
                );
            }
            if method.is_provider {
                diagnostics.push(
                    InterfaceObjectSafetyDiagnostic::UnsupportedMethodRequirement {
                        method_name: method.name.clone(),
                        message: "method requirement cannot be provider-only".to_string(),
                    },
                );
            }
            if !method.type_params.is_empty() {
                diagnostics.push(
                    InterfaceObjectSafetyDiagnostic::UnsupportedMethodRequirement {
                        method_name: method.name.clone(),
                        message: "method requirement cannot declare method-level type parameters"
                            .to_string(),
                    },
                );
            }
            let params = interface_method_signature_params(method);
            match params.first() {
                Some(param) if param.name == "self" && is_self_type_ref(&param.ty) => {
                    for param in params.iter().skip(1) {
                        if type_ref_contains_self(&param.ty) {
                            diagnostics.push(InterfaceObjectSafetyDiagnostic::InvalidSelfUsage {
                                method_name: method.name.clone(),
                                message: "Self can only appear in the first receiver parameter"
                                    .to_string(),
                            });
                        }
                    }
                    if type_ref_contains_self(&method.return_type) {
                        diagnostics.push(InterfaceObjectSafetyDiagnostic::InvalidSelfUsage {
                            method_name: method.name.clone(),
                            message: "Self cannot be used as a return type".to_string(),
                        });
                    }
                }
                _ if params.iter().any(|param| type_ref_contains_self(&param.ty))
                    || type_ref_contains_self(&method.return_type) =>
                {
                    diagnostics.push(InterfaceObjectSafetyDiagnostic::InvalidSelfUsage {
                        method_name: method.name.clone(),
                        message: "Self can only appear in the first receiver parameter".to_string(),
                    });
                }
                _ => diagnostics.push(InterfaceObjectSafetyDiagnostic::MissingSelfReceiver {
                    method_name: method.name.clone(),
                }),
            }
        }
        if diagnostics.is_empty() {
            return Ok(());
        }
        Err(format!(
            "interface selector `{selector_text}` is not object-safe: {}",
            object_safety_diagnostics_display(&diagnostics)
        ))
    }

    pub(super) fn require_package_interface_type_args(
        &self,
        selector_text: &str,
        type_params: &[String],
        args: &[TypeRefIr],
    ) -> Result<(), String> {
        if type_params.len() == args.len() {
            return Ok(());
        }
        Err(format!(
            "interface selector `{selector_text}` expects {} type arguments, found {}",
            type_params.len(),
            args.len()
        ))
    }

    pub(super) fn resolve_type_expr(
        &self,
        expr: &TypeExpr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<TypeRefIr, String> {
        Ok(match expr {
            TypeExpr::EmptyRecord => TypeRefIr::Record {
                fields: BTreeMap::new(),
            },
            TypeExpr::StringLiteral(value) => TypeRefIr::Literal {
                value: LiteralIr::String {
                    value: value.clone(),
                },
            },
            TypeExpr::Named { name, args } => self.resolve_named_type(name, args, context)?,
            TypeExpr::Nullable(inner) => TypeRefIr::Nullable {
                inner: Box::new(self.resolve_type_expr(inner, context)?),
            },
            TypeExpr::Union(items) => TypeRefIr::Union {
                items: items
                    .iter()
                    .map(|item| self.resolve_type_expr(item, context))
                    .collect::<Result<Vec<_>, _>>()?,
            },
            TypeExpr::AnyInterface { interface } => {
                self.resolve_any_interface_type_expr(interface, context)?.ir
            }
            TypeExpr::Record(fields) => TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|field| {
                        Ok((
                            field.name.clone(),
                            self.resolve_type_expr(&field.ty, context)?,
                        ))
                    })
                    .collect::<Result<BTreeMap<_, _>, String>>()?,
            },
            TypeExpr::Function {
                params,
                return_type,
            } => TypeRefIr::Function {
                params: params
                    .iter()
                    .map(|param| {
                        Ok(FunctionTypeParamIr {
                            name: param.name.clone(),
                            ty: self.resolve_type_expr(&param.ty, context)?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?,
                return_type: Box::new(self.resolve_type_expr(return_type, context)?),
            },
        })
    }

    pub(super) fn resolve_named_type(
        &self,
        name: &str,
        args: &[TypeExpr],
        context: &TypeResolutionContext<'_>,
    ) -> Result<TypeRefIr, String> {
        let resolved_args = args
            .iter()
            .map(|arg| self.resolve_type_expr(arg, context))
            .collect::<Result<Vec<_>, _>>()?;
        let name = name.trim();
        let service_name = name.strip_prefix("root.").unwrap_or(name);
        if args.is_empty() && context.type_params.contains(service_name) {
            return Ok(TypeRefIr::TypeParam {
                name: service_name.to_string(),
            });
        }
        let source_type_key = self.resolve_source_type_key(name, context);
        if source_type_key.is_none() {
            if let Some(canonical_name) = canonical_file_ir_builtin_name(name) {
                if canonical_name == BuiltinShape::Map.name()
                    && resolved_args.len() == 2
                    && type_ref_contains_any_interface(&resolved_args[0])
                {
                    return Err(format!(
                        "Map key type `{}` cannot contain an `any` interface value",
                        args[0].to_type_string()
                    ));
                }
                return Ok(TypeRefIr::Builtin {
                    name: canonical_name.to_string(),
                    args: resolved_args,
                });
            }
        }
        if let Some(key) = source_type_key {
            let resolution = self
                .source_types
                .get(&key)
                .ok_or_else(|| format!("missing source type resolution for `{name}`"))?;
            if resolution.type_params.len() != resolved_args.len() {
                return Err(format!(
                    "source type `{name}` expects {} type arguments, found {}",
                    resolution.type_params.len(),
                    resolved_args.len()
                ));
            }
            if !resolved_args.is_empty()
                && (self.source_interfaces.contains(&key)
                    || matches!(
                        resolution.kind,
                        SourceTypeKind::Actor { .. } | SourceTypeKind::External
                    ))
            {
                return Err(format!(
                    "source type `{name}` cannot be used as an applied nominal base"
                ));
            }
            let module = self
                .modules
                .get(context.module_path)
                .ok_or_else(|| format!("missing type resolution module {}", context.module_path))?;
            if key.module_path() == context.module_path {
                if let Some(index) = module.type_indices.get(key.symbol()) {
                    return apply_nominal_arguments(
                        TypeRefIr::LocalType { type_index: *index },
                        resolved_args,
                    );
                }
            }
            return apply_nominal_arguments(
                TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: key.module_path().to_string(),
                        symbol: key.symbol().to_string(),
                    },
                },
                resolved_args,
            );
        }
        if let Some(type_ref) = contextual_prelude_type_ref(name, resolved_args.clone(), context) {
            validate_prelude_type_arity(name, resolved_args.len())?;
            return Ok(type_ref);
        }
        if let Some(type_ref) = prelude_known_type_ref(name, resolved_args.clone()) {
            validate_prelude_type_arity(name, resolved_args.len())?;
            return Ok(type_ref);
        }
        if name.starts_with("std.") || name.starts_with("config.") {
            return Err(format!("unknown compiler-owned type `{name}`"));
        }
        if let Some((_alias, schema_type)) = self.service_api_type(name)? {
            if schema_type.canonical_descriptor.type_params.len() != resolved_args.len() {
                return Err(format!(
                    "service API type `{name}` expects {} type arguments, found {}",
                    schema_type.canonical_descriptor.type_params.len(),
                    resolved_args.len()
                ));
            }
            if !resolved_args.is_empty()
                && matches!(
                    schema_type.canonical_descriptor.descriptor,
                    ContractTypeDescriptor::CallbackInterface { .. }
                )
            {
                return Err(format!(
                    "service API type `{name}` cannot be used as an applied nominal base"
                ));
            }
            return apply_nominal_arguments(
                TypeRefIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::PackageId {
                            package_id: schema_type.package_id.clone(),
                        },
                        symbol_path: schema_type.stable_schema_key.clone(),
                        abi_expectation: None,
                    },
                },
                resolved_args,
            );
        }
        if let Some(package_symbol) = self.resolve_package_type_symbol_path(name) {
            let resolution = self.package_type_resolution_for_view(
                &package_symbol.dependency_ref,
                &package_symbol.symbol_path,
            );
            if let Some(resolution) = resolution {
                if resolution.type_params.len() != resolved_args.len() {
                    return Err(format!(
                        "package type `{name}` expects {} type arguments, found {}",
                        resolution.type_params.len(),
                        resolved_args.len()
                    ));
                }
                if !resolved_args.is_empty()
                    && (matches!(
                        resolution.kind,
                        SourceTypeKind::Actor { .. } | SourceTypeKind::External
                    ) || self
                        .package_interface_fact(
                            &package_symbol.dependency_ref,
                            &package_symbol.symbol_path,
                        )
                        .is_some())
                {
                    return Err(format!(
                        "package type `{name}` cannot be used as an applied nominal base"
                    ));
                }
            } else if !resolved_args.is_empty() {
                return Err(format!(
                    "package type `{name}` has no exact declaration for generic arity validation"
                ));
            }
            let abi_expectation = self
                .package_artifact_identities
                .get(&package_symbol.dependency_ref)
                .map(|(abi, _)| abi.as_str().to_string());
            return apply_nominal_arguments(
                TypeRefIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::Dependency {
                            dependency_ref: if self
                                .is_top_level_package_dependency_ref(&package_symbol.dependency_ref)
                            {
                                package_symbol.dependency_ref.clone()
                            } else {
                                self.canonical_package_dependency_ref(
                                    &package_symbol.dependency_ref,
                                )
                                .to_string()
                            },
                        },
                        symbol_path: package_symbol.symbol_path,
                        abi_expectation,
                    },
                },
                resolved_args,
            );
        }
        let dependency_root = name
            .split_once('/')
            .map(|(root, _)| root)
            .or_else(|| name.split_once('.').map(|(root, _)| root));
        if let Some((dependency_ref, view)) =
            dependency_root.and_then(|root| self.package_dependency_views.get_key_value(root))
        {
            return Err(match view {
                PackageDependencyView::Public => format!(
                    "package dependency `{dependency_ref}` resolves public type paths as `{dependency_ref}.<public-path>` or `{dependency_ref}/<module>.<name>`; path `{name}` did not resolve to a public type"
                ),
                PackageDependencyView::TopLevel => format!(
                    "package dependency `{dependency_ref}` uses top-level type syntax `{dependency_ref}/<source-module>.<name>`; dotted public syntax is unavailable"
                ),
            });
        }
        if let Some(symbol) = self.external_type_symbols.resolve_source_text(name) {
            if !resolved_args.is_empty() {
                return Err(format!(
                    "external type `{name}` has no exact declaration for generic arity validation"
                ));
            }
            return Ok(TypeRefIr::ServiceSymbol {
                symbol: service_symbol_ref_from_source_key(symbol),
            });
        }
        if let Some(symbol) = self.resolve_db_object_symbol(service_name, context)? {
            if !resolved_args.is_empty() {
                return Err(format!(
                    "db object type `{name}` cannot be used as an applied nominal base"
                ));
            }
            return Ok(TypeRefIr::DbObjectSymbol { symbol });
        }
        if name.contains('.') {
            if !resolved_args.is_empty() {
                return Err(format!(
                    "unresolved nominal type `{name}` cannot accept type arguments"
                ));
            }
            return Ok(TypeRefIr::ServiceSymbol {
                symbol: service_symbol_ref(name),
            });
        }
        Err(format!("unresolved type `{name}`"))
    }

    pub(super) fn service_api_type(
        &self,
        name: &str,
    ) -> Result<Option<(&str, &PackageSchemaTypeRecord)>, String> {
        let name = name.strip_prefix("root.").unwrap_or(name);
        let Some((alias, stable_key)) = name.split_once('.') else {
            return Ok(None);
        };
        let Some((canonical_alias, records)) = self.service_api_schemas.get_key_value(alias) else {
            return Ok(None);
        };
        let schema_type = records.get(stable_key).ok_or_else(|| {
            format!("service dependency `{alias}` has no public API type `{stable_key}`")
        })?;
        Ok(Some((canonical_alias.as_str(), schema_type)))
    }

    pub(super) fn service_api_interface(
        &self,
        alias: &str,
        stable_key: &str,
    ) -> Option<PackageInterfaceResolution> {
        let schema_type = self.service_api_schemas.get(alias)?.get(stable_key)?;
        let ContractTypeDescriptor::CallbackInterface { operations } =
            &schema_type.canonical_descriptor.descriptor
        else {
            return None;
        };
        let methods = operations
            .iter()
            .map(|(name, operation)| {
                Some(InterfaceMethodSignature {
                    name: name.clone(),
                    type_params: Vec::new(),
                    params: operation
                        .parameters
                        .iter()
                        .enumerate()
                        .map(|(index, ty)| {
                            Some(FunctionTypeParamIr {
                                name: format!("arg{index}"),
                                ty: contract_type_ref_ir(alias, ty).ok()?,
                            })
                        })
                        .collect::<Option<Vec<_>>>()?,
                    return_type: contract_type_ref_ir(alias, &operation.return_type).ok()?,
                    is_native: false,
                    is_provider: false,
                    is_static: false,
                    implicit_self: None,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(PackageInterfaceResolution {
            identity: TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: schema_type.package_id.clone(),
                    },
                    symbol_path: stable_key.to_string(),
                    abi_expectation: None,
                },
            },
            type_params: schema_type.canonical_descriptor.type_params.clone(),
            methods,
            source_module: alias.to_string(),
        })
    }

    pub(super) fn package_symbol_resolution<'a, V>(
        &self,
        dependency_ref: &str,
        symbol_path: &str,
        map: &'a BTreeMap<PackageSymbolKey, V>,
        full: bool,
    ) -> Option<&'a V> {
        let key = |dependency_ref: &str| PackageSymbolKey {
            dependency_ref: dependency_ref.to_string(),
            symbol_path: symbol_path.to_string(),
        };
        map.get(&key(dependency_ref)).or_else(|| {
            if full {
                let canonical = self.canonical_package_dependency_ref(dependency_ref);
                if let Some(found) = self
                    .package_dependency_canonical_refs
                    .iter()
                    .filter(|(_, candidate)| candidate.as_str() == canonical)
                    .find_map(|(alias, _)| map.get(&key(alias)))
                {
                    return Some(found);
                }
            }
            let by_package_id = self
                .package_dependencies
                .get(dependency_ref)
                .and_then(|package_id| map.get(&key(package_id)));
            if by_package_id.is_some() || !full {
                return by_package_id;
            }
            self.package_dependencies
                .iter()
                .filter(|(_, candidate)| candidate.as_str() == dependency_ref)
                .find_map(|(alias, _)| map.get(&key(alias)))
        })
    }

    pub(super) fn package_type_resolution(
        &self,
        dependency_ref: &str,
        symbol_path: &str,
    ) -> Option<&SourceTypeResolution> {
        self.package_symbol_resolution(dependency_ref, symbol_path, &self.package_types, true)
    }

    pub(super) fn package_type_resolution_for_view(
        &self,
        dependency_ref: &str,
        symbol_path: &str,
    ) -> Option<&SourceTypeResolution> {
        self.package_symbol_resolution(dependency_ref, symbol_path, &self.package_types, false)
    }

    /// Returns the manifest dependency's primary alias for either of its
    /// source-visible views. Lowering uses this to validate a source-only
    /// callee root without leaking that view into File IR.

    pub fn canonical_package_dependency_ref<'a>(&'a self, dependency_ref: &'a str) -> &'a str {
        self.package_dependency_canonical_refs
            .get(dependency_ref)
            .map(String::as_str)
            .unwrap_or(dependency_ref)
    }

    pub(super) fn package_type_source_path(
        &self,
        dependency_ref: &str,
        module_path: &str,
        source_symbol: &str,
    ) -> Option<&String> {
        let key = |alias: &str| {
            (
                alias.to_string(),
                module_path.to_string(),
                source_symbol.to_string(),
            )
        };
        self.package_type_source_paths
            .get(&key(dependency_ref))
            .or_else(|| {
                let canonical = self.canonical_package_dependency_ref(dependency_ref);
                self.package_dependency_canonical_refs
                    .iter()
                    .filter(|(_, candidate)| candidate.as_str() == canonical)
                    .find_map(|(alias, _)| self.package_type_source_paths.get(&key(alias)))
            })
    }

    pub(super) fn package_receiver_source_symbol_path(
        &self,
        dependency_ref: &str,
        selected_path: &str,
    ) -> String {
        let canonical_dependency_ref = self.canonical_package_dependency_ref(dependency_ref);
        let mut matches = self.package_type_source_paths.iter().filter_map(
            |((candidate_dependency_ref, module_path, source_symbol), candidate_selected_path)| {
                (candidate_dependency_ref == canonical_dependency_ref
                    && candidate_selected_path == selected_path)
                    .then(|| source_path(module_path, source_symbol))
            },
        );
        let Some(first) = matches.next() else {
            return selected_path.to_string();
        };
        if matches.next().is_some() {
            return selected_path.to_string();
        }
        first
    }

    /// Resolve a package type by its symbol path alone, searching every indexed
    /// package. Used to recover the shape of a package type referenced through a
    /// package-internal `root.` path that did not carry its originating package id.

    pub(super) fn package_type_by_symbol_path(
        &self,
        symbol_path: &str,
    ) -> Option<&SourceTypeResolution> {
        self.package_types
            .iter()
            .find(|(key, _)| key.symbol_path == symbol_path)
            .map(|(_, resolution)| resolution)
    }

    pub(super) fn package_callable_resolution(
        &self,
        dependency_ref: &str,
        symbol_path: &str,
    ) -> Option<&PackageCallableResolution> {
        self.package_symbol_resolution(dependency_ref, symbol_path, &self.package_callables, false)
    }

    pub(super) fn package_interface_fact(
        &self,
        dependency_ref: &str,
        symbol_path: &str,
    ) -> Option<&PackageInterfaceFact> {
        self.package_symbol_resolution(dependency_ref, symbol_path, &self.package_interfaces, true)
    }

    pub(super) fn package_interface_fact_for_view(
        &self,
        dependency_ref: &str,
        symbol_path: &str,
    ) -> Option<&PackageInterfaceFact> {
        self.package_symbol_resolution(dependency_ref, symbol_path, &self.package_interfaces, false)
    }

    pub(crate) fn resolve_source_type_key(
        &self,
        name: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Option<SourceSymbolKey> {
        let name = name.trim();
        let service_name = name.strip_prefix("root.").unwrap_or(name);
        if let Some((module_path, symbol)) = service_name.rsplit_once('.') {
            let key = SourceSymbolKey::new(module_path, symbol);
            return self.source_types.contains_key(&key).then_some(key);
        }
        let key = SourceSymbolKey::new(context.module_path, service_name);
        self.source_types.contains_key(&key).then_some(key)
    }

    pub(super) fn resolved_named_type(
        &self,
        ty: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
    ) -> Option<ResolvedNamedType<'_>> {
        match ty {
            TypeRefIr::LocalType { type_index } => {
                let resolution = self.local_type_resolution(context.module_path, *type_index)?;
                Some(ResolvedNamedType {
                    resolution,
                    source_module_path: resolution.module_path.clone(),
                    package_root: None,
                    visit_key: InterfaceTypeVisitKey::Source(SourceSymbolKey::new(
                        &resolution.module_path,
                        &resolution.name,
                    )),
                })
            }
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => {
                let resolution = self.local_type_resolution(module_path, *type_index)?;
                Some(ResolvedNamedType {
                    resolution,
                    source_module_path: resolution.module_path.clone(),
                    package_root: None,
                    visit_key: InterfaceTypeVisitKey::Source(SourceSymbolKey::new(
                        &resolution.module_path,
                        &resolution.name,
                    )),
                })
            }
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                let module_path = symbol
                    .module_path
                    .strip_prefix("root.")
                    .unwrap_or(&symbol.module_path);
                let key = SourceSymbolKey::new(module_path, &symbol.symbol);
                self.source_types
                    .get(&key)
                    .map(|resolution| ResolvedNamedType {
                        resolution,
                        source_module_path: module_path.to_string(),
                        package_root: None,
                        visit_key: InterfaceTypeVisitKey::Source(key),
                    })
            }
            TypeRefIr::PackageSymbol { symbol } => {
                let dependency_ref = match &symbol.package {
                    PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
                    PackageRefIr::PackageId { package_id } => package_id.as_str(),
                };
                let resolution =
                    self.package_type_resolution(dependency_ref, &symbol.symbol_path)?;
                let package_id = self
                    .package_dependencies
                    .get(dependency_ref)
                    .map(String::as_str)
                    .unwrap_or(dependency_ref);
                Some(ResolvedNamedType {
                    resolution,
                    source_module_path: resolution.module_path.clone(),
                    package_root: package_root_for_symbol(
                        symbol,
                        &self.package_dependencies,
                        &self.package_dependency_views,
                        &self.package_dependency_canonical_refs,
                    ),
                    visit_key: InterfaceTypeVisitKey::Package(PackageSymbolKey {
                        dependency_ref: package_id.to_string(),
                        symbol_path: source_path(&resolution.module_path, &resolution.name),
                    }),
                })
            }
            TypeRefIr::AppliedNominal { base, .. } => {
                self.resolved_named_type(&nominal_base_type_ref(base), context)
            }
            _ => None,
        }
    }

    pub(super) fn resolve_db_object_symbol(
        &self,
        name: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<ServiceSymbolRef>, String> {
        let Some(module) = self.modules.get(context.module_path) else {
            return Ok(None);
        };
        Ok(module.local_db_objects.resolve(name))
    }

    pub(super) fn expand_alias_text(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<String, String> {
        let Some(module) = self.modules.get(context.module_path) else {
            return Ok(raw.to_string());
        };
        expand_alias_text(raw, &module.alias_targets)
    }

    /// Classifies one validated `implements` selector without inferring owner
    /// from display strings or retrying another owner after a failed handoff.
    pub(crate) fn classify_canonical_interface_owner(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
    ) -> CanonicalInterfaceOwnerResolution {
        let semantic_interface = match self
            .interface_semantics
            .canonical_source_interface_instantiation_from_type_ref(
                context.module_path,
                &TypeRef {
                    name: raw.to_string(),
                },
                &context.type_params,
            ) {
            Ok(interface) => interface,
            Err(error) => {
                return CanonicalInterfaceOwnerResolution::InvalidOrUnresolved {
                    message: error.to_string(),
                };
            }
        };
        match self
            .interface_semantics
            .interface_owner_kind(&semantic_interface.symbol)
        {
            Some(InterfaceOwnerKind::Source) => {
                let interface = match self.resolve_interface_instantiation_text(raw, context) {
                    Ok(Some(interface)) => interface,
                    Ok(None) => {
                        return CanonicalInterfaceOwnerResolution::InvalidOrUnresolved {
                            message: format!("implements entry `{raw}` is not an interface"),
                        };
                    }
                    Err(message) => {
                        return CanonicalInterfaceOwnerResolution::InvalidOrUnresolved { message };
                    }
                };
                CanonicalInterfaceOwnerResolution::SourceDeclaredExact {
                    interface: semantic_interface.symbol,
                    arguments: interface.args,
                }
            }
            Some(InterfaceOwnerKind::CompilerKnown) => {
                CanonicalInterfaceOwnerResolution::CompilerKnown {
                    interface: semantic_interface.symbol,
                    arguments: semantic_interface.args,
                }
            }
            Some(InterfaceOwnerKind::External) => {
                let interface = match self.resolve_interface_instantiation_text(raw, context) {
                    Ok(interface) => interface,
                    Err(message) => {
                        return CanonicalInterfaceOwnerResolution::InvalidOrUnresolved { message };
                    }
                };
                match interface {
                    Some(interface)
                        if matches!(&interface.identity, TypeRefIr::PackageSymbol { .. }) =>
                    {
                        CanonicalInterfaceOwnerResolution::TypedPackage {
                            identity: interface.identity,
                            arguments: interface.args,
                        }
                    }
                    _ => CanonicalInterfaceOwnerResolution::InvalidOrUnresolved {
                        message: format!("implements entry `{raw}` is not an interface"),
                    },
                }
            }
            None => CanonicalInterfaceOwnerResolution::InvalidOrUnresolved {
                message: format!("implements entry `{raw}` is not an interface"),
            },
        }
    }

    pub(super) fn resolve_interface_instantiation_text(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<InterfaceInstantiationResolution>, String> {
        let TypeExpr::Named { name, args } = TypeExpr::parse(raw) else {
            return Ok(None);
        };
        let arguments = self.resolve_interface_selector_args(&args, context)?;
        if let Some(key) = self.resolve_source_type_key(&name, context) {
            if !self.source_type_is_interface(&key) {
                return Ok(None);
            }
            let resolution = self
                .source_types
                .get(&key)
                .ok_or_else(|| format!("missing source interface declaration `{key}`"))?;
            if resolution.type_params.len() != arguments.len() {
                return Err(format!(
                    "interface `{name}` expects {} type arguments, found {}",
                    resolution.type_params.len(),
                    arguments.len()
                ));
            }
            return Ok(Some(InterfaceInstantiationResolution {
                identity: interface_symbol_type_ref(&key),
                args: arguments,
            }));
        }
        if let Some((alias, schema_type)) = self.service_api_type(&name)? {
            let Some(interface) = self.service_api_interface(alias, &schema_type.stable_schema_key)
            else {
                return Ok(None);
            };
            self.require_package_interface_type_args(raw, &interface.type_params, &arguments)?;
            return Ok(Some(InterfaceInstantiationResolution {
                identity: interface.identity,
                args: arguments,
            }));
        }
        let Some(interface) = self.resolve_package_interface(&name) else {
            return Ok(None);
        };
        self.require_package_interface_type_args(raw, &interface.type_params, &arguments)?;
        Ok(Some(InterfaceInstantiationResolution {
            identity: interface.identity,
            args: arguments,
        }))
    }

    pub(super) fn interface_instantiation_from_resolved(
        &self,
        resolved: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<InterfaceInstantiationResolution>, String> {
        if let TypeRefIr::AnyInterface { interface } = &resolved.ir {
            let identity = serde_json::from_str(&interface.interface_abi_id).map_err(|error| {
                format!("resolved any-interface identity is not a canonical TypeRefIr: {error}")
            })?;
            return Ok(Some(InterfaceInstantiationResolution {
                identity,
                args: interface.canonical_type_args.clone(),
            }));
        }
        let Some(identity) = self.interface_identity_for_type_ref(&resolved.ir, context) else {
            return Ok(None);
        };
        let expected_arity = match &identity {
            TypeRefIr::ServiceSymbol { symbol } => {
                let module_path = symbol
                    .module_path
                    .strip_prefix("root.")
                    .unwrap_or(&symbol.module_path);
                self.source_types
                    .get(&SourceSymbolKey::new(module_path, &symbol.symbol))
                    .map_or(0, |resolution| resolution.type_params.len())
            }
            TypeRefIr::PackageSymbol { symbol } => {
                let dependency_ref = match &symbol.package {
                    PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
                    PackageRefIr::PackageId { package_id } => package_id.as_str(),
                };
                self.package_interface_fact(dependency_ref, &symbol.symbol_path)
                    .map_or(0, |interface| interface.type_params.len())
            }
            _ => 0,
        };
        if expected_arity != 0 {
            return Err(format!(
                "resolved generic interface requires {expected_arity} structured type arguments"
            ));
        }
        Ok(Some(InterfaceInstantiationResolution {
            identity,
            args: Vec::new(),
        }))
    }

    pub(super) fn interface_identity_for_type_ref(
        &self,
        ty: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
    ) -> Option<TypeRefIr> {
        match ty {
            TypeRefIr::LocalType { type_index } => {
                let resolution = self.local_type_resolution(context.module_path, *type_index)?;
                self.source_type_is_interface(&SourceSymbolKey::new(
                    &resolution.module_path,
                    &resolution.name,
                ))
                .then(|| TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: resolution.module_path.clone(),
                        symbol: resolution.name.clone(),
                    },
                })
            }
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                let module_path = symbol
                    .module_path
                    .strip_prefix("root.")
                    .unwrap_or(&symbol.module_path);
                let key = SourceSymbolKey::new(module_path, &symbol.symbol);
                self.source_type_is_interface(&key)
                    .then(|| TypeRefIr::ServiceSymbol {
                        symbol: ServiceSymbolRef {
                            module_path: module_path.to_string(),
                            symbol: symbol.symbol.clone(),
                        },
                    })
            }
            TypeRefIr::PackageSymbol { .. } => self
                .package_interface_for_type_ref(ty)
                .map(|interface| interface.identity),
            _ => None,
        }
    }

    pub(super) fn source_type_is_interface(&self, key: &SourceSymbolKey) -> bool {
        self.source_interfaces.contains(key)
    }
}
