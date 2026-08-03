pub(super) use std::collections::{BTreeMap, BTreeSet};

pub(super) use crate::{
    ast::{
        ActorCreateDecl, ActorDecl, AliasDecl, BinaryOp, Block, BlockSourceSpans, BuiltinPackage,
        ConstDecl, DbBlockMode, DbBody, DbChange, DbChangeOp, DbDecl, DbIndexDirection,
        DbIndexEntry, DbIndexField, DbIndexWhereSourceSpans, DbLeaseClaim, DbLeaseDecl,
        DbLeaseRead, DbObjectFieldValue, DbObjectKey, DbOperation, DbOperationKind, DbOrderEntry,
        DbProjection, DbQuery, DbQueryBlock, DbRetention, DbRetentionUnit, DbSelector,
        DbStorageCodec, DbStorageDecl, DbTransaction, DbWhereClause, DependencySourceAddress,
        DispatchTiming, DurationLiteral, ExecutableSourceSpans, Expr, ExprSourceSpans, FieldDecl,
        FieldPath, ForBinding, FunctionDecl, ImplDecl, ImportDecl, InterfaceDecl,
        InterfaceOperation, Literal, MatchArm, PackageId, Param, Pattern, PatternField,
        RecordFieldSourceSpans, SourceFile, SourceSpanTable, Stmt, StmtSourceSpans, TypeDecl,
        TypeRef, UnaryOp, ValueBlock,
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

mod callable;
mod cursor;
mod db;
mod decl;
mod expr;
mod pattern;
mod span;
mod stmt;
mod test;
mod r#type;
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
    /// True while parsing the expression of a statement header (`if`,
    /// `while`, `for ... in`, `match`). In this slot a trailing `{` always
    /// starts the body or arms; nominal constructs and patch constructs must
    /// be parenthesized. Nested expression slots (call arguments, field
    /// values, parenthesized expressions, value blocks, ...) temporarily
    /// reset this flag.
    in_statement_header: bool,
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
            in_statement_header: false,
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
                    "process has been removed; use actors and dispatch instead",
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
#[cfg(test)]
mod tests;
