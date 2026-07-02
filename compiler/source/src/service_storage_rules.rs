use std::collections::{BTreeMap, BTreeSet};

use crate::{
    parsed_sources::ParsedCompilerSource,
    semantic::validate_db_attachments,
    shared::ast::{Block, DbDecl, Expr, Stmt, TypeRef},
    shared::ast_utils::db_collection_name,
    shared::publication_error::PublicationError,
};

use super::SourceSymbolKey;

pub fn validate_service_storage_sources(
    parsed_sources: &[ParsedCompilerSource],
) -> Result<(), PublicationError> {
    let type_index = ServiceTypeIndex::build(parsed_sources);
    let mut violations = Vec::new();
    let mut collection_names = BTreeMap::<String, SourceSymbolKey>::new();
    let mut db_record_names = BTreeSet::<SourceSymbolKey>::new();

    for parsed in parsed_sources {
        violations.extend(validate_db_attachments(
            &parsed.source().module_path,
            parsed.ast(),
        ));
        for db in &parsed.ast().dbs {
            if let Some(record_name) = validate_db_decl(
                parsed,
                db,
                &type_index,
                &mut collection_names,
                &mut violations,
            ) {
                db_record_names.insert(record_name);
            }
        }
    }

    for parsed in parsed_sources {
        validate_source_transaction_effects(parsed, &mut violations);
    }

    if violations.is_empty() {
        return Ok(());
    }
    Err(PublicationError::ContractValidation {
        message: violations
            .into_iter()
            .map(|violation| format!("- {violation}"))
            .collect::<Vec<_>>()
            .join("\n"),
    })
}

fn validate_db_decl(
    parsed: &ParsedCompilerSource,
    db: &DbDecl,
    type_index: &ServiceTypeIndex<'_>,
    collection_names: &mut BTreeMap<String, SourceSymbolKey>,
    violations: &mut Vec<String>,
) -> Option<SourceSymbolKey> {
    let Some(record) = type_index.resolve_local_record(parsed, &db.name) else {
        return None;
    };

    let Some(key) = &db.key else {
        return Some(record.source_key.clone());
    };
    if key.name == "_id" {
        violations.push(format!(
            "db object {} key cannot use reserved _id field",
            db.name
        ));
    }

    let mut field_names = BTreeSet::new();
    for field in &record.fields {
        if field.name == "_id" {
            violations.push(format!(
                "db object {} field cannot use reserved _id field",
                db.name
            ));
        }
        if !field_names.insert(field.name.clone()) {
            violations.push(format!(
                "db object {} field {} is declared more than once",
                db.name, field.name
            ));
        }
    }
    if let Some(collection_name) = &db.collection_name {
        if collection_name.is_empty() {
            violations.push(format!("db object {} name cannot be empty", db.name));
        } else if collection_name.starts_with("_skiff_") {
            violations.push(format!(
                "db object {} name {} uses reserved _skiff_ prefix",
                db.name, collection_name
            ));
        } else if collection_name.ends_with('s') {
            violations.push(format!(
                "db object {} name {} must be singular",
                db.name, collection_name
            ));
        }
    }
    let collection_name = db_collection_name(db);
    if !collection_name.is_empty() {
        if let Some(existing_type) =
            collection_names.insert(collection_name.clone(), record.source_key.clone())
        {
            violations.push(format!(
                "db object name {} is used by both {} and {}",
                collection_name, existing_type, record.source_key
            ));
        }
    }

    let mut index_names = BTreeSet::new();
    for index in &db.indexes {
        if !index_names.insert(index.name.clone()) {
            violations.push(format!(
                "db object {} index name {} is declared more than once",
                db.name, index.name
            ));
        }
        if index.fields.is_empty() {
            violations.push(format!(
                "db object {} index {} must declare at least one field",
                db.name, index.name
            ));
        }
        for field in &index.fields {
            validate_record_field_path(
                "db object index",
                &db.name,
                &field.field_path,
                record,
                type_index,
                violations,
            );
        }
        if let Some(where_expr) = &index.where_expr {
            validate_db_index_where_expr(db, index, where_expr, record, type_index, violations);
        }
    }
    Some(record.source_key.clone())
}

fn validate_db_index_where_expr(
    db: &DbDecl,
    index: &crate::shared::ast::DbIndexEntry,
    expr: &Expr,
    record: &ServiceTypeRecord<'_>,
    type_index: &ServiceTypeIndex<'_>,
    violations: &mut Vec<String>,
) {
    collect_db_index_where_identifiers(expr, &mut |path| {
        if path.is_empty() {
            return;
        }
        validate_record_field_path(
            &format!("db object {} index {} where", db.name, index.name),
            &db.name,
            &path,
            record,
            type_index,
            violations,
        );
    });
}

fn collect_db_index_where_identifiers(expr: &Expr, visit: &mut impl FnMut(Vec<String>)) {
    if let Some(path) = expr_field_path(expr) {
        visit(path);
        return;
    }
    match expr {
        Expr::Binary { left, right, .. } => {
            collect_db_index_where_identifiers(left, visit);
            collect_db_index_where_identifiers(right, visit);
        }
        Expr::Unary { expr, .. } => collect_db_index_where_identifiers(expr, visit),
        Expr::Call { callee, args } => {
            collect_db_index_where_identifiers(callee, visit);
            for arg in args {
                collect_db_index_where_identifiers(arg, visit);
            }
        }
        Expr::Generic { callee, .. } => collect_db_index_where_identifiers(callee, visit),
        Expr::InterfaceBox { value, .. } => collect_db_index_where_identifiers(value, visit),
        Expr::Record { fields, .. } => {
            for (_, value) in fields {
                collect_db_index_where_identifiers(value, visit);
            }
        }
        Expr::ObjectLiteral { entries } => {
            for entry in entries {
                collect_db_index_where_identifiers(&entry.value, visit);
            }
        }
        Expr::Patch { operations, .. } => {
            for operation in operations {
                match operation {
                    crate::shared::ast::PatchOperation::Set { value, .. }
                    | crate::shared::ast::PatchOperation::Inc { value, .. } => {
                        collect_db_index_where_identifiers(value, visit);
                    }
                }
            }
        }
        Expr::Throw { value } => collect_db_index_where_identifiers(value, visit),
        Expr::Rethrow { exception } => collect_db_index_where_identifiers(exception, visit),
        Expr::Catch { try_expr, .. } => collect_db_index_where_identifiers(try_expr, visit),
        Expr::DbOperation(_)
        | Expr::DbQuery(_)
        | Expr::DbTransaction(_)
        | Expr::DbLeaseClaim(_)
        | Expr::DbLeaseRead(_)
        | Expr::Literal(_)
        | Expr::Identifier(_)
        | Expr::RemotePublicInstanceSource(_) => {}
        Expr::Field { .. } => {}
    }
}

fn expr_field_path(expr: &Expr) -> Option<Vec<String>> {
    match expr {
        Expr::Identifier(name) => Some(vec![name.clone()]),
        Expr::Field { object, field } => {
            let mut path = expr_field_path(object)?;
            path.push(field.clone());
            Some(path)
        }
        _ => None,
    }
}

fn validate_source_transaction_effects(
    parsed: &ParsedCompilerSource,
    violations: &mut Vec<String>,
) {
    for function in &parsed.ast().functions {
        collect_transaction_effect_violations(
            &format!("function {}", function.name),
            &function.body,
            false,
            violations,
        );
    }
    for implementation in &parsed.ast().impls {
        for method in &implementation.method_bodies {
            collect_transaction_effect_violations(
                &format!("impl {} function {}", implementation.target, method.name),
                &method.body,
                false,
                violations,
            );
        }
    }
    for constant in &parsed.ast().consts {
        collect_transaction_expr_violations(
            &format!("const {}", constant.name),
            &constant.value,
            false,
            violations,
        );
    }
}

fn collect_transaction_effect_violations(
    context: &str,
    block: &Block,
    in_transaction: bool,
    violations: &mut Vec<String>,
) {
    for stmt in &block.statements {
        collect_transaction_stmt_violations(context, stmt, in_transaction, violations);
    }
}

fn collect_transaction_stmt_violations(
    context: &str,
    stmt: &Stmt,
    in_transaction: bool,
    violations: &mut Vec<String>,
) {
    if in_transaction {
        match stmt {
            Stmt::Assign { target, .. } if is_self_assignment_target(target) => {
                violations.push(format!(
                    "{context}: assignment to self.* is not allowed inside db transaction"
                ));
            }
            _ => {}
        }
    }

    match stmt {
        Stmt::Assert { condition, .. } => {
            collect_transaction_expr_violations(context, condition, in_transaction, violations);
        }
        Stmt::Let { value, .. } => {
            collect_transaction_expr_violations(context, value, in_transaction, violations);
        }
        Stmt::Assign { target, value } => {
            collect_transaction_expr_violations(context, target, in_transaction, violations);
            collect_transaction_expr_violations(context, value, in_transaction, violations);
        }
        Stmt::If {
            condition,
            then_block,
            else_block,
        } => {
            collect_transaction_expr_violations(context, condition, in_transaction, violations);
            collect_transaction_effect_violations(context, then_block, in_transaction, violations);
            if let Some(else_block) = else_block {
                collect_transaction_effect_violations(
                    context,
                    else_block,
                    in_transaction,
                    violations,
                );
            }
        }
        Stmt::For { iterable, body, .. } => {
            collect_transaction_expr_violations(context, iterable, in_transaction, violations);
            collect_transaction_effect_violations(context, body, in_transaction, violations);
        }
        Stmt::Match { value, arms } => {
            collect_transaction_expr_violations(context, value, in_transaction, violations);
            for arm in arms {
                collect_transaction_effect_violations(
                    context,
                    &arm.body,
                    in_transaction,
                    violations,
                );
            }
        }
        Stmt::DbTransaction { body } => {
            collect_transaction_effect_violations(context, body, true, violations);
        }
        Stmt::Throw { value }
        | Stmt::Spawn { call: value }
        | Stmt::Emit(value)
        | Stmt::Expr(value) => {
            collect_transaction_expr_violations(context, value, in_transaction, violations);
        }
        Stmt::Return(value) => {
            if let Some(value) = value {
                collect_transaction_expr_violations(context, value, in_transaction, violations);
            }
        }
        Stmt::Rethrow { exception } => {
            collect_transaction_expr_violations(context, exception, in_transaction, violations);
        }
        Stmt::Break | Stmt::Continue => {}
    }
}

fn collect_transaction_expr_violations(
    context: &str,
    expr: &Expr,
    in_transaction: bool,
    violations: &mut Vec<String>,
) {
    match expr {
        Expr::Binary { left, right, .. } => {
            collect_transaction_expr_violations(context, left, in_transaction, violations);
            collect_transaction_expr_violations(context, right, in_transaction, violations);
        }
        Expr::Unary { expr, .. } => {
            collect_transaction_expr_violations(context, expr, in_transaction, violations);
        }
        Expr::Call { callee, args } => {
            collect_transaction_expr_violations(context, callee, in_transaction, violations);
            for arg in args {
                collect_transaction_expr_violations(context, arg, in_transaction, violations);
            }
        }
        Expr::Generic { callee, .. } => {
            collect_transaction_expr_violations(context, callee, in_transaction, violations);
        }
        Expr::InterfaceBox { value, .. } => {
            collect_transaction_expr_violations(context, value, in_transaction, violations);
        }
        Expr::Field { object, .. } => {
            collect_transaction_expr_violations(context, object, in_transaction, violations);
        }
        Expr::Record { fields, .. } => {
            for (_, value) in fields {
                collect_transaction_expr_violations(context, value, in_transaction, violations);
            }
        }
        Expr::ObjectLiteral { entries } => {
            for entry in entries {
                collect_transaction_expr_violations(
                    context,
                    &entry.value,
                    in_transaction,
                    violations,
                );
            }
        }
        Expr::Patch { operations, .. } => {
            for operation in operations {
                match operation {
                    crate::shared::ast::PatchOperation::Set { value, .. }
                    | crate::shared::ast::PatchOperation::Inc { value, .. } => {
                        collect_transaction_expr_violations(
                            context,
                            value,
                            in_transaction,
                            violations,
                        );
                    }
                }
            }
        }
        Expr::Throw { value } => {
            collect_transaction_expr_violations(context, value, in_transaction, violations);
        }
        Expr::Rethrow { exception } => {
            collect_transaction_expr_violations(context, exception, in_transaction, violations);
        }
        Expr::Catch { try_expr, .. } => {
            collect_transaction_expr_violations(context, try_expr, in_transaction, violations);
        }
        Expr::DbOperation(operation) => {
            collect_db_operation_expr_violations(context, operation, in_transaction, violations);
        }
        Expr::DbQuery(query) => {
            collect_db_query_expr_violations(context, &query.query, in_transaction, violations);
        }
        Expr::DbTransaction(transaction) => {
            collect_transaction_effect_violations(context, &transaction.body, true, violations);
        }
        Expr::DbLeaseClaim(claim) => {
            if in_transaction {
                violations.push(format!(
                    "{context}: db claim is not allowed inside db transaction"
                ));
            }
            collect_transaction_expr_violations(context, &claim.key, in_transaction, violations);
            collect_transaction_effect_violations(context, &claim.body, false, violations);
        }
        Expr::DbLeaseRead(read) => {
            collect_transaction_expr_violations(context, &read.key, in_transaction, violations);
        }
        Expr::Literal(_) | Expr::Identifier(_) | Expr::RemotePublicInstanceSource(_) => {}
    }
}

fn collect_db_operation_expr_violations(
    context: &str,
    operation: &crate::shared::ast::DbOperation,
    in_transaction: bool,
    violations: &mut Vec<String>,
) {
    if let Some(selector) = &operation.selector {
        match selector {
            crate::shared::ast::DbSelector::Key { value } => {
                collect_transaction_expr_violations(context, value, in_transaction, violations);
            }
            crate::shared::ast::DbSelector::Query { query } => {
                collect_db_query_expr_violations(context, query, in_transaction, violations);
            }
        }
    }
    if let Some(query) = &operation.query {
        collect_db_query_expr_violations(context, query, in_transaction, violations);
    }
    for body in [&operation.body, &operation.insert_body]
        .into_iter()
        .flatten()
    {
        match body {
            crate::shared::ast::DbBody::ObjectFields { fields } => {
                for field in fields {
                    collect_transaction_expr_violations(
                        context,
                        &field.value,
                        in_transaction,
                        violations,
                    );
                }
            }
            crate::shared::ast::DbBody::Values { value } => {
                collect_transaction_expr_violations(context, value, in_transaction, violations);
            }
        }
    }
    if let Some(change) = &operation.change {
        for op in &change.ops {
            match op {
                crate::shared::ast::DbChangeOp::Set { value, .. }
                | crate::shared::ast::DbChangeOp::Inc { value, .. }
                | crate::shared::ast::DbChangeOp::AddToSet { value, .. }
                | crate::shared::ast::DbChangeOp::Remove { value, .. } => {
                    collect_transaction_expr_violations(context, value, in_transaction, violations);
                }
                crate::shared::ast::DbChangeOp::Unset { .. } => {}
            }
        }
    }
}

fn collect_db_query_expr_violations(
    context: &str,
    query: &crate::shared::ast::DbQueryBlock,
    in_transaction: bool,
    violations: &mut Vec<String>,
) {
    for clause in &query.where_clauses {
        match clause {
            crate::shared::ast::DbWhereClause::Predicate { predicate } => {
                collect_transaction_expr_violations(context, predicate, in_transaction, violations);
            }
            crate::shared::ast::DbWhereClause::Conditional {
                condition,
                predicate,
            } => {
                collect_transaction_expr_violations(context, condition, in_transaction, violations);
                collect_transaction_expr_violations(context, predicate, in_transaction, violations);
            }
        }
    }
    for value in [&query.limit, &query.offset, &query.after]
        .into_iter()
        .flatten()
    {
        collect_transaction_expr_violations(context, value, in_transaction, violations);
    }
}

fn is_self_assignment_target(expr: &Expr) -> bool {
    match expr {
        Expr::Field { object, .. } => is_self_assignment_target(object),
        Expr::Identifier(name) => name == "self",
        _ => false,
    }
}

fn resolve_decl_record<'a>(
    parsed: &ParsedCompilerSource,
    type_name: &str,
    type_index: &'a ServiceTypeIndex<'a>,
    declaration_kind: &str,
    violations: &mut Vec<String>,
) -> Option<&'a ServiceTypeRecord<'a>> {
    let Some(record) = type_index.resolve(parsed, type_name) else {
        violations.push(format!(
            "{declaration_kind} {type_name} references unknown type"
        ));
        return None;
    };
    if record.alias {
        violations.push(format!(
            "{declaration_kind} {type_name} must reference a record type"
        ));
        return None;
    }
    Some(record)
}

fn validate_record_field_path(
    context: &str,
    type_name: &str,
    field_path: &[String],
    record: &ServiceTypeRecord<'_>,
    type_index: &ServiceTypeIndex<'_>,
    violations: &mut Vec<String>,
) {
    let Some(first) = field_path.first() else {
        violations.push(format!("{context} for {type_name} cannot be empty"));
        return;
    };
    let mut current = record;
    for (index, segment) in field_path.iter().enumerate() {
        let Some(field) = current.fields.iter().find(|field| field.name == *segment) else {
            violations.push(format!(
                "{context} {} on {} references unknown field {}",
                field_path.join("."),
                type_name,
                segment
            ));
            return;
        };
        if index == field_path.len() - 1 {
            return;
        }
        let Some(next_record) = type_index.resolve_from_module(current.module_path, &field.ty.name)
        else {
            violations.push(format!(
                "{context} {} on {} cannot traverse non-record field {}",
                field_path.join("."),
                type_name,
                segment
            ));
            return;
        };
        current = next_record;
    }
    if !record.fields.iter().any(|field| field.name == *first) {
        violations.push(format!(
            "{context} {} on {} references unknown field {}",
            field_path.join("."),
            type_name,
            first
        ));
    }
}

struct ServiceTypeIndex<'a> {
    by_source_key: BTreeMap<SourceSymbolKey, ServiceTypeRecord<'a>>,
}

#[derive(Clone)]
struct ServiceTypeRecord<'a> {
    source_key: SourceSymbolKey,
    module_path: &'a str,
    name: String,
    alias: bool,
    fields: Vec<ServiceFieldRecord<'a>>,
}

#[derive(Clone)]
struct ServiceFieldRecord<'a> {
    name: String,
    ty: &'a TypeRef,
}

impl<'a> ServiceTypeIndex<'a> {
    fn build(parsed_sources: &'a [ParsedCompilerSource]) -> Self {
        let mut by_source_key = BTreeMap::new();
        for parsed in parsed_sources {
            for decl in &parsed.ast().types {
                let source_key = SourceSymbolKey::new(&parsed.source().module_path, &decl.name);
                by_source_key.insert(
                    source_key.clone(),
                    ServiceTypeRecord {
                        source_key,
                        module_path: &parsed.source().module_path,
                        name: decl.name.clone(),
                        alias: decl.alias.is_some(),
                        fields: decl
                            .fields
                            .iter()
                            .map(|field| ServiceFieldRecord {
                                name: field.name.clone(),
                                ty: &field.ty,
                            })
                            .collect(),
                    },
                );
            }
        }
        Self { by_source_key }
    }

    fn resolve_local_record(
        &self,
        parsed: &ParsedCompilerSource,
        type_name: &str,
    ) -> Option<&ServiceTypeRecord<'a>> {
        let record = self.resolve_from_module(&parsed.source().module_path, type_name)?;
        (!record.alias && record.module_path == parsed.source().module_path).then_some(record)
    }

    fn resolve(
        &self,
        parsed: &ParsedCompilerSource,
        type_name: &str,
    ) -> Option<&ServiceTypeRecord<'a>> {
        self.resolve_from_module(&parsed.source().module_path, type_name)
    }

    fn resolve_from_module(
        &self,
        module_path: &str,
        type_name: &str,
    ) -> Option<&ServiceTypeRecord<'a>> {
        let source_key = source_type_lookup_key(module_path, type_name)?;
        self.by_source_key.get(&source_key)
    }

    fn type_ref_matches_record(
        &self,
        parsed: &ParsedCompilerSource,
        type_name: &str,
        record: &ServiceTypeRecord<'_>,
    ) -> bool {
        self.resolve(parsed, type_name)
            .is_some_and(|resolved| resolved.source_key == record.source_key)
    }
}

fn source_type_lookup_key(module_path: &str, type_name: &str) -> Option<SourceSymbolKey> {
    if let Some((source_module, source_symbol)) = type_name.rsplit_once('.') {
        if source_module.is_empty() || source_symbol.is_empty() {
            return None;
        }
        return Some(SourceSymbolKey::new(source_module, source_symbol));
    }
    Some(SourceSymbolKey::new(module_path, type_name))
}

#[cfg(test)]
mod tests;
