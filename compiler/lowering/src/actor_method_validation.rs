use std::collections::BTreeSet;

use skiff_artifact_model::{AssignTargetIr, CallTargetIr, ExprIr, FileIrUnit, StmtIr};
use skiff_syntax::{
    ast::{
        ActorDecl, Block, DbBody, DbChangeOp, DbOperation, DbSelector, DbWhereClause, Expr,
        SourceFile, Stmt,
    },
    error::{CompileError, Result},
};

/// Source-level actor rules:
/// - the key field is platform-owned and read-only inside every member method
///   (including create);
/// - create must definitely assign every non-key field before reading it or
///   returning;
/// - create cannot call other methods of the same instance.
pub(super) fn validate_actor_source_rules(ast: &SourceFile) -> Result<()> {
    for actor in &ast.actors {
        let methods = ast
            .impls
            .iter()
            .filter(|implementation| implementation.target == actor.name)
            .flat_map(|implementation| implementation.method_bodies.iter())
            .filter(|method| !method.is_static)
            .collect::<Vec<_>>();
        let method_names = methods
            .iter()
            .map(|method| method.name.as_str())
            .collect::<BTreeSet<_>>();
        let non_key_fields = ast
            .types
            .iter()
            .find(|ty| ty.name == actor.name)
            .map(|ty| {
                ty.fields
                    .iter()
                    .filter(|field| field.name != actor.key_field)
                    .map(|field| field.name.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for method in &methods {
            validate_key_writes(actor, &method.body)?;
            if method.name == "create" {
                validate_create_body(actor, &method.body, &method_names, &non_key_fields)?;
            }
        }
    }
    Ok(())
}

/// IR-level actor rules (all statements/expressions live in the body tables,
/// so walking the tables is complete):
/// - key field writes are rejected in every actor method executable;
/// - create executables must not dispatch any method of their own actor
///   (catches aliased handles the source rule cannot see).
pub(super) fn validate_actor_method_ir_rules(unit: &FileIrUnit) -> Result<()> {
    for declaration in &unit.actor_declarations {
        let key_field = declaration.abi.key_field.as_str();
        for executable_index in declaration.method_implementations.values().copied().chain(
            declaration
                .create_implementation
                .as_ref()
                .map(|create| create.executable_index),
        ) {
            let executable = unit
                .executables
                .get(executable_index as usize)
                .ok_or_else(|| {
                    CompileError::Semantic(format!(
                        "actor {} method implementation index {executable_index} is out of bounds",
                        declaration.abi.actor_name
                    ))
                })?;
            for statement in &executable.body.statements {
                let StmtIr::Assign {
                    target: AssignTargetIr::ActorSelfField { field, .. },
                    ..
                } = statement
                else {
                    continue;
                };
                if field == key_field {
                    return Err(CompileError::Semantic(format!(
                        "actor {} key field {} is platform-owned and read-only inside member methods",
                        declaration.abi.actor_name, field
                    )));
                }
            }
        }
        if let Some(create) = declaration.create_implementation.as_ref() {
            let executable = unit
                .executables
                .get(create.executable_index as usize)
                .ok_or_else(|| {
                    CompileError::Semantic(format!(
                        "actor {} create implementation index {} is out of bounds",
                        declaration.abi.actor_name, create.executable_index
                    ))
                })?;
            for expression in &executable.body.expressions {
                let ExprIr::Call { call } = expression else {
                    continue;
                };
                let CallTargetIr::ActorMethod { actor, .. } = &call.target else {
                    continue;
                };
                if actor.module_path == unit.module_path
                    && actor.symbol == declaration.abi.actor_name
                {
                    return Err(CompileError::Semantic(format!(
                        "actor {} create cannot call other methods of the same instance",
                        declaration.abi.actor_name
                    )));
                }
            }
        }
    }
    Ok(())
}

fn validate_key_writes(actor: &ActorDecl, block: &Block) -> Result<()> {
    for statement in &block.statements {
        match statement {
            Stmt::Assign { target, .. } => {
                if let Some(field) = self_field(target) {
                    if field == actor.key_field {
                        return Err(CompileError::Semantic(format!(
                            "actor {} key field {} is platform-owned and read-only inside member methods",
                            actor.name, field
                        )));
                    }
                }
            }
            Stmt::Timeout { body, .. }
            | Stmt::Concurrent { body }
            | Stmt::Serial { body }
            | Stmt::DbTransaction { body } => validate_key_writes(actor, body)?,
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                validate_key_writes(actor, then_block)?;
                if let Some(else_block) = else_block {
                    validate_key_writes(actor, else_block)?;
                }
            }
            Stmt::For { body, .. } | Stmt::While { body, .. } => validate_key_writes(actor, body)?,
            Stmt::Match { arms, .. } => {
                for arm in arms {
                    validate_key_writes(actor, &arm.body)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_create_body(
    actor: &ActorDecl,
    block: &Block,
    method_names: &BTreeSet<&str>,
    non_key_fields: &[String],
) -> Result<()> {
    let validator = CreateValidator {
        actor,
        method_names,
        non_key_fields,
    };
    let outcome = validator.analyze_block(block, BTreeSet::new())?;
    if let Some(assigned) = outcome {
        validator.require_all_assigned(&assigned)?;
    }
    Ok(())
}

struct CreateValidator<'a> {
    actor: &'a ActorDecl,
    method_names: &'a BTreeSet<&'a str>,
    non_key_fields: &'a [String],
}

impl CreateValidator<'_> {
    /// Returns `None` when the block always terminates (return/throw), or the
    /// set of definitely-assigned non-key fields on fallthrough.
    fn analyze_block(
        &self,
        block: &Block,
        assigned: BTreeSet<String>,
    ) -> Result<Option<BTreeSet<String>>> {
        let mut current = assigned;
        for statement in &block.statements {
            match statement {
                Stmt::CompilerTestEffectRegister { .. } => {}
                Stmt::Assert { condition, .. } => {
                    self.check_reads(condition, &current)?;
                    self.check_self_calls(condition)?;
                }
                Stmt::Let { value, .. } => {
                    self.check_reads(value, &current)?;
                    self.check_self_calls(value)?;
                }
                Stmt::Assign { target, value } => {
                    self.check_reads(value, &current)?;
                    self.check_self_calls(value)?;
                    if let Some(field) = self_field(target) {
                        if field != self.actor.key_field {
                            current.insert(field.to_string());
                        }
                    }
                }
                Stmt::Timeout { body, .. } | Stmt::DbTransaction { body } => {
                    current = self.require_fallthrough(self.analyze_block(body, current)?)?;
                }
                Stmt::Concurrent { body } | Stmt::Serial { body } => {
                    // Lanes cannot write actor fields; check their reads only.
                    self.analyze_block(body, current.clone())?;
                }
                Stmt::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    self.check_reads(condition, &current)?;
                    self.check_self_calls(condition)?;
                    let then_out = self.analyze_block(then_block, current.clone())?;
                    let else_out = match else_block {
                        Some(block) => self.analyze_block(block, current.clone())?,
                        None => Some(current.clone()),
                    };
                    current = match intersect_outcomes(then_out, else_out) {
                        Some(out) => out,
                        None => return Ok(None),
                    };
                }
                Stmt::For { iterable, body, .. }
                | Stmt::While {
                    condition: iterable,
                    body,
                } => {
                    self.check_reads(iterable, &current)?;
                    self.check_self_calls(iterable)?;
                    // A loop body may run zero times: only pre-loop
                    // assignments are definite after it. Reads inside the
                    // body are checked with a fixpoint over body-local
                    // assignments.
                    let mut body_in = current.clone();
                    loop {
                        let Some(out) = self.analyze_block(body, body_in.clone())? else {
                            break;
                        };
                        let next = body_in.union(&out).cloned().collect::<BTreeSet<_>>();
                        if next == body_in {
                            break;
                        }
                        body_in = next;
                    }
                }
                Stmt::Match { value, arms } => {
                    self.check_reads(value, &current)?;
                    self.check_self_calls(value)?;
                    let mut outs = Vec::new();
                    for arm in arms {
                        if let Some(out) = self.analyze_block(&arm.body, current.clone())? {
                            outs.push(out);
                        }
                    }
                    if outs.is_empty() {
                        return Ok(None);
                    }
                    current = outs
                        .into_iter()
                        .reduce(|left, right| {
                            left.intersection(&right).cloned().collect::<BTreeSet<_>>()
                        })
                        .unwrap_or_default();
                }
                Stmt::Throw { value } => {
                    self.check_reads(value, &current)?;
                    self.check_self_calls(value)?;
                    return Ok(None);
                }
                Stmt::Rethrow { exception } => {
                    self.check_reads(exception, &current)?;
                    self.check_self_calls(exception)?;
                    return Ok(None);
                }
                Stmt::Emit(value) | Stmt::Expr(value) => {
                    self.check_reads(value, &current)?;
                    self.check_self_calls(value)?;
                }
                Stmt::Return(value) => {
                    if let Some(value) = value {
                        self.check_reads(value, &current)?;
                        self.check_self_calls(value)?;
                    }
                    self.require_all_assigned(&current)?;
                    return Ok(None);
                }
                Stmt::Spawn { call } => {
                    self.check_reads(call, &current)?;
                    self.check_self_calls(call)?;
                }
                Stmt::Break | Stmt::Continue => return Ok(Some(current)),
            }
        }
        Ok(Some(current))
    }

    fn require_fallthrough(&self, outcome: Option<BTreeSet<String>>) -> Result<BTreeSet<String>> {
        outcome.ok_or_else(|| {
            CompileError::Semantic(format!(
                "actor {} create block always terminates before completing initialization",
                self.actor.name
            ))
        })
    }

    fn require_all_assigned(&self, assigned: &BTreeSet<String>) -> Result<()> {
        let missing = self
            .non_key_fields
            .iter()
            .filter(|field| !assigned.contains(*field))
            .cloned()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return Ok(());
        }
        Err(CompileError::Semantic(format!(
            "actor {} create returns before assigning field(s): {}",
            self.actor.name,
            missing.join(", ")
        )))
    }

    fn check_reads(&self, expr: &Expr, assigned: &BTreeSet<String>) -> Result<()> {
        match expr {
            Expr::Field { object, field } => {
                if let Expr::Identifier(name) = object.as_ref() {
                    if name == "self" && field != &self.actor.key_field && !assigned.contains(field)
                    {
                        return Err(CompileError::Semantic(format!(
                            "actor {} create reads self.{} before it is assigned",
                            self.actor.name, field
                        )));
                    }
                }
                self.check_reads(object, assigned)?;
            }
            Expr::Binary { left, right, .. } => {
                self.check_reads(left, assigned)?;
                self.check_reads(right, assigned)?;
            }
            Expr::Unary { expr, .. } => self.check_reads(expr, assigned)?,
            Expr::Call { callee, args } => {
                self.check_reads(callee, assigned)?;
                for arg in args {
                    self.check_reads(arg, assigned)?;
                }
            }
            Expr::Generic { callee, .. } => self.check_reads(callee, assigned)?,
            Expr::InterfaceBox { value, .. } => self.check_reads(value, assigned)?,
            Expr::Record { fields, .. } => {
                for (_, value) in fields {
                    self.check_reads(value, assigned)?;
                }
            }
            Expr::ObjectLiteral { entries } => {
                for entry in entries {
                    self.check_reads(&entry.value, assigned)?;
                }
            }
            Expr::Patch { operations, .. } => {
                for operation in operations {
                    match operation {
                        skiff_syntax::ast::PatchOperation::Set { value, .. }
                        | skiff_syntax::ast::PatchOperation::Inc { value, .. } => {
                            self.check_reads(value, assigned)?;
                        }
                    }
                }
            }
            Expr::ValueBlock(block) => {
                let out = self.analyze_block(&block.body, assigned.clone())?;
                let out = self.require_fallthrough(out)?;
                let mut extended = assigned.clone();
                extended.extend(out);
                self.check_reads(&block.tail, &extended)?;
            }
            Expr::ConcurrentValue(block) => {
                self.analyze_block(&block.body, assigned.clone())?;
                self.check_reads(&block.tail, assigned)?;
            }
            Expr::Timeout { value, .. } => self.check_reads(value, assigned)?,
            Expr::Throw { value } => self.check_reads(value, assigned)?,
            Expr::Rethrow { exception } => self.check_reads(exception, assigned)?,
            Expr::Catch { try_expr, .. } => {
                // Conservative: assignments inside try or catch do not become
                // definite after the catch expression.
                self.check_reads(try_expr, assigned)?;
            }
            Expr::DbOperation(operation) => {
                check_db_reads(self, operation, assigned)?;
            }
            Expr::DbQuery(query) => {
                check_query_block_reads(self, &query.query, assigned)?;
            }
            Expr::DbTransaction(transaction) => {
                self.analyze_block(&transaction.body, assigned.clone())?;
            }
            Expr::DbLeaseClaim(claim) => {
                self.check_reads(&claim.key, assigned)?;
                self.analyze_block(&claim.body, assigned.clone())?;
            }
            Expr::DbLeaseRead(read) => {
                self.check_reads(&read.key, assigned)?;
            }
            Expr::Literal(_) | Expr::Identifier(_) | Expr::DependencySourceAddress(_) => {}
        }
        Ok(())
    }

    fn check_self_calls(&self, expr: &Expr) -> Result<()> {
        match expr {
            Expr::Call { callee, args } => {
                let callee = match callee.as_ref() {
                    Expr::Generic { callee, .. } => callee.as_ref(),
                    callee => callee,
                };
                if let Expr::Field { object, field } = callee {
                    if let Expr::Identifier(name) = object.as_ref() {
                        if name == "self" && self.method_names.contains(field.as_str()) {
                            return Err(CompileError::Semantic(format!(
                                "actor {} create cannot call other methods of the same instance",
                                self.actor.name
                            )));
                        }
                    }
                }
                self.check_self_calls(callee)?;
                for arg in args {
                    self.check_self_calls(arg)?;
                }
            }
            Expr::Binary { left, right, .. } => {
                self.check_self_calls(left)?;
                self.check_self_calls(right)?;
            }
            Expr::Unary { expr, .. } => self.check_self_calls(expr)?,
            Expr::Generic { callee, .. } => self.check_self_calls(callee)?,
            Expr::InterfaceBox { value, .. } => self.check_self_calls(value)?,
            Expr::Field { object, .. } => self.check_self_calls(object)?,
            Expr::Record { fields, .. } => {
                for (_, value) in fields {
                    self.check_self_calls(value)?;
                }
            }
            Expr::ObjectLiteral { entries } => {
                for entry in entries {
                    self.check_self_calls(&entry.value)?;
                }
            }
            Expr::Patch { operations, .. } => {
                for operation in operations {
                    match operation {
                        skiff_syntax::ast::PatchOperation::Set { value, .. }
                        | skiff_syntax::ast::PatchOperation::Inc { value, .. } => {
                            self.check_self_calls(value)?;
                        }
                    }
                }
            }
            Expr::ValueBlock(block) => {
                self.check_block_self_calls(&block.body)?;
                self.check_self_calls(&block.tail)?;
            }
            Expr::ConcurrentValue(block) => {
                self.check_block_self_calls(&block.body)?;
                self.check_self_calls(&block.tail)?;
            }
            Expr::Timeout { value, .. } => self.check_self_calls(value)?,
            Expr::Throw { value } => self.check_self_calls(value)?,
            Expr::Rethrow { exception } => self.check_self_calls(exception)?,
            Expr::Catch { try_expr, .. } => self.check_self_calls(try_expr)?,
            Expr::DbOperation(operation) => check_db_self_calls(self, operation)?,
            Expr::DbQuery(query) => check_query_block_self_calls(self, &query.query)?,
            Expr::DbTransaction(transaction) => {
                self.check_block_self_calls(&transaction.body)?;
            }
            Expr::DbLeaseClaim(claim) => {
                self.check_self_calls(&claim.key)?;
                self.check_block_self_calls(&claim.body)?;
            }
            Expr::DbLeaseRead(read) => {
                self.check_self_calls(&read.key)?;
            }
            Expr::Literal(_) | Expr::Identifier(_) | Expr::DependencySourceAddress(_) => {}
        }
        Ok(())
    }

    fn check_block_self_calls(&self, block: &Block) -> Result<()> {
        for statement in &block.statements {
            match statement {
                Stmt::CompilerTestEffectRegister { .. } => {}
                Stmt::Assert { condition, .. } => self.check_self_calls(condition)?,
                Stmt::Let { value, .. } => self.check_self_calls(value)?,
                Stmt::Assign { target, value } => {
                    self.check_self_calls(target)?;
                    self.check_self_calls(value)?;
                }
                Stmt::Timeout { body, .. }
                | Stmt::Concurrent { body }
                | Stmt::Serial { body }
                | Stmt::DbTransaction { body } => self.check_block_self_calls(body)?,
                Stmt::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    self.check_self_calls(condition)?;
                    self.check_block_self_calls(then_block)?;
                    if let Some(else_block) = else_block {
                        self.check_block_self_calls(else_block)?;
                    }
                }
                Stmt::For { iterable, body, .. }
                | Stmt::While {
                    condition: iterable,
                    body,
                } => {
                    self.check_self_calls(iterable)?;
                    self.check_block_self_calls(body)?;
                }
                Stmt::Match { value, arms } => {
                    self.check_self_calls(value)?;
                    for arm in arms {
                        self.check_block_self_calls(&arm.body)?;
                    }
                }
                Stmt::Throw { value } => self.check_self_calls(value)?,
                Stmt::Rethrow { exception } => self.check_self_calls(exception)?,
                Stmt::Emit(value) | Stmt::Expr(value) => self.check_self_calls(value)?,
                Stmt::Return(value) => {
                    if let Some(value) = value {
                        self.check_self_calls(value)?;
                    }
                }
                Stmt::Spawn { call } => self.check_self_calls(call)?,
                Stmt::Break | Stmt::Continue => {}
            }
        }
        Ok(())
    }
}

fn intersect_outcomes(
    left: Option<BTreeSet<String>>,
    right: Option<BTreeSet<String>>,
) -> Option<BTreeSet<String>> {
    match (left, right) {
        (Some(left), Some(right)) => {
            Some(left.intersection(&right).cloned().collect::<BTreeSet<_>>())
        }
        (Some(out), None) | (None, Some(out)) => Some(out),
        (None, None) => None,
    }
}

fn self_field(expr: &Expr) -> Option<&str> {
    let Expr::Field { object, field } = expr else {
        return None;
    };
    matches!(object.as_ref(), Expr::Identifier(name) if name == "self").then_some(field)
}

fn check_db_reads(
    validator: &CreateValidator<'_>,
    operation: &DbOperation,
    assigned: &BTreeSet<String>,
) -> Result<()> {
    if let Some(selector) = &operation.selector {
        match selector {
            DbSelector::Key { value } => validator.check_reads(value, assigned)?,
            DbSelector::Query { query } => check_query_block_reads(validator, query, assigned)?,
        }
    }
    if let Some(query) = &operation.query {
        check_query_block_reads(validator, query, assigned)?;
    }
    check_db_body_reads(validator, operation.body.as_ref(), assigned)?;
    check_db_body_reads(validator, operation.insert_body.as_ref(), assigned)?;
    if let Some(change) = &operation.change {
        for op in &change.ops {
            match op {
                DbChangeOp::Set { value, .. }
                | DbChangeOp::Inc { value, .. }
                | DbChangeOp::AddToSet { value, .. }
                | DbChangeOp::Remove { value, .. } => {
                    validator.check_reads(value, assigned)?;
                }
                DbChangeOp::Unset { .. } => {}
            }
        }
    }
    Ok(())
}

fn check_db_body_reads(
    validator: &CreateValidator<'_>,
    body: Option<&DbBody>,
    assigned: &BTreeSet<String>,
) -> Result<()> {
    match body {
        Some(DbBody::ObjectFields { fields }) => {
            for field in fields {
                validator.check_reads(&field.value, assigned)?;
            }
        }
        Some(DbBody::Values { value }) => validator.check_reads(value, assigned)?,
        None => {}
    }
    Ok(())
}

fn check_query_block_reads(
    validator: &CreateValidator<'_>,
    query: &skiff_syntax::ast::DbQueryBlock,
    assigned: &BTreeSet<String>,
) -> Result<()> {
    for clause in &query.where_clauses {
        match clause {
            DbWhereClause::Predicate { predicate } => validator.check_reads(predicate, assigned)?,
            DbWhereClause::Conditional {
                condition,
                predicate,
            } => {
                validator.check_reads(condition, assigned)?;
                validator.check_reads(predicate, assigned)?;
            }
        }
    }
    for value in [&query.limit, &query.offset, &query.after]
        .into_iter()
        .flatten()
    {
        validator.check_reads(value, assigned)?;
    }
    Ok(())
}

fn check_db_self_calls(validator: &CreateValidator<'_>, operation: &DbOperation) -> Result<()> {
    if let Some(selector) = &operation.selector {
        match selector {
            DbSelector::Key { value } => validator.check_self_calls(value)?,
            DbSelector::Query { query } => check_query_block_self_calls(validator, query)?,
        }
    }
    if let Some(query) = &operation.query {
        check_query_block_self_calls(validator, query)?;
    }
    check_db_body_self_calls(validator, operation.body.as_ref())?;
    check_db_body_self_calls(validator, operation.insert_body.as_ref())?;
    if let Some(change) = &operation.change {
        for op in &change.ops {
            match op {
                DbChangeOp::Set { value, .. }
                | DbChangeOp::Inc { value, .. }
                | DbChangeOp::AddToSet { value, .. }
                | DbChangeOp::Remove { value, .. } => {
                    validator.check_self_calls(value)?;
                }
                DbChangeOp::Unset { .. } => {}
            }
        }
    }
    Ok(())
}

fn check_db_body_self_calls(validator: &CreateValidator<'_>, body: Option<&DbBody>) -> Result<()> {
    match body {
        Some(DbBody::ObjectFields { fields }) => {
            for field in fields {
                validator.check_self_calls(&field.value)?;
            }
        }
        Some(DbBody::Values { value }) => validator.check_self_calls(value)?,
        None => {}
    }
    Ok(())
}

fn check_query_block_self_calls(
    validator: &CreateValidator<'_>,
    query: &skiff_syntax::ast::DbQueryBlock,
) -> Result<()> {
    for clause in &query.where_clauses {
        match clause {
            DbWhereClause::Predicate { predicate } => validator.check_self_calls(predicate)?,
            DbWhereClause::Conditional {
                condition,
                predicate,
            } => {
                validator.check_self_calls(condition)?;
                validator.check_self_calls(predicate)?;
            }
        }
    }
    for value in [&query.limit, &query.offset, &query.after]
        .into_iter()
        .flatten()
    {
        validator.check_self_calls(value)?;
    }
    Ok(())
}
