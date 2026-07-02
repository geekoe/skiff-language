use async_recursion::async_recursion;
use serde_json::{json, Value};
use skiff_runtime_capability_context::{
    DbDocument, DbKey, DbOrderDirection, DbOrderEntry, DbQuery, FieldPath, ServiceDbChange,
    ServiceDbFindOptions,
};
use skiff_runtime_linked_type_plan::{PlanContext, RuntimeRecoverableExpectedTypePlanLinkedExt};
use skiff_runtime_model::{
    recoverable::{RuntimeRecoverableExpectedTypeNode, RuntimeRecoverableExpectedTypePlan},
    request_heap::RequestHeap,
    runtime_value::{RuntimeObject, RuntimeObjectFields, RuntimeValue},
    type_plan::RuntimeTypePlan,
};

use super::{
    capabilities::{DbRecoverableRuntimeExpectedPlans, DbRuntimeChange, DbRuntimeSetOp},
    db_command::{
        DbCommand, DbCommandChange, DbCommandValue, DbDeleteOneCommand, DbExistsKeyCommand,
        DbFindManyCommand, DbFindOneCommand, DbInsertManyCommand, DbInsertOneCommand,
        DbOneCommandSelector, DbQueryCommand, DbReplaceOneCommand, DbUpdateManyCommand,
        DbUpdateOneCommand, DbUpsertKeyCommand,
    },
    env::Env,
    program_execution::ProgramExecutionContext,
    runtime_ops::{runtime_from_wire, runtime_numeric, runtime_to_wire, runtime_truthy},
    Interpreter,
};
use crate::error::{Result, RuntimeError};
use skiff_runtime_linked_program::{
    DbBodyIr, DbChangeIr, DbChangeOpIr, DbIndexDirectionIr, DbOpKindIr, DbOperationIr, DbOrderIr,
    DbPredicateCompareOpIr, DbPredicateIr, DbProjectionIr, DbQueryIr, DbSelectorIr, DbTargetIr,
    ExecutableAddr, ExprRefIr, FieldPathIr, LinkedExecutable, LinkedFileUnit, LinkedTypeRef,
};

pub struct DbIrEvaluator<'a> {
    interpreter: &'a Interpreter,
    program_context: ProgramExecutionContext<'a>,
    heap: &'a mut RequestHeap,
    env: &'a mut Env,
    addr: &'a ExecutableAddr,
    file: &'a LinkedFileUnit,
    executable: &'a LinkedExecutable,
}

impl<'a> DbIrEvaluator<'a> {
    pub fn new(
        interpreter: &'a Interpreter,
        program_context: ProgramExecutionContext<'a>,
        heap: &'a mut RequestHeap,
        env: &'a mut Env,
        addr: &'a ExecutableAddr,
        file: &'a LinkedFileUnit,
        executable: &'a LinkedExecutable,
    ) -> Self {
        Self {
            interpreter,
            program_context,
            heap,
            env,
            addr,
            file,
            executable,
        }
    }

    pub async fn eval_operation(&mut self, operation: &DbOperationIr) -> Result<DbCommand> {
        let type_name = operation.target.type_name.clone();
        let result_plan = self.result_plan(&operation.result_type)?;
        let recoverable_plans = self.recoverable_expected_plans(&operation.target)?;
        let projection = db_projection_paths(operation.projection.as_ref());
        if operation.many && matches!(operation.selector.as_ref(), Some(DbSelectorIr::Key { .. })) {
            return Err(RuntimeError::Decode(
                "db many operation cannot use a key selector".to_string(),
            ));
        }

        match operation.op {
            DbOpKindIr::Find | DbOpKindIr::Optional | DbOpKindIr::Require => {
                if operation.many {
                    let query = self.eval_query(operation.query.as_ref()).await?;
                    let options = self
                        .eval_page_options(
                            &type_name,
                            &query,
                            operation.query.as_ref(),
                            projection.as_ref(),
                        )
                        .await?;
                    return Ok(DbCommand::FindMany(DbFindManyCommand {
                        type_name,
                        result_plan,
                        query,
                        options,
                        projection,
                        recoverable_runtime: self
                            .projection_requires_recoverable_runtime(
                                &recoverable_plans,
                                operation.projection.as_ref(),
                            )
                            .then_some(recoverable_plans),
                    }));
                }

                let selector = match operation.selector.as_ref() {
                    Some(DbSelectorIr::Key { value }) => DbOneCommandSelector::Key {
                        key: DbKey::new(self.eval_expr_wire(*value).await?),
                    },
                    Some(DbSelectorIr::Query { query }) => DbOneCommandSelector::Query {
                        query: self.eval_query(Some(query)).await?,
                        order: db_order_entries(&query.order),
                    },
                    None => {
                        let query_value = self.eval_query(operation.query.as_ref()).await?;
                        let order = operation
                            .query
                            .as_ref()
                            .map(|query| db_order_entries(&query.order))
                            .unwrap_or_default();
                        DbOneCommandSelector::Query {
                            query: query_value,
                            order,
                        }
                    }
                };
                Ok(DbCommand::FindOne(DbFindOneCommand {
                    type_name,
                    result_plan,
                    selector,
                    projection,
                    recoverable_runtime: self
                        .projection_requires_recoverable_runtime(
                            &recoverable_plans,
                            operation.projection.as_ref(),
                        )
                        .then_some(recoverable_plans),
                    required: operation.op == DbOpKindIr::Require,
                }))
            }
            DbOpKindIr::Insert => {
                let body = operation
                    .body
                    .as_ref()
                    .ok_or_else(|| RuntimeError::Decode("db insert requires a body".to_string()))?;
                if operation.many {
                    if self.full_object_requires_recoverable_runtime(&recoverable_plans) {
                        return Err(RuntimeError::Decode(
                            "db insert many for recoverable-envelope behavior fields is not supported in P5".to_string(),
                        ));
                    }
                    Ok(DbCommand::InsertMany(DbInsertManyCommand {
                        type_name,
                        result_plan,
                        values: self.eval_body_values(body).await?,
                    }))
                } else {
                    let value = if self.full_object_requires_recoverable_runtime(&recoverable_plans)
                    {
                        DbCommandValue::Runtime {
                            value: self.eval_body_runtime_object(body).await?,
                            recoverable_runtime: recoverable_plans,
                        }
                    } else {
                        DbCommandValue::Wire(self.eval_body_object(body).await?)
                    };
                    Ok(DbCommand::InsertOne(DbInsertOneCommand {
                        type_name,
                        result_plan,
                        value,
                    }))
                }
            }
            DbOpKindIr::Update => {
                if operation.many {
                    let change = self
                        .eval_change_for_recoverable_plan(
                            operation.change.as_ref(),
                            &recoverable_plans,
                            false,
                        )
                        .await?;
                    if matches!(change, DbCommandChange::Runtime { .. }) {
                        return Err(RuntimeError::Decode(
                            "db update many for recoverable-envelope behavior fields is not supported in P5".to_string(),
                        ));
                    }
                    let DbCommandChange::Wire(change) = change else {
                        unreachable!("runtime update-many rejected above");
                    };
                    Ok(DbCommand::UpdateMany(DbUpdateManyCommand {
                        type_name,
                        result_plan,
                        query: self.eval_query(operation.query.as_ref()).await?,
                        change,
                    }))
                } else {
                    let change = self
                        .eval_change_for_recoverable_plan(
                            operation.change.as_ref(),
                            &recoverable_plans,
                            self.full_object_requires_recoverable_runtime(&recoverable_plans),
                        )
                        .await?;
                    Ok(DbCommand::UpdateOne(DbUpdateOneCommand {
                        type_name,
                        result_plan,
                        selector: self
                            .eval_selector(operation.selector.as_ref(), operation.query.as_ref())
                            .await?,
                        change,
                    }))
                }
            }
            DbOpKindIr::Upsert => {
                if self.full_object_requires_recoverable_runtime(&recoverable_plans)
                    || self.change_requires_recoverable_runtime(
                        operation.change.as_ref(),
                        &recoverable_plans,
                    )?
                {
                    return Err(RuntimeError::Decode(
                        "db upsert for recoverable-envelope behavior fields is not supported in P5"
                            .to_string(),
                    ));
                }
                let Some(DbSelectorIr::Key { value }) = operation.selector.as_ref() else {
                    return Err(RuntimeError::Decode(
                        "db upsert requires a key selector".to_string(),
                    ));
                };
                let insert_body = operation
                    .insert_body
                    .as_ref()
                    .or(operation.body.as_ref())
                    .ok_or_else(|| {
                        RuntimeError::Decode("db upsert requires insert body".to_string())
                    })?;
                Ok(DbCommand::UpsertKey(DbUpsertKeyCommand {
                    type_name,
                    result_plan,
                    key: DbKey::new(self.eval_expr_wire(*value).await?),
                    insert: self.eval_body_object(insert_body).await?,
                    change: self.eval_change(operation.change.as_ref()).await?,
                }))
            }
            DbOpKindIr::Replace => {
                let body = operation.body.as_ref().ok_or_else(|| {
                    RuntimeError::Decode("db replace requires a body".to_string())
                })?;
                let value = if self.full_object_requires_recoverable_runtime(&recoverable_plans) {
                    DbCommandValue::Runtime {
                        value: self.eval_body_runtime_object(body).await?,
                        recoverable_runtime: recoverable_plans,
                    }
                } else {
                    DbCommandValue::Wire(self.eval_body_object(body).await?)
                };
                Ok(DbCommand::ReplaceOne(DbReplaceOneCommand {
                    type_name,
                    result_plan,
                    value,
                    selector: self
                        .eval_selector(operation.selector.as_ref(), operation.query.as_ref())
                        .await?,
                }))
            }
            DbOpKindIr::Delete => {
                if operation.many {
                    Ok(DbCommand::DeleteMany(DbQueryCommand {
                        type_name,
                        query: self.eval_query(operation.query.as_ref()).await?,
                    }))
                } else {
                    Ok(DbCommand::DeleteOne(DbDeleteOneCommand {
                        type_name,
                        selector: self
                            .eval_selector(operation.selector.as_ref(), operation.query.as_ref())
                            .await?,
                    }))
                }
            }
            DbOpKindIr::Count => Ok(DbCommand::Count(DbQueryCommand {
                type_name,
                query: self.eval_query(operation.query.as_ref()).await?,
            })),
            DbOpKindIr::Exists => match operation.selector.as_ref() {
                Some(DbSelectorIr::Key { value }) => Ok(DbCommand::ExistsKey(DbExistsKeyCommand {
                    type_name,
                    key: DbKey::new(self.eval_expr_wire(*value).await?),
                })),
                _ => Ok(DbCommand::ExistsQuery(DbQueryCommand {
                    type_name,
                    query: self
                        .eval_query(
                            operation
                                .selector
                                .as_ref()
                                .and_then(|selector| match selector {
                                    DbSelectorIr::Query { query } => Some(query),
                                    _ => None,
                                })
                                .or(operation.query.as_ref()),
                        )
                        .await?,
                })),
            },
        }
    }

    fn result_plan(&self, ty: &LinkedTypeRef) -> Result<RuntimeTypePlan> {
        self.interpreter
            .type_projection()?
            .plan_from_linked_nested_ref(ty, self.addr)
    }

    fn recoverable_expected_plans(
        &self,
        target: &DbTargetIr,
    ) -> Result<DbRecoverableRuntimeExpectedPlans> {
        let program = self.interpreter.program_projection()?;
        let ctx = PlanContext::from_type_view(program.type_view(), self.addr);
        db_recoverable_expected_plans_from_declaration(self.file, target, &ctx)
    }

    fn db_field_recoverable_expected_plan(
        field_ty: &LinkedTypeRef,
        ctx: &PlanContext,
    ) -> Result<RuntimeRecoverableExpectedTypePlan> {
        Ok(RuntimeRecoverableExpectedTypePlan::from_linked_ref(
            field_ty, ctx,
        )?)
    }

    fn db_object_recoverable_expected_plans(
        target: &DbTargetIr,
        ctx: &PlanContext,
    ) -> Result<DbRecoverableRuntimeExpectedPlans> {
        let expected = RuntimeRecoverableExpectedTypePlan::from_linked_ref(&target.type_ref, ctx)?;
        recoverable_expected_plans_from_record_node(expected.node)
    }

    fn db_declaration_recoverable_expected_plans(
        declaration: &skiff_runtime_linked_program::linked::DbDeclarationIr,
        ctx: &PlanContext,
    ) -> Result<DbRecoverableRuntimeExpectedPlans> {
        let mut plans = DbRecoverableRuntimeExpectedPlans::default();
        for field in &declaration.fields {
            plans.insert_field(
                field.name.clone(),
                Self::db_field_recoverable_expected_plan(&field.ty, ctx)?,
            );
        }
        Ok(plans)
    }

    fn projection_requires_recoverable_runtime(
        &self,
        recoverable_plans: &DbRecoverableRuntimeExpectedPlans,
        projection: Option<&DbProjectionIr>,
    ) -> bool {
        match projection {
            None => recoverable_plans
                .fields()
                .values()
                .any(expected_contains_any_interface),
            Some(projection) => projection.fields.iter().any(|field| {
                self.field_requires_recoverable_runtime(
                    &db_field_path_text(field),
                    recoverable_plans,
                )
            }),
        }
    }

    fn full_object_requires_recoverable_runtime(
        &self,
        recoverable_plans: &DbRecoverableRuntimeExpectedPlans,
    ) -> bool {
        recoverable_plans
            .fields()
            .values()
            .any(expected_contains_any_interface)
    }

    fn change_requires_recoverable_runtime(
        &self,
        change: Option<&DbChangeIr>,
        recoverable_plans: &DbRecoverableRuntimeExpectedPlans,
    ) -> Result<bool> {
        Ok(change.is_some_and(|change| {
            change.ops.iter().any(|op| {
                self.field_requires_recoverable_runtime(
                    &db_field_path_text(change_op_field(op)),
                    recoverable_plans,
                )
            })
        }))
    }

    fn field_requires_recoverable_runtime(
        &self,
        field: &str,
        recoverable_plans: &DbRecoverableRuntimeExpectedPlans,
    ) -> bool {
        recoverable_plan_for_field_path(recoverable_plans, field)
            .is_some_and(expected_contains_any_interface)
    }

    pub async fn eval_query_value(
        &mut self,
        target: &DbTargetIr,
        query: &DbQueryIr,
        projection: Option<&DbProjectionIr>,
    ) -> Result<RuntimeValue> {
        let filter = self.eval_query(Some(query)).await?;
        let limit = self.eval_query_option(query.limit).await?;
        let offset = self.eval_query_option(query.offset).await?;
        let after = self.eval_query_option(query.after).await?;
        let value = json!({
            "target": target,
            "typeName": target.type_name,
            "filter": filter.as_value(),
            "order": query.order,
            "limit": limit,
            "offset": offset,
            "after": after,
            "projection": projection,
        });
        runtime_from_wire(&value, self.heap)
    }

    async fn eval_query(&mut self, query: Option<&DbQueryIr>) -> Result<DbQuery> {
        let Some(query) = query else {
            return Ok(DbQuery::new(Value::Null));
        };
        self.eval_predicate_list(&query.where_)
            .await
            .map(Self::and_filter_value)
            .map(DbQuery::new)
    }

    async fn eval_query_option(&mut self, value: Option<ExprRefIr>) -> Result<Value> {
        let Some(value) = value else {
            return Ok(Value::Null);
        };
        self.eval_expr_wire(value).await
    }

    fn and_filter_value(mut filters: Vec<Value>) -> Value {
        match filters.len() {
            0 => Value::Null,
            1 => filters.remove(0),
            _ => json!({ "$and": filters }),
        }
    }

    #[async_recursion]
    async fn eval_predicate(&mut self, predicate: &DbPredicateIr) -> Result<Option<Value>> {
        match predicate {
            DbPredicateIr::Compare { field, op, value } => {
                let value = self.eval_expr_wire(*value).await?;
                Ok(Some(db_compare_query_value(field, *op, value)))
            }
            DbPredicateIr::Regex {
                field,
                pattern,
                options,
            } => {
                let pattern = self.eval_expr_wire(*pattern).await?;
                let options = match options {
                    Some(options) => Some(self.eval_expr_wire(*options).await?),
                    None => None,
                };
                Ok(Some(db_regex_query_value(field, pattern, options)))
            }
            DbPredicateIr::And { predicates } => {
                let filters = self.eval_predicate_list(predicates).await?;
                Ok(match filters.len() {
                    0 => None,
                    1 => filters.into_iter().next(),
                    _ => Some(json!({ "$and": filters })),
                })
            }
            DbPredicateIr::Or { predicates } => {
                let filters = self.eval_predicate_list(predicates).await?;
                Ok(match filters.len() {
                    0 => None,
                    1 => filters.into_iter().next(),
                    _ => Some(json!({ "$or": filters })),
                })
            }
            DbPredicateIr::Not { predicate } => {
                let Some(filter) = self.eval_predicate(predicate).await? else {
                    return Ok(None);
                };
                Ok(Some(json!({ "$nor": [filter] })))
            }
            DbPredicateIr::Conditional {
                condition,
                predicate,
            } => {
                let condition = self.eval_program_expr_ref(*condition).await?;
                if runtime_truthy(&condition, self.heap)? {
                    self.eval_predicate(predicate).await
                } else {
                    Ok(None)
                }
            }
        }
    }

    async fn eval_predicate_list(&mut self, predicates: &[DbPredicateIr]) -> Result<Vec<Value>> {
        let mut filters = Vec::with_capacity(predicates.len());
        for predicate in predicates {
            if let Some(filter) = self.eval_predicate(predicate).await? {
                filters.push(filter);
            }
        }
        Ok(filters)
    }

    async fn eval_body_object(&mut self, body: &DbBodyIr) -> Result<DbDocument> {
        match body {
            DbBodyIr::ObjectFields { fields } => {
                let mut object = serde_json::Map::new();
                for (field, value) in fields {
                    object.insert(field.clone(), self.eval_expr_wire(*value).await?);
                }
                Ok(DbDocument::new(Value::Object(object)))
            }
            DbBodyIr::Values { value } => self.eval_expr_wire(*value).await.map(DbDocument::new),
        }
    }

    async fn eval_body_values(&mut self, body: &DbBodyIr) -> Result<Vec<DbDocument>> {
        let value = self.eval_body_object(body).await?;
        value
            .as_value()
            .as_array()
            .cloned()
            .map(|items| items.into_iter().map(DbDocument::new).collect())
            .ok_or_else(|| {
                RuntimeError::Decode("db insert many values body must be an array".to_string())
            })
    }

    async fn eval_body_runtime_object(&mut self, body: &DbBodyIr) -> Result<RuntimeValue> {
        match body {
            DbBodyIr::ObjectFields { fields } => {
                let mut object = RuntimeObjectFields::new();
                for (field, value) in fields {
                    object.insert(field.clone(), self.eval_program_expr_ref(*value).await?);
                }
                Ok(RuntimeValue::Heap(
                    self.heap.alloc_object(RuntimeObject::unshaped(object))?,
                ))
            }
            DbBodyIr::Values { value } => self.eval_program_expr_ref(*value).await,
        }
    }

    async fn eval_change(&mut self, change: Option<&DbChangeIr>) -> Result<ServiceDbChange> {
        let mut update = ServiceDbChange::new();
        let Some(change) = change else {
            return Ok(update);
        };
        for op in &change.ops {
            match op {
                DbChangeOpIr::Set { field, value } => update.set(
                    db_field_path_text(field),
                    self.eval_expr_wire(*value).await?,
                ),
                DbChangeOpIr::Inc { field, value } => update.inc(
                    db_field_path_text(field),
                    self.eval_expr_wire(*value).await?,
                ),
                DbChangeOpIr::Unset { field } => update.unset(&db_field_path_text(field)),
                DbChangeOpIr::AddToSet { field, value } => update.add_to_set(
                    db_field_path_text(field),
                    self.eval_expr_wire(*value).await?,
                ),
                DbChangeOpIr::Remove { field, value } => update.pull(
                    db_field_path_text(field),
                    self.eval_expr_wire(*value).await?,
                ),
            }
        }
        Ok(update)
    }

    async fn eval_change_for_recoverable_plan(
        &mut self,
        change: Option<&DbChangeIr>,
        recoverable_plans: &DbRecoverableRuntimeExpectedPlans,
        force_runtime_result: bool,
    ) -> Result<DbCommandChange> {
        let Some(change) = change else {
            return Ok(DbCommandChange::Wire(ServiceDbChange::new()));
        };
        if !force_runtime_result
            && !self.change_requires_recoverable_runtime(Some(change), recoverable_plans)?
        {
            return self
                .eval_change(Some(change))
                .await
                .map(DbCommandChange::Wire);
        }

        let mut runtime_change = DbRuntimeChange::default();
        for op in &change.ops {
            match op {
                DbChangeOpIr::Set { field, value } => {
                    let field_name = db_field_path_text(field);
                    if !self.field_requires_recoverable_runtime(&field_name, recoverable_plans) {
                        runtime_change
                            .wire_change
                            .set(field_name, self.eval_expr_wire(*value).await?);
                        continue;
                    }
                    if !is_top_level_field_path(field) {
                        return Err(RuntimeError::Decode(format!(
                            "recoverable-envelope DB field {} is opaque; nested set on {} is not supported in P5; only full field read/write is supported",
                            top_level_field_path(field),
                            field_name
                        )));
                    }
                    runtime_change.set_ops.push(DbRuntimeSetOp {
                        field: field_name,
                        value: self.eval_program_expr_ref(*value).await?,
                    });
                }
                DbChangeOpIr::Inc { field, .. }
                | DbChangeOpIr::Unset { field }
                | DbChangeOpIr::AddToSet { field, .. }
                | DbChangeOpIr::Remove { field, .. }
                    if self.field_requires_recoverable_runtime(
                        &db_field_path_text(field),
                        recoverable_plans,
                    ) =>
                {
                    return Err(RuntimeError::Decode(format!(
                        "recoverable-envelope DB field {} is opaque; partial change on {} is not supported in P5; only full field read/write is supported",
                        top_level_field_path(field),
                        db_field_path_text(field)
                    )));
                }
                DbChangeOpIr::Inc { field, value } => runtime_change.wire_change.inc(
                    db_field_path_text(field),
                    self.eval_expr_wire(*value).await?,
                ),
                DbChangeOpIr::Unset { field } => {
                    runtime_change.wire_change.unset(db_field_path_text(field))
                }
                DbChangeOpIr::AddToSet { field, value } => runtime_change.wire_change.add_to_set(
                    db_field_path_text(field),
                    self.eval_expr_wire(*value).await?,
                ),
                DbChangeOpIr::Remove { field, value } => runtime_change.wire_change.pull(
                    db_field_path_text(field),
                    self.eval_expr_wire(*value).await?,
                ),
            }
        }
        Ok(DbCommandChange::Runtime {
            change: runtime_change,
            recoverable_runtime: recoverable_plans.clone(),
        })
    }

    async fn eval_selector(
        &mut self,
        selector: Option<&DbSelectorIr>,
        query: Option<&DbQueryIr>,
    ) -> Result<DbOneCommandSelector> {
        match selector {
            Some(DbSelectorIr::Key { value }) => Ok(DbOneCommandSelector::Key {
                key: DbKey::new(self.eval_expr_wire(*value).await?),
            }),
            Some(DbSelectorIr::Query { query }) => Ok(DbOneCommandSelector::Query {
                query: self.eval_query(Some(query)).await?,
                order: db_order_entries(&query.order),
            }),
            None => {
                let Some(query) = query else {
                    return Err(RuntimeError::Decode(
                        "db operation requires a selector or query".to_string(),
                    ));
                };
                Ok(DbOneCommandSelector::Query {
                    query: self.eval_query(Some(query)).await?,
                    order: db_order_entries(&query.order),
                })
            }
        }
    }

    async fn eval_page_options(
        &mut self,
        type_name: &str,
        query: &DbQuery,
        query_ir: Option<&DbQueryIr>,
        projection: Option<&Vec<FieldPath>>,
    ) -> Result<ServiceDbFindOptions> {
        let mut options = ServiceDbFindOptions::default();
        if let Some(query_ir) = query_ir {
            if query_ir.after.is_some() {
                return Err(RuntimeError::Decode(
                    "db find many after is not supported; use offset and limit".to_string(),
                ));
            }
            if !query_ir.order.is_empty() {
                options.order = db_order_entries(&query_ir.order);
            }
            if let Some(limit) = query_ir.limit {
                let value = self.eval_program_expr_ref(limit).await?;
                let number = runtime_numeric(&value)?;
                if !number.is_finite() || number < 0.0 || number.fract() != 0.0 {
                    return Err(RuntimeError::Decode(
                        "db find many limit must be a non-negative integer".to_string(),
                    ));
                }
                options.limit = Some(number as i64);
            }
            if let Some(offset) = query_ir.offset {
                let value = self.eval_program_expr_ref(offset).await?;
                let number = runtime_numeric(&value)?;
                if !number.is_finite() || number < 0.0 || number.fract() != 0.0 {
                    return Err(RuntimeError::Decode(
                        "db find many offset must be a non-negative integer".to_string(),
                    ));
                }
                options.offset = Some(number as u64);
            }
        }
        let _ = (type_name, query, projection);
        Ok(options)
    }

    async fn eval_expr_wire(&mut self, expr_ref: ExprRefIr) -> Result<Value> {
        let value = self.eval_program_expr_ref(expr_ref).await?;
        runtime_to_wire(&value, self.heap)
    }

    async fn eval_program_expr_ref(&mut self, expr_ref: ExprRefIr) -> Result<RuntimeValue> {
        self.interpreter
            .eval_program_expr_ref(
                self.program_context.clone(),
                self.heap,
                self.env,
                self.addr,
                self.file,
                self.executable,
                expr_ref,
            )
            .await
    }
}

fn db_projection_paths(projection: Option<&DbProjectionIr>) -> Option<Vec<FieldPath>> {
    projection.map(|projection| projection.fields.iter().map(db_field_path).collect())
}

fn db_order_entries(order: &[DbOrderIr]) -> Vec<DbOrderEntry> {
    order
        .iter()
        .map(|entry| DbOrderEntry {
            field: db_field_path(&entry.field),
            direction: match entry.direction {
                DbIndexDirectionIr::Asc => DbOrderDirection::Asc,
                DbIndexDirectionIr::Desc => DbOrderDirection::Desc,
            },
        })
        .collect()
}

fn db_compare_query_value(field: &FieldPathIr, op: DbPredicateCompareOpIr, value: Value) -> Value {
    let field_name = db_field_path_text(field);
    json!({
        field_name: {
            db_compare_operator(op): value,
        }
    })
}

fn db_regex_query_value(field: &FieldPathIr, pattern: Value, options: Option<Value>) -> Value {
    let field_name = db_field_path_text(field);
    let mut regex = serde_json::Map::new();
    regex.insert("$regex".to_string(), pattern);
    if let Some(options) = options {
        regex.insert("$options".to_string(), options);
    }
    json!({ field_name: Value::Object(regex) })
}

fn db_compare_operator(op: DbPredicateCompareOpIr) -> &'static str {
    match op {
        DbPredicateCompareOpIr::Eq => "$eq",
        DbPredicateCompareOpIr::Ne => "$ne",
        DbPredicateCompareOpIr::Lt => "$lt",
        DbPredicateCompareOpIr::Lte => "$lte",
        DbPredicateCompareOpIr::Gt => "$gt",
        DbPredicateCompareOpIr::Gte => "$gte",
    }
}

fn db_recoverable_expected_plans_from_declaration(
    file: &LinkedFileUnit,
    target: &DbTargetIr,
    ctx: &PlanContext,
) -> Result<DbRecoverableRuntimeExpectedPlans> {
    if let Some(declaration) = file.declarations.db.get(&target.type_name) {
        return DbIrEvaluator::db_declaration_recoverable_expected_plans(declaration, ctx);
    }
    if let LinkedTypeRef::DbObjectSymbol { symbol } = &target.type_ref {
        if let Some(plans) = db_symbol_recoverable_expected_plans(symbol, ctx)? {
            return Ok(plans);
        }
    }
    DbIrEvaluator::db_object_recoverable_expected_plans(target, ctx)
}

fn db_symbol_recoverable_expected_plans(
    symbol: &skiff_runtime_linked_program::ServiceSymbolRef,
    ctx: &PlanContext,
) -> Result<Option<DbRecoverableRuntimeExpectedPlans>> {
    match &ctx.current_addr.unit {
        skiff_runtime_linked_program::UnitAddr::Service => {
            if let Some(plans) = db_symbol_recoverable_expected_plans_in_files(
                ctx.program.service_files,
                symbol,
                ctx,
            )? {
                return Ok(Some(plans));
            }
        }
        skiff_runtime_linked_program::UnitAddr::Package(slot) => {
            if let Some(files) = ctx.program.package_files.get(*slot) {
                if let Some(plans) =
                    db_symbol_recoverable_expected_plans_in_files(files, symbol, ctx)?
                {
                    return Ok(Some(plans));
                }
            }
        }
    }

    if let Some(plans) =
        db_symbol_recoverable_expected_plans_in_files(ctx.program.service_files, symbol, ctx)?
    {
        return Ok(Some(plans));
    }
    for files in ctx.program.package_files {
        if let Some(plans) = db_symbol_recoverable_expected_plans_in_files(files, symbol, ctx)? {
            return Ok(Some(plans));
        }
    }
    Ok(None)
}

fn db_symbol_recoverable_expected_plans_in_files(
    files: &[std::sync::Arc<LinkedFileUnit>],
    symbol: &skiff_runtime_linked_program::ServiceSymbolRef,
    ctx: &PlanContext,
) -> Result<Option<DbRecoverableRuntimeExpectedPlans>> {
    for file in files {
        if file.module_path != symbol.module_path {
            continue;
        }
        let qualified_symbol = format!("{}.{}", symbol.module_path, symbol.symbol);
        if let Some(declaration) = file.declarations.db.get(&symbol.symbol).or_else(|| {
            file.declarations
                .db
                .values()
                .find(|db| db.type_name == symbol.symbol || db.type_name == qualified_symbol)
        }) {
            return DbIrEvaluator::db_declaration_recoverable_expected_plans(declaration, ctx)
                .map(Some);
        }
    }
    Ok(None)
}

fn recoverable_expected_plans_from_record_node(
    node: RuntimeRecoverableExpectedTypeNode,
) -> Result<DbRecoverableRuntimeExpectedPlans> {
    let mut plans = DbRecoverableRuntimeExpectedPlans::default();
    if let RuntimeRecoverableExpectedTypeNode::Record { fields, .. } = node {
        for field in fields {
            plans.insert_field(field.name, field.ty);
        }
    }
    Ok(plans)
}

fn recoverable_plan_for_field_path<'a>(
    plans: &'a DbRecoverableRuntimeExpectedPlans,
    field: &str,
) -> Option<&'a RuntimeRecoverableExpectedTypePlan> {
    plans.field(field).or_else(|| {
        let top_level = field.split('.').next()?;
        if top_level == field {
            None
        } else {
            plans.field(top_level)
        }
    })
}

fn db_field_path(path: &FieldPathIr) -> FieldPath {
    FieldPath {
        text: db_field_path_text(path),
        segments: path.segments.clone(),
    }
}

fn db_field_path_text(path: &FieldPathIr) -> String {
    if path.text.trim().is_empty() {
        path.segments.join(".")
    } else {
        path.text.clone()
    }
}

fn expected_contains_any_interface(expected: &RuntimeRecoverableExpectedTypePlan) -> bool {
    match &expected.node {
        RuntimeRecoverableExpectedTypeNode::AnyInterface { .. } => true,
        RuntimeRecoverableExpectedTypeNode::Record { fields, .. } => fields
            .iter()
            .any(|field| expected_contains_any_interface(&field.ty)),
        RuntimeRecoverableExpectedTypeNode::Union { items } => {
            items.iter().any(expected_contains_any_interface)
        }
        RuntimeRecoverableExpectedTypeNode::Alias { target: inner }
        | RuntimeRecoverableExpectedTypeNode::Nullable { inner }
        | RuntimeRecoverableExpectedTypeNode::Array { item: inner }
        | RuntimeRecoverableExpectedTypeNode::Stream { item: inner } => {
            expected_contains_any_interface(inner)
        }
        RuntimeRecoverableExpectedTypeNode::Representation { payload, .. } => {
            expected_contains_any_interface(payload)
        }
        RuntimeRecoverableExpectedTypeNode::Map { key, value } => {
            expected_contains_any_interface(key) || expected_contains_any_interface(value)
        }
        RuntimeRecoverableExpectedTypeNode::LiteralString { .. }
        | RuntimeRecoverableExpectedTypeNode::Json
        | RuntimeRecoverableExpectedTypeNode::JsonObject
        | RuntimeRecoverableExpectedTypeNode::Bytes
        | RuntimeRecoverableExpectedTypeNode::Date
        | RuntimeRecoverableExpectedTypeNode::String
        | RuntimeRecoverableExpectedTypeNode::Bool
        | RuntimeRecoverableExpectedTypeNode::Number
        | RuntimeRecoverableExpectedTypeNode::Integer
        | RuntimeRecoverableExpectedTypeNode::Null
        | RuntimeRecoverableExpectedTypeNode::Unresolved { .. } => false,
    }
}

fn change_op_field(op: &DbChangeOpIr) -> &FieldPathIr {
    match op {
        DbChangeOpIr::Set { field, .. }
        | DbChangeOpIr::Inc { field, .. }
        | DbChangeOpIr::Unset { field }
        | DbChangeOpIr::AddToSet { field, .. }
        | DbChangeOpIr::Remove { field, .. } => field,
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use skiff_runtime_linked_program::{
        linked::{DbDeclarationIr, DbObjectFieldIr, DbObjectKeyIr, DbObjectKindIr},
        types::anonymous_type_decl,
        ExecutableAddr, FileAddr, FileDeclarations, FileLinkTargets, LinkedFileUnit,
        LinkedInterfaceInstantiationRef, LinkedTypeDescriptor, LinkedTypeRef, PackageUnit,
        RuntimeTypeContext, ServiceSymbolKey, ServiceSymbolRef, TypeAddr, UnitAddr,
    };
    use skiff_runtime_linked_type_plan::{PlanContext, ProgramTypeView};

    use super::*;

    #[test]
    fn package_db_object_symbol_declaration_in_another_file_generates_runtime_plans() {
        let mut types = RuntimeTypeContext::default();
        register_exported_record_with_any_interface(
            &mut types,
            "tools",
            "AgentRuntimeBindings",
            0,
            BTreeMap::from([(
                "events".to_string(),
                any_interface("events.AgentEventReceiver"),
            )]),
        );

        let service_files: Vec<Arc<LinkedFileUnit>> = Vec::new();
        let packages: Vec<Arc<PackageUnit>> = Vec::new();
        let package_files = vec![vec![
            Arc::new(model_file_with_db_field(
                "AgentRun",
                "runtimeBindings",
                service_symbol_type("tools", "AgentRuntimeBindings"),
            )),
            Arc::new(empty_file("runner")),
        ]];
        let link_overlay = Default::default();
        let current_addr = ExecutableAddr::package(0, 1, 0);
        let ctx = PlanContext::from_type_view(
            ProgramTypeView::new(
                &service_files,
                &packages,
                &package_files,
                &link_overlay,
                &types,
            ),
            &current_addr,
        );
        let target = db_object_target("model", "AgentRun");

        let plans = db_recoverable_expected_plans_from_declaration(
            package_files[0][1].as_ref(),
            &target,
            &ctx,
        )
        .expect("package DB declaration field plans");

        assert!(expected_contains_any_interface(
            plans
                .field("runtimeBindings")
                .expect("runtimeBindings plan")
        ));
        assert!(plans.fields().values().any(expected_contains_any_interface));
    }

    #[test]
    fn service_db_object_symbol_declaration_in_another_file_generates_runtime_plans() {
        let mut types = RuntimeTypeContext::default();
        register_exported_record_with_any_interface(
            &mut types,
            "tools",
            "AgentRuntimeBindings",
            0,
            BTreeMap::from([(
                "events".to_string(),
                any_interface("events.AgentEventReceiver"),
            )]),
        );

        let service_files = vec![
            Arc::new(model_file_with_db_field(
                "AgentRun",
                "runtimeBindings",
                service_symbol_type("tools", "AgentRuntimeBindings"),
            )),
            Arc::new(empty_file("thread")),
        ];
        let packages: Vec<Arc<PackageUnit>> = Vec::new();
        let package_files: Vec<Vec<Arc<LinkedFileUnit>>> = Vec::new();
        let link_overlay = Default::default();
        let current_addr = ExecutableAddr::service(1, 0);
        let ctx = PlanContext::from_type_view(
            ProgramTypeView::new(
                &service_files,
                &packages,
                &package_files,
                &link_overlay,
                &types,
            ),
            &current_addr,
        );
        let target = db_object_target("model", "AgentRun");

        let plans = db_recoverable_expected_plans_from_declaration(
            service_files[1].as_ref(),
            &target,
            &ctx,
        )
        .expect("service DB declaration field plans");

        assert!(expected_contains_any_interface(
            plans
                .field("runtimeBindings")
                .expect("runtimeBindings plan")
        ));
    }

    #[test]
    fn nested_path_on_recoverable_top_level_field_uses_top_level_plan() {
        let mut types = RuntimeTypeContext::default();
        register_exported_record_with_any_interface(
            &mut types,
            "tools",
            "AgentThreadConfig",
            0,
            BTreeMap::from([(
                "runtimeBindings".to_string(),
                service_symbol_type("tools", "AgentRuntimeBindings"),
            )]),
        );
        register_exported_record_with_any_interface(
            &mut types,
            "tools",
            "AgentRuntimeBindings",
            1,
            BTreeMap::from([(
                "events".to_string(),
                any_interface("events.AgentEventReceiver"),
            )]),
        );

        let service_files = vec![
            Arc::new(model_file_with_db_field(
                "AgentThread",
                "currentConfig",
                service_symbol_type("tools", "AgentThreadConfig"),
            )),
            Arc::new(empty_file("runner")),
        ];
        let packages: Vec<Arc<PackageUnit>> = Vec::new();
        let package_files: Vec<Vec<Arc<LinkedFileUnit>>> = Vec::new();
        let link_overlay = Default::default();
        let current_addr = ExecutableAddr::service(1, 0);
        let ctx = PlanContext::from_type_view(
            ProgramTypeView::new(
                &service_files,
                &packages,
                &package_files,
                &link_overlay,
                &types,
            ),
            &current_addr,
        );
        let target = db_object_target("model", "AgentThread");

        let plans = db_recoverable_expected_plans_from_declaration(
            service_files[1].as_ref(),
            &target,
            &ctx,
        )
        .expect("nested recoverable field plans");

        assert!(
            recoverable_plan_for_field_path(&plans, "currentConfig.runtimeBindings")
                .is_some_and(expected_contains_any_interface)
        );
    }

    #[test]
    fn plain_cross_file_db_object_symbol_does_not_require_runtime_plan() {
        let types = RuntimeTypeContext::default();
        let service_files = vec![
            Arc::new(model_file_with_db_field("AgentRun", "title", string_type())),
            Arc::new(empty_file("runner")),
        ];
        let packages: Vec<Arc<PackageUnit>> = Vec::new();
        let package_files: Vec<Vec<Arc<LinkedFileUnit>>> = Vec::new();
        let link_overlay = Default::default();
        let current_addr = ExecutableAddr::service(1, 0);
        let ctx = PlanContext::from_type_view(
            ProgramTypeView::new(
                &service_files,
                &packages,
                &package_files,
                &link_overlay,
                &types,
            ),
            &current_addr,
        );
        let target = db_object_target("model", "AgentRun");

        let plans = db_recoverable_expected_plans_from_declaration(
            service_files[1].as_ref(),
            &target,
            &ctx,
        )
        .expect("plain DB declaration field plans");

        assert!(!plans.fields().values().any(expected_contains_any_interface));
    }

    fn register_exported_record_with_any_interface(
        types: &mut RuntimeTypeContext,
        module_path: &str,
        symbol: &str,
        type_index: usize,
        fields: BTreeMap<String, LinkedTypeRef>,
    ) {
        let addr = TypeAddr {
            unit: UnitAddr::Service,
            file: FileAddr::LoadedFileIndex(0),
            type_index,
        };
        types.descriptors.insert(
            addr.clone(),
            anonymous_type_decl(symbol, LinkedTypeDescriptor::Record { fields }),
        );
        types
            .exported_types
            .insert_service(ServiceSymbolKey::new(module_path, symbol), addr);
    }

    fn model_file_with_db_field(
        db_symbol: &str,
        field_name: &str,
        field_ty: LinkedTypeRef,
    ) -> LinkedFileUnit {
        let mut file = empty_file("model");
        file.declarations.db.insert(
            db_symbol.to_string(),
            DbDeclarationIr {
                type_ref: db_object_type("model", db_symbol),
                type_name: format!("model.{db_symbol}"),
                collection_name: db_symbol.to_string(),
                kind: DbObjectKindIr::Object,
                key: DbObjectKeyIr {
                    name: "id".to_string(),
                    ty: string_type(),
                },
                fields: vec![DbObjectFieldIr {
                    name: field_name.to_string(),
                    ty: field_ty,
                }],
                leases: Vec::new(),
                indexes: Vec::new(),
                source_span: None,
            },
        );
        file
    }

    fn empty_file(module_path: &str) -> LinkedFileUnit {
        LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: format!("file:{module_path}"),
            source_ast_hash: format!("source:{module_path}"),
            module_path: module_path.to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: Default::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            types: Vec::new(),
            constants: Vec::new(),
            executables: Vec::new(),
            external_refs: Default::default(),
        }
    }

    fn db_object_target(module_path: &str, symbol: &str) -> DbTargetIr {
        DbTargetIr {
            type_ref: db_object_type(module_path, symbol),
            type_name: format!("{module_path}.{symbol}"),
        }
    }

    fn db_object_type(module_path: &str, symbol: &str) -> LinkedTypeRef {
        LinkedTypeRef::DbObjectSymbol {
            symbol: service_symbol(module_path, symbol),
        }
    }

    fn service_symbol_type(module_path: &str, symbol: &str) -> LinkedTypeRef {
        LinkedTypeRef::ServiceSymbol {
            symbol: service_symbol(module_path, symbol),
        }
    }

    fn service_symbol(module_path: &str, symbol: &str) -> ServiceSymbolRef {
        ServiceSymbolRef {
            module_path: module_path.to_string(),
            symbol: symbol.to_string(),
        }
    }

    fn any_interface(interface_abi_id: &str) -> LinkedTypeRef {
        LinkedTypeRef::AnyInterface {
            interface: LinkedInterfaceInstantiationRef {
                interface_abi_id: interface_abi_id.to_string(),
                canonical_type_args: Vec::new(),
            },
        }
    }

    fn string_type() -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: "String".to_string(),
            args: Vec::new(),
        }
    }
}

fn top_level_field_path(path: &FieldPathIr) -> String {
    let text = db_field_path_text(path);
    text.split('.').next().unwrap_or(text.as_str()).to_string()
}

fn is_top_level_field_path(path: &FieldPathIr) -> bool {
    let text = db_field_path_text(path);
    !text.contains('.')
}
