pub(super) use std::collections::{BTreeMap, BTreeSet};

pub(super) use crate::{
    ast::{
        ActorCreateDecl, ActorDecl, AliasDecl, BinaryOp, Block, BlockSourceSpans, BuiltinPackage,
        ConstDecl, DbBlockMode, DbBody, DbChange, DbChangeOp, DbDecl, DbIndexDirection,
        DbIndexEntry, DbIndexField, DbIndexWhereSourceSpans, DbLeaseClaim, DbLeaseDecl,
        DbLeaseRead, DbObjectFieldValue, DbObjectKey, DbOperation, DbOperationKind, DbOrderEntry,
        DbProjection, DbQuery, DbQueryBlock, DbRetention, DbRetentionUnit, DbSelector,
        DbStorageCodec, DbStorageDecl, DbTransaction, DbWhereClause, DependencySourceAddress,
        DurationLiteral, ExecutableSourceSpans, Expr, ExprSourceSpans, FieldDecl, FieldPath,
        ForBinding, FunctionDecl, ImplDecl, ImportDecl, InterfaceDecl, InterfaceOperation, Literal,
        MatchArm, PackageId, Param, Pattern, PatternField, RecordFieldSourceSpans, SourceFile,
        SourceSpanTable, Stmt, StmtSourceSpans, TypeDecl, TypeRef, UnaryOp, ValueBlock,
    },
    ast_utils::{expr_path, without_generic},
    error::{CompileError, Result, SourceLocation, SourceSpan},
    lexer::{lex, Token, TokenKind},
    type_syntax::{generic_parts, record_type_fields, split_top_level, string_literal},
};

const IMPORT_NAME_RULE: &str =
    "import name must be a single ASCII identifier matching [A-Za-z_][A-Za-z0-9_]*";
const LEGACY_PROVIDER_REMOVED_MESSAGE: &str =
    "legacy provider syntax has been removed; use native std APIs or package APIs instead";

mod cursor;
mod db;
mod expr;
mod pattern;
mod span;
mod stmt;
mod validate;

use span::ParsedExpr;
pub fn parse_source(source: &str) -> Result<SourceFile> {
    Parser::new(lex(source)?, ParseMode::Full).parse_source_file()
}

pub fn parse_source_metadata(source: &str) -> Result<SourceFile> {
    Parser::new(lex(source)?, ParseMode::Metadata).parse_source_file()
}

pub fn parse_source_with_bodies_tolerant(source: &str) -> Result<SourceFile> {
    Parser::new(lex(source)?, ParseMode::BodiesTolerant).parse_source_file()
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParseMode {
    Full,
    Metadata,
    BodiesTolerant,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct FunctionModifiers {
    is_native: bool,
    is_provider: bool,
    is_static: bool,
    start: Option<SourceLocation>,
}

pub(super) struct Parser {
    tokens: Vec<Token>,
    current: usize,
    mode: ParseMode,
    source_spans: SourceSpanTable,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum CallableNoBodyPolicy {
    EmptyDecl,
    SignatureOnly,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum NativeBodyPolicy {
    Error(&'static str),
    FollowBodyPolicy,
    SkipAndKeepSignature,
    SkipAndDrop,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum CallableBodyPolicy {
    ParseStrict,
    ParseTolerantKeepSignature,
    ParseTolerantDrop,
    SkipAndKeepSignature,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct CallableParseOptions {
    exported: bool,
    provider_without_body: CallableNoBodyPolicy,
    native_without_body: CallableNoBodyPolicy,
    native_with_body: NativeBodyPolicy,
    body_policy: CallableBodyPolicy,
}

pub(super) enum CallableParseResult {
    Decl {
        decl: FunctionDecl,
        spans: Option<ExecutableSourceSpans>,
    },
    Signature(InterfaceOperation),
    None,
}

impl Parser {
    fn new(tokens: Vec<Token>, mode: ParseMode) -> Self {
        Self {
            tokens,
            current: 0,
            mode,
            source_spans: SourceSpanTable::default(),
        }
    }

    fn parse_source_file(&mut self) -> Result<SourceFile> {
        let mut functions = Vec::new();
        let mut function_signatures = Vec::new();
        let mut imports = Vec::new();
        let mut types = Vec::new();
        let mut actors = Vec::new();
        let mut aliases = Vec::new();
        let mut interfaces = Vec::new();
        let mut impls = Vec::new();
        let mut dbs = Vec::new();
        let mut consts = Vec::new();
        let mut tests = Vec::new();
        let mut test_default_run = None;
        let mut test_default_run_span = None;
        while !self.is_at_end() {
            if self.match_symbol(";") {
                continue;
            }
            let export_token_start = self.peek().span.start;
            let exported = self.match_ident("export");
            if self.match_ident("test") {
                if exported {
                    return Err(CompileError::syntax(
                        "test declarations cannot be exported",
                        self.previous().span.start,
                    ));
                }
                if self.check_ident("defaultRun") {
                    let start = self.previous().span.start;
                    let (default_run, span) = self.parse_test_default_run_declaration(start)?;
                    if test_default_run.is_some() {
                        return Err(CompileError::syntax(
                            "duplicated test defaultRun declaration",
                            self.previous().span.start,
                        ));
                    }
                    test_default_run = Some(default_run);
                    test_default_run_span = Some(span);
                    continue;
                }
                let start = self.previous().span.start;
                let name = self.expect_string("expected test name string")?;
                let test = self.parse_test_block(name, start)?;
                tests.push(test);
            } else if self.check_ident("assert") {
                self.advance();
                return Err(CompileError::syntax(
                    "assert can only be used in test blocks",
                    self.previous().span.start,
                ));
            } else if self.check_ident("import") {
                if exported {
                    return Err(CompileError::syntax(
                        "imports cannot be exported",
                        self.peek().span.start,
                    ));
                }
                imports.push(self.parse_import()?);
            } else if self.check_provider_capability_start() {
                return Err(CompileError::syntax(
                    LEGACY_PROVIDER_REMOVED_MESSAGE,
                    self.peek().span.start,
                ));
            } else if self.check_ident("const") {
                self.reject_export_modifier_if_needed(exported, export_token_start)?;
                consts.push(self.parse_const_decl(exported)?);
            } else if self.check_ident("type") {
                self.reject_export_modifier_if_needed(exported, export_token_start)?;
                types.push(self.parse_type_decl(exported)?);
            } else if self.check_ident("actor") {
                self.reject_export_modifier_if_needed(exported, export_token_start)?;
                actors.push(self.parse_actor_decl(exported)?);
            } else if self.check_ident("alias") {
                self.reject_export_modifier_if_needed(exported, export_token_start)?;
                aliases.push(self.parse_alias_decl(exported)?);
            } else if self.check_ident("interface") {
                self.reject_export_modifier_if_needed(exported, export_token_start)?;
                interfaces.push(self.parse_interface(exported)?);
            } else if self.check_ident("impl") {
                self.reject_export_modifier_if_needed(exported, export_token_start)?;
                impls.push(self.parse_impl(exported)?);
            } else if self.check_ident("db") {
                if exported {
                    return Err(CompileError::syntax(
                        "db declarations cannot be exported",
                        self.peek().span.start,
                    ));
                }
                dbs.push(self.parse_db_decl()?);
            } else if self.check_ident("process") {
                return Err(CompileError::syntax(
                    "process has been removed; use actors and spawn instead",
                    self.peek().span.start,
                ));
            } else if self.check_function_start() {
                self.reject_export_modifier_if_needed(exported, export_token_start)?;
                match self.mode {
                    ParseMode::Full => functions.push(self.parse_function(exported)?),
                    ParseMode::Metadata => {
                        function_signatures
                            .push(self.parse_function_signature_and_skip_body(false)?);
                    }
                    ParseMode::BodiesTolerant => {
                        let signature = self.parse_function_signature(false)?;
                        match self.parse_callable_body(
                            signature,
                            CallableParseOptions {
                                exported,
                                provider_without_body: CallableNoBodyPolicy::EmptyDecl,
                                native_without_body: CallableNoBodyPolicy::EmptyDecl,
                                native_with_body: NativeBodyPolicy::FollowBodyPolicy,
                                body_policy: CallableBodyPolicy::ParseTolerantKeepSignature,
                            },
                        )? {
                            CallableParseResult::Decl { decl, spans } => {
                                if let Some(spans) = spans {
                                    self.source_spans.functions.push(spans);
                                }
                                functions.push(decl);
                            }
                            CallableParseResult::Signature(signature) => {
                                function_signatures.push(signature);
                            }
                            CallableParseResult::None => {}
                        }
                    }
                }
            } else {
                return Err(CompileError::syntax(
                    "expected top-level declaration",
                    self.peek().span.start,
                ));
            }
        }
        validate::validate_actor_declarations(&actors, &types, &dbs)?;
        Ok(SourceFile {
            provider_capability: None,
            functions,
            function_signatures,
            imports,
            types,
            actors,
            aliases,
            interfaces,
            impls,
            dbs,
            consts,
            tests,
            test_default_run,
            test_default_run_span,
            source_spans: std::mem::take(&mut self.source_spans),
        })
    }

    fn reject_export_modifier_if_needed(
        &self,
        exported: bool,
        export_start: SourceLocation,
    ) -> Result<()> {
        if exported {
            return Err(CompileError::syntax(
                "the export modifier has been removed; declare public API in api.yml",
                export_start,
            ));
        }
        Ok(())
    }
}
impl Parser {
    fn parse_import(&mut self) -> Result<ImportDecl> {
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

    fn parse_type_decl(&mut self, exported: bool) -> Result<TypeDecl> {
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
                fields = self.parse_field_block()?;
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
            span: SourceSpan { start, end },
        })
    }

    fn parse_actor_decl(&mut self, exported: bool) -> Result<ActorDecl> {
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

    fn parse_actor_create_decl(&mut self) -> Result<ActorCreateDecl> {
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

    fn parse_alias_decl(&mut self, exported: bool) -> Result<AliasDecl> {
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

    fn parse_interface(&mut self, exported: bool) -> Result<InterfaceDecl> {
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

    fn parse_interface_operation(&mut self) -> Result<InterfaceOperation> {
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

    fn interface_operation_start(&self) -> bool {
        self.check_ident("function")
            || self.check_ident("native")
            || self.check_ident("provider")
            || self.check_ident("static")
    }

    fn parse_function_signature(&mut self, allow_static: bool) -> Result<InterfaceOperation> {
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

    fn parse_impl(&mut self, exported: bool) -> Result<ImplDecl> {
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

    fn parse_impl_methods(&mut self, target: &str) -> Result<Vec<InterfaceOperation>> {
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

    fn parse_impl_methods_strict(
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

    fn parse_impl_methods_with_bodies_tolerant(
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

    fn parse_impl_methods_with_options(
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

    fn parse_const_decl(&mut self, exported: bool) -> Result<ConstDecl> {
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

    fn parse_function(&mut self, exported: bool) -> Result<FunctionDecl> {
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

    fn parse_function_signature_and_skip_body(
        &mut self,
        allow_static: bool,
    ) -> Result<InterfaceOperation> {
        let signature = self.parse_function_signature(allow_static)?;
        match self.parse_callable_body(
            signature,
            CallableParseOptions {
                exported: false,
                provider_without_body: CallableNoBodyPolicy::SignatureOnly,
                native_without_body: CallableNoBodyPolicy::SignatureOnly,
                native_with_body: NativeBodyPolicy::SkipAndKeepSignature,
                body_policy: CallableBodyPolicy::SkipAndKeepSignature,
            },
        )? {
            CallableParseResult::Signature(signature) => Ok(signature),
            CallableParseResult::Decl { .. } | CallableParseResult::None => unreachable!(),
        }
    }

    fn parse_callable_body(
        &mut self,
        signature: InterfaceOperation,
        options: CallableParseOptions,
    ) -> Result<CallableParseResult> {
        if signature.is_provider {
            if self.check_symbol("{") {
                return Err(CompileError::syntax(
                    "provider function cannot have a body",
                    self.peek().span.start,
                ));
            }
            return Ok(self.finish_bodyless_callable(
                signature,
                options.exported,
                options.provider_without_body,
            ));
        }

        if signature.is_native && !self.check_symbol("{") {
            return Ok(self.finish_bodyless_callable(
                signature,
                options.exported,
                options.native_without_body,
            ));
        }

        if signature.is_native {
            return match options.native_with_body {
                NativeBodyPolicy::Error(message) => {
                    Err(CompileError::syntax(message, self.peek().span.start))
                }
                NativeBodyPolicy::FollowBodyPolicy => {
                    self.parse_callable_decl_body(signature, options.exported, options.body_policy)
                }
                NativeBodyPolicy::SkipAndKeepSignature => {
                    self.skip_callable_body_and_keep_signature(signature)
                }
                NativeBodyPolicy::SkipAndDrop => self.skip_callable_body_and_drop(),
            };
        }

        self.parse_callable_decl_body(signature, options.exported, options.body_policy)
    }

    fn finish_bodyless_callable(
        &mut self,
        signature: InterfaceOperation,
        exported: bool,
        policy: CallableNoBodyPolicy,
    ) -> CallableParseResult {
        self.match_symbol(";");
        match policy {
            CallableNoBodyPolicy::EmptyDecl => {
                let end = self.previous().span.end;
                CallableParseResult::Decl {
                    decl: self.build_function_decl(
                        exported,
                        signature,
                        Block {
                            statements: Vec::new(),
                        },
                        end,
                    ),
                    spans: None,
                }
            }
            CallableNoBodyPolicy::SignatureOnly => CallableParseResult::Signature(signature),
        }
    }

    fn parse_callable_decl_body(
        &mut self,
        signature: InterfaceOperation,
        exported: bool,
        policy: CallableBodyPolicy,
    ) -> Result<CallableParseResult> {
        match policy {
            CallableBodyPolicy::ParseStrict => {
                self.parse_callable_decl_body_strict(signature, exported)
            }
            CallableBodyPolicy::ParseTolerantKeepSignature => {
                self.parse_callable_decl_body_tolerant(signature, exported, true)
            }
            CallableBodyPolicy::ParseTolerantDrop => {
                self.parse_callable_decl_body_tolerant(signature, exported, false)
            }
            CallableBodyPolicy::SkipAndKeepSignature => {
                self.skip_callable_body_and_keep_signature(signature)
            }
        }
    }

    fn parse_callable_decl_body_strict(
        &mut self,
        signature: InterfaceOperation,
        exported: bool,
    ) -> Result<CallableParseResult> {
        let body = self.parse_block(false)?;
        let end = self.previous().span.end;
        Ok(CallableParseResult::Decl {
            decl: self.build_function_decl(exported, signature, body.block, end),
            spans: Some(ExecutableSourceSpans {
                effects: Vec::new(),
                body: body.spans,
            }),
        })
    }

    fn parse_callable_decl_body_tolerant(
        &mut self,
        signature: InterfaceOperation,
        exported: bool,
        keep_signature_on_failure: bool,
    ) -> Result<CallableParseResult> {
        let body_start = self.snapshot();
        match self.parse_block(false) {
            Ok(body) => {
                let end = self.previous().span.end;
                Ok(CallableParseResult::Decl {
                    decl: self.build_function_decl(exported, signature, body.block, end),
                    spans: Some(ExecutableSourceSpans {
                        effects: Vec::new(),
                        body: body.spans,
                    }),
                })
            }
            Err(_) => {
                self.restore(body_start);
                if self.check_symbol("{") {
                    self.skip_balanced_block("unterminated function body")?;
                } else {
                    return Err(CompileError::syntax(
                        "expected function body",
                        self.peek().span.start,
                    ));
                }
                if keep_signature_on_failure {
                    Ok(CallableParseResult::Signature(signature))
                } else {
                    Ok(CallableParseResult::None)
                }
            }
        }
    }

    fn skip_callable_body_and_keep_signature(
        &mut self,
        signature: InterfaceOperation,
    ) -> Result<CallableParseResult> {
        if self.check_symbol("{") {
            self.skip_balanced_block("unterminated function body")?;
            Ok(CallableParseResult::Signature(signature))
        } else {
            Err(CompileError::syntax(
                "expected function body",
                self.peek().span.start,
            ))
        }
    }

    fn skip_callable_body_and_drop(&mut self) -> Result<CallableParseResult> {
        if self.check_symbol("{") {
            self.skip_balanced_block("unterminated function body")?;
            Ok(CallableParseResult::None)
        } else {
            Err(CompileError::syntax(
                "expected function body",
                self.peek().span.start,
            ))
        }
    }

    fn build_function_decl(
        &self,
        exported: bool,
        signature: InterfaceOperation,
        body: Block,
        end: SourceLocation,
    ) -> FunctionDecl {
        FunctionDecl {
            exported,
            name: signature.name,
            type_params: signature.type_params,
            params: signature.params,
            return_type: signature.return_type,
            body,
            is_native: signature.is_native,
            is_provider: signature.is_provider,
            is_static: signature.is_static,
            implicit_self: signature.implicit_self,
            span: SourceSpan {
                start: signature.span.start,
                end,
            },
        }
    }

    fn parse_type(&mut self) -> Result<TypeRef> {
        let mut name = self.parse_nullable_type()?;
        while self.match_symbol("|") {
            let rhs = self.parse_nullable_type()?;
            name.push_str(" | ");
            name.push_str(&rhs);
        }
        Ok(TypeRef { name })
    }

    fn parse_nullable_type(&mut self) -> Result<String> {
        let mut name = self.parse_primary_type()?;
        while self.match_symbol("?") {
            name.push('?');
        }
        Ok(name)
    }

    fn parse_primary_type(&mut self) -> Result<String> {
        if self.match_ident("any") {
            let interface = self.parse_primary_type()?;
            return Ok(format!("any {interface}"));
        }
        if self.check_ident("fn") {
            return self.parse_function_type_name();
        }
        if self.match_symbol("{") {
            return self.parse_record_type_name();
        }

        let token = self.advance().clone();
        let mut name = match token.kind {
            TokenKind::Ident(value) => value,
            TokenKind::String(value) => quote_string_type(&value),
            _ => return Err(CompileError::syntax("expected type name", token.span.start)),
        };
        if self.match_symbol("/") {
            name.push('/');
            name.push_str(&self.expect_ident("expected source module after dependency /")?);
        }
        while self.match_symbol(".") {
            name.push('.');
            name.push_str(&self.expect_ident("expected qualified type segment")?);
        }

        if self.match_symbol("<") {
            name.push('<');
            if !self.check_symbol(">") {
                loop {
                    name.push_str(&self.parse_type()?.name);
                    if !self.match_symbol(",") {
                        break;
                    }
                    name.push_str(", ");
                }
            }
            self.expect_symbol(">")?;
            name.push('>');
        }
        Ok(name)
    }

    fn parse_function_type_name(&mut self) -> Result<String> {
        self.expect_ident_value("fn")?;
        self.expect_symbol("(")?;
        let params = self.parse_params()?;
        self.expect_symbol(")")?;
        self.expect_symbol("->")?;
        let return_type = self.parse_type()?;
        let params = params
            .iter()
            .map(|param| format!("{}: {}", param.name, param.ty.name))
            .collect::<Vec<_>>()
            .join(", ");
        Ok(format!("fn({params}) -> {}", return_type.name))
    }

    fn parse_record_type_name(&mut self) -> Result<String> {
        let mut fields = Vec::new();
        if !self.check_symbol("}") {
            loop {
                let field = self.expect_ident("expected record field name")?;
                self.expect_symbol(":")?;
                let ty = self.parse_type()?;
                fields.push(format!("{field}: {}", ty.name));
                if !self.match_symbol(",") {
                    break;
                }
                if self.check_symbol("}") {
                    break;
                }
            }
        }
        self.expect_symbol("}")?;
        if fields.is_empty() {
            Ok("{}".to_string())
        } else {
            Ok(format!("{{ {} }}", fields.join(", ")))
        }
    }

    fn parse_field_block(&mut self) -> Result<Vec<FieldDecl>> {
        self.expect_symbol("{")?;
        let mut fields = Vec::new();
        if !self.check_symbol("}") {
            loop {
                let name = self.expect_ident("expected field name")?;
                self.expect_symbol(":")?;
                let ty = self.parse_type()?;
                fields.push(FieldDecl { name, ty });
                if !self.match_symbol(",") {
                    break;
                }
                if self.check_symbol("}") {
                    break;
                }
            }
        }
        self.expect_symbol("}")?;
        Ok(fields)
    }

    fn parse_params(&mut self) -> Result<Vec<Param>> {
        let mut params = Vec::new();
        if !self.check_symbol(")") {
            loop {
                let param_name = self.expect_ident("expected parameter name")?;
                self.expect_symbol(":")?;
                let ty = self.parse_type()?;
                params.push(Param {
                    name: param_name,
                    ty,
                });
                if !self.match_symbol(",") {
                    break;
                }
            }
        }
        Ok(params)
    }

    fn parse_function_modifiers(&mut self) -> Result<FunctionModifiers> {
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

    fn parse_type_param_names(&mut self) -> Result<Vec<String>> {
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

    fn parse_test_default_run_declaration(
        &mut self,
        start: SourceLocation,
    ) -> Result<(bool, SourceSpan)> {
        self.expect_ident_value("defaultRun")?;
        let value = if self.match_ident("true") {
            true
        } else if self.match_ident("false") {
            false
        } else {
            return Err(CompileError::syntax(
                "expected test defaultRun bool literal",
                self.peek().span.start,
            ));
        };
        let mut end = self.previous().span.end;
        if self.match_symbol(";") {
            end = self.previous().span.end;
        }
        Ok((value, SourceSpan { start, end }))
    }

    fn parse_test_block(
        &mut self,
        name: String,
        start: SourceLocation,
    ) -> Result<crate::ast::TestDeclaration> {
        let (effects, effect_spans) = if self.match_ident("effects") {
            self.parse_test_effects()?
        } else {
            (Vec::new(), Vec::new())
        };
        let body = self.parse_block(true)?;
        let end = self.previous().span.end;
        self.source_spans.tests.push(ExecutableSourceSpans {
            effects: effect_spans,
            body: body.spans,
        });
        Ok(crate::ast::TestDeclaration {
            name,
            effects,
            body: body.block,
            span: SourceSpan { start, end },
        })
    }

    fn parse_test_effects(
        &mut self,
    ) -> Result<(
        Vec<crate::ast::TestEffectDeclaration>,
        Vec<crate::ast::TestEffectSourceSpans>,
    )> {
        use std::collections::BTreeSet;

        self.expect_symbol("{")?;
        let mut effects = Vec::new();
        let mut effect_spans = Vec::new();
        let mut targets = BTreeSet::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            let start = self.peek().span.start;
            let target = self.parse_test_effect_target()?;
            if !targets.insert(target.clone()) {
                return Err(CompileError::syntax(
                    format!("duplicate test effect target `{target}`"),
                    start,
                ));
            }
            self.expect_symbol("{")?;
            let mut expect = None;
            let mut expect_spans = None;
            let mut outcome = None;
            let mut outcome_spans = None;
            while !self.check_symbol("}") && !self.is_at_end() {
                let field_location = self.peek().span.start;
                let field = self.expect_ident("expected test effect field")?;
                self.expect_symbol(":")?;
                match field.as_str() {
                    "expect" if expect.is_none() => {
                        let parsed = self.parse_expression()?;
                        expect = Some(parsed.expr);
                        expect_spans = Some(parsed.spans);
                    }
                    "expect" => {
                        return Err(CompileError::syntax(
                            "duplicate test effect `expect` field",
                            field_location,
                        ))
                    }
                    "respond" if outcome.is_none() => {
                        let parsed = self.parse_expression()?;
                        outcome =
                            Some(crate::ast::TestEffectOutcome::Respond { value: parsed.expr });
                        outcome_spans = Some(crate::ast::TestEffectOutcomeSourceSpans::Respond(
                            parsed.spans,
                        ));
                    }
                    "throw" if outcome.is_none() => {
                        let parsed = self.parse_expression()?;
                        outcome = Some(crate::ast::TestEffectOutcome::Throw { value: parsed.expr });
                        outcome_spans = Some(crate::ast::TestEffectOutcomeSourceSpans::Throw(
                            parsed.spans,
                        ));
                    }
                    "stream" if outcome.is_none() => {
                        let parsed = self.parse_test_effect_expression_sequence("stream")?;
                        outcome = Some(crate::ast::TestEffectOutcome::Stream {
                            events: parsed.iter().map(|value| value.expr.clone()).collect(),
                        });
                        outcome_spans = Some(crate::ast::TestEffectOutcomeSourceSpans::Stream(
                            parsed.into_iter().map(|value| value.spans).collect(),
                        ));
                    }
                    "sequence" if outcome.is_none() => {
                        let (steps, step_spans) = self.parse_test_effect_sequence()?;
                        outcome = Some(crate::ast::TestEffectOutcome::Sequence { steps });
                        outcome_spans = Some(crate::ast::TestEffectOutcomeSourceSpans::Sequence {
                            steps: step_spans,
                        });
                    }
                    "respond" | "throw" | "stream" | "sequence" => {
                        return Err(CompileError::syntax(
                            "test effect must declare exactly one outcome field",
                            field_location,
                        ))
                    }
                    _ => {
                        return Err(CompileError::syntax(
                            format!("unknown test effect field `{field}`"),
                            field_location,
                        ))
                    }
                }
                self.match_symbol(",");
            }
            self.expect_symbol("}")?;
            let Some(outcome) = outcome else {
                return Err(CompileError::syntax(
                    "test effect requires an outcome field",
                    start,
                ));
            };
            let end = self.previous().span.end;
            effects.push(crate::ast::TestEffectDeclaration {
                target,
                expect,
                outcome,
                span: SourceSpan { start, end },
            });
            effect_spans.push(crate::ast::TestEffectSourceSpans {
                expect: expect_spans,
                outcome: outcome_spans.expect("parsed test effect outcome spans"),
            });
            self.match_symbol(",");
        }
        self.expect_symbol("}")?;
        Ok((effects, effect_spans))
    }

    fn parse_test_effect_target(&mut self) -> Result<String> {
        let mut target = self.expect_ident("expected test effect target")?;
        if self.match_symbol("/") {
            target.push('/');
            target.push_str(&self.expect_ident("expected source module after /")?);
        }
        while self.match_symbol(".") {
            target.push('.');
            target.push_str(&self.expect_ident("expected test effect target segment")?);
        }
        Ok(target)
    }

    fn parse_test_effect_sequence(
        &mut self,
    ) -> Result<(
        Vec<crate::ast::TestEffectSequenceStep>,
        Vec<crate::ast::TestEffectSequenceStepSourceSpans>,
    )> {
        self.expect_symbol("[")?;
        let mut steps = Vec::new();
        let mut step_spans = Vec::new();
        while !self.check_symbol("]") && !self.is_at_end() {
            let start = self.peek().span.start;
            self.expect_symbol("{")?;
            let mut expect = None;
            let mut expect_spans = None;
            let mut outcome = None;
            let mut outcome_spans = None;
            while !self.check_symbol("}") && !self.is_at_end() {
                let field_location = self.peek().span.start;
                let field = self.expect_ident("expected test effect sequence step field")?;
                self.expect_symbol(":")?;
                match field.as_str() {
                    "expect" if expect.is_none() => {
                        let parsed = self.parse_expression()?;
                        expect = Some(parsed.expr);
                        expect_spans = Some(parsed.spans);
                    }
                    "expect" => {
                        return Err(CompileError::syntax(
                            "duplicate test effect sequence step `expect` field",
                            field_location,
                        ))
                    }
                    "respond" if outcome.is_none() => {
                        let parsed = self.parse_expression()?;
                        outcome =
                            Some(crate::ast::TestEffectStepOutcome::Respond { value: parsed.expr });
                        outcome_spans = Some(
                            crate::ast::TestEffectStepOutcomeSourceSpans::Respond(parsed.spans),
                        );
                    }
                    "throw" if outcome.is_none() => {
                        let parsed = self.parse_expression()?;
                        outcome =
                            Some(crate::ast::TestEffectStepOutcome::Throw { value: parsed.expr });
                        outcome_spans = Some(crate::ast::TestEffectStepOutcomeSourceSpans::Throw(
                            parsed.spans,
                        ));
                    }
                    "stream" if outcome.is_none() => {
                        let parsed = self.parse_test_effect_expression_sequence("stream")?;
                        outcome = Some(crate::ast::TestEffectStepOutcome::Stream {
                            events: parsed.iter().map(|value| value.expr.clone()).collect(),
                        });
                        outcome_spans = Some(crate::ast::TestEffectStepOutcomeSourceSpans::Stream(
                            parsed.into_iter().map(|value| value.spans).collect(),
                        ));
                    }
                    "respond" | "throw" | "stream" => {
                        return Err(CompileError::syntax(
                            "test effect sequence step must declare exactly one outcome field",
                            field_location,
                        ))
                    }
                    _ => {
                        return Err(CompileError::syntax(
                            format!("unknown test effect sequence step field `{field}`"),
                            field_location,
                        ))
                    }
                }
                self.match_symbol(",");
            }
            self.expect_symbol("}")?;
            let Some(outcome) = outcome else {
                return Err(CompileError::syntax(
                    "test effect sequence step requires an outcome field",
                    start,
                ));
            };
            steps.push(crate::ast::TestEffectSequenceStep { expect, outcome });
            step_spans.push(crate::ast::TestEffectSequenceStepSourceSpans {
                expect: expect_spans,
                outcome: outcome_spans.expect("parsed test effect sequence step outcome spans"),
            });
            if !self.match_symbol(",") {
                break;
            }
        }
        self.expect_symbol("]")?;
        if steps.is_empty() {
            return Err(CompileError::syntax(
                "test effect `sequence` cannot be empty",
                self.previous().span.start,
            ));
        }
        Ok((steps, step_spans))
    }

    fn parse_test_effect_expression_sequence(&mut self, field: &str) -> Result<Vec<ParsedExpr>> {
        self.expect_symbol("[")?;
        let mut values = Vec::new();
        while !self.check_symbol("]") && !self.is_at_end() {
            values.push(self.parse_expression()?);
            if !self.match_symbol(",") {
                break;
            }
        }
        self.expect_symbol("]")?;
        if values.is_empty() {
            return Err(CompileError::syntax(
                format!("test effect `{field}` cannot be empty"),
                self.previous().span.start,
            ));
        }
        Ok(values)
    }
}
fn quote_string_type(value: &str) -> String {
    let escaped = value
        .chars()
        .flat_map(|ch| match ch {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            other => vec![other],
        })
        .collect::<String>();
    format!("\"{escaped}\"")
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

#[cfg(test)]
mod tests;
