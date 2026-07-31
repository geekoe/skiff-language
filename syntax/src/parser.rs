use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ast::{
        ActorCreateDecl, ActorDecl, AliasDecl, BinaryOp, Block, BlockSourceSpans, BuiltinPackage, ConstDecl,
        DbBlockMode, DbBody, DbChange, DbChangeOp, DbDecl, DbIndexDirection, DbIndexEntry,
        DbIndexField, DbIndexWhereSourceSpans, DbLeaseClaim, DbLeaseDecl, DbLeaseRead,
        DbObjectFieldValue, DbObjectKey, DbOperation, DbOperationKind, DbOrderEntry, DbProjection,
        DbQuery, DbQueryBlock, DbRetention, DbRetentionUnit, DbSelector, DbStorageCodec,
        DbStorageDecl, DbTransaction, DbWhereClause, DependencySourceAddress, DurationLiteral,
        ExecutableSourceSpans, Expr, ExprSourceSpans, FieldDecl, FieldPath, ForBinding,
        FunctionDecl, ImplDecl, ImportDecl, InterfaceDecl, InterfaceOperation, Literal, MatchArm,
        PackageId, Param, Pattern, PatternField, RecordFieldSourceSpans, SourceFile,
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

pub fn parse_source(source: &str) -> Result<SourceFile> {
    Parser::new(lex(source)?, ParseMode::Full, true).parse_source_file()
}

pub fn parse_source_metadata(source: &str) -> Result<SourceFile> {
    Parser::new(lex(source)?, ParseMode::Metadata, true).parse_source_file()
}

pub fn parse_source_with_bodies_tolerant(source: &str) -> Result<SourceFile> {
    Parser::new(lex(source)?, ParseMode::BodiesTolerant, true).parse_source_file()
}

fn is_old_db_dotted_operation(operation: &str) -> bool {
    matches!(
        operation,
        "get"
            | "require"
            | "exists"
            | "create"
            | "createMany"
            | "create_many"
            | "append"
            | "appendMany"
            | "append_many"
            | "upsert"
            | "findMany"
            | "find_many"
            | "count"
            | "transaction"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseMode {
    Full,
    Metadata,
    BodiesTolerant,
}

#[derive(Debug, Clone, Copy, Default)]
struct FunctionModifiers {
    is_native: bool,
    is_provider: bool,
    is_static: bool,
    start: Option<SourceLocation>,
}

struct Parser {
    tokens: Vec<Token>,
    current: usize,
    mode: ParseMode,
    provider_capability: Option<Vec<String>>,
    source_spans: SourceSpanTable,
    reject_export_modifier: bool,
}

struct ParsedBlock {
    block: Block,
    spans: BlockSourceSpans,
}

struct ParsedStmt {
    stmt: Stmt,
    spans: StmtSourceSpans,
}

struct ParsedExpr {
    expr: Expr,
    spans: ExprSourceSpans,
}

#[derive(Debug, Clone, Copy)]
enum CallableNoBodyPolicy {
    EmptyDecl,
    SignatureOnly,
}

#[derive(Debug, Clone, Copy)]
enum NativeBodyPolicy {
    Error(&'static str),
    FollowBodyPolicy,
    SkipAndKeepSignature,
    SkipAndDrop,
}

#[derive(Debug, Clone, Copy)]
enum CallableBodyPolicy {
    ParseStrict,
    ParseTolerantKeepSignature,
    ParseTolerantDrop,
    SkipAndKeepSignature,
}

#[derive(Debug, Clone, Copy)]
struct CallableParseOptions {
    exported: bool,
    provider_without_body: CallableNoBodyPolicy,
    native_without_body: CallableNoBodyPolicy,
    native_with_body: NativeBodyPolicy,
    body_policy: CallableBodyPolicy,
}

enum CallableParseResult {
    Decl {
        decl: FunctionDecl,
        spans: Option<ExecutableSourceSpans>,
    },
    Signature(InterfaceOperation),
    None,
}

fn parsed_leaf_expr(expr: Expr, span: SourceSpan) -> ParsedExpr {
    ParsedExpr {
        expr,
        spans: expr_source_spans(span, Vec::new()),
    }
}

fn parsed_expression_statement(expression: ParsedExpr) -> ParsedStmt {
    ParsedStmt {
        stmt: Stmt::Expr(expression.expr),
        spans: StmtSourceSpans {
            span: expression.spans.span,
            expressions: vec![expression.spans],
            blocks: Vec::new(),
        },
    }
}

fn expr_source_spans(span: SourceSpan, children: Vec<ExprSourceSpans>) -> ExprSourceSpans {
    ExprSourceSpans {
        span,
        children,
        blocks: Vec::new(),
        record_fields: Vec::new(),
    }
}

fn object_literal_key_name(key: &crate::ast::ObjectLiteralKey) -> Option<String> {
    match key {
        crate::ast::ObjectLiteralKey::Name(name) => Some(name.clone()),
    }
}

fn validate_type_decl_discriminator(
    name: &str,
    ty: &str,
    discriminator: Option<&str>,
    location: SourceLocation,
) -> Result<()> {
    let union = split_top_level(ty.trim(), '|');
    if union.len() <= 1 {
        if discriminator.is_some() {
            return Err(CompileError::syntax(
                format!(
                    "type {name} discriminator can only be used with anonymous record union branches"
                ),
                location,
            ));
        }
        return Ok(());
    }

    let anonymous_record_branches = union
        .iter()
        .filter_map(|part| parser_record_type_fields(part.trim()))
        .collect::<Vec<_>>();
    if anonymous_record_branches.is_empty() {
        if discriminator.is_some() {
            return Err(CompileError::syntax(
                format!(
                    "type {name} discriminator can only be used with anonymous record union branches"
                ),
                location,
            ));
        }
        return Ok(());
    }

    let Some(discriminator) = discriminator else {
        return Err(CompileError::syntax(
            format!(
                "named union type {name} uses anonymous record branches; add discriminator \"tag\" to the type declaration"
            ),
            location,
        ));
    };

    let mut values = BTreeSet::new();
    for fields in anonymous_record_branches {
        let Some(value) = discriminator_record_branch_value(&fields, discriminator) else {
            return Err(CompileError::syntax(
                format!(
                    "anonymous record union branch in {name} must declare {discriminator} as a string literal"
                ),
                location,
            ));
        };
        if !values.insert(value.clone()) {
            return Err(CompileError::syntax(
                format!(
                    "anonymous record union branch {discriminator} \"{value}\" in {name} must be unique"
                ),
                location,
            ));
        }
    }

    Ok(())
}

fn validate_actor_declarations(actors: &[ActorDecl], types: &[TypeDecl], dbs: &[DbDecl]) -> Result<()> {
    let type_by_name = types
        .iter()
        .map(|declaration| (declaration.name.as_str(), declaration))
        .collect::<BTreeMap<_, _>>();
    let db_type_names = dbs
        .iter()
        .map(|declaration| declaration.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut actor_names = BTreeSet::new();
    for actor in actors {
        if !actor_names.insert(actor.name.as_str()) {
            return Err(CompileError::syntax(
                format!("duplicated actor declaration {}", actor.name),
                actor.span.start,
            ));
        }
        let attached = type_by_name.get(actor.name.as_str()).ok_or_else(|| {
            CompileError::syntax(
                format!(
                    "actor {} requires a same-file type declaration of the same name",
                    actor.name
                ),
                actor.span.start,
            )
        })?;
        if !attached.type_params.is_empty() {
            return Err(CompileError::syntax(
                format!(
                    "actor {} must attach to a non-generic type declaration",
                    actor.name
                ),
                actor.span.start,
            ));
        }
        if attached.alias.is_some() || attached.discriminator.is_some() {
            return Err(CompileError::syntax(
                format!(
                    "actor {} must attach to a concrete record type declaration",
                    actor.name
                ),
                actor.span.start,
            ));
        }
        if !attached.fields.iter().any(|field| field.name == actor.key_field) {
            return Err(CompileError::syntax(
                format!(
                    "actor {} key({}) must name a field of the attached type {}",
                    actor.name, actor.key_field, actor.name
                ),
                actor.span.start,
            ));
        }
        if db_type_names.contains(actor.name.as_str()) {
            return Err(CompileError::syntax(
                format!(
                    "type {} cannot attach both db object and actor declarations",
                    actor.name
                ),
                actor.span.start,
            ));
        }
    }
    Ok(())
}

fn discriminator_record_branch_value(
    fields: &[(String, String)],
    discriminator: &str,
) -> Option<String> {
    fields.iter().find_map(|(field_name, field_type)| {
        (field_name == discriminator)
            .then(|| string_literal(field_type))
            .flatten()
    })
}

fn parser_record_type_fields(ty: &str) -> Option<Vec<(String, String)>> {
    record_type_fields(ty).map(|fields| {
        fields
            .into_iter()
            .map(|field| (field.name.to_string(), field.ty.to_string()))
            .collect()
    })
}

impl Parser {
    fn new(tokens: Vec<Token>, mode: ParseMode, reject_export_modifier: bool) -> Self {
        Self {
            tokens,
            current: 0,
            mode,
            provider_capability: None,
            source_spans: SourceSpanTable::default(),
            reject_export_modifier,
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
        let provider_capability = None;
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
        validate_actor_declarations(&actors, &types, &dbs)?;
        Ok(SourceFile {
            provider_capability,
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
        if exported && self.reject_export_modifier {
            return Err(CompileError::syntax(
                "the export modifier has been removed; declare public API in api.yml",
                export_start,
            ));
        }
        Ok(())
    }

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

    fn import_tail_is_terminated(&self) -> bool {
        self.is_at_end()
            || self.check_symbol(";")
            || self.check_ident("export")
            || self.check_ident("test")
            || self.check_ident("import")
            || self.check_ident("provider")
            || self.check_ident("const")
            || self.check_ident("type")
            || self.check_ident("actor")
            || self.check_ident("alias")
            || self.check_ident("interface")
            || self.check_ident("impl")
            || self.check_ident("db")
            || self.check_function_start()
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
            validate_type_decl_discriminator(
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
                            "actor declarations must use `actor Name id IdType { ... }`, not `type implements Actor`",
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
        if modifiers.is_provider && self.provider_capability.is_none() {
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

    fn parse_db_decl(&mut self) -> Result<DbDecl> {
        let start = self.expect_ident_value("db")?.span.start;
        self.expect_ident_value("object")?;
        let name = self.expect_ident("expected db object name")?;
        self.expect_symbol("{")?;
        let mut collection_name = None;
        let mut key = None;
        let mut retention = None;
        let mut leases = Vec::new();
        let mut storage = Vec::new();
        let mut indexes = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_symbol(";") {
                continue;
            }
            if self.check_db_field_entry() {
                return Err(CompileError::syntax(
                    "db object stored fields must be declared on the attached type",
                    self.peek().span.start,
                ));
            } else if self.match_ident("name") {
                collection_name = Some(self.expect_string("expected db collection name string")?);
            } else if self.match_ident("retention") {
                retention = Some(self.parse_db_retention()?);
            } else if self.match_ident("lease") {
                leases.push(self.parse_db_lease_decl()?);
            } else if self.match_ident("storage") {
                let field = self.expect_ident("expected db storage field name")?;
                if self.check_symbol(".") {
                    return Err(CompileError::syntax(
                        "db storage field must be a single top-level identifier",
                        self.peek().span.start,
                    ));
                }
                self.expect_ident_value("using")?;
                let codec = match self
                    .expect_ident("expected db storage codec `encrypted`")?
                    .as_str()
                {
                    "encrypted" => DbStorageCodec::Encrypted,
                    _ => {
                        return Err(CompileError::syntax(
                            "expected db storage codec `encrypted`",
                            self.previous().span.start,
                        ));
                    }
                };
                storage.push(DbStorageDecl { field, codec });
            } else if self.match_ident("key") {
                return Err(CompileError::syntax(
                    "db object key type belongs on the attached type; use primary key(field)",
                    self.previous().span.start,
                ));
            } else if self.match_ident("primary") {
                self.expect_ident_value("key")?;
                self.expect_symbol("(")?;
                if key.is_some() {
                    return Err(CompileError::syntax(
                        "db object key is declared more than once",
                        self.previous().span.start,
                    ));
                }
                let key_name = self.expect_ident("expected db primary key field name")?;
                self.expect_symbol(")")?;
                key = Some(DbObjectKey { name: key_name });
            } else if self.match_ident("relation") {
                return Err(CompileError::syntax(
                    "db object relation declarations are not supported in object DB v1",
                    self.previous().span.start,
                ));
            } else if self.match_ident("unique") {
                self.expect_ident_value("index")?;
                indexes.push(self.parse_db_index_entry(&name, true)?);
            } else if self.match_ident("index") {
                indexes.push(self.parse_db_index_entry(&name, false)?);
            } else {
                return Err(CompileError::syntax(
                    "expected db object declaration entry",
                    self.peek().span.start,
                ));
            }
            self.match_symbol(";");
        }
        self.expect_symbol("}")?;
        let end = self.previous().span.end;
        Ok(DbDecl {
            name,
            collection_name,
            key,
            retention,
            leases,
            storage,
            indexes,
            span: SourceSpan { start, end },
        })
    }

    fn check_db_field_entry(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Ident(_))
            && matches!(
                self.tokens.get(self.current + 1).map(|token| &token.kind),
                Some(TokenKind::Symbol(symbol)) if symbol == ":"
            )
    }

    fn parse_db_retention(&mut self) -> Result<DbRetention> {
        let amount = self.expect_positive_integer("expected db retention amount")?;
        let unit = match self.expect_ident("expected db retention unit")?.as_str() {
            "days" => DbRetentionUnit::Days,
            "hours" => DbRetentionUnit::Hours,
            "minutes" => DbRetentionUnit::Minutes,
            "seconds" => DbRetentionUnit::Seconds,
            _ => {
                return Err(CompileError::syntax(
                    "expected db retention unit days, hours, minutes, or seconds",
                    self.previous().span.start,
                ));
            }
        };
        Ok(DbRetention { amount, unit })
    }

    fn parse_db_lease_decl(&mut self) -> Result<DbLeaseDecl> {
        let name = self.expect_ident("expected db lease name")?;
        self.expect_ident_value("ttl")?;
        let ttl_ms = self.expect_positive_integer("expected db lease ttl milliseconds")?;
        let max_ms = if self.match_ident("max") {
            Some(self.expect_positive_integer("expected db lease max milliseconds")?)
        } else {
            None
        };
        Ok(DbLeaseDecl {
            name,
            ttl_ms,
            max_ms,
        })
    }

    fn parse_db_index_entry(&mut self, db_name: &str, unique: bool) -> Result<DbIndexEntry> {
        let name = self.expect_ident("expected db index name")?;
        self.expect_symbol("(")?;
        let mut fields = Vec::new();
        if !self.check_symbol(")") {
            loop {
                let field_path = self.parse_field_path("expected db index field")?;
                let direction = if self.match_ident("desc") {
                    DbIndexDirection::Desc
                } else {
                    self.match_ident("asc");
                    DbIndexDirection::Asc
                };
                fields.push(DbIndexField {
                    field_path,
                    direction,
                });
                if !self.match_symbol(",") {
                    break;
                }
            }
        }
        self.expect_symbol(")")?;
        let where_expr = if self.match_ident("where") {
            let parsed = self.parse_expression()?;
            self.source_spans
                .db_index_wheres
                .push(DbIndexWhereSourceSpans {
                    db_name: db_name.to_string(),
                    index_name: name.clone(),
                    expression: parsed.spans,
                });
            Some(parsed.expr)
        } else {
            None
        };
        Ok(DbIndexEntry {
            name,
            unique,
            fields,
            where_expr,
        })
    }

    fn parse_field_path(&mut self, message: &str) -> Result<Vec<String>> {
        let mut path = vec![self.expect_ident(message)?];
        while self.match_symbol(".") {
            path.push(self.expect_ident("expected field path segment")?);
        }
        Ok(path)
    }

    fn parse_impl_methods(&mut self, target: &str) -> Result<Vec<InterfaceOperation>> {
        self.expect_symbol("{")?;
        let mut methods = Vec::new();
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
            let signature = self.parse_function_signature_and_skip_body(true)?;
            methods.push(with_impl_receiver(target, signature));
            self.match_symbol(";");
        }
        self.expect_symbol("}")?;
        Ok(methods)
    }

    fn parse_impl_methods_strict(
        &mut self,
        target: &str,
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

            match self.parse_callable_body(
                signature,
                CallableParseOptions {
                    exported: false,
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
                        self.source_spans.impl_methods.push(spans);
                    }
                    method_bodies.push(decl);
                }
                CallableParseResult::Signature(_) | CallableParseResult::None => unreachable!(),
            }
            self.match_symbol(";");
        }
        self.expect_symbol("}")?;
        Ok((methods, method_bodies))
    }

    fn parse_impl_methods_with_bodies_tolerant(
        &mut self,
        target: &str,
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

            if let CallableParseResult::Decl { decl, spans } = self.parse_callable_body(
                signature,
                CallableParseOptions {
                    exported: false,
                    provider_without_body: CallableNoBodyPolicy::EmptyDecl,
                    native_without_body: CallableNoBodyPolicy::SignatureOnly,
                    native_with_body: NativeBodyPolicy::SkipAndDrop,
                    body_policy: CallableBodyPolicy::ParseTolerantDrop,
                },
            )? {
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
        self.source_spans.consts.push(value.spans.clone());
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
        let spans = body.spans.clone();
        let end = self.previous().span.end;
        Ok(CallableParseResult::Decl {
            decl: self.build_function_decl(exported, signature, body.block, end),
            spans: Some(ExecutableSourceSpans {
                effects: Vec::new(),
                body: spans,
            }),
        })
    }

    fn parse_callable_decl_body_tolerant(
        &mut self,
        signature: InterfaceOperation,
        exported: bool,
        keep_signature_on_failure: bool,
    ) -> Result<CallableParseResult> {
        let body_start = self.current;
        match self.parse_block(false) {
            Ok(body) => {
                let spans = body.spans.clone();
                let end = self.previous().span.end;
                Ok(CallableParseResult::Decl {
                    decl: self.build_function_decl(exported, signature, body.block, end),
                    spans: Some(ExecutableSourceSpans {
                        effects: Vec::new(),
                        body: spans,
                    }),
                })
            }
            Err(_) => {
                self.current = body_start;
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

    fn skip_balanced_block(&mut self, unterminated_message: &str) -> Result<()> {
        self.expect_symbol("{")?;
        let mut depth = 1usize;
        while depth > 0 {
            let token = self.advance().clone();
            match token.kind {
                TokenKind::Symbol(value) if value == "{" => depth += 1,
                TokenKind::Symbol(value) if value == "}" => depth -= 1,
                TokenKind::Eof => {
                    return Err(CompileError::syntax(unterminated_message, token.span.start))
                }
                _ => {}
            }
        }
        Ok(())
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
            body: body.spans.clone(),
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

    fn parse_block(&mut self, in_test: bool) -> Result<ParsedBlock> {
        let start = self.peek().span.start;
        self.expect_symbol("{")?;
        let mut statements = Vec::new();
        let mut statement_spans = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_symbol(";") {
                continue;
            }
            let mut statement = self.parse_statement(in_test)?;
            if self.match_symbol(";") {
                statement.spans.span.end = self.previous().span.end;
            }
            statements.push(statement.stmt);
            statement_spans.push(statement.spans);
        }
        self.expect_symbol("}")?;
        let end = self.previous().span.end;
        Ok(ParsedBlock {
            block: Block { statements },
            spans: BlockSourceSpans {
                span: SourceSpan { start, end },
                statements: statement_spans,
            },
        })
    }

    fn parse_statement(&mut self, in_test: bool) -> Result<ParsedStmt> {
        if self.match_ident("const") {
            return self.parse_let(false, self.previous().span.start);
        }
        if self.match_ident("let") {
            return self.parse_let(true, self.previous().span.start);
        }
        if self.match_ident("timeout") {
            return self.parse_timeout_statement(in_test, self.previous().span.start);
        }
        if self.match_ident("concurrent") {
            return self.parse_concurrent_statement(in_test, self.previous().span.start);
        }
        if self.match_ident("serial") {
            return self.parse_serial_statement(in_test, self.previous().span.start);
        }
        if self.match_ident("if") {
            return self.parse_if(in_test, self.previous().span.start);
        }
        if self.match_ident("for") {
            return self.parse_for(in_test, self.previous().span.start);
        }
        if self.match_ident("while") {
            return self.parse_while(in_test, self.previous().span.start);
        }
        if self.match_ident("match") {
            return self.parse_match(in_test, self.previous().span.start);
        }
        if self.match_ident("assert") {
            let start = self.previous().span.start;
            if !in_test {
                return Err(CompileError::syntax(
                    "assert can only be used in test blocks",
                    self.previous().span.start,
                ));
            }
            return self.parse_assert_statement(start);
        }
        if self.match_ident("return") {
            let start = self.previous().span.start;
            if self.check_symbol("}") || self.check_symbol(";") {
                return Ok(ParsedStmt {
                    stmt: Stmt::Return(None),
                    spans: StmtSourceSpans {
                        span: SourceSpan {
                            start,
                            end: self.previous().span.end,
                        },
                        expressions: Vec::new(),
                        blocks: Vec::new(),
                    },
                });
            }
            let value = self.parse_expression()?;
            let end = value.spans.span.end;
            return Ok(ParsedStmt {
                stmt: Stmt::Return(Some(value.expr)),
                spans: StmtSourceSpans {
                    span: SourceSpan { start, end },
                    expressions: vec![value.spans],
                    blocks: Vec::new(),
                },
            });
        }
        if self.match_ident("spawn") {
            let start = self.previous().span.start;
            let call = self.parse_expression()?;
            if !matches!(call.expr, Expr::Call { .. }) {
                return Err(CompileError::syntax(
                    "spawn statement expects a call expression",
                    call.spans.span.start,
                ));
            }
            let end = call.spans.span.end;
            return Ok(ParsedStmt {
                stmt: Stmt::Spawn { call: call.expr },
                spans: StmtSourceSpans {
                    span: SourceSpan { start, end },
                    expressions: vec![call.spans],
                    blocks: Vec::new(),
                },
            });
        }
        if self.match_ident("throw") {
            let start = self.previous().span.start;
            let value = self.parse_expression()?;
            let end = value.spans.span.end;
            return Ok(ParsedStmt {
                stmt: Stmt::Throw { value: value.expr },
                spans: StmtSourceSpans {
                    span: SourceSpan { start, end },
                    expressions: vec![value.spans],
                    blocks: Vec::new(),
                },
            });
        }
        if self.match_ident("rethrow") {
            let start = self.previous().span.start;
            let exception = self.parse_expression()?;
            let end = exception.spans.span.end;
            return Ok(ParsedStmt {
                stmt: Stmt::Rethrow {
                    exception: exception.expr,
                },
                spans: StmtSourceSpans {
                    span: SourceSpan { start, end },
                    expressions: vec![exception.spans],
                    blocks: Vec::new(),
                },
            });
        }
        if self.match_ident("emit") {
            let start = self.previous().span.start;
            let value = if self.match_symbol("(") {
                let value = self.parse_expression()?;
                self.expect_symbol(")")?;
                value
            } else {
                self.parse_expression()?
            };
            let end = self.previous().span.end;
            return Ok(ParsedStmt {
                stmt: Stmt::Emit(value.expr),
                spans: StmtSourceSpans {
                    span: SourceSpan { start, end },
                    expressions: vec![value.spans],
                    blocks: Vec::new(),
                },
            });
        }
        if self.match_ident("break") {
            let span = self.previous().span;
            return Ok(ParsedStmt {
                stmt: Stmt::Break,
                spans: StmtSourceSpans {
                    span,
                    expressions: Vec::new(),
                    blocks: Vec::new(),
                },
            });
        }
        if self.match_ident("continue") {
            let span = self.previous().span;
            return Ok(ParsedStmt {
                stmt: Stmt::Continue,
                spans: StmtSourceSpans {
                    span,
                    expressions: Vec::new(),
                    blocks: Vec::new(),
                },
            });
        }
        let expr = self.parse_expression()?;
        if self.match_symbol("=") {
            let value = self.parse_expression()?;
            let span = SourceSpan {
                start: expr.spans.span.start,
                end: value.spans.span.end,
            };
            return Ok(ParsedStmt {
                stmt: Stmt::Assign {
                    target: expr.expr,
                    value: value.expr,
                },
                spans: StmtSourceSpans {
                    span,
                    expressions: vec![expr.spans, value.spans],
                    blocks: Vec::new(),
                },
            });
        }
        Ok(ParsedStmt {
            stmt: Stmt::Expr(expr.expr),
            spans: StmtSourceSpans {
                span: expr.spans.span,
                expressions: vec![expr.spans],
                blocks: Vec::new(),
            },
        })
    }

    fn parse_timeout_statement(
        &mut self,
        in_test: bool,
        start: SourceLocation,
    ) -> Result<ParsedStmt> {
        let duration = self.parse_timeout_duration()?;
        if self.check_ident("value") || self.check_ident("concurrent") {
            return self
                .parse_timeout_value_after_duration(start, duration)
                .map(parsed_expression_statement);
        }
        if !self.check_symbol("{") {
            return Err(CompileError::syntax(
                "expected timeout body",
                self.peek().span.start,
            ));
        }
        let body = self.parse_block(in_test)?;
        let end = body.spans.span.end;
        Ok(ParsedStmt {
            stmt: Stmt::Timeout {
                duration,
                body: body.block,
            },
            spans: StmtSourceSpans {
                span: SourceSpan { start, end },
                expressions: Vec::new(),
                blocks: vec![body.spans],
            },
        })
    }

    fn parse_concurrent_statement(
        &mut self,
        in_test: bool,
        start: SourceLocation,
    ) -> Result<ParsedStmt> {
        if self.match_ident("value") {
            return self
                .parse_value_block_expression(start, true)
                .map(parsed_expression_statement);
        }
        if !self.check_symbol("{") {
            let message = if self.check_ident("timeout")
                || self.check_ident("serial")
                || self.check_ident("concurrent")
            {
                "noncanonical modifier order; use `timeout(...) concurrent value { ... }`"
            } else {
                "expected concurrent body"
            };
            return Err(CompileError::syntax(message, self.peek().span.start));
        }
        let body = self.parse_block(in_test)?;
        let end = body.spans.span.end;
        Ok(ParsedStmt {
            stmt: Stmt::Concurrent { body: body.block },
            spans: StmtSourceSpans {
                span: SourceSpan { start, end },
                expressions: Vec::new(),
                blocks: vec![body.spans],
            },
        })
    }

    fn parse_serial_statement(
        &mut self,
        in_test: bool,
        start: SourceLocation,
    ) -> Result<ParsedStmt> {
        if !self.check_symbol("{") {
            return Err(CompileError::syntax(
                "expected serial body",
                self.peek().span.start,
            ));
        }
        let body = self.parse_block(in_test)?;
        let end = body.spans.span.end;
        Ok(ParsedStmt {
            stmt: Stmt::Serial { body: body.block },
            spans: StmtSourceSpans {
                span: SourceSpan { start, end },
                expressions: Vec::new(),
                blocks: vec![body.spans],
            },
        })
    }

    fn parse_assert_statement(&mut self, start: SourceLocation) -> Result<ParsedStmt> {
        let condition = self.parse_expression()?;
        let message = if self.match_symbol(",") {
            Some(self.expect_string("expected assert message string")?)
        } else {
            None
        };
        let end = if message.is_some() {
            self.previous().span.end
        } else {
            condition.spans.span.end
        };
        Ok(ParsedStmt {
            stmt: Stmt::Assert {
                condition: condition.expr,
                message,
            },
            spans: StmtSourceSpans {
                span: SourceSpan { start, end },
                expressions: vec![condition.spans],
                blocks: Vec::new(),
            },
        })
    }

    fn parse_let(&mut self, mutable: bool, start: SourceLocation) -> Result<ParsedStmt> {
        let name = self.expect_ident("expected binding name")?;
        let ty = if self.match_symbol(":") {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect_symbol("=")?;
        let value = self.parse_expression()?;
        let end = value.spans.span.end;
        Ok(ParsedStmt {
            stmt: Stmt::Let {
                mutable,
                name,
                ty,
                value: value.expr,
            },
            spans: StmtSourceSpans {
                span: SourceSpan { start, end },
                expressions: vec![value.spans],
                blocks: Vec::new(),
            },
        })
    }

    fn parse_if(&mut self, in_test: bool, start: SourceLocation) -> Result<ParsedStmt> {
        let condition = self.parse_expression()?;
        let then_block = self.parse_block(in_test)?;
        let else_block = if self.match_ident("else") {
            if self.match_ident("if") {
                let nested_if = self.parse_if(in_test, self.previous().span.start)?;
                Some(ParsedBlock {
                    spans: BlockSourceSpans {
                        span: nested_if.spans.span,
                        statements: vec![nested_if.spans],
                    },
                    block: Block {
                        statements: vec![nested_if.stmt],
                    },
                })
            } else {
                Some(self.parse_block(in_test)?)
            }
        } else {
            None
        };
        let end = else_block
            .as_ref()
            .map(|block| block.spans.span.end)
            .unwrap_or(then_block.spans.span.end);
        let mut blocks = vec![then_block.spans.clone()];
        if let Some(else_block) = &else_block {
            blocks.push(else_block.spans.clone());
        }
        Ok(ParsedStmt {
            stmt: Stmt::If {
                condition: condition.expr,
                then_block: then_block.block,
                else_block: else_block.map(|block| block.block),
            },
            spans: StmtSourceSpans {
                span: SourceSpan { start, end },
                expressions: vec![condition.spans],
                blocks,
            },
        })
    }

    fn parse_for(&mut self, in_test: bool, start: SourceLocation) -> Result<ParsedStmt> {
        let first = self.expect_ident("expected loop item name")?;
        let binding = if self.match_symbol(",") {
            let value = self.expect_ident("expected loop value name")?;
            ForBinding::Entry { key: first, value }
        } else {
            ForBinding::Item { item: first }
        };
        self.expect_ident_value("in")?;
        let iterable = self.parse_expression()?;
        let body = self.parse_block(in_test)?;
        let end = body.spans.span.end;
        Ok(ParsedStmt {
            stmt: Stmt::For {
                binding,
                iterable: iterable.expr,
                body: body.block,
            },
            spans: StmtSourceSpans {
                span: SourceSpan { start, end },
                expressions: vec![iterable.spans],
                blocks: vec![body.spans],
            },
        })
    }

    fn parse_while(&mut self, in_test: bool, start: SourceLocation) -> Result<ParsedStmt> {
        let condition = self.parse_expression()?;
        let body = self.parse_block(in_test)?;
        let end = body.spans.span.end;
        Ok(ParsedStmt {
            stmt: Stmt::While {
                condition: condition.expr,
                body: body.block,
            },
            spans: StmtSourceSpans {
                span: SourceSpan { start, end },
                expressions: vec![condition.spans],
                blocks: vec![body.spans],
            },
        })
    }

    fn parse_match(&mut self, in_test: bool, start: SourceLocation) -> Result<ParsedStmt> {
        let value = self.parse_expression()?;
        let mut arms = Vec::new();
        let mut blocks = Vec::new();
        self.expect_symbol("{")?;
        while !self.check_symbol("}") && !self.is_at_end() {
            let pattern = self.parse_pattern()?;
            self.expect_symbol("=>")?;
            let body = self.parse_block(in_test)?;
            blocks.push(body.spans);
            arms.push(MatchArm {
                pattern,
                body: body.block,
            });
        }
        self.expect_symbol("}")?;
        let end = self.previous().span.end;
        Ok(ParsedStmt {
            stmt: Stmt::Match {
                value: value.expr,
                arms,
            },
            spans: StmtSourceSpans {
                span: SourceSpan { start, end },
                expressions: vec![value.spans],
                blocks,
            },
        })
    }

    fn parse_pattern(&mut self) -> Result<Pattern> {
        let mut patterns = vec![self.parse_primary_pattern()?];
        while self.match_symbol("|") {
            patterns.push(self.parse_primary_pattern()?);
        }
        if patterns.len() == 1 {
            Ok(patterns.pop().expect("one pattern"))
        } else {
            Ok(Pattern::Or(patterns))
        }
    }

    fn parse_primary_pattern(&mut self) -> Result<Pattern> {
        if self.match_ident("_") {
            return Ok(Pattern::Wildcard);
        }
        if self.match_symbol("{") {
            return Ok(Pattern::Record {
                fields: self.parse_pattern_fields_after_open_brace()?,
            });
        }
        if self.match_ident("true") {
            return Ok(Pattern::Literal(Literal::Bool(true)));
        }
        if self.match_ident("false") {
            return Ok(Pattern::Literal(Literal::Bool(false)));
        }
        if self.match_ident("null") {
            return Ok(Pattern::Literal(Literal::Null));
        }
        if matches!(self.peek().kind, TokenKind::String(_)) {
            let TokenKind::String(value) = self.advance().kind.clone() else {
                unreachable!();
            };
            return Ok(Pattern::Literal(Literal::String(value)));
        }
        if matches!(self.peek().kind, TokenKind::Number(_)) {
            let TokenKind::Number(value) = self.advance().kind.clone() else {
                unreachable!();
            };
            return Ok(Pattern::Literal(Literal::Number(value)));
        }

        let name = self.expect_ident("expected pattern")?;
        let snapshot = self.current;
        let type_args = if self.check_symbol("<") {
            match self.parse_generic_args() {
                Ok(type_args) => type_args,
                Err(_) => {
                    self.current = snapshot;
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        if self.match_symbol("{") {
            return Ok(Pattern::Nominal {
                name,
                type_args,
                fields: self.parse_pattern_fields_after_open_brace()?,
            });
        }
        self.current = snapshot;
        Ok(Pattern::Binding(name))
    }

    fn parse_pattern_fields_after_open_brace(&mut self) -> Result<Vec<PatternField>> {
        let mut fields = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            let name = self.expect_ident("expected record pattern field name")?;
            let pattern = if self.match_symbol(":") {
                Some(self.parse_pattern()?)
            } else {
                None
            };
            fields.push(PatternField { name, pattern });
            self.match_symbol(",");
        }
        self.expect_symbol("}")?;
        Ok(fields)
    }

    fn parse_expression(&mut self) -> Result<ParsedExpr> {
        self.parse_binary(0)
    }

    fn parse_binary(&mut self, min_prec: u8) -> Result<ParsedExpr> {
        let mut left = self.parse_unary()?;
        while let Some((op, prec)) = self.peek_binary_op() {
            if prec < min_prec {
                break;
            }
            self.advance();
            let right = self.parse_binary(prec + 1)?;
            let span = SourceSpan {
                start: left.spans.span.start,
                end: right.spans.span.end,
            };
            left = ParsedExpr {
                expr: Expr::Binary {
                    op,
                    left: Box::new(left.expr),
                    right: Box::new(right.expr),
                },
                spans: expr_source_spans(span, vec![left.spans, right.spans]),
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<ParsedExpr> {
        if self.match_symbol("!") {
            let start = self.previous().span.start;
            let expr = self.parse_unary()?;
            return Ok(ParsedExpr {
                expr: Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr.expr),
                },
                spans: expr_source_spans(
                    SourceSpan {
                        start,
                        end: expr.spans.span.end,
                    },
                    vec![expr.spans],
                ),
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<ParsedExpr> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.check_dependency_source_address_suffix(&expr) {
                expr = self.parse_dependency_source_address(expr)?;
                continue;
            }
            if self.match_symbol(".") {
                let field = self.expect_ident("expected field name after .")?;
                let span = SourceSpan {
                    start: expr.spans.span.start,
                    end: self.previous().span.end,
                };
                expr = ParsedExpr {
                    expr: Expr::Field {
                        object: Box::new(expr.expr),
                        field,
                    },
                    spans: expr_source_spans(span, vec![expr.spans]),
                };
                continue;
            }
            if self.check_symbol("<") && self.looks_like_generic_call_suffix() {
                let type_args = self.parse_generic_args()?;
                let span = SourceSpan {
                    start: expr.spans.span.start,
                    end: self.previous().span.end,
                };
                expr = ParsedExpr {
                    expr: Expr::Generic {
                        callee: Box::new(expr.expr),
                        type_args,
                    },
                    spans: expr_source_spans(span, vec![expr.spans]),
                };
                continue;
            }
            if self.match_symbol("(") {
                let mut args = Vec::new();
                let mut children = vec![expr.spans];
                if !self.check_symbol(")") {
                    loop {
                        let arg = self.parse_expression()?;
                        children.push(arg.spans);
                        args.push(arg.expr);
                        if !self.match_symbol(",") {
                            break;
                        }
                    }
                }
                self.expect_symbol(")")?;
                let span = SourceSpan {
                    start: children[0].span.start,
                    end: self.previous().span.end,
                };
                expr = ParsedExpr {
                    expr: Expr::Call {
                        callee: Box::new(expr.expr),
                        args,
                    },
                    spans: expr_source_spans(span, children),
                };
                continue;
            }
            if self.match_ident("as") {
                let as_start = self.previous().span.start;
                if self.check_ident("any") {
                    return Err(CompileError::syntax(
                        "`as` expects an interface selector; use `as I`, not `as any I`",
                        self.peek().span.start,
                    ));
                }
                let interface = self.parse_type()?;
                let span = SourceSpan {
                    start: expr.spans.span.start,
                    end: self.previous().span.end,
                };
                if interface.name.trim().is_empty() {
                    return Err(CompileError::syntax(
                        "expected interface selector after `as`",
                        as_start,
                    ));
                }
                expr = ParsedExpr {
                    expr: Expr::InterfaceBox {
                        value: Box::new(expr.expr),
                        interface,
                    },
                    spans: expr_source_spans(span, vec![expr.spans]),
                };
                continue;
            }
            if self.check_symbol("{") {
                if let Some(target) = Self::patch_construct_target(&expr.expr) {
                    self.advance();
                    let (operations, operation_spans) = self.parse_patch_operations()?;
                    let mut children = vec![expr.spans];
                    children.extend(operation_spans);
                    let span = SourceSpan {
                        start: children[0].span.start,
                        end: self.previous().span.end,
                    };
                    expr = ParsedExpr {
                        expr: Expr::Patch { target, operations },
                        spans: expr_source_spans(span, children),
                    };
                    continue;
                }
                if let Some((type_name, type_args)) = Self::nominal_construct_parts(&expr.expr) {
                    self.advance();
                    let (fields, field_spans, record_fields) =
                        self.parse_record_construct_fields()?;
                    let mut children = vec![expr.spans];
                    children.extend(field_spans);
                    let span = SourceSpan {
                        start: children[0].span.start,
                        end: self.previous().span.end,
                    };
                    expr = ParsedExpr {
                        expr: Expr::Record {
                            type_name,
                            type_args,
                            fields,
                        },
                        spans: ExprSourceSpans {
                            span,
                            children,
                            blocks: Vec::new(),
                            record_fields,
                        },
                    };
                    continue;
                }
            }
            break;
        }
        Ok(expr)
    }

    fn patch_construct_target(expr: &Expr) -> Option<TypeRef> {
        let Expr::Generic { callee, type_args } = expr else {
            return None;
        };
        let Expr::Identifier(name) = callee.as_ref() else {
            return None;
        };
        if name != "patch" || type_args.len() != 1 {
            return None;
        }
        type_args.first().cloned()
    }

    fn looks_like_generic_call_suffix(&self) -> bool {
        let mut probe = Parser {
            tokens: self.tokens.clone(),
            current: self.current,
            mode: self.mode,
            provider_capability: self.provider_capability.clone(),
            source_spans: SourceSpanTable::default(),
            reject_export_modifier: self.reject_export_modifier,
        };
        probe.parse_generic_args().is_ok() && (probe.check_symbol("(") || probe.check_symbol("{"))
    }

    fn nominal_construct_parts(expr: &Expr) -> Option<(String, Vec<TypeRef>)> {
        let (callee, type_args) = match expr {
            Expr::Generic { callee, type_args } => (callee.as_ref(), type_args.clone()),
            _ => (expr, Vec::new()),
        };
        let type_name = expr_path(without_generic(callee))?;
        let tail = type_name.rsplit('.').next()?;
        tail.chars()
            .next()
            .is_some_and(char::is_uppercase)
            .then_some((type_name, type_args))
    }

    fn parse_generic_args(&mut self) -> Result<Vec<TypeRef>> {
        self.expect_symbol("<")?;
        let mut type_args = Vec::new();
        if !self.check_symbol(">") {
            loop {
                type_args.push(self.parse_type()?);
                if !self.match_symbol(",") {
                    break;
                }
            }
        }
        self.expect_symbol(">")?;
        Ok(type_args)
    }

    fn parse_primary(&mut self) -> Result<ParsedExpr> {
        let token = self.advance().clone();
        let start = token.span.start;
        match token.kind {
            TokenKind::Number(value) => Ok(parsed_leaf_expr(
                Expr::Literal(Literal::Number(value)),
                token.span,
            )),
            TokenKind::Duration(_) => Err(CompileError::syntax(
                "duration literal is only allowed as a timeout duration",
                token.span.start,
            )),
            TokenKind::String(value) => Ok(parsed_leaf_expr(
                Expr::Literal(Literal::String(value)),
                token.span,
            )),
            TokenKind::Ident(value) if value == "true" => Ok(parsed_leaf_expr(
                Expr::Literal(Literal::Bool(true)),
                token.span,
            )),
            TokenKind::Ident(value) if value == "false" => Ok(parsed_leaf_expr(
                Expr::Literal(Literal::Bool(false)),
                token.span,
            )),
            TokenKind::Ident(value) if value == "null" => {
                Ok(parsed_leaf_expr(Expr::Literal(Literal::Null), token.span))
            }
            TokenKind::Ident(value) if value == "value" && self.check_symbol("{") => {
                self.parse_value_block_expression(start, false)
            }
            TokenKind::Ident(value)
                if value == "value"
                    && (self.check_ident("timeout")
                        || self.check_ident("concurrent")
                        || self.check_ident("serial")
                        || self.check_ident("value")) =>
            {
                Err(CompileError::syntax(
                    "noncanonical modifier order; use `value`, `concurrent value`, `timeout(...) value`, or `timeout(...) concurrent value`",
                    self.peek().span.start,
                ))
            }
            TokenKind::Ident(value) if value == "concurrent" && self.check_ident("value") => {
                self.advance();
                self.parse_value_block_expression(start, true)
            }
            TokenKind::Ident(value)
                if value == "concurrent"
                    && (self.check_ident("timeout")
                        || self.check_ident("serial")
                        || self.check_ident("concurrent")) =>
            {
                Err(CompileError::syntax(
                    "noncanonical modifier order; expression form is `concurrent value { ... }`",
                    self.peek().span.start,
                ))
            }
            TokenKind::Ident(value) if value == "timeout" && self.check_symbol("(") => {
                self.parse_timeout_expression(start)
            }
            TokenKind::Ident(value)
                if value == "timeout"
                    && (self.check_ident("value")
                        || self.check_ident("concurrent")
                        || self.check_ident("serial")
                        || self.check_ident("timeout")) =>
            {
                Err(CompileError::syntax(
                    "noncanonical modifier order; timeout must be followed by `(duration)`",
                    self.peek().span.start,
                ))
            }
            TokenKind::Ident(value) if value == "serial" && self.check_symbol("{") => {
                Err(CompileError::syntax(
                    "serial is a statement and cannot be used as an expression",
                    token.span.start,
                ))
            }
            TokenKind::Ident(value)
                if value == "serial"
                    && (self.check_ident("value")
                        || self.check_ident("concurrent")
                        || self.check_ident("timeout")
                        || self.check_ident("serial")) =>
            {
                Err(CompileError::syntax(
                    "noncanonical modifier order; serial is only `serial { ... }`",
                    self.peek().span.start,
                ))
            }
            TokenKind::Ident(value) if value == "concurrent" && self.check_symbol("{") => {
                Err(CompileError::syntax(
                    "concurrent is a statement and cannot be used as an expression",
                    token.span.start,
                ))
            }
            TokenKind::Ident(value) if value == "throw" => {
                let value = self.parse_expression()?;
                Ok(ParsedExpr {
                    expr: Expr::Throw {
                        value: Box::new(value.expr),
                    },
                    spans: expr_source_spans(
                        SourceSpan {
                            start,
                            end: value.spans.span.end,
                        },
                        vec![value.spans],
                    ),
                })
            }
            TokenKind::Ident(value) if value == "rethrow" => {
                let exception = self.parse_expression()?;
                Ok(ParsedExpr {
                    expr: Expr::Rethrow {
                        exception: Box::new(exception.expr),
                    },
                    spans: expr_source_spans(
                        SourceSpan {
                            start,
                            end: exception.spans.span.end,
                        },
                        vec![exception.spans],
                    ),
                })
            }
            TokenKind::Ident(value) if value == "catch" => self.parse_catch_expression(start),
            TokenKind::Ident(value) if value == "db" => self.parse_db_expression(token.span),
            TokenKind::Ident(value) if value == "process" => Err(CompileError::syntax(
                "process has been removed; use actors and spawn instead",
                token.span.start,
            )),
            TokenKind::Ident(value) if value == "spawn" => Err(CompileError::syntax(
                "spawn is a statement and cannot be used as an expression",
                token.span.start,
            )),
            TokenKind::Ident(value) => {
                if self.check_symbol("{") && value.chars().next().is_some_and(char::is_uppercase) {
                    self.advance();
                    let (fields, children, record_fields) = self.parse_record_construct_fields()?;
                    Ok(ParsedExpr {
                        expr: Expr::Record {
                            type_name: value,
                            type_args: Vec::new(),
                            fields,
                        },
                        spans: ExprSourceSpans {
                            span: SourceSpan {
                                start,
                                end: self.previous().span.end,
                            },
                            children,
                            blocks: Vec::new(),
                            record_fields,
                        },
                    })
                } else {
                    Ok(parsed_leaf_expr(Expr::Identifier(value), token.span))
                }
            }
            TokenKind::Symbol(value) if value == "(" => {
                let expr = self.parse_expression()?;
                self.expect_symbol(")")?;
                Ok(expr)
            }
            TokenKind::Symbol(value) if value == "{" => {
                let (entries, children, record_fields) = self.parse_object_literal_entries()?;
                Ok(ParsedExpr {
                    expr: Expr::ObjectLiteral { entries },
                    spans: ExprSourceSpans {
                        span: SourceSpan {
                            start,
                            end: self.previous().span.end,
                        },
                        children,
                        blocks: Vec::new(),
                        record_fields,
                    },
                })
            }
            _ => Err(CompileError::syntax(
                "expected expression",
                token.span.start,
            )),
        }
    }

    fn parse_timeout_duration(&mut self) -> Result<DurationLiteral> {
        self.expect_symbol("(")?;
        let token = self.advance().clone();
        let TokenKind::Duration(duration) = token.kind else {
            return Err(CompileError::syntax(
                "expected a duration literal in timeout(...)",
                token.span.start,
            ));
        };
        duration
            .checked_milliseconds()
            .map_err(|error| CompileError::syntax(error.to_string(), duration.span.start))?;
        self.expect_symbol(")")?;
        Ok(duration)
    }

    fn parse_timeout_expression(&mut self, start: SourceLocation) -> Result<ParsedExpr> {
        let duration = self.parse_timeout_duration()?;
        self.parse_timeout_value_after_duration(start, duration)
    }

    fn parse_timeout_value_after_duration(
        &mut self,
        start: SourceLocation,
        duration: DurationLiteral,
    ) -> Result<ParsedExpr> {
        let value = if self.match_ident("value") {
            let value_start = self.previous().span.start;
            self.parse_value_block_expression(value_start, false)?
        } else if self.match_ident("concurrent") {
            let concurrent_start = self.previous().span.start;
            if !self.match_ident("value") {
                return Err(CompileError::syntax(
                    "noncanonical modifier order; use `timeout(...) concurrent value { ... }`",
                    self.peek().span.start,
                ));
            }
            self.parse_value_block_expression(concurrent_start, true)?
        } else {
            return Err(CompileError::syntax(
                "noncanonical modifier order; timeout value form must be `timeout(...) value { ... }` or `timeout(...) concurrent value { ... }`",
                self.peek().span.start,
            ));
        };
        let end = value.spans.span.end;
        Ok(ParsedExpr {
            expr: Expr::Timeout {
                duration,
                value: Box::new(value.expr),
            },
            spans: expr_source_spans(SourceSpan { start, end }, vec![value.spans]),
        })
    }

    fn parse_value_block_expression(
        &mut self,
        start: SourceLocation,
        concurrent: bool,
    ) -> Result<ParsedExpr> {
        if self.check_ident("timeout")
            || self.check_ident("concurrent")
            || self.check_ident("serial")
            || self.check_ident("value")
        {
            return Err(CompileError::syntax(
                "noncanonical modifier order; use `value`, `concurrent value`, `timeout(...) value`, or `timeout(...) concurrent value`",
                self.peek().span.start,
            ));
        }
        self.expect_symbol("{")?;
        let block_start = self.previous().span.start;
        let mut statements = Vec::new();
        let mut statement_spans = Vec::new();
        let mut statement_terminated = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_symbol(";") {
                continue;
            }
            if self.check_symbol("{") {
                return Err(CompileError::syntax(
                    "value block object literal tail must be parenthesized",
                    self.peek().span.start,
                ));
            }
            let mut statement = self.parse_statement(false)?;
            let terminated = self.match_symbol(";");
            if terminated {
                statement.spans.span.end = self.previous().span.end;
            }
            statements.push(statement.stmt);
            statement_spans.push(statement.spans);
            statement_terminated.push(terminated);
        }
        self.expect_symbol("}")?;
        let block_end = self.previous().span.end;
        let missing_tail = || {
            CompileError::syntax(
                "value block requires a tail expression",
                self.previous().span.start,
            )
        };
        let Some(last_statement) = statements.pop() else {
            return Err(missing_tail());
        };
        let Some(last_spans) = statement_spans.pop() else {
            unreachable!("value block statement and span counts must match");
        };
        let Some(last_terminated) = statement_terminated.pop() else {
            unreachable!("value block statement and terminator counts must match");
        };
        if last_terminated {
            return Err(missing_tail());
        }
        let (tail, tail_spans) = match last_statement {
            Stmt::Expr(tail) => {
                let mut tail_expression_spans = last_spans.expressions.into_iter();
                let tail_spans = tail_expression_spans
                    .next()
                    .expect("expression statement must carry expression spans");
                debug_assert!(tail_expression_spans.next().is_none());
                (tail, tail_spans)
            }
            Stmt::Throw { value } => (
                Expr::Throw {
                    value: Box::new(value),
                },
                expr_source_spans(last_spans.span, last_spans.expressions),
            ),
            Stmt::Rethrow { exception } => (
                Expr::Rethrow {
                    exception: Box::new(exception),
                },
                expr_source_spans(last_spans.span, last_spans.expressions),
            ),
            _ => return Err(missing_tail()),
        };
        let body_spans = BlockSourceSpans {
            span: SourceSpan {
                start: block_start,
                end: block_end,
            },
            statements: statement_spans,
        };
        let value = ValueBlock {
            body: Block { statements },
            tail: Box::new(tail),
        };
        Ok(ParsedExpr {
            expr: if concurrent {
                Expr::ConcurrentValue(value)
            } else {
                Expr::ValueBlock(value)
            },
            spans: ExprSourceSpans {
                span: SourceSpan {
                    start,
                    end: block_end,
                },
                children: vec![tail_spans],
                blocks: vec![body_spans],
                record_fields: Vec::new(),
            },
        })
    }

    fn check_dependency_source_address_suffix(&self, expr: &ParsedExpr) -> bool {
        if !matches!(expr.expr, Expr::Identifier(_)) {
            return false;
        }
        let slash = self.peek();
        if !matches!(&slash.kind, TokenKind::Symbol(value) if value == "/") {
            return false;
        }
        let Some(segment) = self.tokens.get(self.current + 1) else {
            return false;
        };
        matches!(segment.kind, TokenKind::Ident(_))
            && contiguous_locations(expr.spans.span.end, slash.span.start)
            && contiguous_locations(slash.span.end, segment.span.start)
    }

    fn parse_dependency_source_address(&mut self, expr: ParsedExpr) -> Result<ParsedExpr> {
        let Expr::Identifier(dependency_ref) = expr.expr else {
            unreachable!("remote source suffix is only checked for identifiers");
        };
        self.advance();
        let first_segment = self.expect_ident("expected public instance key after /")?;
        let mut segments = vec![first_segment];
        let mut end = self.previous().span.end;
        while self.check_symbol("/") {
            let slash_token = self.peek().clone();
            let Some(next) = self.tokens.get(self.current + 1) else {
                break;
            };
            if !matches!(next.kind, TokenKind::Ident(_))
                || !contiguous_locations(end, slash_token.span.start)
                || !contiguous_locations(slash_token.span.end, next.span.start)
            {
                break;
            }
            self.advance();
            segments.push(self.expect_ident("expected public instance key segment after /")?);
            end = self.previous().span.end;
        }
        let span = SourceSpan {
            start: expr.spans.span.start,
            end,
        };
        Ok(ParsedExpr {
            expr: Expr::DependencySourceAddress(DependencySourceAddress {
                dependency_ref,
                public_path: segments.join("."),
            }),
            spans: expr_source_spans(span, Vec::new()),
        })
    }

    fn parse_db_expression(&mut self, span: SourceSpan) -> Result<ParsedExpr> {
        if self.check_symbol(".") {
            if let Some(TokenKind::Ident(operation)) =
                self.tokens.get(self.current + 1).map(|token| &token.kind)
            {
                if is_old_db_dotted_operation(operation) {
                    return Err(CompileError::syntax(
                        "old db.transaction/db.* syntax is not supported; use `db transaction` or explicit db operations",
                        self.peek().span.start,
                    ));
                }
            }
            return Ok(parsed_leaf_expr(Expr::Identifier("db".to_string()), span));
        }
        let start = span.start;
        if self.match_ident("transaction") {
            return self.parse_db_transaction_expr(start);
        }
        if self.match_ident("claim") {
            return self.parse_db_lease_claim_expr(start);
        }
        if self.match_ident("lease") {
            return self.parse_db_lease_read_expr(start);
        }
        if self.match_ident("query") {
            return self.parse_db_query_expr(start);
        }
        self.parse_db_operation_expr(start)
    }

    fn parse_db_transaction_expr(&mut self, start: SourceLocation) -> Result<ParsedExpr> {
        let mode = if self.match_ident("value") {
            DbBlockMode::Value
        } else {
            DbBlockMode::Effect
        };
        let body = self.parse_block(false)?;
        let body_end = body.spans.span.end;
        let body_spans = body.spans.clone();
        Ok(ParsedExpr {
            expr: Expr::DbTransaction(DbTransaction {
                mode,
                body: body.block,
            }),
            spans: ExprSourceSpans {
                blocks: vec![body_spans],
                record_fields: Vec::new(),
                children: Vec::new(),
                span: SourceSpan {
                    start,
                    end: body_end,
                },
            },
        })
    }

    fn parse_db_query_expr(&mut self, start: SourceLocation) -> Result<ParsedExpr> {
        let target = self.parse_db_operation_target()?;
        let (query, children) = self.parse_db_query_block()?;
        Ok(ParsedExpr {
            expr: Expr::DbQuery(DbQuery { target, query }),
            spans: expr_source_spans(
                SourceSpan {
                    start,
                    end: self.previous().span.end,
                },
                children,
            ),
        })
    }

    fn parse_db_lease_claim_expr(&mut self, start: SourceLocation) -> Result<ParsedExpr> {
        let (target, key, slot, key_spans) = self.parse_db_lease_access()?;
        let binding = if self.match_ident("as") {
            Some(self.expect_ident("expected db claim binding name")?)
        } else {
            None
        };
        let body = self.parse_block(false)?;
        let body_end = body.spans.span.end;
        let body_spans = body.spans.clone();
        Ok(ParsedExpr {
            expr: Expr::DbLeaseClaim(DbLeaseClaim {
                target,
                key: Box::new(key),
                slot,
                binding,
                body: body.block,
            }),
            spans: ExprSourceSpans {
                blocks: vec![body_spans],
                record_fields: Vec::new(),
                children: vec![key_spans],
                span: SourceSpan {
                    start,
                    end: body_end,
                },
            },
        })
    }

    fn parse_db_lease_read_expr(&mut self, start: SourceLocation) -> Result<ParsedExpr> {
        let (target, key, slot, key_spans) = self.parse_db_lease_access()?;
        Ok(ParsedExpr {
            expr: Expr::DbLeaseRead(DbLeaseRead {
                target,
                key: Box::new(key),
                slot,
            }),
            spans: expr_source_spans(
                SourceSpan {
                    start,
                    end: self.previous().span.end,
                },
                vec![key_spans],
            ),
        })
    }

    fn parse_db_lease_access(&mut self) -> Result<(TypeRef, Expr, String, ExprSourceSpans)> {
        let target = self.parse_db_operation_target()?;
        self.expect_symbol("(")?;
        let key = self.parse_expression()?;
        self.expect_symbol(")")?;
        self.expect_symbol(".")?;
        let slot = self.expect_ident("expected db lease slot name")?;
        Ok((target, key.expr, slot, key.spans))
    }

    fn parse_db_operation_expr(&mut self, start: SourceLocation) -> Result<ParsedExpr> {
        let op_token = self.advance().clone();
        let op = match &op_token.kind {
            TokenKind::Ident(value) => match value.as_str() {
                "find" => DbOperationKind::Find,
                "optional" => DbOperationKind::Optional,
                "require" => DbOperationKind::Require,
                "insert" => DbOperationKind::Insert,
                "update" => DbOperationKind::Update,
                "upsert" => DbOperationKind::Upsert,
                "replace" => DbOperationKind::Replace,
                "delete" => DbOperationKind::Delete,
                "count" => DbOperationKind::Count,
                "exists" => DbOperationKind::Exists,
                _ => {
                    return Err(CompileError::syntax(
                        "expected db operation",
                        op_token.span.start,
                    ));
                }
            },
            _ => {
                return Err(CompileError::syntax(
                    "expected db operation",
                    op_token.span.start,
                ))
            }
        };
        let supports_many = matches!(
            op,
            DbOperationKind::Find
                | DbOperationKind::Insert
                | DbOperationKind::Update
                | DbOperationKind::Delete
        );
        if !supports_many && self.check_ident("many") {
            return Err(CompileError::syntax(
                "many is not supported for this db operation",
                self.peek().span.start,
            ));
        }
        let many = supports_many && self.match_ident("many");
        let target = self.parse_db_operation_target()?;

        let mut selector = None;
        let mut query = None;
        let mut projection = None;
        let mut body = None;
        let mut insert_body = None;
        let mut change = None;
        let mut children = Vec::new();

        match op {
            DbOperationKind::Find | DbOperationKind::Optional | DbOperationKind::Require => {
                if self.match_symbol("(") {
                    if many {
                        return Err(CompileError::syntax(
                            "db many operations do not support key selectors",
                            self.previous().span.start,
                        ));
                    }
                    let key = self.parse_expression()?;
                    self.expect_symbol(")")?;
                    children.push(key.spans.clone());
                    selector = Some(DbSelector::Key {
                        value: Box::new(key.expr),
                    });
                }
                if self.check_symbol("{") {
                    let (read_query, read_projection, read_children) =
                        self.parse_db_read_block()?;
                    if matches!(selector, Some(DbSelector::Key { .. })) && !read_query.is_empty() {
                        return Err(CompileError::syntax(
                            "db key reads only support fields in the following block",
                            op_token.span.start,
                        ));
                    }
                    if !read_query.is_empty() {
                        query = Some(read_query);
                    }
                    projection = read_projection;
                    children.extend(read_children);
                }
            }
            DbOperationKind::Count | DbOperationKind::Exists => {
                if self.match_symbol("(") {
                    let key = self.parse_expression()?;
                    self.expect_symbol(")")?;
                    children.push(key.spans.clone());
                    selector = Some(DbSelector::Key {
                        value: Box::new(key.expr),
                    });
                } else if self.check_symbol("{") {
                    let (parsed_query, query_children) = self.parse_db_query_block()?;
                    selector = Some(DbSelector::Query {
                        query: parsed_query.clone(),
                    });
                    query = Some(parsed_query);
                    children.extend(query_children);
                }
            }
            DbOperationKind::Insert if many => {
                self.expect_ident_value("values")?;
                let values = self.parse_expression()?;
                children.push(values.spans.clone());
                body = Some(DbBody::Values {
                    value: Box::new(values.expr),
                });
            }
            DbOperationKind::Insert => {
                let (fields, field_children) = self.parse_db_object_body()?;
                children.extend(field_children);
                body = Some(DbBody::ObjectFields { fields });
            }
            DbOperationKind::Update => {
                let (parsed_selector, parsed_query, selector_children) =
                    self.parse_db_selector_or_query()?;
                if many && matches!(parsed_selector, Some(DbSelector::Key { .. })) {
                    return Err(CompileError::syntax(
                        "db many operations do not support key selectors",
                        op_token.span.start,
                    ));
                }
                selector = parsed_selector;
                query = parsed_query;
                children.extend(selector_children);
                let (parsed_change, change_children) = self.parse_db_change_block()?;
                change = Some(parsed_change);
                children.extend(change_children);
            }
            DbOperationKind::Upsert => {
                let (parsed_selector, parsed_query, selector_children) =
                    self.parse_db_selector_or_query()?;
                if matches!(parsed_selector, Some(DbSelector::Query { .. })) {
                    return Err(CompileError::syntax(
                        "db upsert only supports key selectors in the first Object DB version",
                        op_token.span.start,
                    ));
                }
                selector = parsed_selector;
                query = parsed_query;
                children.extend(selector_children);
                let (fields, field_children) = self.parse_db_object_body()?;
                insert_body = Some(DbBody::ObjectFields { fields });
                children.extend(field_children);
                let (parsed_change, change_children) = self.parse_db_change_block()?;
                change = Some(parsed_change);
                children.extend(change_children);
            }
            DbOperationKind::Replace => {
                let (parsed_selector, parsed_query, selector_children) =
                    self.parse_db_selector_or_query()?;
                selector = parsed_selector;
                query = parsed_query;
                children.extend(selector_children);
                let (fields, field_children) = self.parse_db_object_body()?;
                body = Some(DbBody::ObjectFields { fields });
                children.extend(field_children);
            }
            DbOperationKind::Delete => {
                let (parsed_selector, parsed_query, selector_children) =
                    self.parse_db_selector_or_query()?;
                if many && matches!(parsed_selector, Some(DbSelector::Key { .. })) {
                    return Err(CompileError::syntax(
                        "db many operations do not support key selectors",
                        op_token.span.start,
                    ));
                }
                selector = parsed_selector;
                query = parsed_query;
                children.extend(selector_children);
            }
        }

        Ok(ParsedExpr {
            expr: Expr::DbOperation(DbOperation {
                op,
                many,
                target,
                selector,
                query,
                projection,
                body,
                insert_body,
                change,
            }),
            spans: expr_source_spans(
                SourceSpan {
                    start,
                    end: self.previous().span.end,
                },
                children,
            ),
        })
    }

    fn parse_db_operation_target(&mut self) -> Result<TypeRef> {
        self.parse_type()
    }

    fn parse_db_selector_or_query(
        &mut self,
    ) -> Result<(
        Option<DbSelector>,
        Option<DbQueryBlock>,
        Vec<ExprSourceSpans>,
    )> {
        if self.match_symbol("(") {
            let key = self.parse_expression()?;
            self.expect_symbol(")")?;
            return Ok((
                Some(DbSelector::Key {
                    value: Box::new(key.expr),
                }),
                None,
                vec![key.spans],
            ));
        }
        if self.check_symbol("{") {
            let (query, children) = self.parse_db_query_block()?;
            return Ok((
                Some(DbSelector::Query {
                    query: query.clone(),
                }),
                Some(query),
                children,
            ));
        }
        Ok((None, None, Vec::new()))
    }

    fn parse_db_read_block(
        &mut self,
    ) -> Result<(DbQueryBlock, Option<DbProjection>, Vec<ExprSourceSpans>)> {
        self.expect_symbol("{")?;
        let mut query = DbQueryBlock::default();
        let mut projection = None;
        let mut children = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_symbol(";") || self.match_symbol(",") {
                continue;
            }
            if self.match_ident("fields") {
                if !self.check_symbol("{") {
                    return Err(CompileError::syntax(
                        "db read projection now uses `fields { ... }`; replace old `fields name visits` syntax with `fields { name, visits }`",
                        self.peek().span.start,
                    ));
                }
                projection = Some(DbProjection {
                    fields: self.parse_db_projection_fields()?,
                });
            } else if self.check_ident("load") {
                return Err(CompileError::syntax(
                    "db load blocks are not supported in the first Object DB version",
                    self.peek().span.start,
                ));
            } else {
                self.parse_db_query_entry(&mut query, &mut children)?;
            }
        }
        self.expect_symbol("}")?;
        Ok((query, projection, children))
    }

    fn parse_db_query_block(&mut self) -> Result<(DbQueryBlock, Vec<ExprSourceSpans>)> {
        self.expect_symbol("{")?;
        let mut query = DbQueryBlock::default();
        let mut children = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_symbol(";") || self.match_symbol(",") {
                continue;
            }
            if self.check_ident("fields") {
                return Err(CompileError::syntax(
                    "fields is only supported on db read operations",
                    self.peek().span.start,
                ));
            }
            if self.check_ident("load") {
                return Err(CompileError::syntax(
                    "db load blocks are not supported in the first Object DB version",
                    self.peek().span.start,
                ));
            }
            self.parse_db_query_entry(&mut query, &mut children)?;
        }
        self.expect_symbol("}")?;
        Ok((query, children))
    }

    fn parse_db_query_entry(
        &mut self,
        query: &mut DbQueryBlock,
        children: &mut Vec<ExprSourceSpans>,
    ) -> Result<()> {
        if self.match_ident("where") {
            if self.match_ident("if") {
                let condition = self.parse_expression()?;
                self.expect_symbol("{")?;
                let predicate = self.parse_expression()?;
                self.expect_symbol("}")?;
                children.push(condition.spans.clone());
                children.push(predicate.spans.clone());
                query.where_clauses.push(DbWhereClause::Conditional {
                    condition: condition.expr,
                    predicate: predicate.expr,
                });
            } else {
                let predicate = self.parse_expression()?;
                children.push(predicate.spans.clone());
                query.where_clauses.push(DbWhereClause::Predicate {
                    predicate: predicate.expr,
                });
            }
        } else if self.match_ident("order") {
            let field = self.parse_db_field_path("expected db order field")?;
            let direction = if self.match_ident("desc") {
                DbIndexDirection::Desc
            } else {
                self.match_ident("asc");
                DbIndexDirection::Asc
            };
            query.order.push(DbOrderEntry { field, direction });
        } else if self.match_ident("limit") {
            let limit = self.parse_expression()?;
            children.push(limit.spans.clone());
            query.limit = Some(Box::new(limit.expr));
        } else if self.match_ident("offset") {
            let offset = self.parse_expression()?;
            children.push(offset.spans.clone());
            query.offset = Some(Box::new(offset.expr));
        } else if self.check_ident("after") {
            return Err(CompileError::syntax(
                "db query after is not supported; use offset",
                self.peek().span.start,
            ));
        } else {
            return Err(CompileError::syntax(
                "expected db query entry",
                self.peek().span.start,
            ));
        }
        let _ = self.match_symbol(";") || self.match_symbol(",");
        Ok(())
    }

    fn parse_db_projection_fields(&mut self) -> Result<Vec<FieldPath>> {
        self.expect_symbol("{")?;
        let mut fields = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_symbol(",") || self.match_symbol(";") {
                continue;
            }
            fields.push(self.parse_db_field_path("expected db fields entry")?);
            let _ = self.match_symbol(",") || self.match_symbol(";");
        }
        self.expect_symbol("}")?;
        Ok(fields)
    }

    fn parse_db_object_body(&mut self) -> Result<(Vec<DbObjectFieldValue>, Vec<ExprSourceSpans>)> {
        self.expect_symbol("{")?;
        let mut fields = Vec::new();
        let mut children = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_symbol(";") || self.match_symbol(",") {
                continue;
            }
            let field = self.expect_ident("expected db object body field")?;
            self.expect_symbol("=")?;
            let value = self.parse_expression()?;
            children.push(value.spans.clone());
            fields.push(DbObjectFieldValue {
                field,
                value: value.expr,
            });
            let _ = self.match_symbol(";") || self.match_symbol(",");
        }
        self.expect_symbol("}")?;
        Ok((fields, children))
    }

    fn parse_db_change_block(&mut self) -> Result<(DbChange, Vec<ExprSourceSpans>)> {
        self.expect_symbol("{")?;
        let mut ops = Vec::new();
        let mut children = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_symbol(";") || self.match_symbol(",") {
                continue;
            }
            if self.match_ident("unset") {
                let path = self.parse_db_field_path("expected db unset field")?;
                ops.push(DbChangeOp::Unset { path });
            } else if self.match_ident("add") || self.match_ident("addToSet") {
                let path = self.parse_db_field_path("expected db add field")?;
                let value = self.parse_expression()?;
                children.push(value.spans.clone());
                ops.push(DbChangeOp::AddToSet {
                    path,
                    value: value.expr,
                });
            } else if self.match_ident("remove") {
                let path = self.parse_db_field_path("expected db remove field")?;
                let value = self.parse_expression()?;
                children.push(value.spans.clone());
                ops.push(DbChangeOp::Remove {
                    path,
                    value: value.expr,
                });
            } else {
                let path = self.parse_db_field_path("expected db change field")?;
                if path.segments.len() > 1 {
                    return Err(CompileError::syntax(
                        "db change block assignments must use a top-level field",
                        self.previous().span.start,
                    ));
                }
                if self.match_symbol("=") {
                    let value = self.parse_expression()?;
                    children.push(value.spans.clone());
                    ops.push(DbChangeOp::Set {
                        path,
                        value: value.expr,
                    });
                } else if self.match_symbol("+") {
                    self.expect_symbol("=")?;
                    let value = self.parse_expression()?;
                    children.push(value.spans.clone());
                    ops.push(DbChangeOp::Inc {
                        path,
                        value: value.expr,
                    });
                } else if self.match_symbol("-") {
                    self.expect_symbol("=")?;
                    let value = self.parse_expression()?;
                    children.push(value.spans.clone());
                    ops.push(DbChangeOp::Inc {
                        path,
                        value: Expr::Binary {
                            op: BinaryOp::Sub,
                            left: Box::new(Expr::Literal(Literal::Number(0.0))),
                            right: Box::new(value.expr),
                        },
                    });
                } else {
                    return Err(CompileError::syntax(
                        "expected db change operator",
                        self.peek().span.start,
                    ));
                }
            }
            let _ = self.match_symbol(";") || self.match_symbol(",");
        }
        self.expect_symbol("}")?;
        Ok((DbChange { ops }, children))
    }

    fn parse_db_field_path(&mut self, message: &str) -> Result<FieldPath> {
        let segments = self.parse_field_path(message)?;
        Ok(FieldPath {
            text: segments.join("."),
            segments,
        })
    }

    fn parse_catch_expression(&mut self, start: SourceLocation) -> Result<ParsedExpr> {
        self.expect_symbol("<")?;
        let catch_type = self.parse_type()?;
        self.expect_symbol(">")?;
        self.expect_symbol("(")?;
        let try_expr = self.parse_expression()?;
        self.expect_symbol(")")?;
        Ok(ParsedExpr {
            expr: Expr::Catch {
                catch_type,
                try_expr: Box::new(try_expr.expr),
            },
            spans: expr_source_spans(
                SourceSpan {
                    start,
                    end: self.previous().span.end,
                },
                vec![try_expr.spans],
            ),
        })
    }

    fn parse_record_construct_fields(
        &mut self,
    ) -> Result<(
        Vec<(String, Expr)>,
        Vec<ExprSourceSpans>,
        Vec<RecordFieldSourceSpans>,
    )> {
        let mut fields = Vec::new();
        let mut spans = Vec::new();
        let mut record_fields = Vec::new();
        if !self.check_symbol("}") {
            loop {
                let field = self.expect_ident("expected record field name")?;
                let field_name_span = self.previous().span;
                self.expect_symbol(":")?;
                let field_value = self.parse_expression()?;
                record_fields.push(RecordFieldSourceSpans {
                    name: field.clone(),
                    name_span: field_name_span,
                    value_span: field_value.spans.span,
                });
                spans.push(field_value.spans);
                fields.push((field, field_value.expr));
                if !self.match_symbol(",") {
                    break;
                }
                if self.check_symbol("}") {
                    break;
                }
            }
        }
        self.expect_symbol("}")?;
        Ok((fields, spans, record_fields))
    }

    fn parse_patch_operations(
        &mut self,
    ) -> Result<(Vec<crate::ast::PatchOperation>, Vec<ExprSourceSpans>)> {
        let mut operations = Vec::new();
        let mut spans = Vec::new();
        while !self.check_symbol("}") {
            let op = self.expect_ident("expected patch operation")?;
            let path = self.parse_patch_field_path()?;
            match op.as_str() {
                "set" => {
                    self.expect_symbol("=")?;
                    let value = self.parse_expression()?;
                    spans.push(value.spans.clone());
                    operations.push(crate::ast::PatchOperation::Set {
                        path,
                        value: value.expr,
                    });
                }
                "inc" => {
                    self.expect_ident_value("by")?;
                    let value = self.parse_expression()?;
                    spans.push(value.spans.clone());
                    operations.push(crate::ast::PatchOperation::Inc {
                        path,
                        value: value.expr,
                    });
                }
                _ => {
                    return Err(CompileError::syntax(
                        "expected patch operation set or inc",
                        self.previous().span.start,
                    ));
                }
            }
            let _ = self.match_symbol(";") || self.match_symbol(",");
        }
        self.expect_symbol("}")?;
        Ok((operations, spans))
    }

    fn parse_patch_field_path(&mut self) -> Result<Vec<String>> {
        let mut path = vec![self.expect_ident("expected patch field path")?];
        while self.match_symbol(".") {
            path.push(self.expect_ident("expected patch field path segment")?);
        }
        Ok(path)
    }

    fn parse_object_literal_entries(
        &mut self,
    ) -> Result<(
        Vec<crate::ast::ObjectLiteralEntry>,
        Vec<ExprSourceSpans>,
        Vec<RecordFieldSourceSpans>,
    )> {
        let mut entries = Vec::new();
        let mut spans = Vec::new();
        let mut record_fields = Vec::new();
        if !self.check_symbol("}") {
            loop {
                let (key, key_span) = self.parse_object_literal_key()?;
                self.expect_symbol(":")?;
                let value = self.parse_expression()?;
                let field_name = object_literal_key_name(&key);
                if let Some(field_name) = field_name {
                    record_fields.push(RecordFieldSourceSpans {
                        name: field_name,
                        name_span: key_span,
                        value_span: value.spans.span,
                    });
                }
                spans.push(value.spans);
                entries.push(crate::ast::ObjectLiteralEntry {
                    key,
                    key_span: Some(key_span),
                    value: value.expr,
                });
                if !self.match_symbol(",") {
                    break;
                }
                if self.check_symbol("}") {
                    break;
                }
            }
        }
        self.expect_symbol("}")?;
        Ok((entries, spans, record_fields))
    }

    fn parse_object_literal_key(&mut self) -> Result<(crate::ast::ObjectLiteralKey, SourceSpan)> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::Ident(value) => Ok((crate::ast::ObjectLiteralKey::Name(value), token.span)),
            TokenKind::Symbol(value) if value == "[" => Err(CompileError::syntax(
                "computed object literal keys are not supported; construct an empty object and call set",
                token.span.start,
            )),
            _ => Err(CompileError::syntax(
                "expected object literal key",
                token.span.start,
            )),
        }
    }

    fn peek_binary_op(&self) -> Option<(BinaryOp, u8)> {
        let TokenKind::Symbol(value) = &self.peek().kind else {
            return None;
        };
        Some(match value.as_str() {
            "||" => (BinaryOp::Or, 1),
            "&&" => (BinaryOp::And, 2),
            "==" => (BinaryOp::Eq, 3),
            "!=" => (BinaryOp::Ne, 3),
            "<" => (BinaryOp::Lt, 4),
            "<=" => (BinaryOp::Le, 4),
            ">" => (BinaryOp::Gt, 4),
            ">=" => (BinaryOp::Ge, 4),
            "+" => (BinaryOp::Add, 5),
            "-" => (BinaryOp::Sub, 5),
            "*" => (BinaryOp::Mul, 6),
            "/" => (BinaryOp::Div, 6),
            _ => return None,
        })
    }

    fn expect_ident(&mut self, message: &str) -> Result<String> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::Ident(value) => Ok(value),
            _ => Err(CompileError::syntax(message, token.span.start)),
        }
    }

    fn expect_ident_value(&mut self, expected: &str) -> Result<Token> {
        let token = self.advance().clone();
        match &token.kind {
            TokenKind::Ident(value) if value == expected => Ok(token),
            _ => Err(CompileError::syntax(
                format!("expected {expected}"),
                token.span.start,
            )),
        }
    }

    fn expect_string(&mut self, message: &str) -> Result<String> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::String(value) => Ok(value),
            _ => Err(CompileError::syntax(message, token.span.start)),
        }
    }

    fn expect_positive_integer(&mut self, message: &str) -> Result<u64> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::Number(value)
                if value.is_finite()
                    && value.fract() == 0.0
                    && value > 0.0
                    && value <= u64::MAX as f64 =>
            {
                Ok(value as u64)
            }
            _ => Err(CompileError::syntax(message, token.span.start)),
        }
    }

    fn expect_symbol(&mut self, expected: &str) -> Result<()> {
        let token = self.advance().clone();
        match &token.kind {
            TokenKind::Symbol(value) if value == expected => Ok(()),
            _ => Err(CompileError::syntax(
                format!("expected symbol {expected}"),
                token.span.start,
            )),
        }
    }

    fn match_ident(&mut self, expected: &str) -> bool {
        if self.check_ident(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn check_ident(&self, expected: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Ident(value) if value == expected)
    }

    fn check_function_start(&self) -> bool {
        self.check_ident("function")
            || self.check_ident("native")
            || self.check_ident("provider")
            || self.check_ident("static")
    }

    fn check_provider_capability_start(&self) -> bool {
        if !self.check_ident("provider") {
            return false;
        }
        let Some(token) = self.tokens.get(self.current + 1) else {
            return false;
        };
        matches!(
            &token.kind,
            TokenKind::Ident(value)
                if value != "function"
                    && value != "native"
                    && value != "provider"
                    && value != "static"
        )
    }

    fn match_symbol(&mut self, expected: &str) -> bool {
        if self.check_symbol(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn check_symbol(&self, expected: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Symbol(value) if value == expected)
    }

    fn is_at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.current - 1]
    }

    fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        self.previous()
    }
}

fn contiguous_locations(left: SourceLocation, right: SourceLocation) -> bool {
    left.line == right.line && left.column == right.column
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
