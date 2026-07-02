use std::collections::BTreeMap;

use super::*;
use skiff_artifact_model::{FunctionTypeParamIr, LiteralIr};
use skiff_compiler_core::type_ref::substitute_type_params_in_type_ref_ref;

impl TypeResolutionModel {
    pub fn assignable_in_context(
        &self,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> bool {
        if self.assignable(actual, expected) {
            return true;
        }
        let actual = self.externalize_local_type_refs(actual, context.module_path);
        let expected = self.externalize_local_type_refs(expected, context.module_path);
        if type_assignable(&actual.ir, &expected.ir) {
            return true;
        }
        if self.contextual_assignable_ir(&actual.ir, &expected.ir, context) {
            return true;
        }
        let actual = self.transparent_alias_ir(&actual.ir, context);
        let expected = self.transparent_alias_ir(&expected.ir, context);
        type_assignable(&actual, &expected)
            || self.contextual_assignable_ir(&actual, &expected, context)
    }

    pub fn concrete_type_conforms_to_interface(
        &self,
        actual: &ResolvedTypeRef,
        interface: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<InterfaceConformanceMatch>, String> {
        let Some(expected_interface) =
            self.interface_instantiation_from_resolved(interface, context)?
        else {
            return Ok(None);
        };
        let Some(receiver) = self.actual_receiver_symbol(actual, context) else {
            return Ok(None);
        };
        let receiver_args = self.resolved_named_type_args(&actual.source_text, context)?;
        for conformance in self
            .interface_conformances
            .iter()
            .filter(|conformance| conformance.receiver == receiver)
        {
            let Some(substitutions) =
                self.receiver_type_param_substitutions(actual, conformance, context)?
            else {
                continue;
            };
            let implemented = InterfaceInstantiationResolution {
                identity: conformance.interface.identity.clone(),
                args: conformance
                    .interface
                    .args
                    .iter()
                    .map(|arg| substitute_type_params_in_type_ref_ref(arg, &substitutions))
                    .collect(),
            };
            if self.interface_instantiations_match(&implemented, &expected_interface) {
                if matches!(expected_interface.identity, TypeRefIr::PackageSymbol { .. })
                    && self
                        .package_method_slots_for_local_conformance(
                            &receiver,
                            &receiver_args,
                            &expected_interface,
                            conformance,
                        )?
                        .is_none()
                {
                    continue;
                }
                return Ok(Some(InterfaceConformanceMatch {
                    receiver,
                    implemented_interface_identity: self
                        .canonicalize_type_ref(&implemented.identity),
                    implemented_interface_args: implemented
                        .args
                        .iter()
                        .map(|arg| self.canonicalize_type_ref(arg))
                        .collect(),
                    expected_interface_identity: self
                        .canonicalize_type_ref(&expected_interface.identity),
                    expected_interface_args: expected_interface
                        .args
                        .iter()
                        .map(|arg| self.canonicalize_type_ref(arg))
                        .collect(),
                }));
            }
        }
        Ok(None)
    }

    pub fn local_any_interface_conformance_for_boxing(
        &self,
        actual: &ResolvedTypeRef,
        interface: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<LocalAnyInterfaceConformanceResolution>, String> {
        let Some(expected_interface) =
            self.interface_instantiation_from_resolved(interface, context)?
        else {
            return Ok(None);
        };
        let Some(receiver) = self.actual_receiver_symbol(actual, context) else {
            return Ok(None);
        };
        let receiver_args = self.resolved_named_type_args(&actual.source_text, context)?;
        for conformance in self
            .interface_conformances
            .iter()
            .filter(|conformance| conformance.receiver == receiver)
        {
            let Some(substitutions) =
                self.receiver_type_param_substitutions(actual, conformance, context)?
            else {
                continue;
            };
            let implemented = InterfaceInstantiationResolution {
                identity: conformance.interface.identity.clone(),
                args: conformance
                    .interface
                    .args
                    .iter()
                    .map(|arg| substitute_type_params_in_type_ref_ref(arg, &substitutions))
                    .collect(),
            };
            if !self.interface_instantiations_match(&implemented, &expected_interface) {
                continue;
            }
            return self.local_any_interface_conformance_resolution(
                receiver,
                receiver_args,
                &expected_interface,
                conformance,
            );
        }
        Ok(None)
    }

    fn local_any_interface_conformance_resolution(
        &self,
        receiver: SourceSymbolKey,
        receiver_args: Vec<TypeRefIr>,
        expected_interface: &InterfaceInstantiationResolution,
        conformance: &InterfaceConformanceResolution,
    ) -> Result<Option<LocalAnyInterfaceConformanceResolution>, String> {
        match &expected_interface.identity {
            TypeRefIr::ServiceSymbol { symbol } => {
                let interface_symbol = SourceSymbolKey::new(
                    symbol
                        .module_path
                        .strip_prefix("root.")
                        .unwrap_or(&symbol.module_path),
                    &symbol.symbol,
                );
                let semantic_conformance = crate::semantic::interface::InterfaceConformanceFact {
                    receiver_type_params: conformance.receiver_type_params.clone(),
                    receiver: TypeInstantiationPattern {
                        symbol: receiver.clone(),
                        args: receiver_args,
                    },
                    interface: InterfaceInstantiation {
                        symbol: interface_symbol,
                        args: expected_interface.args.clone(),
                    },
                };
                let slots = self
                    .interface_semantics
                    .method_slots_for_local_conformance(&semantic_conformance)
                    .map_err(|error| error.to_string())?;
                Ok(Some(LocalAnyInterfaceConformanceResolution {
                    receiver,
                    interface: self
                        .interface_semantics
                        .canonical_interface_instantiation_ref(&semantic_conformance.interface),
                    slots,
                }))
            }
            TypeRefIr::PackageSymbol { .. } => {
                let Some(slots) = self.package_method_slots_for_local_conformance(
                    &receiver,
                    &receiver_args,
                    expected_interface,
                    conformance,
                )?
                else {
                    return Ok(None);
                };
                Ok(Some(LocalAnyInterfaceConformanceResolution {
                    receiver,
                    interface: interface_instantiation_ref(
                        expected_interface.identity.clone(),
                        expected_interface.args.clone(),
                    ),
                    slots,
                }))
            }
            _ => Err(format!(
                "local interface boxing requires a source or package interface selector, found {}",
                type_ref_debug_text(&expected_interface.identity)
            )),
        }
    }

    fn package_method_slots_for_local_conformance(
        &self,
        receiver: &SourceSymbolKey,
        receiver_args: &[TypeRefIr],
        interface: &InterfaceInstantiationResolution,
        conformance: &InterfaceConformanceResolution,
    ) -> Result<Option<Vec<InterfaceMethodSlotFact>>, String> {
        let package_interface = self
            .package_interface_for_type_ref(&interface.identity)
            .ok_or_else(|| {
                format!(
                    "local interface boxing package selector {} does not resolve to a package interface",
                    type_ref_debug_text(&interface.identity)
                )
            })?
            .instantiate_methods(&interface.args)?;
        let canonical =
            interface_instantiation_ref(interface.identity.clone(), interface.args.clone());
        let concrete_self = TypeRefIr::ServiceSymbol {
            symbol: service_symbol_ref_from_source_key(receiver),
        };
        let receiver_substitutions = conformance
            .receiver_type_params
            .iter()
            .cloned()
            .zip(receiver_args.iter().cloned())
            .collect::<BTreeMap<_, _>>();
        let mut slots = Vec::with_capacity(package_interface.methods.len());
        for (slot, method) in package_interface.methods.into_iter().enumerate() {
            let expected_params = interface_method_signature_params(&method)
                .into_iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name,
                    ty: replace_self_type_ref(param.ty, &concrete_self),
                })
                .collect::<Vec<_>>();
            let expected_return = replace_self_type_ref(method.return_type.clone(), &concrete_self);
            let Some(actual_method) = self
                .local_impl_methods
                .get(receiver)
                .and_then(|methods| methods.get(&method.name))
            else {
                return Ok(None);
            };
            if !actual_method.type_params.is_empty() {
                return Ok(None);
            }
            let actual_params = actual_method
                .params
                .iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: substitute_type_params_in_type_ref_ref(&param.ty, &receiver_substitutions),
                })
                .collect::<Vec<_>>();
            let actual_return = substitute_type_params_in_type_ref_ref(
                &actual_method.return_type,
                &receiver_substitutions,
            );
            if actual_params.len() != expected_params.len()
                || actual_params
                    .iter()
                    .zip(&expected_params)
                    .any(|(actual, expected)| {
                        self.canonicalize_impl_signature_type_ref_for_module(
                            receiver.module_path(),
                            &actual.ty,
                        ) != self.canonicalize_impl_signature_type_ref_for_module(
                            receiver.module_path(),
                            &expected.ty,
                        )
                    })
                || self.canonicalize_impl_signature_type_ref_for_module(
                    receiver.module_path(),
                    &actual_return,
                ) != self.canonicalize_impl_signature_type_ref_for_module(
                    receiver.module_path(),
                    &expected_return,
                )
            {
                return Ok(None);
            }
            let method_name = method.name;
            slots.push(InterfaceMethodSlotFact {
                slot: slot as u32,
                method_abi_id: canonical_interface_method_abi_id(&canonical, &method_name),
                name: method_name,
                params: expected_params,
                return_type: expected_return,
            });
        }
        Ok(Some(slots))
    }

    fn canonicalize_impl_signature_type_ref_for_module(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
    ) -> TypeRefIr {
        let context = TypeResolutionContext::source(module_path);
        let transparent = self.transparent_alias_ir(ty, &context);
        self.canonicalize_type_ref_for_module(module_path, &transparent)
    }

    pub(super) fn actual_receiver_symbol(
        &self,
        actual: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Option<SourceSymbolKey> {
        match &actual.ir {
            TypeRefIr::LocalType { type_index } => self
                .local_type_resolution(context.module_path, *type_index)
                .filter(|resolution| matches!(resolution.kind, SourceTypeKind::Record { .. }))
                .map(|resolution| SourceSymbolKey::new(&resolution.module_path, &resolution.name)),
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                let module_path = symbol
                    .module_path
                    .strip_prefix("root.")
                    .unwrap_or(&symbol.module_path);
                let key = SourceSymbolKey::new(module_path, &symbol.symbol);
                self.source_types
                    .get(&key)
                    .filter(|resolution| matches!(resolution.kind, SourceTypeKind::Record { .. }))
                    .map(|_| key)
            }
            _ => None,
        }
    }

    fn receiver_type_param_substitutions(
        &self,
        actual: &ResolvedTypeRef,
        conformance: &InterfaceConformanceResolution,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<BTreeMap<String, TypeRefIr>>, String> {
        let args = self.resolved_named_type_args(&actual.source_text, context)?;
        if args.len() != conformance.receiver_type_params.len() {
            return Ok(None);
        }
        Ok(Some(
            conformance
                .receiver_type_params
                .iter()
                .cloned()
                .zip(args)
                .collect(),
        ))
    }

    fn resolved_named_type_args(
        &self,
        source_text: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Vec<TypeRefIr>, String> {
        let TypeExpr::Named { args, .. } = TypeExpr::parse(source_text) else {
            return Ok(Vec::new());
        };
        args.iter()
            .map(|arg| self.resolve_type_expr(arg, context))
            .collect::<Result<Vec<_>, _>>()
    }

    fn interface_instantiations_match(
        &self,
        implemented: &InterfaceInstantiationResolution,
        expected: &InterfaceInstantiationResolution,
    ) -> bool {
        self.canonicalize_type_ref(&implemented.identity)
            == self.canonicalize_type_ref(&expected.identity)
            && implemented.args.len() == expected.args.len()
            && implemented
                .args
                .iter()
                .zip(&expected.args)
                .all(|(implemented, expected)| {
                    self.canonicalize_type_ref(implemented) == self.canonicalize_type_ref(expected)
                })
    }

    fn contextual_assignable_ir(
        &self,
        actual: &TypeRefIr,
        expected: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
    ) -> bool {
        match expected {
            TypeRefIr::Native { name, .. } if name == "Json" => {
                self.json_assignable_in_context(actual, context)
            }
            TypeRefIr::Native { name, .. } if name == "JsonObject" => {
                self.json_object_assignable_in_context(actual, context)
            }
            TypeRefIr::Nullable { inner } => {
                is_null_type_ir(actual) || self.contextual_assignable_ir(actual, inner, context)
            }
            TypeRefIr::Union { items } => items
                .iter()
                .any(|item| self.contextual_assignable_ir(actual, item, context)),
            _ => false,
        }
    }

    fn json_assignable_in_context(
        &self,
        actual: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
    ) -> bool {
        self.json_assignable_in_context_inner(actual, context, 0)
    }

    fn json_assignable_in_context_inner(
        &self,
        actual: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
        depth: usize,
    ) -> bool {
        if depth > 32 {
            return false;
        }
        match actual {
            TypeRefIr::Native { name, args } => {
                matches!(
                    name.as_str(),
                    "string" | "integer" | "number" | "bool" | "null" | "Json" | "JsonObject"
                ) || name == "Array"
                    && args.len() == 1
                    && self.json_assignable_in_context_inner(&args[0], context, depth + 1)
                    || name == "Map"
                        && args.len() == 2
                        && self.json_assignable_in_context_inner(&args[1], context, depth + 1)
            }
            TypeRefIr::Literal { value } => matches!(
                value,
                LiteralIr::String { .. }
                    | LiteralIr::Number { .. }
                    | LiteralIr::Bool { .. }
                    | LiteralIr::Null
            ),
            TypeRefIr::Record { fields } => fields
                .values()
                .all(|field| self.json_assignable_in_context_inner(field, context, depth + 1)),
            TypeRefIr::Nullable { inner } => {
                self.json_assignable_in_context_inner(inner, context, depth + 1)
            }
            TypeRefIr::Union { items } => items
                .iter()
                .all(|item| self.json_assignable_in_context_inner(item, context, depth + 1)),
            _ => self
                .type_shape_ir(
                    &ResolvedTypeRef {
                        ir: actual.clone(),
                        source_text: type_ref_debug_text(actual),
                    },
                    context,
                )
                .is_some_and(|shape| {
                    self.json_assignable_in_context_inner(&shape, context, depth + 1)
                }),
        }
    }

    fn json_object_assignable_in_context(
        &self,
        actual: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
    ) -> bool {
        self.json_object_assignable_in_context_inner(actual, context, 0)
    }

    fn json_object_assignable_in_context_inner(
        &self,
        actual: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
        depth: usize,
    ) -> bool {
        if depth > 32 {
            return false;
        }
        match actual {
            TypeRefIr::Native { name, .. } if name == "JsonObject" => true,
            TypeRefIr::Record { fields } => fields
                .values()
                .all(|field| self.json_assignable_in_context_inner(field, context, depth + 1)),
            _ => self
                .type_shape_ir(
                    &ResolvedTypeRef {
                        ir: actual.clone(),
                        source_text: type_ref_debug_text(actual),
                    },
                    context,
                )
                .is_some_and(|shape| {
                    self.json_object_assignable_in_context_inner(&shape, context, depth + 1)
                }),
        }
    }

    pub fn externalize_local_type_refs(
        &self,
        ty: &ResolvedTypeRef,
        module_path: &str,
    ) -> ResolvedTypeRef {
        let ir = self.externalize_local_type_ir(&ty.ir, module_path);
        ResolvedTypeRef {
            source_text: type_ref_debug_text(&ir),
            ir,
        }
    }

    pub fn record_field_type(
        &self,
        ty: &ResolvedTypeRef,
        field: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Option<ResolvedTypeRef> {
        if let TypeRefIr::Record { fields } = &ty.ir {
            return fields.get(field).map(|ty| ResolvedTypeRef {
                ir: ty.clone(),
                source_text: type_ref_debug_text(ty),
            });
        }
        if let Some(shape) = self.type_shape_ir(ty, context) {
            if let Some(field_ty) = record_field_type_from_ir(&shape, field) {
                return Some(ResolvedTypeRef {
                    source_text: type_ref_debug_text(&field_ty),
                    ir: field_ty,
                });
            }
        }
        self.resolve_constructor_target_text(&ty.source_text, context)
            .ok()
            .and_then(|target| target.fields.get(field).cloned())
    }

    pub fn type_shape_ir(
        &self,
        ty: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Option<TypeRefIr> {
        match &ty.ir {
            TypeRefIr::Record { .. } | TypeRefIr::Union { .. } => Some(ty.ir.clone()),
            TypeRefIr::LocalType { type_index } => self
                .local_type_resolution(context.module_path, *type_index)
                .and_then(|resolution| {
                    self.source_type_shape_ir(resolution, context, None, context.module_path)
                }),
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                let key = SourceSymbolKey::new(&symbol.module_path, &symbol.symbol);
                self.source_types
                    .get(&key)
                    .and_then(|resolution| {
                        self.source_type_shape_ir(
                            resolution,
                            context,
                            None,
                            symbol.module_path.as_str(),
                        )
                    })
                    .or_else(|| {
                        // A package callable signature may refer to its own module as
                        // `root.<module>.<symbol>`, which resolves to a ServiceSymbol that
                        // is not in this unit's source types. Recover the shape from the
                        // package type index by its internal symbol path.
                        self.package_type_shape_by_symbol_path(
                            &symbol.module_path,
                            &symbol.symbol,
                            context,
                        )
                    })
            }
            TypeRefIr::PackageSymbol { symbol } => {
                if let Some(shape) = self.std_package_symbol_shape_ir(symbol, context) {
                    return Some(shape);
                }

                let dependency_ref = match &symbol.package {
                    PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
                    PackageRefIr::PackageId { package_id } => package_id.as_str(),
                };
                let package_root = package_root_for_symbol(symbol);
                self.package_type_resolution(dependency_ref, &symbol.symbol_path)
                    .and_then(|resolution| {
                        self.source_type_shape_ir(
                            resolution,
                            context,
                            package_root,
                            resolution.module_path.as_str(),
                        )
                    })
            }
            TypeRefIr::Native { name, args } => self
                .prelude_type_shape_ir(name, args, context)
                .or_else(|| self.prelude_type_shape_ir(&ty.source_text, args, context)),
            _ => None,
        }
    }

    fn std_package_symbol_shape_ir(
        &self,
        symbol: &PackageSymbolRef,
        context: &TypeResolutionContext<'_>,
    ) -> Option<TypeRefIr> {
        if !is_std_package_ref(&symbol.package) {
            return None;
        }

        self.prelude_type_shape_ir(&symbol.symbol_path, &[], context)
            .or_else(|| {
                self.prelude_type_shape_ir(&format!("std.{}", symbol.symbol_path), &[], context)
            })
    }

    /// Resolve the shape of a package type that was referenced through a
    /// `root.<module>.<symbol>` (or otherwise package-internal) path that lost its
    /// originating package id. Searches the package type index by internal symbol
    /// path across packages, normalizing public api names toward internal names.
    fn package_type_shape_by_symbol_path(
        &self,
        module_path: &str,
        symbol: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Option<TypeRefIr> {
        let module = module_path.strip_prefix("root.").unwrap_or(module_path);
        let candidate = self.canonical_symbol_path(&format!("{module}.{symbol}"));
        let resolution = self.package_type_by_symbol_path(&candidate)?;
        let package_root = package_root_for_module(&resolution.module_path);
        let source_module = resolution.module_path.clone();
        self.source_type_shape_ir(resolution, context, package_root, source_module.as_str())
    }

    pub(super) fn local_type_resolution(
        &self,
        module_path: &str,
        type_index: u32,
    ) -> Option<&SourceTypeResolution> {
        let symbol = self
            .modules
            .get(module_path)?
            .type_indices
            .iter()
            .find_map(|(symbol, index)| (*index == type_index).then_some(symbol.as_str()))?;
        self.source_types
            .get(&SourceSymbolKey::new(module_path, symbol))
    }

    fn source_type_shape_ir(
        &self,
        resolved: &SourceTypeResolution,
        caller_context: &TypeResolutionContext<'_>,
        package_root: Option<&str>,
        source_module_path: &str,
    ) -> Option<TypeRefIr> {
        let source_context = TypeResolutionContext::with_type_params(
            source_module_path,
            caller_context.type_params.clone(),
        );
        let resolved = match &resolved.kind {
            SourceTypeKind::Record { fields } => TypeRefIr::Record {
                fields: fields
                    .iter()
                    .filter_map(|(name, ty)| {
                        let ty = package_root
                            .map(|package_root| {
                                qualify_package_type_text(
                                    ty,
                                    package_root,
                                    &resolved.local_type_names,
                                )
                            })
                            .unwrap_or_else(|| ty.clone());
                        self.resolve_type_text(&ty, &source_context)
                            .ok()
                            .map(|resolved| (name.clone(), resolved.ir))
                    })
                    .collect(),
            },
            SourceTypeKind::Alias { target } | SourceTypeKind::Representation { target } => {
                let target = package_root
                    .map(|package_root| {
                        qualify_package_type_text(target, package_root, &resolved.local_type_names)
                    })
                    .unwrap_or_else(|| target.clone());
                self.resolve_type_text(&target, &source_context).ok()?.ir
            }
            SourceTypeKind::External => return None,
        };
        Some(if source_module_path == caller_context.module_path {
            resolved
        } else {
            self.externalize_local_type_ir(&resolved, source_module_path)
        })
    }

    fn prelude_type_shape_ir(
        &self,
        type_name: &str,
        args: &[TypeRefIr],
        context: &TypeResolutionContext<'_>,
    ) -> Option<TypeRefIr> {
        let registry = prelude_registry();
        let decl_name = registry.prelude_type_decl_name(type_name)?;
        let decl = registry.type_decl(decl_name)?;
        let module_path = registry.type_decl_module(decl_name)?;
        let source_context =
            TypeResolutionContext::with_type_params(module_path, context.type_params.clone());
        let substitutions = decl
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().map(type_ref_debug_text))
            .collect::<BTreeMap<_, _>>();
        if let Some(alias) = &decl.alias {
            let target = substitute_type_params(&alias.name, &substitutions);
            return self
                .resolve_type_text(&target, &source_context)
                .ok()
                .map(|resolved| resolved.ir);
        }
        Some(TypeRefIr::Record {
            fields: decl
                .fields
                .iter()
                .filter_map(|field| {
                    let ty = substitute_type_params(&field.ty.name, &substitutions);
                    self.resolve_type_text(&ty, &source_context)
                        .ok()
                        .map(|resolved| (field.name.clone(), resolved.ir))
                })
                .collect(),
        })
    }

    fn externalize_local_type_ir(&self, ty: &TypeRefIr, module_path: &str) -> TypeRefIr {
        match ty {
            TypeRefIr::LocalType { type_index } => self
                .modules
                .get(module_path)
                .and_then(|module| {
                    module
                        .type_indices
                        .iter()
                        .find_map(|(symbol, index)| (*index == *type_index).then_some(symbol))
                })
                .map(|symbol| TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: module_path.to_string(),
                        symbol: symbol.clone(),
                    },
                })
                .unwrap_or_else(|| ty.clone()),
            TypeRefIr::Native { name, args } => TypeRefIr::Native {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| self.externalize_local_type_ir(arg, module_path))
                    .collect(),
            },
            TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
                inner: Box::new(self.externalize_local_type_ir(inner, module_path)),
            },
            TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
                interface: skiff_artifact_model::InterfaceInstantiationRef {
                    interface_abi_id: interface.interface_abi_id.clone(),
                    canonical_type_args: interface
                        .canonical_type_args
                        .iter()
                        .map(|arg| self.externalize_local_type_ir(arg, module_path))
                        .collect(),
                },
            },
            TypeRefIr::Union { items } => TypeRefIr::Union {
                items: items
                    .iter()
                    .map(|item| self.externalize_local_type_ir(item, module_path))
                    .collect(),
            },
            TypeRefIr::Record { fields } => TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| {
                        (
                            name.clone(),
                            self.externalize_local_type_ir(ty, module_path),
                        )
                    })
                    .collect(),
            },
            TypeRefIr::Function {
                params,
                return_type,
            } => TypeRefIr::Function {
                params: params
                    .iter()
                    .map(|param| FunctionTypeParamIr {
                        name: param.name.clone(),
                        ty: self.externalize_local_type_ir(&param.ty, module_path),
                    })
                    .collect(),
                return_type: Box::new(self.externalize_local_type_ir(return_type, module_path)),
            },
            TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::PackageSymbol { .. } => ty.clone(),
        }
    }

    fn transparent_alias_ir(
        &self,
        ty: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
    ) -> TypeRefIr {
        match ty {
            TypeRefIr::LocalType { type_index } => self
                .local_type_resolution(context.module_path, *type_index)
                .and_then(|resolution| {
                    self.transparent_source_type_ir(resolution, context, None, context.module_path)
                })
                .unwrap_or_else(|| ty.clone()),
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                let key = SourceSymbolKey::new(&symbol.module_path, &symbol.symbol);
                self.source_types
                    .get(&key)
                    .and_then(|resolution| {
                        self.transparent_source_type_ir(
                            resolution,
                            context,
                            None,
                            symbol.module_path.as_str(),
                        )
                    })
                    .unwrap_or_else(|| ty.clone())
            }
            TypeRefIr::PackageSymbol { symbol } => {
                let dependency_ref = match &symbol.package {
                    PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
                    PackageRefIr::PackageId { package_id } => package_id.as_str(),
                };
                let package_root = package_root_for_symbol(symbol);
                self.package_type_resolution(dependency_ref, &symbol.symbol_path)
                    .and_then(|resolution| {
                        self.transparent_source_type_ir(
                            resolution,
                            context,
                            package_root,
                            resolution.module_path.as_str(),
                        )
                    })
                    .unwrap_or_else(|| ty.clone())
            }
            TypeRefIr::Native { name, args } => TypeRefIr::Native {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| self.transparent_alias_ir(arg, context))
                    .collect(),
            },
            TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
                inner: Box::new(self.transparent_alias_ir(inner, context)),
            },
            TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
                interface: skiff_artifact_model::InterfaceInstantiationRef {
                    interface_abi_id: interface.interface_abi_id.clone(),
                    canonical_type_args: interface
                        .canonical_type_args
                        .iter()
                        .map(|arg| self.transparent_alias_ir(arg, context))
                        .collect(),
                },
            },
            TypeRefIr::Union { items } => union_type_ir(
                items
                    .iter()
                    .map(|item| self.transparent_alias_ir(item, context))
                    .collect(),
            ),
            TypeRefIr::Record { fields } => TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| (name.clone(), self.transparent_alias_ir(ty, context)))
                    .collect(),
            },
            TypeRefIr::Function {
                params,
                return_type,
            } => TypeRefIr::Function {
                params: params
                    .iter()
                    .map(|param| FunctionTypeParamIr {
                        name: param.name.clone(),
                        ty: self.transparent_alias_ir(&param.ty, context),
                    })
                    .collect(),
                return_type: Box::new(self.transparent_alias_ir(return_type, context)),
            },
            TypeRefIr::Literal { .. } | TypeRefIr::TypeParam { .. } => ty.clone(),
        }
    }

    fn transparent_source_type_ir(
        &self,
        resolved: &SourceTypeResolution,
        caller_context: &TypeResolutionContext<'_>,
        package_root: Option<&str>,
        source_module_path: &str,
    ) -> Option<TypeRefIr> {
        let target = match &resolved.kind {
            SourceTypeKind::Alias { target } => target,
            SourceTypeKind::Record { .. }
            | SourceTypeKind::Representation { .. }
            | SourceTypeKind::External => return None,
        };
        let target = package_root
            .map(|package_root| {
                qualify_package_type_text(target, package_root, &resolved.local_type_names)
            })
            .unwrap_or_else(|| target.clone());
        let source_context = TypeResolutionContext::with_type_params(
            source_module_path,
            caller_context.type_params.clone(),
        );
        let resolved_target = self.resolve_type_text(&target, &source_context).ok()?;
        let resolved_target = self.transparent_alias_ir(&resolved_target.ir, &source_context);
        Some(if source_module_path == caller_context.module_path {
            resolved_target
        } else {
            self.externalize_local_type_ir(&resolved_target, source_module_path)
        })
    }
}

fn is_std_package_ref(package: &PackageRefIr) -> bool {
    match package {
        PackageRefIr::PackageId { package_id } => package_id == SKIFF_STD_PUBLICATION_ID,
        PackageRefIr::Dependency { dependency_ref } => dependency_ref == "std",
    }
}

fn replace_self_type_ref(ty: TypeRefIr, concrete_self: &TypeRefIr) -> TypeRefIr {
    if is_self_type_ref(&ty) {
        return concrete_self.clone();
    }
    match ty {
        TypeRefIr::Native { name, args } => TypeRefIr::Native {
            name,
            args: args
                .into_iter()
                .map(|arg| replace_self_type_ref(arg, concrete_self))
                .collect(),
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .into_iter()
                .map(|(name, ty)| (name, replace_self_type_ref(ty, concrete_self)))
                .collect(),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .into_iter()
                .map(|item| replace_self_type_ref(item, concrete_self))
                .collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(replace_self_type_ref(*inner, concrete_self)),
        },
        TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
            interface: skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id,
                canonical_type_args: interface
                    .canonical_type_args
                    .into_iter()
                    .map(|arg| replace_self_type_ref(arg, concrete_self))
                    .collect(),
            },
        },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .into_iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name,
                    ty: replace_self_type_ref(param.ty, concrete_self),
                })
                .collect(),
            return_type: Box::new(replace_self_type_ref(*return_type, concrete_self)),
        },
        TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::LocalType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. } => ty,
    }
}
