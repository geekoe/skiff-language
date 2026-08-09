use super::*;
use super::{IMPORT_NAME_RULE, LEGACY_PROVIDER_REMOVED_MESSAGE};

impl Parser {
    pub(super) fn parse_import(&mut self) -> Result<ImportDecl> {
        let start = self.expect_ident_value("import")?.span.start;
        let name = self.expect_ident(IMPORT_NAME_RULE)?;
        if !self.import_tail_is_terminated() {
            return Err(CompileError::syntax(IMPORT_NAME_RULE, start));
        }
        let package = Some(match name.as_str() {
            "std" => PackageId::Builtin {
                name: BuiltinPackage::Std,
            },
            "ext" => PackageId::Builtin {
                name: BuiltinPackage::Ext,
            },
            _ => PackageId::Simple { name: name.clone() },
        });
        let path = vec![name.clone()];
        let local_binding = Some(name);

        self.match_symbol(";");
        let end = self.previous().span.end;
        Ok(ImportDecl {
            path,
            alias: None,
            package,
            local_binding,
            span: SourceSpan { start, end },
        })
    }

    pub(super) fn parse_type_decl(&mut self, exported: bool) -> Result<TypeDecl> {
        let start = self.expect_ident_value("type")?.span.start;
        let name = self.expect_ident("expected type name")?;
        let type_params = if self.check_symbol("<") {
            self.parse_type_param_names()?
        } else {
            Vec::new()
        };
        let discriminator = if self.match_ident("discriminator") {
            let value = self.expect_string("expected discriminator field string")?;
            if value.is_empty() {
                return Err(CompileError::syntax(
                    "discriminator field cannot be empty",
                    self.previous().span.start,
                ));
            }
            Some(value)
        } else {
            None
        };
        let mut alias = None;
        let mut implements = Vec::new();
        let mut fields = Vec::new();
        let mut spreads = Vec::new();

        if discriminator.is_some() && !self.check_symbol("=") {
            return Err(CompileError::syntax(
                "discriminator can only be used on type representation declarations",
                self.peek().span.start,
            ));
        }

        if self.match_symbol("=") {
            let target_type = self.parse_type()?;
            validate::validate_type_decl_discriminator(
                &name,
                &target_type.name,
                discriminator.as_deref(),
                start,
            )?;
            alias = Some(target_type);
        } else {
            if self.match_ident("implements") {
                loop {
                    let implemented = self.parse_type()?;
                    let implemented_root = generic_parts(&implemented.name)
                        .map_or(implemented.name.as_str(), |parts| parts.root);
                    if matches!(
                        implemented_root,
                        "Actor" | "actor.Actor" | "std.actor.Actor"
                    ) {
                        return Err(CompileError::syntax(
                            "actor declarations must use `actor Name { key(field) ... }` attached to a same-file type, not `type implements Actor`",
                            start,
                        ));
                    }
                    implements.push(implemented);
                    if !self.match_symbol(",") {
                        break;
                    }
                }
            }
            if self.check_symbol("{") {
                (fields, spreads) = self.parse_field_block()?;
            } else if implements.is_empty() {
                return Err(CompileError::syntax(
                    "expected =, implements, or record body in type declaration",
                    self.peek().span.start,
                ));
            }
        }
        self.match_symbol(";");
        let end = self.previous().span.end;
        Ok(TypeDecl {
            exported,
            name,
            type_params,
            discriminator,
            alias,
            implements,
            fields,
            spreads,
            span: SourceSpan { start, end },
        })
    }

    pub(super) fn parse_actor_decl(&mut self, exported: bool) -> Result<ActorDecl> {
        let start = self.expect_ident_value("actor")?.span.start;
        let name = self.expect_ident("expected actor name")?;
        if self.check_symbol("<") {
            return Err(CompileError::syntax(
                "actor declarations cannot be generic",
                self.peek().span.start,
            ));
        }
        self.expect_symbol("{")?;
        self.expect_ident_value("key")?;
        self.expect_symbol("(")?;
        let key_field = self.expect_ident("expected actor key field name")?;
        self.expect_symbol(")")?;
        let mut create = None;
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_symbol(";") {
                continue;
            }
            if !self.check_ident("create") {
                return Err(CompileError::syntax(
                    "actor declaration body only supports key(field) and create(...)",
                    self.peek().span.start,
                ));
            }
            if create.is_some() {
                return Err(CompileError::syntax(
                    format!("actor {name} declares create more than once"),
                    self.peek().span.start,
                ));
            }
            create = Some(self.parse_actor_create_decl()?);
        }
        self.expect_symbol("}")?;
        self.match_symbol(";");
        let end = self.previous().span.end;
        Ok(ActorDecl {
            exported,
            name,
            key_field,
            create,
            span: SourceSpan { start, end },
        })
    }

    pub(super) fn parse_actor_create_decl(&mut self) -> Result<ActorCreateDecl> {
        let start = self.expect_ident_value("create")?.span.start;
        self.expect_symbol("(")?;
        let params = self.parse_params()?;
        self.expect_symbol(")")?;
        let end = self.previous().span.end;
        Ok(ActorCreateDecl {
            params,
            span: SourceSpan { start, end },
        })
    }

    pub(super) fn parse_alias_decl(&mut self, exported: bool) -> Result<AliasDecl> {
        let start = self.expect_ident_value("alias")?.span.start;
        let name = self.expect_ident("expected alias name")?;
        self.expect_symbol("=")?;
        let target_type = self.parse_type()?;
        self.match_symbol(";");
        let end = self.previous().span.end;
        Ok(AliasDecl {
            exported,
            name,
            target_type,
            span: SourceSpan { start, end },
        })
    }

    pub(super) fn parse_interface(&mut self, exported: bool) -> Result<InterfaceDecl> {
        let start = self.expect_ident_value("interface")?.span.start;
        let name = self.expect_ident("expected interface name")?;
        let type_params = if self.check_symbol("<") {
            self.parse_type_param_names()?
        } else {
            Vec::new()
        };
        self.expect_symbol("{")?;
        let mut operations = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_symbol(";") {
                continue;
            }
            if !self.interface_operation_start() {
                return Err(CompileError::syntax(
                    "interface body only supports function requirements",
                    self.peek().span.start,
                ));
            }
            operations.push(self.parse_interface_operation()?);
            self.match_symbol(";");
        }
        self.expect_symbol("}")?;
        let end = self.previous().span.end;
        Ok(InterfaceDecl {
            exported,
            name,
            type_params,
            operations,
            span: SourceSpan { start, end },
        })
    }

    pub(super) fn parse_interface_operation(&mut self) -> Result<InterfaceOperation> {
        let operation = self.parse_function_signature(true)?;
        if operation.is_provider {
            return Err(CompileError::syntax(
                "provider function is only allowed for functions and impl methods",
                operation.span.start,
            ));
        }
        if operation.is_static {
            return Err(CompileError::syntax(
                "static interface method requirements are not supported",
                operation.span.start,
            ));
        }
        if operation.is_native {
            return Err(CompileError::syntax(
                "native interface method requirements are not supported",
                operation.span.start,
            ));
        }
        if !operation.type_params.is_empty() {
            return Err(CompileError::syntax(
                "interface method requirements cannot declare type parameters",
                operation.span.start,
            ));
        }
        if self.check_symbol("{") {
            return Err(CompileError::syntax(
                "interface method requirements cannot have a body",
                self.peek().span.start,
            ));
        }
        Ok(operation)
    }

    pub(super) fn interface_operation_start(&self) -> bool {
        self.check_ident("function")
            || self.check_ident("native")
            || self.check_ident("provider")
            || self.check_ident("static")
    }

    pub(super) fn parse_function_signature(
        &mut self,
        allow_static: bool,
    ) -> Result<InterfaceOperation> {
        let modifiers = self.parse_function_modifiers()?;
        if modifiers.is_static && !allow_static {
            return Err(CompileError::syntax(
                "static functions are only allowed inside impl blocks",
                modifiers.start.unwrap_or(self.peek().span.start),
            ));
        }
        if modifiers.is_provider {
            return Err(CompileError::syntax(
                LEGACY_PROVIDER_REMOVED_MESSAGE,
                modifiers.start.unwrap_or(self.peek().span.start),
            ));
        }
        let function_token = self.expect_ident_value("function")?;
        let start = modifiers.start.unwrap_or(function_token.span.start);
        let name = self.expect_ident("expected interface operation name")?;
        let type_params = if self.check_symbol("<") {
            self.parse_type_param_names()?
        } else {
            Vec::new()
        };
        self.expect_symbol("(")?;
        let params = self.parse_params()?;
        self.expect_symbol(")")?;
        self.expect_symbol("->")?;
        let return_type = self.parse_type()?;
        let end = self.previous().span.end;
        Ok(InterfaceOperation {
            name,
            type_params,
            params,
            return_type,
            is_native: modifiers.is_native,
            is_provider: modifiers.is_provider,
            is_static: modifiers.is_static,
            implicit_self: None,
            span: SourceSpan { start, end },
        })
    }

    pub(super) fn parse_impl(&mut self, exported: bool) -> Result<ImplDecl> {
        let start = self.expect_ident_value("impl")?.span.start;
        let target = self.parse_type()?.name;
        let mut methods = Vec::new();
        let mut method_bodies = Vec::new();
        if self.check_symbol("{") {
            if self.mode == ParseMode::Full {
                (methods, method_bodies) = self.parse_impl_methods_strict(&target)?;
            } else if self.mode == ParseMode::BodiesTolerant {
                (methods, method_bodies) = self.parse_impl_methods_with_bodies_tolerant(&target)?;
            } else if self.mode == ParseMode::Metadata {
                methods = self.parse_impl_methods(&target)?;
            }
        }
        let end = self.previous().span.end;
        Ok(ImplDecl {
            exported,
            target,
            methods,
            method_bodies,
            span: SourceSpan { start, end },
        })
    }

    pub(super) fn parse_impl_methods(&mut self, target: &str) -> Result<Vec<InterfaceOperation>> {
        let (methods, _method_bodies) = self.parse_impl_methods_with_options(
            target,
            CallableParseOptions {
                exported: false,
                provider_without_body: CallableNoBodyPolicy::SignatureOnly,
                native_without_body: CallableNoBodyPolicy::SignatureOnly,
                native_with_body: NativeBodyPolicy::SkipAndKeepSignature,
                body_policy: CallableBodyPolicy::SkipAndKeepSignature,
            },
        )?;
        Ok(methods)
    }

    pub(super) fn parse_impl_methods_strict(
        &mut self,
        target: &str,
    ) -> Result<(Vec<InterfaceOperation>, Vec<FunctionDecl>)> {
        self.parse_impl_methods_with_options(
            target,
            CallableParseOptions {
                exported: false,
                provider_without_body: CallableNoBodyPolicy::EmptyDecl,
                native_without_body: CallableNoBodyPolicy::EmptyDecl,
                native_with_body: NativeBodyPolicy::Error(
                    "native functions cannot have a Skiff body",
                ),
                body_policy: CallableBodyPolicy::ParseStrict,
            },
        )
    }

    pub(super) fn parse_impl_methods_with_bodies_tolerant(
        &mut self,
        target: &str,
    ) -> Result<(Vec<InterfaceOperation>, Vec<FunctionDecl>)> {
        self.parse_impl_methods_with_options(
            target,
            CallableParseOptions {
                exported: false,
                provider_without_body: CallableNoBodyPolicy::EmptyDecl,
                native_without_body: CallableNoBodyPolicy::SignatureOnly,
                native_with_body: NativeBodyPolicy::SkipAndDrop,
                body_policy: CallableBodyPolicy::ParseTolerantDrop,
            },
        )
    }

    pub(super) fn parse_impl_methods_with_options(
        &mut self,
        target: &str,
        options: CallableParseOptions,
    ) -> Result<(Vec<InterfaceOperation>, Vec<FunctionDecl>)> {
        self.expect_symbol("{")?;
        let mut methods = Vec::new();
        let mut method_bodies = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_symbol(";") {
                continue;
            }
            let exported = self.match_ident("export");
            if exported {
                return Err(CompileError::syntax(
                    "impl methods cannot be exported",
                    self.previous().span.start,
                ));
            }
            if !self.check_function_start() {
                return Err(CompileError::syntax(
                    "expected impl method declaration",
                    self.peek().span.start,
                ));
            }
            let signature = with_impl_receiver(target, self.parse_function_signature(true)?);
            methods.push(signature.clone());
            if let CallableParseResult::Decl { decl, spans } =
                self.parse_callable_body(signature, options)?
            {
                if let Some(spans) = spans {
                    self.source_spans.impl_methods.push(spans);
                }
                method_bodies.push(decl);
            }
            self.match_symbol(";");
        }
        self.expect_symbol("}")?;
        Ok((methods, method_bodies))
    }

    pub(super) fn parse_const_decl(&mut self, exported: bool) -> Result<ConstDecl> {
        let start = self.expect_ident_value("const")?.span.start;
        let name = self.expect_ident("expected const name")?;
        let ty = if self.match_symbol(":") {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect_symbol("=")?;
        let value = self.parse_expression()?;
        self.source_spans.consts.push(value.spans);
        self.match_symbol(";");
        let end = self.previous().span.end;
        Ok(ConstDecl {
            exported,
            name,
            ty,
            value: value.expr,
            span: SourceSpan { start, end },
        })
    }

    pub(super) fn parse_function(&mut self, exported: bool) -> Result<FunctionDecl> {
        let signature = self.parse_function_signature(false)?;
        match self.parse_callable_body(
            signature,
            CallableParseOptions {
                exported,
                provider_without_body: CallableNoBodyPolicy::EmptyDecl,
                native_without_body: CallableNoBodyPolicy::EmptyDecl,
                native_with_body: NativeBodyPolicy::Error(
                    "native functions cannot have a Skiff body",
                ),
                body_policy: CallableBodyPolicy::ParseStrict,
            },
        )? {
            CallableParseResult::Decl { decl, spans } => {
                if let Some(spans) = spans {
                    self.source_spans.functions.push(spans);
                }
                Ok(decl)
            }
            CallableParseResult::Signature(_) | CallableParseResult::None => unreachable!(),
        }
    }

    pub(super) fn parse_field_block(&mut self) -> Result<(Vec<FieldDecl>, Vec<TypeRef>)> {
        self.expect_symbol("{")?;
        let mut fields = Vec::new();
        let mut spreads = Vec::new();
        if !self.check_symbol("}") {
            loop {
                let spread_entry = self.check_ident("spread")
                    && !matches!(
                        self.tokens.get(self.current + 1).map(|token| &token.kind),
                        Some(TokenKind::Symbol(symbol)) if symbol == ":"
                    );
                if spread_entry {
                    self.advance();
                    let source = self.parse_type()?;
                    spreads.push(source);
                } else {
                    let name = self.expect_ident("expected field name")?;
                    self.expect_symbol(":")?;
                    let ty = self.parse_type()?;
                    fields.push(FieldDecl { name, ty });
                }
                if !self.match_symbol(",") {
                    break;
                }
                if self.check_symbol("}") {
                    break;
                }
            }
        }
        self.expect_symbol("}")?;
        Ok((fields, spreads))
    }

    pub(super) fn parse_params(&mut self) -> Result<Vec<Param>> {
        let mut params = Vec::new();
        if !self.check_symbol(")") {
            loop {
                let mode = if self.match_ident("inout") {
                    ParamMode::InOut
                } else {
                    ParamMode::Value
                };
                let param_name = self.expect_ident("expected parameter name")?;
                self.expect_symbol(":")?;
                let ty = self.parse_type()?;
                params.push(Param {
                    name: param_name,
                    mode,
                    ty,
                });
                if !self.match_symbol(",") {
                    break;
                }
            }
        }
        Ok(params)
    }

    pub(super) fn parse_function_modifiers(&mut self) -> Result<FunctionModifiers> {
        let mut modifiers = FunctionModifiers::default();
        loop {
            if self.check_ident("native") {
                let token = self.advance().clone();
                if modifiers.is_native {
                    return Err(CompileError::syntax(
                        "duplicate native function modifier",
                        token.span.start,
                    ));
                }
                modifiers.is_native = true;
                modifiers.start.get_or_insert(token.span.start);
                continue;
            }
            if self.check_ident("provider") {
                let token = self.advance().clone();
                return Err(CompileError::syntax(
                    LEGACY_PROVIDER_REMOVED_MESSAGE,
                    token.span.start,
                ));
            }
            if self.check_ident("static") {
                let token = self.advance().clone();
                if modifiers.is_static {
                    return Err(CompileError::syntax(
                        "duplicate static function modifier",
                        token.span.start,
                    ));
                }
                modifiers.is_static = true;
                modifiers.start.get_or_insert(token.span.start);
                continue;
            }
            break;
        }
        if modifiers.is_native && modifiers.is_provider {
            return Err(CompileError::syntax(
                "function cannot be both native and provider",
                modifiers.start.unwrap_or(self.peek().span.start),
            ));
        }
        Ok(modifiers)
    }

    pub(super) fn parse_type_param_names(&mut self) -> Result<Vec<String>> {
        self.expect_symbol("<")?;
        let mut params = Vec::new();
        if !self.check_symbol(">") {
            loop {
                params.push(self.expect_ident("expected type parameter name")?);
                if !self.match_symbol(",") {
                    break;
                }
                if self.check_symbol(">") {
                    break;
                }
            }
        }
        self.expect_symbol(">")?;
        Ok(params)
    }
}

fn with_impl_receiver(target: &str, mut signature: InterfaceOperation) -> InterfaceOperation {
    let has_explicit_self = signature
        .params
        .first()
        .is_some_and(|param| param.name == "self");
    if !signature.is_static && !has_explicit_self {
        signature.implicit_self = Some(TypeRef {
            name: target.to_string(),
        });
    }
    signature
}
