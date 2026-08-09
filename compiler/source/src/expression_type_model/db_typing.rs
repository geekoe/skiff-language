use super::*;

impl<'a> OwnerChecker<'a> {
    pub(super) fn check_db_predicate_expr(
        &mut self,
        expr: &Expr,
        fields: &BTreeMap<String, ResolvedTypeRef>,
    ) -> Option<ResolvedTypeRef> {
        self.check_expr_with_field_diagnostics(expr, true, Some(fields))
    }

    pub(super) fn check_db_operation_expr(
        &mut self,
        operation: &crate::shared::ast::DbOperation,
    ) -> Option<ResolvedTypeRef> {
        self.check_db_operation_children(operation);
        self.db_operation_type(operation)
    }

    pub(super) fn check_db_query_expr(
        &mut self,
        query: &crate::shared::ast::DbQuery,
    ) -> Option<ResolvedTypeRef> {
        self.check_db_query_block(&query.query, &query.target);
        self.db_query_type(&query.target)
    }

    pub(super) fn check_db_transaction_expr(
        &mut self,
        transaction: &crate::shared::ast::DbTransaction,
    ) -> Option<ResolvedTypeRef> {
        self.db_transaction_depth += 1;
        let mut last = None;
        for stmt in &transaction.body.statements {
            if let Stmt::Expr(value) = stmt {
                last = self.check_expr(value);
            } else {
                self.check_stmt(stmt);
            }
        }
        self.db_transaction_depth -= 1;
        match transaction.mode {
            DbBlockMode::Effect => self.resolve_builtin(BuiltinShape::Null.name()),
            DbBlockMode::Value => last,
        }
    }

    pub(super) fn check_db_lease_claim_expr(
        &mut self,
        claim: &crate::shared::ast::DbLeaseClaim,
    ) -> Option<ResolvedTypeRef> {
        self.check_expr(&claim.key);
        if let Some(binding) = &claim.binding {
            if let Ok(target) = self
                .type_resolution
                .resolve_type_ref(&claim.target, &self.type_context)
            {
                let previous = self.env.insert(binding.clone(), target);
                self.check_block(&claim.body);
                if let Some(previous) = previous {
                    self.env.insert(binding.clone(), previous);
                } else {
                    self.env.remove(binding);
                }
            } else {
                self.check_block(&claim.body);
            }
        } else {
            self.check_block(&claim.body);
        }
        self.resolve_builtin(BuiltinShape::Bool.name())
    }

    pub(super) fn check_db_lease_read_expr(
        &mut self,
        read: &crate::shared::ast::DbLeaseRead,
    ) -> Option<ResolvedTypeRef> {
        self.check_expr(&read.key);
        Some(db_lease_read_type())
    }

    pub(super) fn check_db_operation_children(
        &mut self,
        operation: &crate::shared::ast::DbOperation,
    ) {
        if let Some(selector) = &operation.selector {
            self.check_db_selector(selector, &operation.target);
        }
        if let Some(query) = operation.independent_query() {
            self.check_db_query_block(query, &operation.target);
        }
        if let Some(body) = &operation.body {
            self.check_db_body(body);
        }
        if let Some(body) = &operation.insert_body {
            self.check_db_body(body);
        }
        if let Some(change) = &operation.change {
            for op in &change.ops {
                match op {
                    DbChangeOp::Set { path, value }
                    | DbChangeOp::Inc { path, value }
                    | DbChangeOp::AddToSet { path, value }
                    | DbChangeOp::Remove { path, value } => {
                        self.validate_db_change_path(&operation.target, &path.segments);
                        self.check_expr(value);
                    }
                    DbChangeOp::Unset { path } => {
                        self.validate_db_change_path(&operation.target, &path.segments);
                    }
                }
            }
        }
    }

    pub(super) fn validate_db_change_path(&mut self, target: &TypeRef, path: &[String]) {
        let Ok(target_type) = self
            .type_resolution
            .resolve_type_ref(target, &self.type_context)
        else {
            return;
        };
        if let Err(error) = DbProjectionTypeResolver::new(
            self.module_path,
            self.type_resolution,
            self.publication_db_metadata,
        )
        .project_read_type(&target.name, target_type.ir, &[path.to_vec()])
        {
            if error.contains("has no DB metadata") {
                return;
            }
            self.outputs.diagnostics.push(format!(
                "{}: db change field path `{}` is invalid: {error}",
                self.module_path,
                path.join(".")
            ));
        }
    }

    pub(super) fn check_db_selector(&mut self, selector: &DbSelector, target: &TypeRef) {
        match selector {
            DbSelector::Key { value } => {
                self.check_expr(value);
            }
            DbSelector::Query { query } => self.check_db_query_block(query, target),
        }
    }

    pub(super) fn check_db_query_block(&mut self, query: &DbQueryBlock, target: &TypeRef) {
        for clause in &query.where_clauses {
            match clause {
                DbWhereClause::Predicate { predicate } => {
                    self.check_db_predicate(predicate, target);
                }
                DbWhereClause::Conditional {
                    condition,
                    predicate,
                } => {
                    self.check_condition(condition, "db where condition");
                    self.check_db_predicate(predicate, target);
                }
            }
        }
        if let Some(limit) = &query.limit {
            self.check_expr(limit);
        }
        if let Some(offset) = &query.offset {
            self.check_expr(offset);
        }
        if let Some(after) = &query.after {
            self.check_expr(after);
        }
    }

    pub(super) fn check_db_predicate(&mut self, predicate: &Expr, target: &TypeRef) {
        let fields = self
            .type_resolution
            .resolve_constructor_target_text(&target.name, &self.type_context)
            .map(|target| target.fields)
            .unwrap_or_default();
        let actual = self.check_db_predicate_expr(predicate, &fields);
        let (Some(actual), Some(expected)) =
            (actual, self.resolve_builtin(BuiltinShape::Bool.name()))
        else {
            return;
        };
        if !self
            .type_resolution
            .assignable_in_context(&actual, &expected, &self.type_context)
        {
            self.outputs.diagnostics.push(format!(
                "{}: db where predicate type mismatch at {}: expected bool, found {}",
                self.module_path,
                self.current_expression_span_label(),
                actual
            ));
        }
    }

    pub(super) fn check_db_field_operand(
        &mut self,
        expr: &Expr,
        fields: &BTreeMap<String, ResolvedTypeRef>,
    ) -> Option<ResolvedTypeRef> {
        let Some(root) = Self::db_field_operand_root(expr, fields) else {
            return self.check_expr(expr);
        };
        let target_type = fields
            .get(&root)
            .expect("DB field operand root must come from target fields")
            .clone();
        let previous_env = self.env.insert(root.clone(), target_type);
        let descendant_prefix = format!("{root}.");
        let conflicting_paths = self
            .path_refinements
            .keys()
            .filter(|path| *path == &root || path.starts_with(&descendant_prefix))
            .cloned()
            .collect::<Vec<_>>();
        let previous_refinements = conflicting_paths
            .into_iter()
            .filter_map(|path| self.path_refinements.remove(&path).map(|ty| (path, ty)))
            .collect::<Vec<_>>();
        let ty = self.check_expr(expr);
        if let Some(previous_env) = previous_env {
            self.env.insert(root.clone(), previous_env);
        } else {
            self.env.remove(&root);
        }
        for (path, ty) in previous_refinements {
            self.path_refinements.insert(path, ty);
        }
        ty
    }

    pub(super) fn invalidate_path_refinements_for_write(&mut self, target: &Expr) {
        let Some(path) = expr_path(target) else {
            self.path_refinements.clear();
            return;
        };
        // A root narrowing lives in `env`; a write through the bare root
        // (whole-binding assignment or inout loan) reverts it to the recorded
        // base (R-196). Member writes leave the root's nullability narrowing
        // alone — only the member path and its subpaths lose their narrowing.
        let root = path.split('.').next().unwrap_or(&path);
        if path == root {
            if let Some(base) = self.narrowing_base.remove(root) {
                self.env.insert(root.to_string(), base);
            }
        }
        let descendant_prefix = format!("{path}.");
        self.path_refinements
            .retain(|refined, _| refined != &path && !refined.starts_with(&descendant_prefix));
    }

    pub(super) fn is_db_field_operand(
        expr: &Expr,
        fields: &BTreeMap<String, ResolvedTypeRef>,
    ) -> bool {
        Self::db_field_operand_root(expr, fields).is_some()
    }

    pub(super) fn db_field_operand_root(
        expr: &Expr,
        fields: &BTreeMap<String, ResolvedTypeRef>,
    ) -> Option<String> {
        expr_path(expr)
            .and_then(|path| path.split('.').next().map(str::to_string))
            .filter(|root| fields.contains_key(root))
    }

    pub(super) fn check_db_body(&mut self, body: &DbBody) {
        match body {
            DbBody::ObjectFields { fields } => {
                for field in fields {
                    self.check_expr(&field.value);
                }
            }
            DbBody::Values { value } => {
                self.check_expr(value);
            }
        }
    }

    pub(super) fn db_operation_type(
        &mut self,
        operation: &crate::shared::ast::DbOperation,
    ) -> Option<ResolvedTypeRef> {
        let target = self
            .type_resolution
            .resolve_type_ref(&operation.target, &self.type_context)
            .ok()?;
        if self
            .type_resolution
            .actor_type_resolution(&target, &self.type_context)
            .is_some()
        {
            self.outputs.diagnostics.push(format!(
                "{}: actor handle type `{}` cannot be used as a database object",
                self.module_path, target
            ));
            return None;
        }
        if matches!(
            operation.op,
            crate::shared::ast::DbOperationKind::Insert
                | crate::shared::ast::DbOperationKind::Replace
                | crate::shared::ast::DbOperationKind::Upsert
        ) && self.db_operation_target_is_contract(&operation.target.name)
        {
            self.outputs.diagnostics.push(format!(
                "{}: db {} on contract target `{}` is not allowed: the engine contract view cannot insert or replace the whole shared document; the host owns the collection",
                self.module_path,
                db_operation_kind_text(operation.op),
                operation.target.name
            ));
            return None;
        }
        let read = self.db_read_type(operation, &target)?;
        match operation.op {
            crate::shared::ast::DbOperationKind::Find if operation.many => Some(array_type(read)),
            crate::shared::ast::DbOperationKind::Find
            | crate::shared::ast::DbOperationKind::Optional => Some(nullable_type(read)),
            crate::shared::ast::DbOperationKind::Require => Some(read),
            crate::shared::ast::DbOperationKind::Insert if operation.many => {
                self.resolve_builtin("DbInsertManyResult")
            }
            crate::shared::ast::DbOperationKind::Insert => Some(target.clone()),
            crate::shared::ast::DbOperationKind::Update if operation.many => {
                self.resolve_builtin("DbUpdateManyResult")
            }
            crate::shared::ast::DbOperationKind::Update
            | crate::shared::ast::DbOperationKind::Replace => Some(nullable_type(target.clone())),
            crate::shared::ast::DbOperationKind::Delete if operation.many => {
                self.resolve_builtin("DbDeleteManyResult")
            }
            crate::shared::ast::DbOperationKind::Upsert => Some(projection_record_type(
                BuiltinShape::DbUpsertResult.name(),
                &target,
            )),
            crate::shared::ast::DbOperationKind::Delete
            | crate::shared::ast::DbOperationKind::Exists => {
                self.resolve_builtin(BuiltinShape::Bool.name())
            }
            crate::shared::ast::DbOperationKind::Count => {
                self.resolve_builtin(BuiltinShape::Number.name())
            }
        }
    }

    pub(super) fn db_read_type(
        &mut self,
        operation: &crate::shared::ast::DbOperation,
        target: &ResolvedTypeRef,
    ) -> Option<ResolvedTypeRef> {
        let Some(projection) = operation.projection.as_ref() else {
            return Some(target.clone());
        };
        let paths = projection
            .fields
            .iter()
            .map(|field| field.segments.clone())
            .collect::<Vec<_>>();
        match DbProjectionTypeResolver::new(
            self.module_path,
            self.type_resolution,
            self.publication_db_metadata,
        )
        .project_read_type(&operation.target.name, target.ir.clone(), &paths)
        {
            Ok(ty) => Some(resolved_type_from_ir(&ty)),
            Err(error) => {
                self.outputs
                    .diagnostics
                    .push(format!("{}: {error}", self.module_path));
                None
            }
        }
    }

    pub(super) fn db_query_type(&self, target: &TypeRef) -> Option<ResolvedTypeRef> {
        let target = self
            .type_resolution
            .resolve_type_ref(target, &self.type_context)
            .ok()?;
        Some(projection_record_type("DbQuery", &target))
    }

    fn db_operation_target_is_contract(&self, target_name: &str) -> bool {
        match DbProjectionTypeResolver::new(
            self.module_path,
            self.type_resolution,
            self.publication_db_metadata,
        )
        .resolve_metadata(target_name)
        {
            Ok(Some(metadata)) => metadata.kind == skiff_artifact_model::DbObjectKindIr::Contract,
            _ => false,
        }
    }
}

fn db_operation_kind_text(operation: crate::shared::ast::DbOperationKind) -> &'static str {
    match operation {
        crate::shared::ast::DbOperationKind::Insert => "insert",
        crate::shared::ast::DbOperationKind::Replace => "replace",
        crate::shared::ast::DbOperationKind::Upsert => "upsert",
        _ => "operation",
    }
}

fn projection_record_type(name: &str, target: &ResolvedTypeRef) -> ResolvedTypeRef {
    let text = format!("{name}<{target}>");
    ResolvedTypeRef::with_text(
        TypeRefIr::Builtin {
            name: name.to_string(),
            args: vec![target.ir.clone()],
        },
        text,
    )
}

fn db_lease_read_type() -> ResolvedTypeRef {
    let string = TypeRefIr::Builtin {
        name: BuiltinShape::String.name().to_string(),
        args: Vec::new(),
    };
    nullable_type(ResolvedTypeRef::with_text(
        TypeRefIr::Record {
            fields: BTreeMap::from([
                ("expiresAt".to_string(), string.clone()),
                ("owner".to_string(), string.clone()),
                ("requestId".to_string(), string),
            ]),
        },
        "{ expiresAt: string, owner: string, requestId: string }".to_string(),
    ))
}

fn array_type(item: ResolvedTypeRef) -> ResolvedTypeRef {
    let text = format!("Array<{item}>");
    ResolvedTypeRef::with_text(
        TypeRefIr::Builtin {
            name: BuiltinShape::Array.name().to_string(),
            args: vec![item.ir],
        },
        text,
    )
}
