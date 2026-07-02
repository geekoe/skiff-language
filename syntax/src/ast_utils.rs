use std::collections::BTreeSet;

use crate::ast::{
    Block, DbBody, DbChangeOp, DbDecl, DbOperation, DbSelector, DbWhereClause, Expr, ForBinding,
    FunctionDecl, InterfaceOperation, MatchArm, Pattern, SourceFile, Stmt, TypeRef,
};
use crate::type_syntax::{generic_parts, split_top_level, string_literal};

pub fn expr_path(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Identifier(name) => Some(name.clone()),
        Expr::Field { object, field } => Some(format!("{}.{}", expr_path(object)?, field)),
        Expr::Generic { callee, .. } => expr_path(callee),
        Expr::RemotePublicInstanceSource(source) => Some(format!(
            "{}/{}",
            source.dependency_ref, source.public_instance_key
        )),
        _ => None,
    }
}

pub fn without_generic(expr: &Expr) -> &Expr {
    match expr {
        Expr::Generic { callee, .. } => callee,
        _ => expr,
    }
}

pub fn db_collection_name(db: &DbDecl) -> String {
    db.collection_name
        .clone()
        .unwrap_or_else(|| db.name.clone())
}

pub trait AstVisitor {
    fn visit_block(&mut self, block: &Block) {
        walk_block(self, block);
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        walk_stmt(self, stmt);
    }

    fn visit_match_arm(&mut self, arm: &MatchArm) {
        walk_match_arm(self, arm);
    }

    fn visit_pattern(&mut self, pattern: &Pattern) {
        walk_pattern(self, pattern);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        walk_expr(self, expr);
    }

    fn visit_type_ref(&mut self, _ty: &TypeRef) {}
}

pub fn walk_block(visitor: &mut (impl AstVisitor + ?Sized), block: &Block) {
    for stmt in &block.statements {
        visitor.visit_stmt(stmt);
    }
}

pub fn walk_stmt(visitor: &mut (impl AstVisitor + ?Sized), stmt: &Stmt) {
    match stmt {
        Stmt::Let { ty, value, .. } => {
            if let Some(ty) = ty {
                visitor.visit_type_ref(ty);
            }
            visitor.visit_expr(value);
        }
        Stmt::Assign { target, value } => {
            visitor.visit_expr(target);
            visitor.visit_expr(value);
        }
        Stmt::If {
            condition,
            then_block,
            else_block,
        } => {
            visitor.visit_expr(condition);
            visitor.visit_block(then_block);
            if let Some(else_block) = else_block {
                visitor.visit_block(else_block);
            }
        }
        Stmt::For { iterable, body, .. } => {
            visitor.visit_expr(iterable);
            visitor.visit_block(body);
        }
        Stmt::Match { value, arms } => {
            visitor.visit_expr(value);
            for arm in arms {
                visitor.visit_match_arm(arm);
            }
        }
        Stmt::DbTransaction { body } => visitor.visit_block(body),
        Stmt::Assert { condition, .. } => visitor.visit_expr(condition),
        Stmt::Throw { value } => visitor.visit_expr(value),
        Stmt::Rethrow { exception } => visitor.visit_expr(exception),
        Stmt::Spawn { call } => visitor.visit_expr(call),
        Stmt::Emit(value) | Stmt::Expr(value) => visitor.visit_expr(value),
        Stmt::Return(value) => {
            if let Some(value) = value {
                visitor.visit_expr(value);
            }
        }
        Stmt::Break | Stmt::Continue => {}
    }
}

pub fn walk_match_arm(visitor: &mut (impl AstVisitor + ?Sized), arm: &MatchArm) {
    visitor.visit_pattern(&arm.pattern);
    visitor.visit_block(&arm.body);
}

pub fn walk_pattern(visitor: &mut (impl AstVisitor + ?Sized), pattern: &Pattern) {
    match pattern {
        Pattern::Nominal {
            type_args, fields, ..
        } => {
            for type_arg in type_args {
                visitor.visit_type_ref(type_arg);
            }
            for field in fields {
                if let Some(pattern) = &field.pattern {
                    visitor.visit_pattern(pattern);
                }
            }
        }
        Pattern::Record { fields } => {
            for field in fields {
                if let Some(pattern) = &field.pattern {
                    visitor.visit_pattern(pattern);
                }
            }
        }
        Pattern::Or(patterns) => {
            for pattern in patterns {
                visitor.visit_pattern(pattern);
            }
        }
        Pattern::Wildcard | Pattern::Binding(_) | Pattern::Literal(_) => {}
    }
}

pub fn walk_expr(visitor: &mut (impl AstVisitor + ?Sized), expr: &Expr) {
    match expr {
        Expr::Literal(_) | Expr::Identifier(_) | Expr::RemotePublicInstanceSource(_) => {}
        Expr::Binary { left, right, .. } => {
            visitor.visit_expr(left);
            visitor.visit_expr(right);
        }
        Expr::Unary { expr, .. } => visitor.visit_expr(expr),
        Expr::Call { callee, args } => {
            visitor.visit_expr(callee);
            for arg in args {
                visitor.visit_expr(arg);
            }
        }
        Expr::Generic { callee, type_args } => {
            visitor.visit_expr(callee);
            for type_arg in type_args {
                visitor.visit_type_ref(type_arg);
            }
        }
        Expr::InterfaceBox { value, interface } => {
            visitor.visit_expr(value);
            visitor.visit_type_ref(interface);
        }
        Expr::Field { object, .. } => visitor.visit_expr(object),
        Expr::Record {
            type_args, fields, ..
        } => {
            for type_arg in type_args {
                visitor.visit_type_ref(type_arg);
            }
            for (_, value) in fields {
                visitor.visit_expr(value);
            }
        }
        Expr::ObjectLiteral { entries } => {
            for entry in entries {
                visitor.visit_expr(&entry.value);
            }
        }
        Expr::Patch { target, operations } => {
            visitor.visit_type_ref(target);
            for operation in operations {
                match operation {
                    crate::ast::PatchOperation::Set { value, .. }
                    | crate::ast::PatchOperation::Inc { value, .. } => visitor.visit_expr(value),
                }
            }
        }
        Expr::Throw { value } => visitor.visit_expr(value),
        Expr::Rethrow { exception } => visitor.visit_expr(exception),
        Expr::Catch {
            catch_type,
            try_expr,
        } => {
            visitor.visit_type_ref(catch_type);
            visitor.visit_expr(try_expr);
        }
        Expr::DbOperation(operation) => walk_db_operation(visitor, operation),
        Expr::DbQuery(query) => {
            visitor.visit_type_ref(&query.target);
            walk_db_query(visitor, &query.query);
        }
        Expr::DbTransaction(transaction) => visitor.visit_block(&transaction.body),
        Expr::DbLeaseClaim(claim) => {
            visitor.visit_type_ref(&claim.target);
            visitor.visit_expr(&claim.key);
            visitor.visit_block(&claim.body);
        }
        Expr::DbLeaseRead(read) => {
            visitor.visit_type_ref(&read.target);
            visitor.visit_expr(&read.key);
        }
    }
}

fn walk_db_operation(visitor: &mut (impl AstVisitor + ?Sized), operation: &DbOperation) {
    visitor.visit_type_ref(&operation.target);
    if let Some(selector) = &operation.selector {
        walk_db_selector(visitor, selector);
    }
    if let Some(query) = &operation.query {
        walk_db_query(visitor, query);
    }
    for body in [&operation.body, &operation.insert_body]
        .into_iter()
        .flatten()
    {
        match body {
            DbBody::ObjectFields { fields } => {
                for field in fields {
                    visitor.visit_expr(&field.value);
                }
            }
            DbBody::Values { value } => visitor.visit_expr(value),
        }
    }
    if let Some(change) = &operation.change {
        for op in &change.ops {
            match op {
                DbChangeOp::Set { value, .. }
                | DbChangeOp::Inc { value, .. }
                | DbChangeOp::AddToSet { value, .. }
                | DbChangeOp::Remove { value, .. } => visitor.visit_expr(value),
                DbChangeOp::Unset { .. } => {}
            }
        }
    }
}

fn walk_db_selector(visitor: &mut (impl AstVisitor + ?Sized), selector: &DbSelector) {
    match selector {
        DbSelector::Key { value } => visitor.visit_expr(value),
        DbSelector::Query { query } => walk_db_query(visitor, query),
    }
}

fn walk_db_query(visitor: &mut (impl AstVisitor + ?Sized), query: &crate::ast::DbQueryBlock) {
    for clause in &query.where_clauses {
        walk_db_where_clause(visitor, clause);
    }
    if let Some(limit) = &query.limit {
        visitor.visit_expr(limit);
    }
    if let Some(offset) = &query.offset {
        visitor.visit_expr(offset);
    }
    if let Some(after) = &query.after {
        visitor.visit_expr(after);
    }
}

fn walk_db_where_clause(visitor: &mut (impl AstVisitor + ?Sized), clause: &DbWhereClause) {
    match clause {
        DbWhereClause::Predicate { predicate } => visitor.visit_expr(predicate),
        DbWhereClause::Conditional {
            condition,
            predicate,
        } => {
            visitor.visit_expr(condition);
            visitor.visit_expr(predicate);
        }
    }
}

pub fn collect_reserved_binding_violations(
    path: &str,
    block: &Block,
    violations: &mut Vec<String>,
    is_reserved_name: impl FnMut(&str) -> bool,
) {
    struct ReservedBindingVisitor<'path, 'violations, F> {
        path: &'path str,
        violations: &'violations mut Vec<String>,
        is_reserved_name: F,
    }

    impl<F: FnMut(&str) -> bool> ReservedBindingVisitor<'_, '_, F> {
        fn check_local_binding(&mut self, name: &str) {
            if (self.is_reserved_name)(name) {
                self.violations.push(format!(
                    "{}: local binding {} uses reserved prelude name",
                    self.path, name
                ));
            }
        }

        fn check_pattern_binding(&mut self, name: &str) {
            if (self.is_reserved_name)(name) {
                self.violations.push(format!(
                    "{}: pattern binding {} uses reserved prelude name",
                    self.path, name
                ));
            }
        }
    }

    impl<F: FnMut(&str) -> bool> AstVisitor for ReservedBindingVisitor<'_, '_, F> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            match stmt {
                Stmt::Let { name, .. } => self.check_local_binding(name),
                Stmt::For { binding, .. } => match binding {
                    ForBinding::Item { item } => self.check_local_binding(item),
                    ForBinding::Entry { key, value } => {
                        self.check_local_binding(key);
                        self.check_local_binding(value);
                    }
                },
                _ => {}
            }
            walk_stmt(self, stmt);
        }

        fn visit_pattern(&mut self, pattern: &Pattern) {
            match pattern {
                Pattern::Binding(name) => self.check_pattern_binding(name),
                Pattern::Nominal { fields, .. } | Pattern::Record { fields } => {
                    for field in fields {
                        if let Some(pattern) = &field.pattern {
                            self.visit_pattern(pattern);
                        } else {
                            self.check_pattern_binding(&field.name);
                        }
                    }
                }
                Pattern::Or(patterns) => {
                    for pattern in patterns {
                        self.visit_pattern(pattern);
                    }
                }
                Pattern::Wildcard | Pattern::Literal(_) => {}
            }
        }

        fn visit_expr(&mut self, _expr: &Expr) {}
    }

    let mut visitor = ReservedBindingVisitor {
        path,
        violations,
        is_reserved_name,
    };
    visitor.visit_block(block);
}

pub trait AstVisitorMut {
    fn visit_block(&mut self, block: &mut Block) {
        walk_block_mut(self, block);
    }

    fn visit_stmt(&mut self, stmt: &mut Stmt) {
        walk_stmt_mut(self, stmt);
    }

    fn visit_match_arm(&mut self, arm: &mut MatchArm) {
        walk_match_arm_mut(self, arm);
    }

    fn visit_pattern(&mut self, pattern: &mut Pattern) {
        walk_pattern_mut(self, pattern);
    }

    fn visit_expr(&mut self, expr: &mut Expr) {
        walk_expr_mut(self, expr);
    }

    fn visit_type_ref(&mut self, _ty: &mut TypeRef) {}
}

pub fn walk_block_mut(visitor: &mut (impl AstVisitorMut + ?Sized), block: &mut Block) {
    for stmt in &mut block.statements {
        visitor.visit_stmt(stmt);
    }
}

pub fn walk_stmt_mut(visitor: &mut (impl AstVisitorMut + ?Sized), stmt: &mut Stmt) {
    match stmt {
        Stmt::Let { ty, value, .. } => {
            if let Some(ty) = ty {
                visitor.visit_type_ref(ty);
            }
            visitor.visit_expr(value);
        }
        Stmt::Assign { target, value } => {
            visitor.visit_expr(target);
            visitor.visit_expr(value);
        }
        Stmt::If {
            condition,
            then_block,
            else_block,
        } => {
            visitor.visit_expr(condition);
            visitor.visit_block(then_block);
            if let Some(else_block) = else_block {
                visitor.visit_block(else_block);
            }
        }
        Stmt::For { iterable, body, .. } => {
            visitor.visit_expr(iterable);
            visitor.visit_block(body);
        }
        Stmt::Match { value, arms } => {
            visitor.visit_expr(value);
            for arm in arms {
                visitor.visit_match_arm(arm);
            }
        }
        Stmt::DbTransaction { body } => visitor.visit_block(body),
        Stmt::Assert { condition, .. } => visitor.visit_expr(condition),
        Stmt::Throw { value } => visitor.visit_expr(value),
        Stmt::Rethrow { exception } => visitor.visit_expr(exception),
        Stmt::Spawn { call } => visitor.visit_expr(call),
        Stmt::Emit(value) | Stmt::Expr(value) => visitor.visit_expr(value),
        Stmt::Return(value) => {
            if let Some(value) = value {
                visitor.visit_expr(value);
            }
        }
        Stmt::Break | Stmt::Continue => {}
    }
}

pub fn walk_match_arm_mut(visitor: &mut (impl AstVisitorMut + ?Sized), arm: &mut MatchArm) {
    visitor.visit_pattern(&mut arm.pattern);
    visitor.visit_block(&mut arm.body);
}

pub fn walk_pattern_mut(visitor: &mut (impl AstVisitorMut + ?Sized), pattern: &mut Pattern) {
    match pattern {
        Pattern::Nominal {
            type_args, fields, ..
        } => {
            for type_arg in type_args {
                visitor.visit_type_ref(type_arg);
            }
            for field in fields {
                if let Some(pattern) = &mut field.pattern {
                    visitor.visit_pattern(pattern);
                }
            }
        }
        Pattern::Record { fields } => {
            for field in fields {
                if let Some(pattern) = &mut field.pattern {
                    visitor.visit_pattern(pattern);
                }
            }
        }
        Pattern::Or(patterns) => {
            for pattern in patterns {
                visitor.visit_pattern(pattern);
            }
        }
        Pattern::Wildcard | Pattern::Binding(_) | Pattern::Literal(_) => {}
    }
}

pub fn walk_expr_mut(visitor: &mut (impl AstVisitorMut + ?Sized), expr: &mut Expr) {
    match expr {
        Expr::Literal(_) | Expr::Identifier(_) | Expr::RemotePublicInstanceSource(_) => {}
        Expr::Binary { left, right, .. } => {
            visitor.visit_expr(left);
            visitor.visit_expr(right);
        }
        Expr::Unary { expr, .. } => visitor.visit_expr(expr),
        Expr::Call { callee, args } => {
            visitor.visit_expr(callee);
            for arg in args {
                visitor.visit_expr(arg);
            }
        }
        Expr::Generic { callee, type_args } => {
            visitor.visit_expr(callee);
            for type_arg in type_args {
                visitor.visit_type_ref(type_arg);
            }
        }
        Expr::InterfaceBox { value, interface } => {
            visitor.visit_expr(value);
            visitor.visit_type_ref(interface);
        }
        Expr::Field { object, .. } => visitor.visit_expr(object),
        Expr::Record {
            type_args, fields, ..
        } => {
            for type_arg in type_args {
                visitor.visit_type_ref(type_arg);
            }
            for (_, value) in fields {
                visitor.visit_expr(value);
            }
        }
        Expr::ObjectLiteral { entries } => {
            for entry in entries {
                visitor.visit_expr(&mut entry.value);
            }
        }
        Expr::Patch { target, operations } => {
            visitor.visit_type_ref(target);
            for operation in operations {
                match operation {
                    crate::ast::PatchOperation::Set { value, .. }
                    | crate::ast::PatchOperation::Inc { value, .. } => visitor.visit_expr(value),
                }
            }
        }
        Expr::Throw { value } => visitor.visit_expr(value),
        Expr::Rethrow { exception } => visitor.visit_expr(exception),
        Expr::Catch {
            catch_type,
            try_expr,
        } => {
            visitor.visit_type_ref(catch_type);
            visitor.visit_expr(try_expr);
        }
        Expr::DbOperation(operation) => walk_db_operation_mut(visitor, operation),
        Expr::DbQuery(query) => {
            visitor.visit_type_ref(&mut query.target);
            walk_db_query_mut(visitor, &mut query.query);
        }
        Expr::DbTransaction(transaction) => visitor.visit_block(&mut transaction.body),
        Expr::DbLeaseClaim(claim) => {
            visitor.visit_type_ref(&mut claim.target);
            visitor.visit_expr(&mut claim.key);
            visitor.visit_block(&mut claim.body);
        }
        Expr::DbLeaseRead(read) => {
            visitor.visit_type_ref(&mut read.target);
            visitor.visit_expr(&mut read.key);
        }
    }
}

fn walk_db_operation_mut(visitor: &mut (impl AstVisitorMut + ?Sized), operation: &mut DbOperation) {
    visitor.visit_type_ref(&mut operation.target);
    if let Some(selector) = &mut operation.selector {
        walk_db_selector_mut(visitor, selector);
    }
    if let Some(query) = &mut operation.query {
        walk_db_query_mut(visitor, query);
    }
    for body in [&mut operation.body, &mut operation.insert_body]
        .into_iter()
        .flatten()
    {
        match body {
            DbBody::ObjectFields { fields } => {
                for field in fields {
                    visitor.visit_expr(&mut field.value);
                }
            }
            DbBody::Values { value } => visitor.visit_expr(value),
        }
    }
    if let Some(change) = &mut operation.change {
        for op in &mut change.ops {
            match op {
                DbChangeOp::Set { value, .. }
                | DbChangeOp::Inc { value, .. }
                | DbChangeOp::AddToSet { value, .. }
                | DbChangeOp::Remove { value, .. } => visitor.visit_expr(value),
                DbChangeOp::Unset { .. } => {}
            }
        }
    }
}

fn walk_db_selector_mut(visitor: &mut (impl AstVisitorMut + ?Sized), selector: &mut DbSelector) {
    match selector {
        DbSelector::Key { value } => visitor.visit_expr(value),
        DbSelector::Query { query } => walk_db_query_mut(visitor, query),
    }
}

fn walk_db_query_mut(
    visitor: &mut (impl AstVisitorMut + ?Sized),
    query: &mut crate::ast::DbQueryBlock,
) {
    for clause in &mut query.where_clauses {
        walk_db_where_clause_mut(visitor, clause);
    }
    if let Some(limit) = &mut query.limit {
        visitor.visit_expr(limit);
    }
    if let Some(offset) = &mut query.offset {
        visitor.visit_expr(offset);
    }
    if let Some(after) = &mut query.after {
        visitor.visit_expr(after);
    }
}

fn walk_db_where_clause_mut(
    visitor: &mut (impl AstVisitorMut + ?Sized),
    clause: &mut DbWhereClause,
) {
    match clause {
        DbWhereClause::Predicate { predicate } => visitor.visit_expr(predicate),
        DbWhereClause::Conditional {
            condition,
            predicate,
        } => {
            visitor.visit_expr(condition);
            visitor.visit_expr(predicate);
        }
    }
}

pub fn expr_contains(expr: &Expr, mut predicate: impl FnMut(&Expr) -> bool) -> bool {
    expr_contains_with(expr, &mut predicate)
}

pub fn expr_contains_with(expr: &Expr, predicate: &mut impl FnMut(&Expr) -> bool) -> bool {
    if predicate(expr) {
        return true;
    }
    match expr {
        Expr::Binary { left, right, .. } => {
            expr_contains_with(left, predicate) || expr_contains_with(right, predicate)
        }
        Expr::Unary { expr, .. } => expr_contains_with(expr, predicate),
        Expr::Call { callee, args } => {
            expr_contains_with(callee, predicate)
                || args.iter().any(|arg| expr_contains_with(arg, predicate))
        }
        Expr::Generic { callee, .. } => expr_contains_with(callee, predicate),
        Expr::InterfaceBox { value, .. } => expr_contains_with(value, predicate),
        Expr::Field { object, .. } => expr_contains_with(object, predicate),
        Expr::Record { fields, .. } => fields
            .iter()
            .any(|(_, value)| expr_contains_with(value, predicate)),
        Expr::ObjectLiteral { entries } => entries
            .iter()
            .any(|entry| expr_contains_with(&entry.value, predicate)),
        Expr::Patch { operations, .. } => operations.iter().any(|operation| match operation {
            crate::ast::PatchOperation::Set { value, .. }
            | crate::ast::PatchOperation::Inc { value, .. } => expr_contains_with(value, predicate),
        }),
        Expr::Throw { value } => expr_contains_with(value, predicate),
        Expr::Rethrow { exception } => expr_contains_with(exception, predicate),
        Expr::Catch { try_expr, .. } => expr_contains_with(try_expr, predicate),
        Expr::DbOperation(operation) => db_operation_contains_expr(operation, predicate),
        Expr::DbQuery(query) => db_query_contains_expr(&query.query, predicate),
        Expr::DbTransaction(transaction) => block_contains_expr(&transaction.body, predicate),
        Expr::DbLeaseClaim(claim) => {
            expr_contains_with(&claim.key, predicate) || block_contains_expr(&claim.body, predicate)
        }
        Expr::DbLeaseRead(read) => expr_contains_with(&read.key, predicate),
        Expr::Literal(_) | Expr::Identifier(_) | Expr::RemotePublicInstanceSource(_) => false,
    }
}

fn db_operation_contains_expr(
    operation: &DbOperation,
    predicate: &mut impl FnMut(&Expr) -> bool,
) -> bool {
    operation
        .selector
        .as_ref()
        .is_some_and(|selector| db_selector_contains_expr(selector, predicate))
        || operation
            .query
            .as_ref()
            .is_some_and(|query| db_query_contains_expr(query, predicate))
        || [&operation.body, &operation.insert_body]
            .into_iter()
            .flatten()
            .any(|body| match body {
                DbBody::ObjectFields { fields } => fields
                    .iter()
                    .any(|field| expr_contains_with(&field.value, predicate)),
                DbBody::Values { value } => expr_contains_with(value, predicate),
            })
        || operation.change.as_ref().is_some_and(|change| {
            change.ops.iter().any(|op| match op {
                DbChangeOp::Set { value, .. }
                | DbChangeOp::Inc { value, .. }
                | DbChangeOp::AddToSet { value, .. }
                | DbChangeOp::Remove { value, .. } => expr_contains_with(value, predicate),
                DbChangeOp::Unset { .. } => false,
            })
        })
}

fn db_selector_contains_expr(
    selector: &DbSelector,
    predicate: &mut impl FnMut(&Expr) -> bool,
) -> bool {
    match selector {
        DbSelector::Key { value } => expr_contains_with(value, predicate),
        DbSelector::Query { query } => db_query_contains_expr(query, predicate),
    }
}

fn db_query_contains_expr(
    query: &crate::ast::DbQueryBlock,
    predicate: &mut impl FnMut(&Expr) -> bool,
) -> bool {
    query
        .where_clauses
        .iter()
        .any(|clause| db_where_clause_contains_expr(clause, predicate))
        || query
            .limit
            .as_ref()
            .is_some_and(|expr| expr_contains_with(expr, predicate))
        || query
            .offset
            .as_ref()
            .is_some_and(|expr| expr_contains_with(expr, predicate))
        || query
            .after
            .as_ref()
            .is_some_and(|expr| expr_contains_with(expr, predicate))
}

fn db_where_clause_contains_expr(
    clause: &DbWhereClause,
    predicate: &mut impl FnMut(&Expr) -> bool,
) -> bool {
    match clause {
        DbWhereClause::Predicate { predicate: expr } => expr_contains_with(expr, predicate),
        DbWhereClause::Conditional {
            condition,
            predicate: expr,
        } => expr_contains_with(condition, predicate) || expr_contains_with(expr, predicate),
    }
}

pub fn block_contains_expr(block: &Block, predicate: &mut impl FnMut(&Expr) -> bool) -> bool {
    block
        .statements
        .iter()
        .any(|statement| stmt_contains_expr(statement, predicate))
}

pub fn stmt_contains_expr(stmt: &Stmt, predicate: &mut impl FnMut(&Expr) -> bool) -> bool {
    match stmt {
        Stmt::Let { value, .. } => expr_contains_with(value, predicate),
        Stmt::Assign { target, value } => {
            expr_contains_with(target, predicate) || expr_contains_with(value, predicate)
        }
        Stmt::If {
            condition,
            then_block,
            else_block,
        } => {
            expr_contains_with(condition, predicate)
                || block_contains_expr(then_block, predicate)
                || else_block
                    .as_ref()
                    .is_some_and(|block| block_contains_expr(block, predicate))
        }
        Stmt::For { iterable, body, .. } => {
            expr_contains_with(iterable, predicate) || block_contains_expr(body, predicate)
        }
        Stmt::Match { value, arms } => {
            expr_contains_with(value, predicate)
                || arms
                    .iter()
                    .any(|arm| block_contains_expr(&arm.body, predicate))
        }
        Stmt::DbTransaction { body } => block_contains_expr(body, predicate),
        Stmt::Assert { condition, .. } => expr_contains_with(condition, predicate),
        Stmt::Throw { value } => expr_contains_with(value, predicate),
        Stmt::Rethrow { exception } => expr_contains_with(exception, predicate),
        Stmt::Spawn { call } | Stmt::Emit(call) | Stmt::Expr(call) => {
            expr_contains_with(call, predicate)
        }
        Stmt::Return(value) => value
            .as_ref()
            .is_some_and(|value| expr_contains_with(value, predicate)),
        Stmt::Break | Stmt::Continue => false,
    }
}

#[allow(dead_code)]
pub fn pattern_contains(pattern: &Pattern, predicate: &mut impl FnMut(&Pattern) -> bool) -> bool {
    if predicate(pattern) {
        return true;
    }
    match pattern {
        Pattern::Nominal { fields, .. } => fields.iter().any(|field| {
            field
                .pattern
                .as_ref()
                .is_some_and(|pattern| pattern_contains(pattern, predicate))
        }),
        Pattern::Record { fields } => fields.iter().any(|field| {
            field
                .pattern
                .as_ref()
                .is_some_and(|pattern| pattern_contains(pattern, predicate))
        }),
        Pattern::Or(patterns) => patterns
            .iter()
            .any(|pattern| pattern_contains(pattern, predicate)),
        Pattern::Wildcard | Pattern::Binding(_) | Pattern::Literal(_) => false,
    }
}

pub fn source_expressions_reference_dotted_root(source: &SourceFile, root: &str) -> bool {
    !source_referenced_dotted_root_expression_imports(source, root).is_empty()
}

pub fn source_referenced_dotted_root_imports(
    source: &SourceFile,
    root: &str,
) -> BTreeSet<Vec<String>> {
    let mut imports = source_referenced_dotted_root_expression_imports(source, root);
    collect_source_type_ref_dotted_root_imports(source, root, &mut imports);
    imports
}

pub fn source_referenced_dotted_root_expression_imports(
    source: &SourceFile,
    root: &str,
) -> BTreeSet<Vec<String>> {
    let mut imports = BTreeSet::new();
    collect_source_expression_dotted_root_imports(source, root, &mut imports);
    imports
}

fn collect_source_expression_dotted_root_imports(
    source: &SourceFile,
    root: &str,
    imports: &mut BTreeSet<Vec<String>>,
) {
    for constant in &source.consts {
        collect_expr_dotted_root_imports(&constant.value, root, imports);
    }
    for function in &source.functions {
        collect_block_dotted_root_imports(&function.body, root, imports);
    }
    for implementation in &source.impls {
        for method in &implementation.method_bodies {
            collect_block_dotted_root_imports(&method.body, root, imports);
        }
    }
    for test in &source.tests {
        collect_block_dotted_root_imports(&test.body, root, imports);
    }
}

fn collect_source_type_ref_dotted_root_imports(
    source: &SourceFile,
    root: &str,
    imports: &mut BTreeSet<Vec<String>>,
) {
    for ty in &source.types {
        for implemented in &ty.implements {
            collect_type_ref_dotted_root_imports(implemented, root, imports);
        }
        if let Some(alias) = &ty.alias {
            collect_type_ref_dotted_root_imports(alias, root, imports);
        }
        for field in &ty.fields {
            collect_type_ref_dotted_root_imports(&field.ty, root, imports);
        }
    }
    for alias in &source.aliases {
        collect_type_ref_dotted_root_imports(&alias.target_type, root, imports);
    }
    for constant in &source.consts {
        if let Some(ty) = &constant.ty {
            collect_type_ref_dotted_root_imports(ty, root, imports);
        }
        collect_expr_type_ref_dotted_root_imports(&constant.value, root, imports);
    }
    for interface in &source.interfaces {
        for operation in &interface.operations {
            collect_operation_type_ref_dotted_root_imports(operation, root, imports);
        }
    }
    for operation in &source.function_signatures {
        collect_operation_type_ref_dotted_root_imports(operation, root, imports);
    }
    for function in &source.functions {
        collect_function_type_ref_dotted_root_imports(function, root, imports);
    }
    for implementation in &source.impls {
        collect_type_name_dotted_root_imports(&implementation.target, root, imports);
        for method in &implementation.methods {
            collect_operation_type_ref_dotted_root_imports(method, root, imports);
        }
        for method in &implementation.method_bodies {
            collect_function_type_ref_dotted_root_imports(method, root, imports);
        }
    }
}

fn collect_operation_type_ref_dotted_root_imports(
    operation: &InterfaceOperation,
    root: &str,
    imports: &mut BTreeSet<Vec<String>>,
) {
    if let Some(implicit_self) = &operation.implicit_self {
        collect_type_ref_dotted_root_imports(implicit_self, root, imports);
    }
    for param in &operation.params {
        collect_type_ref_dotted_root_imports(&param.ty, root, imports);
    }
    collect_type_ref_dotted_root_imports(&operation.return_type, root, imports);
}

fn collect_function_type_ref_dotted_root_imports(
    function: &FunctionDecl,
    root: &str,
    imports: &mut BTreeSet<Vec<String>>,
) {
    if let Some(implicit_self) = &function.implicit_self {
        collect_type_ref_dotted_root_imports(implicit_self, root, imports);
    }
    for param in &function.params {
        collect_type_ref_dotted_root_imports(&param.ty, root, imports);
    }
    collect_type_ref_dotted_root_imports(&function.return_type, root, imports);
    collect_block_type_ref_dotted_root_imports(&function.body, root, imports);
}

fn collect_block_type_ref_dotted_root_imports(
    block: &Block,
    root: &str,
    imports: &mut BTreeSet<Vec<String>>,
) {
    for stmt in &block.statements {
        collect_stmt_type_ref_dotted_root_imports(stmt, root, imports);
    }
}

fn collect_stmt_type_ref_dotted_root_imports(
    stmt: &Stmt,
    root: &str,
    imports: &mut BTreeSet<Vec<String>>,
) {
    match stmt {
        Stmt::Let { ty, value, .. } => {
            if let Some(ty) = ty {
                collect_type_ref_dotted_root_imports(ty, root, imports);
            }
            collect_expr_type_ref_dotted_root_imports(value, root, imports);
        }
        Stmt::Assign { target, value } => {
            collect_expr_type_ref_dotted_root_imports(target, root, imports);
            collect_expr_type_ref_dotted_root_imports(value, root, imports);
        }
        Stmt::If {
            condition,
            then_block,
            else_block,
        } => {
            collect_expr_type_ref_dotted_root_imports(condition, root, imports);
            collect_block_type_ref_dotted_root_imports(then_block, root, imports);
            if let Some(else_block) = else_block {
                collect_block_type_ref_dotted_root_imports(else_block, root, imports);
            }
        }
        Stmt::For { iterable, body, .. } => {
            collect_expr_type_ref_dotted_root_imports(iterable, root, imports);
            collect_block_type_ref_dotted_root_imports(body, root, imports);
        }
        Stmt::Match { value, arms } => {
            collect_expr_type_ref_dotted_root_imports(value, root, imports);
            for arm in arms {
                collect_pattern_type_ref_dotted_root_imports(&arm.pattern, root, imports);
                collect_block_type_ref_dotted_root_imports(&arm.body, root, imports);
            }
        }
        Stmt::DbTransaction { body } => {
            collect_block_type_ref_dotted_root_imports(body, root, imports);
        }
        Stmt::Assert { condition, .. } => {
            collect_expr_type_ref_dotted_root_imports(condition, root, imports);
        }
        Stmt::Throw { value }
        | Stmt::Rethrow { exception: value }
        | Stmt::Spawn { call: value }
        | Stmt::Emit(value)
        | Stmt::Expr(value) => collect_expr_type_ref_dotted_root_imports(value, root, imports),
        Stmt::Return(value) => {
            if let Some(value) = value {
                collect_expr_type_ref_dotted_root_imports(value, root, imports);
            }
        }
        Stmt::Break | Stmt::Continue => {}
    }
}

fn collect_expr_type_ref_dotted_root_imports(
    expr: &Expr,
    root: &str,
    imports: &mut BTreeSet<Vec<String>>,
) {
    match expr {
        Expr::Generic { callee, type_args } => {
            collect_expr_type_ref_dotted_root_imports(callee, root, imports);
            for type_arg in type_args {
                collect_type_ref_dotted_root_imports(type_arg, root, imports);
            }
        }
        Expr::InterfaceBox { value, interface } => {
            collect_expr_type_ref_dotted_root_imports(value, root, imports);
            collect_type_ref_dotted_root_imports(interface, root, imports);
        }
        Expr::Record {
            type_name,
            type_args,
            fields,
        } => {
            collect_type_name_dotted_root_imports(type_name, root, imports);
            for type_arg in type_args {
                collect_type_ref_dotted_root_imports(type_arg, root, imports);
            }
            for (_, value) in fields {
                collect_expr_type_ref_dotted_root_imports(value, root, imports);
            }
        }
        Expr::Binary { left, right, .. } => {
            collect_expr_type_ref_dotted_root_imports(left, root, imports);
            collect_expr_type_ref_dotted_root_imports(right, root, imports);
        }
        Expr::Unary { expr, .. } | Expr::Field { object: expr, .. } => {
            collect_expr_type_ref_dotted_root_imports(expr, root, imports);
        }
        Expr::Call { callee, args } => {
            collect_expr_type_ref_dotted_root_imports(callee, root, imports);
            for arg in args {
                collect_expr_type_ref_dotted_root_imports(arg, root, imports);
            }
        }
        Expr::ObjectLiteral { entries } => {
            for entry in entries {
                collect_expr_type_ref_dotted_root_imports(&entry.value, root, imports);
            }
        }
        Expr::Patch { target, operations } => {
            collect_type_ref_dotted_root_imports(target, root, imports);
            for operation in operations {
                match operation {
                    crate::ast::PatchOperation::Set { value, .. }
                    | crate::ast::PatchOperation::Inc { value, .. } => {
                        collect_expr_type_ref_dotted_root_imports(value, root, imports);
                    }
                }
            }
        }
        Expr::Throw { value } | Expr::Rethrow { exception: value } => {
            collect_expr_type_ref_dotted_root_imports(value, root, imports);
        }
        Expr::Catch {
            catch_type,
            try_expr,
        } => {
            collect_type_ref_dotted_root_imports(catch_type, root, imports);
            collect_expr_type_ref_dotted_root_imports(try_expr, root, imports);
        }
        Expr::DbOperation(operation) => {
            collect_type_ref_dotted_root_imports(&operation.target, root, imports);
            if let Some(selector) = &operation.selector {
                collect_db_selector_type_ref_dotted_root_imports(selector, root, imports);
            }
            if let Some(query) = &operation.query {
                collect_db_query_type_ref_dotted_root_imports(query, root, imports);
            }
            for body in [&operation.body, &operation.insert_body]
                .into_iter()
                .flatten()
            {
                match body {
                    DbBody::ObjectFields { fields } => {
                        for field in fields {
                            collect_expr_type_ref_dotted_root_imports(&field.value, root, imports);
                        }
                    }
                    DbBody::Values { value } => {
                        collect_expr_type_ref_dotted_root_imports(value, root, imports);
                    }
                }
            }
            if let Some(change) = &operation.change {
                for op in &change.ops {
                    match op {
                        DbChangeOp::Set { value, .. }
                        | DbChangeOp::Inc { value, .. }
                        | DbChangeOp::AddToSet { value, .. }
                        | DbChangeOp::Remove { value, .. } => {
                            collect_expr_type_ref_dotted_root_imports(value, root, imports);
                        }
                        DbChangeOp::Unset { .. } => {}
                    }
                }
            }
        }
        Expr::DbQuery(query) => {
            collect_type_ref_dotted_root_imports(&query.target, root, imports);
            collect_db_query_type_ref_dotted_root_imports(&query.query, root, imports);
        }
        Expr::DbTransaction(transaction) => {
            collect_block_type_ref_dotted_root_imports(&transaction.body, root, imports);
        }
        Expr::DbLeaseClaim(claim) => {
            collect_type_ref_dotted_root_imports(&claim.target, root, imports);
            collect_expr_type_ref_dotted_root_imports(&claim.key, root, imports);
            collect_block_type_ref_dotted_root_imports(&claim.body, root, imports);
        }
        Expr::DbLeaseRead(read) => {
            collect_type_ref_dotted_root_imports(&read.target, root, imports);
            collect_expr_type_ref_dotted_root_imports(&read.key, root, imports);
        }
        Expr::Literal(_) | Expr::Identifier(_) | Expr::RemotePublicInstanceSource(_) => {}
    }
}

fn collect_db_selector_type_ref_dotted_root_imports(
    selector: &DbSelector,
    root: &str,
    imports: &mut BTreeSet<Vec<String>>,
) {
    match selector {
        DbSelector::Key { value } => {
            collect_expr_type_ref_dotted_root_imports(value, root, imports)
        }
        DbSelector::Query { query } => {
            collect_db_query_type_ref_dotted_root_imports(query, root, imports);
        }
    }
}

fn collect_db_query_type_ref_dotted_root_imports(
    query: &crate::ast::DbQueryBlock,
    root: &str,
    imports: &mut BTreeSet<Vec<String>>,
) {
    for clause in &query.where_clauses {
        match clause {
            DbWhereClause::Predicate { predicate } => {
                collect_expr_type_ref_dotted_root_imports(predicate, root, imports);
            }
            DbWhereClause::Conditional {
                condition,
                predicate,
            } => {
                collect_expr_type_ref_dotted_root_imports(condition, root, imports);
                collect_expr_type_ref_dotted_root_imports(predicate, root, imports);
            }
        }
    }
    if let Some(limit) = &query.limit {
        collect_expr_type_ref_dotted_root_imports(limit, root, imports);
    }
    if let Some(offset) = &query.offset {
        collect_expr_type_ref_dotted_root_imports(offset, root, imports);
    }
    if let Some(after) = &query.after {
        collect_expr_type_ref_dotted_root_imports(after, root, imports);
    }
}

fn collect_block_dotted_root_imports(
    block: &Block,
    root: &str,
    imports: &mut BTreeSet<Vec<String>>,
) {
    for stmt in &block.statements {
        collect_stmt_dotted_root_imports(stmt, root, imports);
    }
}

fn collect_stmt_dotted_root_imports(stmt: &Stmt, root: &str, imports: &mut BTreeSet<Vec<String>>) {
    match stmt {
        Stmt::Let { value, .. } => collect_expr_dotted_root_imports(value, root, imports),
        Stmt::Assign { target, value } => {
            collect_expr_dotted_root_imports(target, root, imports);
            collect_expr_dotted_root_imports(value, root, imports);
        }
        Stmt::If {
            condition,
            then_block,
            else_block,
        } => {
            collect_expr_dotted_root_imports(condition, root, imports);
            collect_block_dotted_root_imports(then_block, root, imports);
            if let Some(else_block) = else_block {
                collect_block_dotted_root_imports(else_block, root, imports);
            }
        }
        Stmt::For { iterable, body, .. } => {
            collect_expr_dotted_root_imports(iterable, root, imports);
            collect_block_dotted_root_imports(body, root, imports);
        }
        Stmt::Match { value, arms } => {
            collect_expr_dotted_root_imports(value, root, imports);
            for arm in arms {
                collect_pattern_type_ref_dotted_root_imports(&arm.pattern, root, imports);
                collect_block_dotted_root_imports(&arm.body, root, imports);
            }
        }
        Stmt::DbTransaction { body } => collect_block_dotted_root_imports(body, root, imports),
        Stmt::Assert { condition, .. } => {
            collect_expr_dotted_root_imports(condition, root, imports)
        }
        Stmt::Throw { value }
        | Stmt::Rethrow { exception: value }
        | Stmt::Spawn { call: value }
        | Stmt::Emit(value)
        | Stmt::Expr(value) => collect_expr_dotted_root_imports(value, root, imports),
        Stmt::Return(value) => {
            if let Some(value) = value {
                collect_expr_dotted_root_imports(value, root, imports);
            }
        }
        Stmt::Break | Stmt::Continue => {}
    }
}

fn collect_expr_dotted_root_imports(expr: &Expr, root: &str, imports: &mut BTreeSet<Vec<String>>) {
    if let Some(path) = expr_path(expr) {
        collect_path_dotted_root_import(&path, root, imports);
    }
    collect_expr_type_ref_dotted_root_imports(expr, root, imports);
    expr_contains(expr, |expr| {
        if let Some(path) = expr_path(expr) {
            collect_path_dotted_root_import(&path, root, imports);
        }
        false
    });
}

fn collect_pattern_type_ref_dotted_root_imports(
    pattern: &Pattern,
    root: &str,
    imports: &mut BTreeSet<Vec<String>>,
) {
    match pattern {
        Pattern::Nominal {
            name,
            type_args,
            fields,
        } => {
            collect_type_name_dotted_root_imports(name, root, imports);
            for type_arg in type_args {
                collect_type_ref_dotted_root_imports(type_arg, root, imports);
            }
            for field in fields {
                if let Some(pattern) = &field.pattern {
                    collect_pattern_type_ref_dotted_root_imports(pattern, root, imports);
                }
            }
        }
        Pattern::Record { fields } => {
            for field in fields {
                if let Some(pattern) = &field.pattern {
                    collect_pattern_type_ref_dotted_root_imports(pattern, root, imports);
                }
            }
        }
        Pattern::Or(patterns) => {
            for pattern in patterns {
                collect_pattern_type_ref_dotted_root_imports(pattern, root, imports);
            }
        }
        Pattern::Wildcard | Pattern::Binding(_) | Pattern::Literal(_) => {}
    }
}

fn collect_type_ref_dotted_root_imports(
    ty: &TypeRef,
    root: &str,
    imports: &mut BTreeSet<Vec<String>>,
) {
    collect_type_name_dotted_root_imports(&ty.name, root, imports);
}

fn collect_type_name_dotted_root_imports(
    ty: &str,
    root: &str,
    imports: &mut BTreeSet<Vec<String>>,
) {
    let ty = ty.trim();
    if ty.is_empty() || ty == "{}" || string_literal(ty).is_some() {
        return;
    }
    if let Some(inner) = ty.strip_suffix('?') {
        collect_type_name_dotted_root_imports(inner, root, imports);
        return;
    }
    let union = split_top_level(ty, '|');
    if union.len() > 1 {
        for part in union {
            collect_type_name_dotted_root_imports(part, root, imports);
        }
        return;
    }
    if let Some(parts) = generic_parts(ty) {
        collect_type_name_token_dotted_root_import(parts.root, root, imports);
        for arg in parts.args {
            collect_type_name_dotted_root_imports(arg, root, imports);
        }
        return;
    }
    for part in ty.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
    {
        collect_type_name_token_dotted_root_import(part, root, imports);
    }
}

fn collect_type_name_token_dotted_root_import(
    token: &str,
    root: &str,
    imports: &mut BTreeSet<Vec<String>>,
) {
    collect_path_dotted_root_import(token, root, imports);
}

fn collect_path_dotted_root_import(path: &str, root: &str, imports: &mut BTreeSet<Vec<String>>) {
    let Some(rest) = path.strip_prefix(&format!("{root}.")) else {
        return;
    };
    let Some(module) = rest.split('.').next().filter(|module| !module.is_empty()) else {
        return;
    };
    imports.insert(vec![root.to_string(), module.to_string()]);
}

#[cfg(test)]
mod tests;
