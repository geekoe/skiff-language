use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use crate::{
    shared::{
        ast::{
            AliasDecl, ConstDecl, DbDecl, Expr, FunctionDecl, ImplDecl, InterfaceDecl,
            InterfaceOperation, Pattern, SourceFile, TypeDecl, TypeRef,
        },
        ast_utils::{
            walk_expr, walk_expr_mut, walk_pattern, walk_pattern_mut, AstVisitor, AstVisitorMut,
        },
        publication_error::PublicationError,
    },
    source_graph::CompilerSourceFile,
};

#[cfg(test)]
use crate::shared::lexer::{lex, Token, TokenKind};

#[derive(Debug, Clone, Default)]
pub struct ModuleExports {
    pub canonical_module_path: String,
    pub types: BTreeSet<String>,
    pub interfaces: BTreeSet<String>,
    pub functions: BTreeSet<String>,
    pub consts: BTreeSet<String>,
}

impl ModuleExports {
    pub fn contains(&self, symbol: &str) -> bool {
        self.types.contains(symbol)
            || self.interfaces.contains(symbol)
            || self.functions.contains(symbol)
            || self.consts.contains(symbol)
    }
}

#[derive(Debug, Clone, Default)]
pub struct RootRefIndex {
    by_module: BTreeMap<String, ModuleExports>,
}

impl RootRefIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_module(&mut self, module_path: &str, ast: &SourceFile) {
        self.insert_module_all_symbols(module_path, ast);
    }

    pub fn insert_module_all_symbols(&mut self, module_path: &str, ast: &SourceFile) {
        let entry = self.module_entry(module_path, module_path);
        for ty in &ast.types {
            entry.types.insert(ty.name.clone());
        }
        for alias in &ast.aliases {
            entry.types.insert(alias.name.clone());
        }
        for interface in &ast.interfaces {
            entry.interfaces.insert(interface.name.clone());
        }
        for function in &ast.functions {
            entry.functions.insert(function.name.clone());
        }
        for function in &ast.function_signatures {
            entry.functions.insert(function.name.clone());
        }
        for constant in &ast.consts {
            entry.consts.insert(constant.name.clone());
        }
    }

    pub fn insert_module_with_public_path(
        &mut self,
        module_path: &str,
        public_path: Option<&str>,
        ast: &SourceFile,
    ) {
        self.insert_module_all_symbols(module_path, ast);
        if let Some(public_path) = public_path {
            if public_path != module_path {
                self.insert_module_all_symbols_with_canonical_path(public_path, module_path, ast);
            }
        }
    }

    pub fn insert_module_with_root_path(
        &mut self,
        module_path: &str,
        root_module_path: &str,
        ast: &SourceFile,
    ) {
        self.insert_module_all_symbols_with_canonical_path(root_module_path, module_path, ast);
    }

    fn insert_module_all_symbols_with_canonical_path(
        &mut self,
        module_path: &str,
        canonical_module_path: &str,
        ast: &SourceFile,
    ) {
        let entry = self.module_entry(module_path, canonical_module_path);
        for ty in &ast.types {
            entry.types.insert(ty.name.clone());
        }
        for alias in &ast.aliases {
            entry.types.insert(alias.name.clone());
        }
        for interface in &ast.interfaces {
            entry.interfaces.insert(interface.name.clone());
        }
        for function in &ast.functions {
            entry.functions.insert(function.name.clone());
        }
        for function in &ast.function_signatures {
            entry.functions.insert(function.name.clone());
        }
        for constant in &ast.consts {
            entry.consts.insert(constant.name.clone());
        }
    }

    fn module_entry(
        &mut self,
        module_path: &str,
        canonical_module_path: &str,
    ) -> &mut ModuleExports {
        let entry = self.by_module.entry(module_path.to_string()).or_default();
        if entry.canonical_module_path.is_empty() {
            entry.canonical_module_path = canonical_module_path.to_string();
        }
        entry
    }

    pub fn module_exports(&self, module_path: &str) -> Option<&ModuleExports> {
        self.by_module.get(module_path)
    }

    pub fn has_module(&self, module_path: &str) -> bool {
        self.by_module.contains_key(module_path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootRefError {
    pub path: String,
    pub reason: RootRefErrorReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootRefErrorReason {
    MissingTrailingSegment,
    UnknownModule { module_path: String },
    UnknownSymbol { module_path: String, symbol: String },
    RemovedPackageSyntax,
}

impl std::fmt::Display for RootRefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.reason {
            RootRefErrorReason::MissingTrailingSegment => {
                write!(
                    f,
                    "root reference `{}` must include at least one module segment and a symbol",
                    self.path
                )
            }
            RootRefErrorReason::UnknownModule { module_path } => {
                write!(
                    f,
                    "root reference `{}` resolves to module `{}.skiff` which does not exist",
                    self.path,
                    module_path.replace('.', "/")
                )
            }
            RootRefErrorReason::UnknownSymbol {
                module_path,
                symbol,
            } => {
                write!(
                    f,
                    "root reference `{}` requires module `{}.skiff` to export `{}`",
                    self.path,
                    module_path.replace('.', "/"),
                    symbol
                )
            }
            RootRefErrorReason::RemovedPackageSyntax => {
                let suggestion = self
                    .path
                    .strip_prefix("package")
                    .map(|suffix| format!("root{suffix}"))
                    .unwrap_or_else(|| "root.<module>.<Symbol>".to_string());
                write!(
                    f,
                    "root reference `{}` uses removed `package.*` spelling; use `{}`",
                    self.path, suggestion
                )
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRootRef {
    pub module_path: String,
    pub symbol: String,
}

impl ResolvedRootRef {
    pub fn canonical_path(&self) -> String {
        format!("{}.{}", self.module_path, self.symbol)
    }
}

pub fn resolve_root_segments(
    segments: &[&str],
    index: &RootRefIndex,
) -> Result<ResolvedRootRef, RootRefError> {
    let display_path = format!("root.{}", segments.join("."));
    if segments.len() < 2 {
        return Err(RootRefError {
            path: display_path,
            reason: RootRefErrorReason::MissingTrailingSegment,
        });
    }
    let (symbol_segment, module_segments) = segments.split_last().expect("non-empty by check");
    let module_path = module_segments.join(".");
    let Some(exports) = index.module_exports(&module_path) else {
        return Err(RootRefError {
            path: display_path,
            reason: RootRefErrorReason::UnknownModule { module_path },
        });
    };
    if !exports.contains(symbol_segment) {
        return Err(RootRefError {
            path: display_path,
            reason: RootRefErrorReason::UnknownSymbol {
                module_path,
                symbol: (*symbol_segment).to_string(),
            },
        });
    }
    Ok(ResolvedRootRef {
        module_path: exports.canonical_module_path.clone(),
        symbol: (*symbol_segment).to_string(),
    })
}

#[derive(Debug, Clone, Default)]
pub struct RootRefResolution {
    pub synthetic_imports: BTreeSet<(String, String)>,
    pub errors: Vec<RootRefError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootRefValidationPolicy {
    pub include_tests: bool,
    pub include_std_private_projection_roots: bool,
}

impl RootRefValidationPolicy {
    pub const fn service_sources() -> Self {
        Self {
            include_tests: false,
            include_std_private_projection_roots: false,
        }
    }

    pub const fn parsed_publication_sources() -> Self {
        Self {
            include_tests: true,
            include_std_private_projection_roots: true,
        }
    }
}

const OFFICIAL_STD_PRIVATE_MODULE_PREFIX: &str = "std.__private";

pub fn validate_source_root_refs(
    root: &Path,
    sources: &[CompilerSourceFile],
    policy: RootRefValidationPolicy,
) -> Result<(), PublicationError> {
    let index = source_root_ref_index(sources, policy);
    for source in sources
        .iter()
        .filter(|source| policy.include_tests || !source.is_test_file)
    {
        let path = source.diagnostic_path_from_root(root);
        let outcome = collect_root_refs_in_ast(&source.ast, &index);
        if !outcome.errors.is_empty() {
            let message = outcome
                .errors
                .iter()
                .map(|error| format!("- {error}"))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(PublicationError::RootPathReference { path, message });
        }
    }
    Ok(())
}

fn source_root_ref_index(
    sources: &[CompilerSourceFile],
    policy: RootRefValidationPolicy,
) -> RootRefIndex {
    let mut index = RootRefIndex::new();
    for source in sources
        .iter()
        .filter(|source| policy.include_tests || !source.is_test_file)
    {
        index.insert_module_all_symbols(&source.module_path, &source.ast);
        if policy.include_std_private_projection_roots
            && !is_official_std_private_module_path(&source.module_path)
        {
            if let Some(stripped) = source.module_path.strip_prefix("std.") {
                index.insert_module_with_root_path(&source.module_path, stripped, &source.ast);
            }
        }
    }
    index
}

pub fn is_official_std_private_module_path(module_path: &str) -> bool {
    module_path
        .strip_prefix(OFFICIAL_STD_PRIVATE_MODULE_PREFIX)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

pub fn resolve_root_refs_in_ast(ast: &mut SourceFile, index: &RootRefIndex) -> RootRefResolution {
    let mut visitor = RootRefResolver {
        index,
        outcome: RootRefResolution::default(),
    };
    visitor.visit_source_file(ast);
    visitor.outcome
}

pub fn collect_root_refs_in_ast(ast: &SourceFile, index: &RootRefIndex) -> RootRefResolution {
    let mut visitor = RootRefCollector {
        index,
        outcome: RootRefResolution::default(),
    };
    visitor.visit_source_file(ast);
    visitor.outcome
}

struct RootRefResolver<'a> {
    index: &'a RootRefIndex,
    outcome: RootRefResolution,
}

impl RootRefResolver<'_> {
    fn visit_source_file(&mut self, ast: &mut SourceFile) {
        for ty in &mut ast.types {
            self.visit_type_decl(&mut AliasOrTypeDecl::Type(ty));
        }
        for alias in &mut ast.aliases {
            self.visit_alias_decl(alias);
        }
        for interface in &mut ast.interfaces {
            self.visit_interface_decl(interface);
        }
        for implementation in &mut ast.impls {
            self.visit_impl_decl(implementation);
        }
        for db in &mut ast.dbs {
            self.visit_db_decl(db);
        }
        for constant in &mut ast.consts {
            self.visit_const_decl(constant);
        }
        for function in &mut ast.functions {
            self.visit_function_decl(function);
        }
        for signature in &mut ast.function_signatures {
            self.visit_interface_operation(signature);
        }
        for test in &mut ast.tests {
            self.visit_block(&mut test.body);
        }
    }

    fn visit_type_decl(&mut self, decl: &mut AliasOrTypeDecl<'_>) {
        match decl {
            AliasOrTypeDecl::Type(decl) => {
                if let Some(alias) = decl.alias.as_mut() {
                    self.visit_type_ref(alias);
                }
                for implements in &mut decl.implements {
                    self.visit_type_ref(implements);
                }
                for field in &mut decl.fields {
                    self.visit_type_ref(&mut field.ty);
                }
            }
            AliasOrTypeDecl::Alias(decl) => self.visit_type_ref(&mut decl.target_type),
        }
    }

    fn visit_alias_decl(&mut self, decl: &mut AliasDecl) {
        self.visit_type_decl(&mut AliasOrTypeDecl::Alias(decl));
    }

    fn visit_interface_decl(&mut self, decl: &mut InterfaceDecl) {
        for op in &mut decl.operations {
            self.visit_interface_operation(op);
        }
    }

    fn visit_impl_decl(&mut self, decl: &mut ImplDecl) {
        decl.target = rewrite_type_name(&decl.target, self.index, &mut self.outcome);
        for op in &mut decl.methods {
            self.visit_interface_operation(op);
        }
        for body in &mut decl.method_bodies {
            self.visit_function_decl(body);
        }
    }

    fn visit_db_decl(&mut self, decl: &mut DbDecl) {
        for index_entry in &mut decl.indexes {
            if let Some(where_expr) = index_entry.where_expr.as_mut() {
                self.visit_expr(where_expr);
            }
        }
    }

    fn visit_const_decl(&mut self, decl: &mut ConstDecl) {
        if let Some(ty) = decl.ty.as_mut() {
            self.visit_type_ref(ty);
        }
        self.visit_expr(&mut decl.value);
    }

    fn visit_function_decl(&mut self, decl: &mut FunctionDecl) {
        for param in &mut decl.params {
            self.visit_type_ref(&mut param.ty);
        }
        self.visit_type_ref(&mut decl.return_type);
        if let Some(implicit_self) = decl.implicit_self.as_mut() {
            self.visit_type_ref(implicit_self);
        }
        self.visit_block(&mut decl.body);
    }

    fn visit_interface_operation(&mut self, op: &mut InterfaceOperation) {
        for param in &mut op.params {
            self.visit_type_ref(&mut param.ty);
        }
        self.visit_type_ref(&mut op.return_type);
        if let Some(implicit_self) = op.implicit_self.as_mut() {
            self.visit_type_ref(implicit_self);
        }
    }
}

impl AstVisitorMut for RootRefResolver<'_> {
    fn visit_pattern(&mut self, pattern: &mut Pattern) {
        if let Pattern::Nominal { name, .. } = pattern {
            *name = rewrite_type_name(name, self.index, &mut self.outcome);
        }
        walk_pattern_mut(self, pattern);
    }

    fn visit_expr(&mut self, expr: &mut Expr) {
        if try_resolve_root_expr(expr, self.index, &mut self.outcome) {
            return;
        }
        if let Expr::Record { type_name, .. } = expr {
            *type_name = rewrite_type_name(type_name, self.index, &mut self.outcome);
        }
        walk_expr_mut(self, expr);
    }

    fn visit_type_ref(&mut self, ty: &mut TypeRef) {
        ty.name = rewrite_type_name(&ty.name, self.index, &mut self.outcome);
    }
}

struct RootRefCollector<'a> {
    index: &'a RootRefIndex,
    outcome: RootRefResolution,
}

impl RootRefCollector<'_> {
    fn visit_source_file(&mut self, ast: &SourceFile) {
        for ty in &ast.types {
            self.visit_type_decl(ty);
        }
        for alias in &ast.aliases {
            self.visit_alias_decl(alias);
        }
        for interface in &ast.interfaces {
            self.visit_interface_decl(interface);
        }
        for implementation in &ast.impls {
            self.visit_impl_decl(implementation);
        }
        for db in &ast.dbs {
            self.visit_db_decl(db);
        }
        for constant in &ast.consts {
            self.visit_const_decl(constant);
        }
        for function in &ast.functions {
            self.visit_function_decl(function);
        }
        for signature in &ast.function_signatures {
            self.visit_interface_operation(signature);
        }
        for test in &ast.tests {
            self.visit_block(&test.body);
        }
    }

    fn visit_type_decl(&mut self, decl: &TypeDecl) {
        if let Some(alias) = &decl.alias {
            self.visit_type_ref(alias);
        }
        for implements in &decl.implements {
            self.visit_type_ref(implements);
        }
        for field in &decl.fields {
            self.visit_type_ref(&field.ty);
        }
    }

    fn visit_alias_decl(&mut self, decl: &AliasDecl) {
        self.visit_type_ref(&decl.target_type);
    }

    fn visit_interface_decl(&mut self, decl: &InterfaceDecl) {
        for op in &decl.operations {
            self.visit_interface_operation(op);
        }
    }

    fn visit_impl_decl(&mut self, decl: &ImplDecl) {
        collect_type_name(&decl.target, self.index, &mut self.outcome);
        for op in &decl.methods {
            self.visit_interface_operation(op);
        }
        for body in &decl.method_bodies {
            self.visit_function_decl(body);
        }
    }

    fn visit_db_decl(&mut self, decl: &DbDecl) {
        for index_entry in &decl.indexes {
            if let Some(where_expr) = &index_entry.where_expr {
                self.visit_expr(where_expr);
            }
        }
    }

    fn visit_const_decl(&mut self, decl: &ConstDecl) {
        if let Some(ty) = &decl.ty {
            self.visit_type_ref(ty);
        }
        self.visit_expr(&decl.value);
    }

    fn visit_function_decl(&mut self, decl: &FunctionDecl) {
        for param in &decl.params {
            self.visit_type_ref(&param.ty);
        }
        self.visit_type_ref(&decl.return_type);
        if let Some(implicit_self) = &decl.implicit_self {
            self.visit_type_ref(implicit_self);
        }
        self.visit_block(&decl.body);
    }

    fn visit_interface_operation(&mut self, op: &InterfaceOperation) {
        for param in &op.params {
            self.visit_type_ref(&param.ty);
        }
        self.visit_type_ref(&op.return_type);
        if let Some(implicit_self) = &op.implicit_self {
            self.visit_type_ref(implicit_self);
        }
    }
}

impl AstVisitor for RootRefCollector<'_> {
    fn visit_pattern(&mut self, pattern: &Pattern) {
        if let Pattern::Nominal { name, .. } = pattern {
            collect_type_name(name, self.index, &mut self.outcome);
        }
        walk_pattern(self, pattern);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        if try_collect_root_expr(expr, self.index, &mut self.outcome) {
            return;
        }
        if let Expr::Record { type_name, .. } = expr {
            collect_type_name(type_name, self.index, &mut self.outcome);
        }
        walk_expr(self, expr);
    }

    fn visit_type_ref(&mut self, ty: &TypeRef) {
        collect_type_name(&ty.name, self.index, &mut self.outcome);
    }
}

enum AliasOrTypeDecl<'a> {
    Type(&'a mut TypeDecl),
    Alias(&'a mut AliasDecl),
}

fn try_resolve_root_expr(
    expr: &mut Expr,
    index: &RootRefIndex,
    outcome: &mut RootRefResolution,
) -> bool {
    let Some(chain) = collect_root_chain(expr) else {
        return false;
    };
    if collect_root_chain_resolution(&chain.head, &chain.segments, index, outcome) {
        *expr = Expr::Identifier(chain.segments.last().cloned().unwrap_or_default());
        return true;
    }
    let segment_refs: Vec<&str> = chain.segments.iter().map(String::as_str).collect();
    if let Ok(resolved) = resolve_root_segments(&segment_refs, index) {
        *expr = expr_from_path(&resolved.canonical_path());
    }
    true
}

fn try_collect_root_expr(
    expr: &Expr,
    index: &RootRefIndex,
    outcome: &mut RootRefResolution,
) -> bool {
    let Some(chain) = collect_root_chain(expr) else {
        return false;
    };
    collect_root_chain_resolution(&chain.head, &chain.segments, index, outcome);
    true
}

fn collect_root_chain_resolution(
    head: &str,
    segments: &[String],
    index: &RootRefIndex,
    outcome: &mut RootRefResolution,
) -> bool {
    if head == "package" {
        outcome.errors.push(RootRefError {
            path: root_chain_path(head, segments),
            reason: RootRefErrorReason::RemovedPackageSyntax,
        });
        return true;
    }
    let segment_refs: Vec<&str> = segments.iter().map(String::as_str).collect();
    match resolve_root_segments(&segment_refs, index) {
        Ok(resolved) => {
            outcome
                .synthetic_imports
                .insert((resolved.module_path.clone(), resolved.symbol.clone()));
        }
        Err(error) => {
            outcome.errors.push(error);
        }
    }
    false
}

fn expr_from_path(path: &str) -> Expr {
    let mut parts = path.split('.');
    let first = parts.next().unwrap_or_default().to_string();
    parts.fold(Expr::Identifier(first), |object, field| Expr::Field {
        object: Box::new(object),
        field: field.to_string(),
    })
}

struct RootChain {
    head: String,
    segments: Vec<String>,
}

fn root_chain_path(head: &str, segments: &[String]) -> String {
    std::iter::once(head)
        .chain(segments.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(".")
}

fn collect_root_chain(expr: &Expr) -> Option<RootChain> {
    let mut segments = Vec::new();
    let mut cursor = expr;
    loop {
        match cursor {
            Expr::Field { object, field } => {
                segments.push(field.clone());
                cursor = object;
            }
            Expr::Identifier(name) if name == "root" || name == "package" => {
                segments.reverse();
                return Some(RootChain {
                    head: name.clone(),
                    segments,
                });
            }
            _ => return None,
        }
    }
}

fn rewrite_type_name(input: &str, index: &RootRefIndex, outcome: &mut RootRefResolution) -> String {
    let mut buffer = String::with_capacity(input.len());
    resolve_type_name_into(input, index, outcome, &mut buffer);
    buffer
}

fn collect_type_name(input: &str, index: &RootRefIndex, outcome: &mut RootRefResolution) {
    let mut ignored_rewrite = String::new();
    resolve_type_name_into(input, index, outcome, &mut ignored_rewrite);
}

fn resolve_type_name_into(
    input: &str,
    index: &RootRefIndex,
    outcome: &mut RootRefResolution,
    out: &mut String,
) {
    let bytes = input.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if is_type_name_start(byte) {
            let start = cursor;
            while cursor < bytes.len() && is_type_name_continuation(bytes[cursor]) {
                cursor += 1;
            }
            let token = &input[start..cursor];
            if (token == "root" || token == "package")
                && (start == 0 || bytes[start - 1] != b'.')
                && cursor < bytes.len()
                && bytes[cursor] == b'.'
            {
                let mut segments: Vec<String> = Vec::new();
                while cursor < bytes.len() && bytes[cursor] == b'.' {
                    cursor += 1;
                    let seg_start = cursor;
                    while cursor < bytes.len() && is_type_name_continuation(bytes[cursor]) {
                        cursor += 1;
                    }
                    if seg_start == cursor {
                        break;
                    }
                    segments.push(input[seg_start..cursor].to_string());
                }
                if token == "package" {
                    outcome.errors.push(RootRefError {
                        path: root_chain_path(token, &segments),
                        reason: RootRefErrorReason::RemovedPackageSyntax,
                    });
                    out.push_str(segments.last().map(String::as_str).unwrap_or(""));
                    continue;
                }
                let segment_refs: Vec<&str> = segments.iter().map(String::as_str).collect();
                match resolve_root_segments(&segment_refs, index) {
                    Ok(resolved) => {
                        outcome
                            .synthetic_imports
                            .insert((resolved.module_path.clone(), resolved.symbol.clone()));
                        out.push_str(&resolved.canonical_path());
                    }
                    Err(error) => {
                        outcome.errors.push(error);
                    }
                }
            } else {
                out.push_str(token);
            }
            continue;
        }
        // Pass through any non-identifier byte (delimiters, whitespace, punctuation).
        out.push(byte as char);
        cursor += 1;
    }
}

fn is_type_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_type_name_continuation(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
pub fn validate_root_refs_in_text(
    source: &str,
    index: &RootRefIndex,
) -> Result<RootRefResolution, crate::shared::error::CompileError> {
    let tokens = lex(source)?;
    let mut outcome = RootRefResolution::default();
    let mut i = 0;
    while i < tokens.len() {
        let Some(span_match) = match_root_chain(&tokens, i) else {
            i += 1;
            continue;
        };
        let (head, segments, range) = span_match;
        if head == "package" {
            let path = std::iter::once(head.as_str())
                .chain(segments.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(".");
            outcome.errors.push(RootRefError {
                path,
                reason: RootRefErrorReason::RemovedPackageSyntax,
            });
            i = range.2;
            continue;
        }
        let segment_refs: Vec<&str> = segments.iter().map(String::as_str).collect();
        match resolve_root_segments(&segment_refs, index) {
            Ok(resolved) => {
                outcome
                    .synthetic_imports
                    .insert((resolved.module_path.clone(), resolved.symbol.clone()));
            }
            Err(error) => outcome.errors.push(error),
        }
        // Skip past the matched chain.
        i = range.2;
    }

    Ok(outcome)
}

#[cfg(test)]
fn match_root_chain(
    tokens: &[Token],
    start: usize,
) -> Option<(String, Vec<String>, (usize, usize, usize))> {
    let head = tokens.get(start)?;
    if start > 0
        && matches!(
            tokens.get(start - 1).map(|token| &token.kind),
            Some(TokenKind::Symbol(symbol)) if symbol == "."
        )
    {
        return None;
    }
    let TokenKind::Ident(name) = &head.kind else {
        return None;
    };
    if name != "root" && name != "package" {
        return None;
    }
    let mut segments: Vec<String> = Vec::new();
    let mut cursor = start + 1;
    let chain_start_byte = head.span.start.offset;
    let mut chain_end_byte = head.span.end.offset;
    loop {
        let Some(dot) = tokens.get(cursor) else {
            break;
        };
        let TokenKind::Symbol(symbol) = &dot.kind else {
            break;
        };
        if symbol != "." {
            break;
        }
        let Some(ident_token) = tokens.get(cursor + 1) else {
            break;
        };
        let TokenKind::Ident(ident_name) = &ident_token.kind else {
            break;
        };
        segments.push(ident_name.clone());
        chain_end_byte = ident_token.span.end.offset;
        cursor += 2;
    }
    if segments.is_empty() {
        return None;
    }
    Some((
        name.clone(),
        segments,
        (chain_start_byte, chain_end_byte, cursor),
    ))
}

#[cfg(test)]
mod tests;
