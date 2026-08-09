use super::*;

type DependencyParameterExpectation = (String, Result<(ResolvedTypeRef, PackageTypeRef), String>);

impl<'a> OwnerChecker<'a> {
    pub(super) fn call_type(
        &mut self,
        key: &ExpressionKey,
        callee: &Expr,
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Option<ResolvedTypeRef> {
        let (callee, type_args) = match callee {
            Expr::Generic { callee, type_args } => (callee.as_ref(), type_args.as_slice()),
            _ => (callee, &[][..]),
        };
        if let Some(return_type) =
            self.receiver_dispatch_call_type(key, callee, type_args, args, arg_types)
        {
            return Some(return_type);
        }
        let path = expr_path(callee)?;
        if !path
            .split('.')
            .next()
            .is_some_and(|root| self.env.contains_key(root))
        {
            match self.contract_call_type(key, &path, type_args, arg_types) {
                Ok(Some(return_type)) => return Some(return_type),
                Err(()) => return None,
                Ok(None) => {}
            }
        }
        if type_args.is_empty() {
            match self.dependency_package_call_type(key, &path, args, arg_types) {
                Ok(Some(return_type)) => return Some(return_type),
                Err(()) => return None,
                Ok(None) => {}
            }
        }
        if let Some(return_type) = self.config_intrinsic_call_type(&path, type_args) {
            return Some(return_type);
        }
        if path.as_str() == "std.actor.get" {
            return self.actor_registry_intrinsic_call_type(&path, type_args, args, arg_types);
        }
        match self.representation_constructor_call_type(key, &path, type_args, args, arg_types) {
            Ok(Some(return_type)) => return Some(return_type),
            Err(()) => return None,
            Ok(None) => {}
        }
        match self.prelude_native_call_type(&path, type_args, args, arg_types) {
            Ok(Some(return_type)) => return Some(return_type),
            Err(()) => return None,
            Ok(None) => {}
        }
        match self.local_callable_call_type(key, &path, type_args, args, arg_types) {
            Ok(Some(return_type)) => return Some(return_type),
            Err(()) => return None,
            Ok(None) => {}
        }
        match self.package_callable_call_type(key, &path, type_args, args, arg_types) {
            Ok(Some(return_type)) => return Some(return_type),
            Err(()) => return None,
            Ok(None) => {}
        }
        self.known_path_call_type(&path, type_args)
    }

    pub(super) fn receiver_dispatch_call_type(
        &mut self,
        key: &ExpressionKey,
        callee: &Expr,
        type_args: &[TypeRef],
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Option<ResolvedTypeRef> {
        if let Some(return_type) = self.runtime_receiver_call_type(key, callee, args, arg_types) {
            return Some(return_type);
        }
        if let Some(return_type) =
            self.actor_receiver_call_type(key, callee, type_args, args, arg_types)
        {
            return Some(return_type);
        }
        if let Some(return_type) =
            self.any_interface_receiver_call_type(key, callee, type_args, args, arg_types)
        {
            return Some(return_type);
        }
        if let Some(return_type) =
            self.package_interface_receiver_call_type(key, callee, type_args, args, arg_types)
        {
            return Some(return_type);
        }
        if let Some(return_type) =
            self.package_receiver_call_type(key, callee, type_args, args, arg_types)
        {
            return Some(return_type);
        }
        None
    }

    pub(super) fn contract_call_type(
        &mut self,
        key: &ExpressionKey,
        path: &str,
        type_args: &[TypeRef],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Result<Option<ResolvedTypeRef>, ()> {
        let Some(dependency_analysis) = self.dependency_analysis else {
            return Ok(None);
        };
        match ContractCallTyping::new(
            self.type_resolution,
            dependency_analysis,
            &self.type_context,
        )
        .check_call(
            path,
            type_args.len(),
            arg_types,
            self.contract_projection.expression_types(),
        ) {
            ContractCallOutcome::NotContract => Ok(None),
            ContractCallOutcome::Typed {
                return_type,
                projected_return_type,
            } => {
                self.contract_projection
                    .record_expression_type(key.clone(), projected_return_type);
                Ok(Some(*return_type))
            }
            ContractCallOutcome::Invalid(diagnostics) => {
                let location = self.expression_span_label(key);
                self.outputs.diagnostics.extend(
                    diagnostics.into_iter().map(|diagnostic| {
                        format!("{}: {diagnostic} at {location}", self.module_path)
                    }),
                );
                Err(())
            }
        }
    }

    pub(super) fn dependency_package_call_type(
        &mut self,
        key: &ExpressionKey,
        path: &str,
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Result<Option<ResolvedTypeRef>, ()> {
        let signature = self.dependency_analysis.and_then(|dependency_analysis| {
            let (canonical_dependency_ref, callable) =
                dependency_analysis.package_callable_by_source_path(&path)?;
            let type_dependency_ref = dependency_source_address_parts(&path)
                .map(|(dependency_ref, _)| dependency_ref)
                .filter(|dependency_ref| {
                    self.type_resolution
                        .is_top_level_package_dependency_ref(dependency_ref)
                })
                .unwrap_or(canonical_dependency_ref);
            Some((
                type_dependency_ref.to_string(),
                callable.signature()?.clone(),
            ))
        });
        let Some((dependency_ref, signature)) = signature else {
            return Ok(None);
        };
        let canonical_dependency_ref = self
            .type_resolution
            .canonical_package_dependency_ref(&dependency_ref)
            .to_string();
        // Resolve each parameter independently: an owner/slot diagnostic
        // must fail the compile without erasing an exact return fact.
        let expected = signature
            .parameters
            .iter()
            .map(|parameter| {
                (
                    parameter.name.clone(),
                    self.type_resolution
                        .rehydrate_package_signature_type_for_dependency(
                            &canonical_dependency_ref,
                            &parameter.ty,
                        )
                        .or_else(|_| {
                            self.type_resolution
                                .rehydrate_package_signature_type_for_dependency(
                                    &dependency_ref,
                                    &parameter.ty,
                                )
                        })
                        .map(|exact| {
                            let ordinary =
                                self.type_resolution.bind_package_type_refs_to_dependency(
                                    &resolved_package_type_ref(&exact),
                                    &canonical_dependency_ref,
                                );
                            (ordinary, exact)
                        }),
                )
            })
            .collect::<Vec<_>>();
        self.validate_dependency_package_call_params(key, &path, &expected, args, arg_types);

        let exact_projection = match self
            .type_resolution
            .rehydrate_package_signature_type_for_dependency(
                &dependency_ref,
                &signature.return_type,
            ) {
            Ok(return_type) => return_type,
            Err(error) => {
                self.outputs.diagnostics.push(format!(
                    "{}: call `{path}` return dependency type resolution failed at {}: {error}",
                    self.module_path,
                    self.expression_span_label(key),
                ));
                return Err(());
            }
        };
        let resolved_return = self.type_resolution.bind_package_type_refs_to_dependency(
            &resolved_package_type_ref(&exact_projection),
            &dependency_ref,
        );
        let projected_return = self
            .type_resolution
            .rehydrate_package_signature_type_for_dependency(
                &canonical_dependency_ref,
                &signature.return_type,
            )
            .unwrap_or_else(|_| exact_projection.clone());
        self.contract_projection
            .record_expression_type(key.clone(), projected_return);
        Ok(Some(resolved_return))
    }

    pub(super) fn representation_constructor_call_type(
        &mut self,
        key: &ExpressionKey,
        path: &str,
        type_args: &[TypeRef],
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Result<Option<ResolvedTypeRef>, ()> {
        match self.type_resolution.resolve_representation_constructor(
            &path,
            type_args,
            &self.type_context,
        ) {
            Ok(Some(representation)) => {
                self.validate_resolved_call_params(
                    &path,
                    vec![("value".to_string(), representation.payload.clone())],
                    args,
                    arg_types,
                );
                if let Some((payload, _)) = arg_types.first() {
                    self.outputs.representation_constructor_validations.insert(
                        key.clone(),
                        RepresentationConstructorValidation {
                            target: representation.wrapper.clone(),
                            payload: payload.clone(),
                        },
                    );
                }
                Ok(Some(representation.wrapper))
            }
            Ok(None) => Ok(None),
            Err(error) => {
                self.outputs.diagnostics.push(format!(
                    "{}: representation constructor `{path}` failed to resolve: {error}",
                    self.module_path
                ));
                Err(())
            }
        }
    }

    pub(super) fn prelude_native_call_type(
        &mut self,
        path: &str,
        type_args: &[TypeRef],
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Result<Option<ResolvedTypeRef>, ()> {
        let Some(return_type) = prelude_registry().native_return_type(path) else {
            return Ok(None);
        };
        let native_context = native_return_type_context(&path, &self.type_context);
        if let Some(params) = prelude_registry().native_params(&path) {
            let mut expected = self.resolve_callable_param_types(
                &path,
                params.iter().map(String::as_str),
                &native_context,
                prelude_registry().builtin_type_params(&path).unwrap_or(&[]),
                type_args,
            );
            if native_context.module_path != self.module_path {
                expected.params = expected
                    .params
                    .into_iter()
                    .map(|(name, ty)| {
                        (
                            name,
                            self.type_resolution
                                .externalize_local_type_refs(&ty, native_context.module_path),
                        )
                    })
                    .collect();
            }
            if expected.complete {
                self.validate_resolved_call_params(&path, expected.params, args, arg_types);
            }
        }
        let resolved_return_type = self
            .resolve_callable_return_type(
                &return_type,
                &native_context,
                prelude_registry().builtin_type_params(&path).unwrap_or(&[]),
                type_args,
            )
            .ok_or(())?;
        Ok(Some(if native_context.module_path == self.module_path {
            resolved_return_type
        } else {
            self.type_resolution
                .externalize_local_type_refs(&resolved_return_type, native_context.module_path)
        }))
    }

    pub(super) fn local_callable_call_type(
        &mut self,
        key: &ExpressionKey,
        path: &str,
        type_args: &[TypeRef],
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Result<Option<ResolvedTypeRef>, ()> {
        let Some(signature) = self.local_callable_signature(path).cloned() else {
            return Ok(None);
        };
        let signature_context = TypeResolutionContext::with_type_params(
            &signature.module_path,
            signature.type_params.iter().cloned().collect(),
        );
        let type_params = signature.type_params.clone();
        let params = signature.params.clone();
        let return_type = signature.return_type.clone();
        let declaration_name = signature.declaration_name.clone();
        let projected_params = match params
            .iter()
            .map(|param| {
                self.project_callable_package_type(
                    &param.ty,
                    &signature_context,
                    &type_params,
                    type_args,
                )
            })
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(projected) => projected,
            Err(error) => {
                self.outputs.diagnostics.push(format!(
                    "{}: call `{declaration_name}` exact parameter type projection failed: {error}",
                    self.module_path
                ));
                return Err(());
            }
        };
        let mut expected = self.resolve_callable_param_types(
            &declaration_name,
            params.iter().map(|param| param.ty.name.as_str()),
            &signature_context,
            &type_params,
            type_args,
        );
        if signature.module_path != self.module_path {
            expected.params = expected
                .params
                .into_iter()
                .map(|(name, ty)| {
                    (
                        name,
                        self.type_resolution
                            .externalize_local_type_refs(&ty, &signature.module_path),
                    )
                })
                .collect();
        }
        if expected.complete {
            self.validate_resolved_call_params_with_projections(
                &declaration_name,
                expected.params,
                &projected_params,
                args,
                arg_types,
            );
        }
        let projected_return_type = match self.project_callable_package_type(
            &return_type,
            &signature_context,
            &type_params,
            type_args,
        ) {
            Ok(projected) => projected,
            Err(error) => {
                self.outputs.diagnostics.push(format!(
                    "{}: call `{declaration_name}` exact return type projection failed: {error}",
                    self.module_path
                ));
                return Err(());
            }
        };
        let resolved_return_type = self
            .resolve_callable_return_type(
                &return_type.name,
                &signature_context,
                &type_params,
                type_args,
            )
            .ok_or(())?;
        let resolved_return_type = if signature.module_path == self.module_path {
            resolved_return_type
        } else {
            self.type_resolution
                .externalize_local_type_refs(&resolved_return_type, &signature.module_path)
        };
        if let Some(projected_return_type) = projected_return_type {
            self.contract_projection
                .record_expression_type(key.clone(), projected_return_type);
        }
        Ok(Some(resolved_return_type))
    }

    pub(super) fn package_callable_call_type(
        &mut self,
        key: &ExpressionKey,
        path: &str,
        type_args: &[TypeRef],
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Result<Option<ResolvedTypeRef>, ()> {
        let Some(signature) = self.type_resolution.resolve_package_callable(path).cloned() else {
            return Ok(None);
        };
        let package_root = package_callable_public_root(&path, &signature.source_symbol);
        let signature_context = TypeResolutionContext::with_type_params(
            &signature.module_path,
            signature.type_params.iter().cloned().collect(),
        );
        let params = signature
            .params
            .iter()
            .map(|param| {
                qualify_package_signature_type_text(
                    param,
                    &package_root,
                    &signature.local_type_names,
                )
            })
            .collect::<Vec<_>>();
        let expected = self.resolve_callable_param_types(
            &path,
            params.iter().map(String::as_str),
            &signature_context,
            &signature.type_params,
            type_args,
        );
        if expected.complete {
            self.validate_resolved_call_params(&path, expected.params, args, arg_types);
        }
        if let Some(exact_signature) = signature.exact_signature {
            let substitutions = signature
                    .type_params
                    .iter()
                    .zip(type_args)
                    .map(|(param, argument)| {
                        self.project_source_binding_type(argument)
                            .and_then(|projected| {
                                projected.ok_or_else(|| {
                                    format!(
                                        "call `{path}` type argument `{param}` has no exact package projection"
                                    )
                                })
                            })
                            .map(|projected| (param.clone(), projected))
                    })
                    .collect::<Result<BTreeMap<_, _>, _>>();
            let projected_return = match substitutions {
                Ok(substitutions) => {
                    substitute_package_type(&exact_signature.return_type, &substitutions)
                }
                Err(error) => Err(error),
            };
            match projected_return {
                Ok(projected_return) => {
                    let resolved_return = resolved_package_type_ref(&projected_return);
                    self.contract_projection
                        .record_expression_type(key.clone(), projected_return);
                    return Ok(Some(resolved_return));
                }
                Err(error) => {
                    self.outputs.diagnostics.push(format!(
                        "{}: call `{path}` exact return type substitution failed: {error}",
                        self.module_path
                    ));
                    return Err(());
                }
            }
        }
        let package_return_type = qualify_package_signature_type_text(
            &signature.return_type,
            &package_root,
            &signature.local_type_names,
        );
        return Ok(self.resolve_callable_return_type(
            &package_return_type,
            &signature_context,
            &signature.type_params,
            type_args,
        ));
    }

    pub(super) fn known_path_call_type(
        &self,
        path: &str,
        type_args: &[TypeRef],
    ) -> Option<ResolvedTypeRef> {
        match path {
            "db.get" | "db.require" | "db.create" | "db.append" | "db.upsert" => {
                type_args.first().and_then(|ty| {
                    self.type_resolution
                        .resolve_type_ref(ty, &self.type_context)
                        .ok()
                })
            }
            "db.findMany" | "db.createMany" | "db.create_many" | "db.appendMany"
            | "db.append_many" => type_args.first().and_then(|ty| {
                self.type_resolution
                    .resolve_type_ref(ty, &self.type_context)
                    .ok()
                    .map(|item| {
                        let text = format!("Array<{}>", item);
                        ResolvedTypeRef::with_text(
                            TypeRefIr::Builtin {
                                name: BuiltinShape::Array.name().to_string(),
                                args: vec![item.ir],
                            },
                            text,
                        )
                    })
            }),
            "db.exists" => self.resolve_builtin(BuiltinShape::Bool.name()),
            "db.count" => self.resolve_builtin(BuiltinShape::Number.name()),
            _ => None,
        }
    }

    pub(super) fn local_callable_signature(&self, path: &str) -> Option<&CallableSignature> {
        if !path.contains('.') {
            let module_qualified = format!("{}.{}", self.module_path, path);
            if let Some(signature) = self.callable_signatures.get(&module_qualified) {
                return Some(signature);
            }
        }
        self.callable_signatures.get(path).or_else(|| {
            path.strip_prefix("root.")
                .and_then(|source_path| self.callable_signatures.get(source_path))
        })
    }

    pub(super) fn resolve_callable_param_types<'b>(
        &mut self,
        callable: &str,
        params: impl Iterator<Item = &'b str>,
        context: &TypeResolutionContext<'_>,
        type_params: &[String],
        type_args: &[TypeRef],
    ) -> ResolvedCallableParams {
        let mut complete = true;
        let params = params
            .enumerate()
            .filter_map(|(index, raw)| {
                match self.resolve_callable_signature_type(raw, context, type_params, type_args) {
                    Some(resolved) => Some((format!("arg{index}"), resolved)),
                    None => {
                        let _ = callable;
                        complete = false;
                        None
                    }
                }
            })
            .collect();
        ResolvedCallableParams { params, complete }
    }

    pub(super) fn resolve_callable_return_type(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
        type_params: &[String],
        type_args: &[TypeRef],
    ) -> Option<ResolvedTypeRef> {
        self.resolve_callable_signature_type(raw, context, type_params, type_args)
    }

    pub(super) fn resolve_callable_signature_type(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
        type_params: &[String],
        type_args: &[TypeRef],
    ) -> Option<ResolvedTypeRef> {
        self.exact_type_arg_substitution(raw, type_params, type_args)
            .or_else(|| self.structured_type_arg_substitution(raw, context, type_params, type_args))
            .or_else(|| {
                // Omitted generic arguments can still leave a declaration type
                // concrete when the type does not depend on any type parameter.
                (type_params.is_empty() || type_args.is_empty())
                    .then(|| self.type_resolution.resolve_type_text(raw, context).ok())
                    .flatten()
                    .filter(|resolved| !contains_type_param(&resolved.ir))
            })
    }

    pub(super) fn exact_type_arg_substitution(
        &self,
        raw: &str,
        type_params: &[String],
        type_args: &[TypeRef],
    ) -> Option<ResolvedTypeRef> {
        let raw = raw.trim();
        let index = type_params.iter().position(|param| param == raw)?;
        let arg = type_args.get(index)?;
        self.type_resolution
            .resolve_type_ref(arg, &self.type_context)
            .ok()
    }

    pub(super) fn structured_type_arg_substitution(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
        type_params: &[String],
        type_args: &[TypeRef],
    ) -> Option<ResolvedTypeRef> {
        if type_params.is_empty() || type_params.len() != type_args.len() {
            return None;
        }
        let generic_context = TypeResolutionContext::with_type_params(
            context.module_path,
            type_params.iter().cloned().collect(),
        );
        let generic = self
            .type_resolution
            .resolve_type_text(raw, &generic_context)
            .ok()?;
        let substitutions = type_params
            .iter()
            .zip(type_args)
            .map(|(param, argument)| {
                self.type_resolution
                    .resolve_type_ref(argument, &self.type_context)
                    .map(|resolved| (param.clone(), resolved.ir))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()
            .ok()?;
        Some(resolved_type_from_ir(&substitute_type_params_in_ir(
            &generic.ir,
            &substitutions,
        )))
    }

    pub(super) fn project_callable_package_type(
        &self,
        raw: &TypeRef,
        context: &TypeResolutionContext<'_>,
        type_params: &[String],
        type_args: &[TypeRef],
    ) -> Result<Option<PackageTypeRef>, String> {
        let Some(dependency_analysis) = self.dependency_analysis else {
            return Ok(None);
        };
        let projected = ContractProjectionState::project_source_type_ref(
            raw,
            self.type_resolution,
            dependency_analysis,
            context,
        )?;
        let substitutions = type_params
            .iter()
            .zip(type_args)
            .map(|(param, argument)| {
                Ok((
                    param.clone(),
                    ContractProjectionState::project_source_type_ref(
                        argument,
                        self.type_resolution,
                        dependency_analysis,
                        &self.type_context,
                    )?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        substitute_package_type(&projected, &substitutions).map(Some)
    }

    pub(super) fn resolve_type_arg_substitutions(
        &mut self,
        callable: &str,
        type_params: &[String],
        type_args: &[TypeRef],
    ) -> ResolvedTypeArgSubstitutions {
        if type_args.len() > type_params.len() {
            self.outputs.diagnostics.push(format!(
                "{}: call `{callable}` type arity mismatch: expected {} type arguments, found {}",
                self.module_path,
                type_params.len(),
                type_args.len()
            ));
        }
        let mut complete = true;
        let mut types = BTreeMap::new();
        for (param, arg) in type_params.iter().zip(type_args) {
            match self
                .type_resolution
                .resolve_type_ref(arg, &self.type_context)
            {
                Ok(resolved) => {
                    types.insert(param.clone(), resolved.ir);
                }
                Err(_) => complete = false,
            }
        }
        ResolvedTypeArgSubstitutions { types, complete }
    }

    pub(super) fn validate_resolved_call_params(
        &mut self,
        callable: &str,
        expected: Vec<(String, ResolvedTypeRef)>,
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) {
        self.validate_resolved_call_params_with_projections(
            callable,
            expected,
            &[],
            args,
            arg_types,
        );
    }

    pub(super) fn validate_resolved_call_params_with_projections(
        &mut self,
        callable: &str,
        expected: Vec<(String, ResolvedTypeRef)>,
        exact_expected: &[Option<PackageTypeRef>],
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) {
        if expected.len() != args.len() {
            self.outputs.diagnostics.push(format!(
                "{}: call `{callable}` arity mismatch: expected {} arguments, found {}",
                self.module_path,
                expected.len(),
                args.len()
            ));
        }
        for (index, ((_, expected), (key, actual))) in expected.iter().zip(arg_types).enumerate() {
            let Some(actual) = actual else {
                continue;
            };
            if contains_type_param(&expected.ir) || contains_type_param(&actual.ir) {
                continue;
            }
            let context = format!("call `{callable}` argument {}", index + 1);
            self.check_value_assignable_to_expected(
                args[index].expr(),
                key,
                actual,
                expected,
                ValueAssignmentContext {
                    annotation: None,
                    exact_expected: exact_expected.get(index).and_then(Option::as_ref),
                    diagnostic_context: &context,
                    fallback_span: self.expression_span(key),
                },
            );
        }
    }

    pub(super) fn validate_dependency_package_call_params(
        &mut self,
        call_key: &ExpressionKey,
        callable: &str,
        expected: &[DependencyParameterExpectation],
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) {
        if expected.len() != args.len() {
            self.outputs.diagnostics.push(format!(
                "{}: call `{callable}` arity mismatch: expected {} arguments, found {}",
                self.module_path,
                expected.len(),
                args.len()
            ));
        }
        for (index, (name, expected)) in expected.iter().enumerate() {
            let (expected, exact_expected) = match expected {
                Ok(expected) => expected,
                Err(error) => {
                    self.outputs.diagnostics.push(format!(
                        "{}: call `{callable}` parameter {} `{name}` dependency type resolution failed at {}: {error}",
                        self.module_path,
                        index + 1,
                        self.expression_span_label(call_key),
                    ));
                    continue;
                }
            };
            let Some((key, actual)) = arg_types.get(index) else {
                continue;
            };
            let Some(actual) = actual else {
                continue;
            };
            if contains_type_param(&expected.ir) || contains_type_param(&actual.ir) {
                continue;
            }
            let context = format!("call `{callable}` argument {}", index + 1);
            self.check_value_assignable_to_expected(
                args[index].expr(),
                key,
                actual,
                expected,
                ValueAssignmentContext {
                    annotation: None,
                    exact_expected: Some(exact_expected),
                    diagnostic_context: &context,
                    fallback_span: self.expression_span(key),
                },
            );
        }
    }

    pub(super) fn config_intrinsic_call_type(
        &self,
        path: &str,
        type_args: &[TypeRef],
    ) -> Option<ResolvedTypeRef> {
        match path {
            "config.require" => type_args.first().and_then(|ty| {
                self.type_resolution
                    .resolve_type_ref(ty, &self.type_context)
                    .ok()
            }),
            "config.optional" => type_args
                .first()
                .and_then(|ty| {
                    self.type_resolution
                        .resolve_type_ref(ty, &self.type_context)
                        .ok()
                })
                .map(nullable_type),
            "config.has" => self.resolve_builtin(BuiltinShape::Bool.name()),
            _ => None,
        }
    }

    pub(super) fn runtime_receiver_call_type(
        &mut self,
        key: &ExpressionKey,
        callee: &Expr,
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Option<ResolvedTypeRef> {
        let (_, method_name) = receiver_call_parts(callee)?;
        let offset = 1 + receiver_object_offset_in_callee(callee)?;
        let receiver_ty = self.expression_type_at_offset(key, offset)?;
        let return_type = builtin_receiver_call_return_type(&receiver_ty, method_name)?;
        let receiver_root = runtime_receiver_root_from_type_ref(&receiver_ty.ir);
        if receiver_root.as_deref() == Some(BuiltinShape::Array.name()) && method_name == "push" {
            self.validate_array_push_args(&receiver_ty, args, arg_types);
        }
        if receiver_root.as_deref() == Some(BuiltinShape::String.name())
            && method_name == "contains"
        {
            self.validate_resolved_call_params(
                "string.contains",
                vec![(
                    "needle".to_string(),
                    resolved_type_from_ir(&builtin_type(BuiltinShape::String.name())),
                )],
                args,
                arg_types,
            );
        }
        if receiver_root.as_deref() == Some(BuiltinShape::JsonObject.name()) {
            match method_name {
                "get" | "has" | "delete" => self.validate_resolved_call_params(
                    &format!("JsonObject.{method_name}"),
                    vec![(
                        "field".to_string(),
                        resolved_type_from_ir(&builtin_type(BuiltinShape::String.name())),
                    )],
                    args,
                    arg_types,
                ),
                "set" => self.validate_resolved_call_params(
                    "JsonObject.set",
                    vec![
                        (
                            "field".to_string(),
                            resolved_type_from_ir(&builtin_type(BuiltinShape::String.name())),
                        ),
                        (
                            "value".to_string(),
                            resolved_type_from_ir(&builtin_type(BuiltinShape::Json.name())),
                        ),
                    ],
                    args,
                    arg_types,
                ),
                _ => {}
            }
        }
        if receiver_root.as_deref() == Some(BuiltinShape::Map.name())
            && matches!(method_name, "has" | "set")
        {
            self.validate_map_has_or_set_args(&receiver_ty, method_name, args, arg_types);
        }
        if receiver_root.as_deref() == Some("bytes") && method_name == "toHex" {
            self.validate_resolved_call_params("bytes.toHex", Vec::new(), args, arg_types);
        }
        if let Some(projected) =
            self.expression_projection_at_offset(key, offset)
                .and_then(|receiver| {
                    builtin_receiver_call_return_projection(&receiver_ty, receiver, method_name)
                })
        {
            self.contract_projection
                .record_expression_type(key.clone(), projected);
        }
        Some(return_type)
    }

    pub(super) fn actor_receiver_call_type(
        &mut self,
        key: &ExpressionKey,
        callee: &Expr,
        type_args: &[TypeRef],
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Option<ResolvedTypeRef> {
        let (_, method_name) = receiver_call_parts(callee)?;
        let offset = 1 + receiver_object_offset_in_callee(callee)?;
        let receiver_ty = self.expression_type_at_offset(key, offset)?;
        let (params, return_type) = self.type_resolution.actor_method_signature(
            &receiver_ty,
            method_name,
            &self.type_context,
        )?;
        let callable = format!("{}.{}", receiver_ty, method_name);
        if !type_args.is_empty() {
            self.outputs.diagnostics.push(format!(
                "{}: actor method `{callable}` does not accept explicit method type arguments",
                self.module_path
            ));
        }
        let params = params
            .iter()
            .skip(usize::from(
                params.first().is_some_and(|param| param.name == "self"),
            ))
            .enumerate()
            .map(|(index, param)| {
                (
                    format!("arg{index}"),
                    ResolvedTypeRef::new(param.ty.clone()),
                )
            })
            .collect();
        self.validate_resolved_call_params(&callable, params, args, arg_types);
        Some(ResolvedTypeRef::new(return_type))
    }

    pub(super) fn actor_registry_intrinsic_call_type(
        &mut self,
        path: &str,
        type_args: &[TypeRef],
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Option<ResolvedTypeRef> {
        if type_args.len() != 1 {
            self.outputs.diagnostics.push(format!(
                "{}: actor registry intrinsic `{path}` expects exactly one actor type argument, found {}",
                self.module_path,
                type_args.len()
            ));
            return None;
        }
        let actor_ty = match self
            .type_resolution
            .resolve_type_ref(&type_args[0], &self.type_context)
        {
            Ok(actor_ty) => actor_ty,
            Err(error) => {
                self.outputs.diagnostics.push(format!(
                    "{}: actor registry intrinsic `{path}` has unresolved actor type: {error}",
                    self.module_path
                ));
                return None;
            }
        };
        let Some(actor) = self
            .type_resolution
            .actor_type_resolution(&actor_ty, &self.type_context)
        else {
            self.outputs.diagnostics.push(format!(
                "{}: actor registry intrinsic `{path}` type argument `{}` is not an actor declaration",
                self.module_path, actor_ty
            ));
            return None;
        };
        let create_params = actor.create.clone().unwrap_or_default();
        let expected_arity = create_params.len() + 1;
        if args.len() != expected_arity {
            self.outputs.diagnostics.push(format!(
                "{}: actor registry intrinsic `{path}` expects id and {} create argument(s), found {}",
                self.module_path,
                create_params.len(),
                args.len()
            ));
        } else {
            let mut params = vec![("id".to_string(), actor.id_type.clone())];
            params.extend(create_params);
            self.validate_resolved_call_params(path, params, args, arg_types);
        }
        Some(actor.ty)
    }

    pub(super) fn validate_array_push_args(
        &mut self,
        receiver_ty: &ResolvedTypeRef,
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) {
        let Some(expected) =
            array_item_type_ir(&receiver_ty.ir).map(|ty| resolved_type_from_ir(&ty))
        else {
            return;
        };
        if args.len() != 1 {
            self.outputs.diagnostics.push(format!(
                "{}: call `Array.push` arity mismatch: expected 1 arguments, found {}",
                self.module_path,
                args.len()
            ));
            return;
        }
        let Some((key, Some(actual))) = arg_types.first() else {
            return;
        };
        self.check_value_assignable_to_expected(
            args[0].expr(),
            key,
            actual,
            &expected,
            ValueAssignmentContext {
                annotation: None,
                exact_expected: None,
                diagnostic_context: "call `Array.push` argument 1",
                fallback_span: self.expression_span(key),
            },
        );
    }

    pub(super) fn validate_map_has_or_set_args(
        &mut self,
        receiver_ty: &ResolvedTypeRef,
        method_name: &str,
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) {
        let Some(key_ty) = map_key_type_ir(&receiver_ty.ir).map(|ty| resolved_type_from_ir(&ty))
        else {
            return;
        };
        let mut params = vec![("key".to_string(), key_ty)];
        if method_name == "set" {
            let Some(value_ty) =
                map_value_type_ir(&receiver_ty.ir).map(|ty| resolved_type_from_ir(&ty))
            else {
                return;
            };
            params.push(("value".to_string(), value_ty));
        }
        self.validate_resolved_call_params(&format!("Map.{method_name}"), params, args, arg_types);
    }

    pub(super) fn any_interface_receiver_call_type(
        &mut self,
        key: &ExpressionKey,
        callee: &Expr,
        type_args: &[TypeRef],
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Option<ResolvedTypeRef> {
        let (_, method_name) = receiver_call_parts(callee)?;
        let offset = 1 + receiver_object_offset_in_callee(callee)?;
        let receiver_ty = self.expression_type_at_offset(key, offset)?;
        let operation = self
            .type_resolution
            .any_interface_method_signature(&receiver_ty.ir, method_name)?;
        let callable = format!("{}.{}", receiver_ty, method_name);
        if !type_args.is_empty() {
            self.outputs.diagnostics.push(format!(
                "{}: any interface method `{callable}` does not accept method type arguments",
                self.module_path
            ));
        }
        let params = operation
            .params
            .iter()
            .skip(usize::from(
                operation
                    .params
                    .first()
                    .is_some_and(|param| param.name == "self"),
            ))
            .enumerate()
            .map(|(index, param)| {
                (
                    format!("arg{index}"),
                    ResolvedTypeRef::new(param.ty.clone()),
                )
            })
            .collect();
        self.validate_resolved_call_params(&callable, params, args, arg_types);
        Some(ResolvedTypeRef::new(operation.return_type))
    }

    pub(super) fn package_interface_receiver_call_type(
        &mut self,
        key: &ExpressionKey,
        callee: &Expr,
        type_args: &[TypeRef],
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Option<ResolvedTypeRef> {
        let (_, method_name) = receiver_call_parts(callee)?;
        let offset = 1 + receiver_object_offset_in_callee(callee)?;
        let receiver_ty = self.expression_type_at_offset(key, offset)?;
        let interface = self
            .type_resolution
            .package_interface_for_type_ref(&receiver_ty.ir)?;
        let operation = interface
            .methods
            .iter()
            .find(|operation| operation.name == method_name)
            .cloned()?;
        let callable = format!("{}.{}", receiver_ty, method_name);
        let substitutions =
            self.resolve_type_arg_substitutions(&callable, &operation.type_params, type_args);
        if substitutions.complete {
            let params = operation
                .params
                .iter()
                .skip(usize::from(
                    operation
                        .params
                        .first()
                        .is_some_and(|param| param.name == "self"),
                ))
                .enumerate()
                .map(|(index, param)| {
                    let ty = substitute_type_params_in_ir(&param.ty, &substitutions.types);
                    (format!("arg{index}"), ResolvedTypeRef::new(ty))
                })
                .collect();
            self.validate_resolved_call_params(&callable, params, args, arg_types);
        }
        let return_type =
            substitute_type_params_in_ir(&operation.return_type, &substitutions.types);
        Some(ResolvedTypeRef::new(return_type))
    }

    pub(super) fn package_receiver_call_type(
        &mut self,
        key: &ExpressionKey,
        callee: &Expr,
        type_args: &[TypeRef],
        args: &[CallArg],
        arg_types: &[(ExpressionKey, Option<ResolvedTypeRef>)],
    ) -> Option<ResolvedTypeRef> {
        let (_, method_name) = receiver_call_parts(callee)?;
        let offset = 1 + receiver_object_offset_in_callee(callee)?;
        let receiver_key = ExpressionKey::new(
            key.module_path().to_string(),
            key.owner().clone(),
            key.preorder_index().checked_add(offset)?,
        );
        let receiver_ty = self.expression_type_at_offset(key, offset)?;
        let receiver_method = self
            .type_resolution
            .package_receiver_method_resolution(&receiver_ty.ir, method_name)?;
        let source_path = format!(
            "{}/{}",
            receiver_method.dependency_ref, receiver_method.source_method_path
        );
        let dependency_analysis = self.dependency_analysis?;
        let Some((canonical_dependency_ref, callable)) =
            dependency_analysis.package_callable_by_source_path(&source_path)
        else {
            self.outputs.diagnostics.push(format!(
                "{}: package receiver method `{source_path}` has no exact callable implementation member at {}",
                self.module_path,
                self.expression_span_label(key)
            ));
            return None;
        };
        if canonical_dependency_ref != receiver_method.canonical_dependency_ref {
            self.outputs.diagnostics.push(format!(
                "{}: package receiver method `{source_path}` resolves to dependency `{canonical_dependency_ref}` instead of `{}`",
                self.module_path, receiver_method.canonical_dependency_ref
            ));
            return None;
        }
        let signature = callable.signature()?.clone();
        let receiver_param_count = receiver_method.receiver_type_params.len();
        if signature
            .parameters
            .first()
            .map(|parameter| parameter.name.as_str())
            != Some("self")
            || signature.type_params.len() < receiver_param_count
            || signature.type_params.len() - receiver_param_count != type_args.len()
        {
            self.outputs.diagnostics.push(format!(
                "{}: package receiver method `{source_path}` has an invalid receiver/generic signature",
                self.module_path
            ));
            return None;
        }
        let mut substitutions = signature
            .type_params
            .iter()
            .take(receiver_param_count)
            .cloned()
            .zip(
                receiver_method
                    .receiver_type_arguments
                    .iter()
                    .cloned()
                    .map(|local_type| PackageTypeRef::Local { local_type }),
            )
            .collect::<BTreeMap<_, _>>();
        for (type_param, type_arg) in signature
            .type_params
            .iter()
            .skip(receiver_param_count)
            .zip(type_args)
        {
            let projected = match self.project_source_binding_type(type_arg) {
                Ok(Some(projected)) => projected,
                Ok(None) => {
                    self.outputs.diagnostics.push(format!(
                        "{}: package receiver method `{source_path}` type argument `{type_param}` has no exact package projection",
                        self.module_path
                    ));
                    return None;
                }
                Err(error) => {
                    self.outputs.diagnostics.push(format!(
                        "{}: package receiver method `{source_path}` type argument `{type_param}` projection failed: {error}",
                        self.module_path
                    ));
                    return None;
                }
            };
            substitutions.insert(type_param.clone(), projected);
        }

        let exact_parameters = signature
            .parameters
            .iter()
            .map(|parameter| {
                substitute_package_type(&parameter.ty, &substitutions).and_then(|ty| {
                    self.type_resolution
                        .rehydrate_package_signature_type_for_dependency(
                            &receiver_method.dependency_ref,
                            &ty,
                        )
                        .map(|exact| (parameter.name.clone(), exact))
                })
            })
            .collect::<Result<Vec<_>, _>>();
        let exact_parameters = match exact_parameters {
            Ok(parameters) => parameters,
            Err(error) => {
                self.outputs.diagnostics.push(format!(
                    "{}: package receiver method `{source_path}` parameter substitution failed at {}: {error}",
                    self.module_path,
                    self.expression_span_label(key)
                ));
                return None;
            }
        };
        let expected_receiver = self.type_resolution.bind_package_type_refs_to_dependency(
            &resolved_package_type_ref(&exact_parameters[0].1),
            &receiver_method.dependency_ref,
        );
        if !self.type_resolution.assignable_in_context(
            &receiver_ty,
            &expected_receiver,
            &self.type_context,
        ) {
            self.outputs.diagnostics.push(format!(
                "{}: package receiver method `{source_path}` receiver type mismatch at {}: expected {}, found {}",
                self.module_path,
                self.expression_span_label(&receiver_key),
                expected_receiver,
                receiver_ty
            ));
            return None;
        }
        let expected = exact_parameters
            .iter()
            .skip(1)
            .map(|(name, exact)| {
                (
                    name.clone(),
                    Ok((
                        self.type_resolution.bind_package_type_refs_to_dependency(
                            &resolved_package_type_ref(exact),
                            &receiver_method.dependency_ref,
                        ),
                        exact.clone(),
                    )),
                )
            })
            .collect::<Vec<_>>();
        self.validate_dependency_package_call_params(key, &source_path, &expected, args, arg_types);

        let exact_return = match substitute_package_type(&signature.return_type, &substitutions)
            .and_then(|ty| {
                self.type_resolution
                    .rehydrate_package_signature_type_for_dependency(
                        &receiver_method.dependency_ref,
                        &ty,
                    )
            }) {
            Ok(return_type) => return_type,
            Err(error) => {
                self.outputs.diagnostics.push(format!(
                    "{}: package receiver method `{source_path}` return substitution failed at {}: {error}",
                    self.module_path,
                    self.expression_span_label(key)
                ));
                return None;
            }
        };
        let resolved_return = self.type_resolution.bind_package_type_refs_to_dependency(
            &resolved_package_type_ref(&exact_return),
            &receiver_method.dependency_ref,
        );
        self.contract_projection
            .record_expression_type(key.clone(), exact_return);
        Some(resolved_return)
    }
}

fn package_callable_public_root(path: &str, source_symbol: &str) -> String {
    let suffix = format!(".{source_symbol}");
    if let Some(root) = path.strip_suffix(&suffix) {
        return root.to_string();
    }
    path.split('.').next().unwrap_or(path).to_string()
}

fn native_return_type_context<'a>(
    path: &'a str,
    fallback: &TypeResolutionContext<'a>,
) -> TypeResolutionContext<'a> {
    path.rsplit_once('.')
        .and_then(|(owner, _)| {
            prelude_registry()
                .type_decl_module(owner)
                .or_else(|| (!prelude_registry().is_prelude_type_name(owner)).then_some(owner))
        })
        .map(|module_path| {
            TypeResolutionContext::with_type_params(module_path, fallback.type_params.clone())
        })
        .unwrap_or_else(|| {
            TypeResolutionContext::with_type_params(
                fallback.module_path,
                fallback.type_params.clone(),
            )
        })
}

fn receiver_call_parts(expr: &Expr) -> Option<(&Expr, &str)> {
    match expr {
        Expr::Field { object, field } => Some((object, field)),
        Expr::Generic { callee, .. } => receiver_call_parts(callee),
        _ => None,
    }
}

fn receiver_object_offset_in_callee(expr: &Expr) -> Option<u32> {
    match expr {
        Expr::Field { .. } => Some(1),
        Expr::Generic { callee, .. } => receiver_object_offset_in_callee(callee).map(|offset| {
            offset
                .checked_add(1)
                .expect("receiver expression preorder offset should fit in u32")
        }),
        _ => None,
    }
}

fn builtin_receiver_call_return_type(
    receiver_ty: &ResolvedTypeRef,
    method_name: &str,
) -> Option<ResolvedTypeRef> {
    let root = runtime_receiver_root_from_type_ref(&receiver_ty.ir)?;
    let spec = builtin_receiver_op_spec_by_name(&root, method_name)?;
    let ty = match spec.public_return_type {
        BuiltinReceiverPublicReturnType::Fixed(name) => builtin_type(name),
        BuiltinReceiverPublicReturnType::Receiver => receiver_ty.ir.clone(),
        BuiltinReceiverPublicReturnType::ArrayItem => array_item_type_ir(&receiver_ty.ir)?,
        BuiltinReceiverPublicReturnType::MapValue => map_value_type_ir(&receiver_ty.ir)?,
        BuiltinReceiverPublicReturnType::MapKeyArray => TypeRefIr::Builtin {
            name: BuiltinShape::Array.name().to_string(),
            args: vec![map_key_type_ir(&receiver_ty.ir)?],
        },
    };
    Some(resolved_type_from_ir(&ty))
}

fn builtin_receiver_call_return_projection(
    receiver_ty: &ResolvedTypeRef,
    receiver_projection: &PackageTypeRef,
    method_name: &str,
) -> Option<PackageTypeRef> {
    let root = runtime_receiver_root_from_type_ref(&receiver_ty.ir)?;
    let spec = builtin_receiver_op_spec_by_name(&root, method_name)?;
    match spec.public_return_type {
        BuiltinReceiverPublicReturnType::Fixed(name) => Some(PackageTypeRef::Container {
            name: name.to_string(),
            arguments: Vec::new(),
        }),
        BuiltinReceiverPublicReturnType::Receiver => Some(receiver_projection.clone()),
        BuiltinReceiverPublicReturnType::ArrayItem => {
            let PackageTypeRef::Container { arguments, .. } = receiver_projection else {
                return None;
            };
            (arguments.len() == 1).then(|| arguments[0].clone())
        }
        BuiltinReceiverPublicReturnType::MapValue => {
            let PackageTypeRef::Container { arguments, .. } = receiver_projection else {
                return None;
            };
            (arguments.len() == 2).then(|| arguments[1].clone())
        }
        BuiltinReceiverPublicReturnType::MapKeyArray => {
            let PackageTypeRef::Container { arguments, .. } = receiver_projection else {
                return None;
            };
            (arguments.len() == 2).then(|| PackageTypeRef::Container {
                name: BuiltinShape::Array.name().to_string(),
                arguments: vec![arguments[0].clone()],
            })
        }
    }
}

pub fn runtime_receiver_root_from_type_ref(ty: &TypeRefIr) -> Option<String> {
    match ty {
        TypeRefIr::Builtin { name, .. } => Some(canonical_runtime_receiver_root(name).to_string()),
        TypeRefIr::PackageSymbol { symbol } if is_official_std_package_ref(&symbol.package) => {
            Some(canonical_runtime_receiver_root(&symbol.symbol_path).to_string())
        }
        TypeRefIr::ServiceSymbol { symbol }
            if prelude_registry()
                .known_type_symbol(&format!("{}.{}", symbol.module_path, symbol.symbol))
                == Some(format!("{}.{}", symbol.module_path, symbol.symbol)) =>
        {
            Some(
                canonical_runtime_receiver_root(&format!(
                    "{}.{}",
                    symbol.module_path, symbol.symbol
                ))
                .to_string(),
            )
        }
        TypeRefIr::Literal {
            value: LiteralIr::String { .. },
        } => Some(BuiltinShape::String.name().to_string()),
        TypeRefIr::Literal {
            value: LiteralIr::Number { .. },
        } => Some(BuiltinShape::Number.name().to_string()),
        TypeRefIr::Nullable { inner } => runtime_receiver_root_from_type_ref(inner),
        _ => None,
    }
}

fn is_official_std_package_ref(package: &PackageRefIr) -> bool {
    match package {
        PackageRefIr::PackageId { package_id } => {
            package_id == crate::shared::id::SKIFF_STD_PUBLICATION_ID
        }
        PackageRefIr::Dependency { dependency_ref } => dependency_ref == "std",
    }
}

fn canonical_runtime_receiver_root(root: &str) -> &str {
    skiff_artifact_model::canonical_runtime_receiver_root(root)
}

fn qualify_package_signature_type_text(
    raw: &str,
    package_root: &str,
    local_type_names: &BTreeSet<String>,
) -> String {
    TypeExpr::parse(raw)
        .map_named_types(|name| {
            if local_type_names.contains(name) {
                format!("{package_root}.{name}")
            } else {
                name.to_string()
            }
        })
        .to_type_string()
}
