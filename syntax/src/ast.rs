use serde::{Deserialize, Serialize};

use crate::error::SourceSpan;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceFile {
    #[serde(default)]
    pub provider_capability: Option<Vec<String>>,
    pub functions: Vec<FunctionDecl>,
    #[serde(default)]
    pub function_signatures: Vec<InterfaceOperation>,
    #[serde(default)]
    pub imports: Vec<ImportDecl>,
    #[serde(default)]
    pub types: Vec<TypeDecl>,
    #[serde(default)]
    pub aliases: Vec<AliasDecl>,
    #[serde(default)]
    pub interfaces: Vec<InterfaceDecl>,
    #[serde(default)]
    pub impls: Vec<ImplDecl>,
    #[serde(default)]
    pub dbs: Vec<DbDecl>,
    #[serde(default)]
    pub consts: Vec<ConstDecl>,
    #[serde(default)]
    pub tests: Vec<TestDeclaration>,
    #[serde(default)]
    pub test_default_run: Option<bool>,
    #[serde(skip)]
    pub test_default_run_span: Option<SourceSpan>,
    #[serde(skip)]
    pub source_spans: SourceSpanTable,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceSpanTable {
    pub functions: Vec<ExecutableSourceSpans>,
    pub impl_methods: Vec<ExecutableSourceSpans>,
    pub tests: Vec<ExecutableSourceSpans>,
    pub consts: Vec<ExprSourceSpans>,
    pub db_index_wheres: Vec<DbIndexWhereSourceSpans>,
}

impl SourceSpanTable {
    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
            && self.impl_methods.is_empty()
            && self.tests.is_empty()
            && self.consts.is_empty()
            && self.db_index_wheres.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutableSourceSpans {
    pub body: BlockSourceSpans,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockSourceSpans {
    pub span: SourceSpan,
    pub statements: Vec<StmtSourceSpans>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StmtSourceSpans {
    pub span: SourceSpan,
    pub expressions: Vec<ExprSourceSpans>,
    pub blocks: Vec<BlockSourceSpans>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExprSourceSpans {
    pub span: SourceSpan,
    pub children: Vec<ExprSourceSpans>,
    pub blocks: Vec<BlockSourceSpans>,
    pub record_fields: Vec<RecordFieldSourceSpans>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordFieldSourceSpans {
    pub name: String,
    pub name_span: SourceSpan,
    pub value_span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DbIndexWhereSourceSpans {
    pub db_name: String,
    pub index_name: String,
    pub expression: ExprSourceSpans,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportDecl {
    pub path: Vec<String>,
    pub alias: Option<String>,
    #[serde(default)]
    pub package: Option<PackageId>,
    #[serde(default)]
    pub local_binding: Option<String>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PackageId {
    Builtin { name: BuiltinPackage },
    Simple { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuiltinPackage {
    Std,
    Ext,
}

impl BuiltinPackage {
    pub fn as_str(&self) -> &'static str {
        match self {
            BuiltinPackage::Std => "std",
            BuiltinPackage::Ext => "ext",
        }
    }
}

impl PackageId {
    pub fn display_id(&self) -> String {
        match self {
            PackageId::Builtin { name } => name.as_str().to_string(),
            PackageId::Simple { name } => name.clone(),
        }
    }

    pub fn default_local_binding(&self) -> Option<String> {
        match self {
            PackageId::Builtin { name } => Some(name.as_str().to_string()),
            PackageId::Simple { name } => Some(name.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeDecl {
    pub exported: bool,
    #[serde(default)]
    pub is_native: bool,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_params: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discriminator: Option<String>,
    pub alias: Option<TypeRef>,
    pub implements: Vec<TypeRef>,
    pub fields: Vec<FieldDecl>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasDecl {
    pub exported: bool,
    pub name: String,
    pub target_type: TypeRef,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDecl {
    pub name: String,
    pub ty: TypeRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbDecl {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<DbObjectKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<DbRetention>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leases: Vec<DbLeaseDecl>,
    pub indexes: Vec<DbIndexEntry>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbLeaseDecl {
    pub name: String,
    pub ttl_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbRetention {
    pub amount: u64,
    pub unit: DbRetentionUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbRetentionUnit {
    Days,
    Hours,
    Minutes,
    Seconds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbObjectKey {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbIndexEntry {
    pub name: String,
    pub unique: bool,
    pub fields: Vec<DbIndexField>,
    pub where_expr: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbIndexField {
    pub field_path: Vec<String>,
    pub direction: DbIndexDirection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldPath {
    pub text: String,
    pub segments: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbIndexDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceDecl {
    pub exported: bool,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_params: Vec<String>,
    pub operations: Vec<InterfaceOperation>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceOperation {
    pub name: String,
    #[serde(default)]
    pub type_params: Vec<String>,
    pub params: Vec<Param>,
    pub return_type: TypeRef,
    #[serde(default)]
    pub is_native: bool,
    #[serde(default)]
    pub is_provider: bool,
    #[serde(default)]
    pub is_static: bool,
    #[serde(default)]
    pub implicit_self: Option<TypeRef>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImplDecl {
    pub exported: bool,
    pub target: String,
    #[serde(default)]
    pub methods: Vec<InterfaceOperation>,
    #[serde(default)]
    pub method_bodies: Vec<FunctionDecl>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstDecl {
    pub exported: bool,
    pub name: String,
    pub ty: Option<TypeRef>,
    pub value: Expr,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionDecl {
    pub exported: bool,
    pub name: String,
    #[serde(default)]
    pub type_params: Vec<String>,
    pub params: Vec<Param>,
    pub return_type: TypeRef,
    pub body: Block,
    #[serde(default)]
    pub is_native: bool,
    #[serde(default)]
    pub is_provider: bool,
    #[serde(default)]
    pub is_static: bool,
    #[serde(default)]
    pub implicit_self: Option<TypeRef>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Param {
    pub name: String,
    pub ty: TypeRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeRef {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub statements: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ForBinding {
    Item { item: String },
    Entry { key: String, value: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Stmt {
    Assert {
        condition: Expr,
        message: Option<String>,
    },
    Let {
        mutable: bool,
        name: String,
        ty: Option<TypeRef>,
        value: Expr,
    },
    Assign {
        target: Expr,
        value: Expr,
    },
    If {
        condition: Expr,
        then_block: Block,
        else_block: Option<Block>,
    },
    For {
        binding: ForBinding,
        iterable: Expr,
        body: Block,
    },
    Match {
        value: Expr,
        arms: Vec<MatchArm>,
    },
    DbTransaction {
        body: Block,
    },
    Throw {
        value: Expr,
    },
    Rethrow {
        exception: Expr,
    },
    Emit(Expr),
    Return(Option<Expr>),
    Spawn {
        call: Expr,
    },
    Break,
    Continue,
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbTransaction {
    pub mode: DbBlockMode,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbLeaseClaim {
    pub target: TypeRef,
    pub key: Box<Expr>,
    pub slot: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<String>,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbLeaseRead {
    pub target: TypeRef,
    pub key: Box<Expr>,
    pub slot: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbBlockMode {
    Effect,
    Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbOperation {
    pub op: DbOperationKind,
    pub many: bool,
    pub target: TypeRef,
    pub selector: Option<DbSelector>,
    pub query: Option<DbQueryBlock>,
    pub projection: Option<DbProjection>,
    pub body: Option<DbBody>,
    pub insert_body: Option<DbBody>,
    pub change: Option<DbChange>,
}

impl DbOperation {
    pub fn has_query_selector(&self) -> bool {
        matches!(self.selector.as_ref(), Some(DbSelector::Query { .. }))
    }

    pub fn independent_query(&self) -> Option<&DbQueryBlock> {
        if self.has_query_selector() {
            None
        } else {
            self.query.as_ref()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbQuery {
    pub target: TypeRef,
    pub query: DbQueryBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DbOperationKind {
    Find,
    Optional,
    Require,
    Insert,
    Update,
    Upsert,
    Replace,
    Delete,
    Count,
    Exists,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DbSelector {
    Key { value: Box<Expr> },
    Query { query: DbQueryBlock },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DbQueryBlock {
    #[serde(default)]
    pub where_clauses: Vec<DbWhereClause>,
    #[serde(default)]
    pub order: Vec<DbOrderEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<Box<Expr>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<Box<Expr>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Box<Expr>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DbWhereClause {
    Predicate { predicate: Expr },
    Conditional { condition: Expr, predicate: Expr },
}

impl DbQueryBlock {
    pub fn is_empty(&self) -> bool {
        self.where_clauses.is_empty()
            && self.order.is_empty()
            && self.limit.is_none()
            && self.offset.is_none()
            && self.after.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbOrderEntry {
    pub field: FieldPath,
    pub direction: DbIndexDirection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbProjection {
    pub fields: Vec<FieldPath>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DbBody {
    ObjectFields { fields: Vec<DbObjectFieldValue> },
    Values { value: Box<Expr> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbObjectFieldValue {
    pub field: String,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbChange {
    pub ops: Vec<DbChangeOp>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DbChangeOp {
    Set { path: FieldPath, value: Expr },
    Inc { path: FieldPath, value: Expr },
    Unset { path: FieldPath },
    AddToSet { path: FieldPath, value: Expr },
    Remove { path: FieldPath, value: Expr },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestDeclaration {
    pub name: String,
    pub body: Block,
    pub span: SourceSpan,
}

pub fn source_text_without_test_declarations(source: &str, ast: &SourceFile) -> String {
    let mut ranges = Vec::new();
    if let Some(span) = ast.test_default_run_span {
        ranges.push(expand_to_declaration_range(
            source,
            span.start.offset,
            span.end.offset,
        ));
    }
    for test in &ast.tests {
        ranges.push(expand_to_declaration_range(
            source,
            test.span.start.offset,
            test.span.end.offset,
        ));
    }
    if ranges.is_empty() {
        return source.to_string();
    }

    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        if start >= end {
            continue;
        }
        if let Some((_, last_end)) = merged.last_mut() {
            if start <= *last_end {
                *last_end = (*last_end).max(end);
                continue;
            }
        }
        merged.push((start, end));
    }

    let mut sanitized = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for (start, end) in merged {
        let start = start.min(source.len());
        let end = end.min(source.len());
        if cursor < start {
            sanitized.push_str(&source[cursor..start]);
        }
        if needs_inline_separator_after_removed_range(source, start, end) {
            sanitized.push(' ');
        }
        cursor = cursor.max(end);
    }
    if cursor < source.len() {
        sanitized.push_str(&source[cursor..]);
    }
    sanitized
}

fn needs_inline_separator_after_removed_range(source: &str, start: usize, end: usize) -> bool {
    let bytes = source.as_bytes();
    if start == 0 || end >= bytes.len() {
        return false;
    }
    !bytes[start - 1].is_ascii_whitespace() && !bytes[end].is_ascii_whitespace()
}

fn expand_to_declaration_range(source: &str, start: usize, end: usize) -> (usize, usize) {
    let bytes = source.as_bytes();
    let mut line_start = start.min(bytes.len());
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }

    let mut line_end = end.min(bytes.len());
    while line_end < bytes.len() && bytes[line_end] != b'\n' {
        line_end += 1;
    }
    if line_end < bytes.len() {
        line_end += 1;
    }

    let start = start.min(source.len());
    let end = end.min(source.len());
    let before_is_blank = source[line_start..start].trim().is_empty();
    let after_is_blank = source[end..line_end].trim().is_empty();
    let declaration_is_only_line_content = before_is_blank && after_is_blank;
    let expanded_start = if declaration_is_only_line_content {
        line_start
    } else {
        trim_inline_whitespace_before(source, start)
    };
    let expanded_end = if declaration_is_only_line_content {
        line_end
    } else {
        trim_inline_whitespace_after(source, end)
    };

    (expanded_start, expanded_end)
}

fn trim_inline_whitespace_before(source: &str, offset: usize) -> usize {
    let bytes = source.as_bytes();
    let mut cursor = offset.min(bytes.len());
    while cursor > 0 && matches!(bytes[cursor - 1], b' ' | b'\t' | b'\r') {
        cursor -= 1;
    }
    cursor
}

fn trim_inline_whitespace_after(source: &str, offset: usize) -> usize {
    let bytes = source.as_bytes();
    let mut cursor = offset.min(bytes.len());
    while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t' | b'\r') {
        cursor += 1;
    }
    cursor
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Pattern {
    Wildcard,
    Binding(String),
    Literal(Literal),
    Nominal {
        name: String,
        type_args: Vec<TypeRef>,
        fields: Vec<PatternField>,
    },
    Record {
        fields: Vec<PatternField>,
    },
    Or(Vec<Pattern>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternField {
    pub name: String,
    pub pattern: Option<Pattern>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expr {
    Literal(Literal),
    Identifier(String),
    RemotePublicInstanceSource(RemotePublicInstanceSource),
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Generic {
        callee: Box<Expr>,
        type_args: Vec<TypeRef>,
    },
    InterfaceBox {
        value: Box<Expr>,
        interface: TypeRef,
    },
    Field {
        object: Box<Expr>,
        field: String,
    },
    Record {
        type_name: String,
        #[serde(default)]
        type_args: Vec<TypeRef>,
        fields: Vec<(String, Expr)>,
    },
    ObjectLiteral {
        entries: Vec<ObjectLiteralEntry>,
    },
    Patch {
        target: TypeRef,
        operations: Vec<PatchOperation>,
    },
    Throw {
        value: Box<Expr>,
    },
    Rethrow {
        exception: Box<Expr>,
    },
    Catch {
        catch_type: TypeRef,
        try_expr: Box<Expr>,
    },
    DbOperation(DbOperation),
    DbQuery(DbQuery),
    DbTransaction(DbTransaction),
    DbLeaseClaim(DbLeaseClaim),
    DbLeaseRead(DbLeaseRead),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemotePublicInstanceSource {
    pub dependency_ref: String,
    pub public_instance_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectLiteralEntry {
    pub key: ObjectLiteralKey,
    #[serde(skip)]
    pub key_span: Option<SourceSpan>,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ObjectLiteralKey {
    Name(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PatchOperation {
    Set { path: Vec<String>, value: Expr },
    Inc { path: Vec<String>, value: Expr },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Literal {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnaryOp {
    Not,
}
