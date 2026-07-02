use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use skiff_runtime_boundary::plan::BoundaryUse;
use skiff_runtime_capability_context::{DbKey, DbOneSelector};
use skiff_runtime_model::{
    recoverable::{
        RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
        RuntimeRecoverableServiceRef, RuntimeRecoverableStorageLane,
        RuntimeRecoverableTrustBoundary,
    },
    request_heap::RequestHeap,
    runtime_value::RuntimeValue,
    type_plan::RuntimeTypePlan,
};

use super::{
    capabilities::{
        DbCapabilityContext, DbCapabilityStore, DbRecoverableRuntimeContext,
        DbRecoverableRuntimeExpectedPlans,
    },
    db_command::{DbCommand, DbCommandChange, DbCommandValue, DbOneCommandSelector},
    db_eval::DbIrEvaluator,
    env::{Env, Flow},
    invocation::EvalProgramProjection,
    program_execution::ProgramExecutionContext,
    recoverable_behavior::EvalRecoverableBehaviorHooks,
    runtime_ops::{runtime_from_wire, runtime_from_wire_required_plan_with_use, runtime_to_wire},
    Interpreter,
};
use crate::error::{Result, RuntimeError};
use skiff_runtime_linked_program::{
    CallIr, DbLeaseClaimIr, DbLeaseReadIr, DbOperationIr, DbProjectionIr, DbQueryIr, DbTargetIr,
    DbTransactionIr, DbTransactionModeIr, ExecutableAddr, LinkedCallTarget, LinkedExecutable,
    LinkedFileUnit,
};
use skiff_runtime_native_contract::native_target_name;

const SERVICE_DB_UNCONFIGURED_REASON: &str =
    "serviceDb is not configured for this service activation";

pub fn program_call_db_op(target: &LinkedCallTarget) -> Option<String> {
    match target {
        LinkedCallTarget::Builtin { op } if is_db_builtin_op(op) => Some(op.clone()),
        LinkedCallTarget::Native { target } => {
            let op = native_target_name(target);
            is_db_builtin_op(&op).then_some(op)
        }
        _ => None,
    }
}

pub fn is_db_builtin_op(op: &str) -> bool {
    matches!(
        op,
        "db.get"
            | "db.require"
            | "db.exists"
            | "db.upsert"
            | "db.create"
            | "db.createMany"
            | "db.create_many"
            | "db.append"
            | "db.appendMany"
            | "db.append_many"
            | "db.findMany"
            | "db.find_many"
            | "db.count"
            | "db.transaction"
    )
}

impl Interpreter {
    #[allow(clippy::too_many_arguments)]
    pub async fn eval_program_db_operation(
        &self,
        program_context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        operation: &DbOperationIr,
    ) -> Result<RuntimeValue> {
        let db_context = program_context.db_context();
        self.eval_program_db_operation_with_context(
            program_context,
            &db_context,
            heap,
            env,
            addr,
            file,
            executable,
            operation,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn eval_program_db_operation_with_context(
        &self,
        program_context: ProgramExecutionContext<'_>,
        db_context: &DbCapabilityContext,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        operation: &DbOperationIr,
    ) -> Result<RuntimeValue> {
        let store = require_db_store(db_context, "db operation")?;
        let command = {
            let mut evaluator = DbIrEvaluator::new(
                self,
                program_context.clone(),
                heap,
                env,
                addr,
                file,
                executable,
            );
            evaluator.eval_operation(operation).await?
        };
        execute_db_command(
            &store,
            self.program_projection()?,
            &program_context,
            heap,
            command,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn eval_program_db_transaction(
        &self,
        db_context: &DbCapabilityContext,
        program_context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        call: &CallIr,
    ) -> Result<RuntimeValue> {
        let store = require_db_store(db_context, "db.transaction")?;
        let body = *call.args.first().ok_or_else(|| {
            RuntimeError::Decode("db.transaction requires a body expression argument".to_string())
        })?;
        if call.args.len() != 1 {
            return Err(RuntimeError::Decode(
                "db.transaction requires exactly one body expression argument".to_string(),
            ));
        }

        store.begin_transaction().await?;
        let checkpoint = heap.checkpoint();
        let result = self
            .eval_program_expr_ref(program_context, heap, env, addr, file, executable, body)
            .await;
        match result {
            Ok(value) => {
                if let Err(error) = store.commit_transaction().await {
                    store.abort_transaction().await;
                    heap.rollback_to_checkpoint(checkpoint);
                    return Err(error.into());
                }
                Ok(value)
            }
            Err(error) => {
                store.abort_transaction().await;
                heap.rollback_to_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn eval_program_explicit_db_transaction(
        &self,
        program_context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        transaction: &DbTransactionIr,
    ) -> Result<RuntimeValue> {
        let db_context = program_context.db_context();
        self.eval_program_explicit_db_transaction_with_context(
            program_context,
            &db_context,
            heap,
            env,
            addr,
            file,
            executable,
            transaction,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn eval_program_explicit_db_transaction_with_context(
        &self,
        program_context: ProgramExecutionContext<'_>,
        db_context: &DbCapabilityContext,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        transaction: &DbTransactionIr,
    ) -> Result<RuntimeValue> {
        let store = require_db_store(db_context, "db.transaction")?;
        store.begin_transaction().await?;
        let checkpoint = heap.checkpoint();
        let flow = self
            .exec_program_block(
                program_context.clone(),
                heap,
                env,
                addr,
                file,
                executable,
                &transaction.body,
            )
            .await;
        match flow {
            Ok(Flow::Continue) => {
                let result = match transaction.mode {
                    DbTransactionModeIr::Effect => Ok(RuntimeValue::Null),
                    DbTransactionModeIr::Value => match transaction.result {
                        Some(result) => {
                            self.eval_program_expr_ref(
                                program_context,
                                heap,
                                env,
                                addr,
                                file,
                                executable,
                                result,
                            )
                            .await
                        }
                        None => Err(RuntimeError::Decode(
                            "db transaction value requires a result expression".to_string(),
                        )),
                    },
                };
                let result = match result {
                    Ok(result) => result,
                    Err(error) => {
                        store.abort_transaction().await;
                        heap.rollback_to_checkpoint(checkpoint);
                        return Err(error);
                    }
                };
                if let Err(error) = store.commit_transaction().await {
                    store.abort_transaction().await;
                    heap.rollback_to_checkpoint(checkpoint);
                    return Err(error.into());
                }
                Ok(result)
            }
            Ok(Flow::Return(_)) => {
                store.abort_transaction().await;
                heap.rollback_to_checkpoint(checkpoint);
                Err(RuntimeError::Decode(
                    "return is not allowed inside db transaction blocks".to_string(),
                ))
            }
            Ok(Flow::Parked | Flow::ContinueConsumer) => {
                store.abort_transaction().await;
                heap.rollback_to_checkpoint(checkpoint);
                Err(RuntimeError::Decode(
                    "control flow is not allowed inside db transaction blocks".to_string(),
                ))
            }
            Ok(Flow::Break | Flow::LoopContinue) => {
                store.abort_transaction().await;
                heap.rollback_to_checkpoint(checkpoint);
                Err(RuntimeError::Decode(
                    "db transaction exited with break/continue outside a loop".to_string(),
                ))
            }
            Err(error) => {
                store.abort_transaction().await;
                heap.rollback_to_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn eval_program_db_query_value(
        &self,
        program_context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        target: &DbTargetIr,
        query: &DbQueryIr,
        projection: Option<&DbProjectionIr>,
    ) -> Result<RuntimeValue> {
        let mut evaluator =
            DbIrEvaluator::new(self, program_context, heap, env, addr, file, executable);
        evaluator.eval_query_value(target, query, projection).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn eval_program_db_lease_claim(
        &self,
        program_context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        claim: &DbLeaseClaimIr,
    ) -> Result<RuntimeValue> {
        let store = require_db_store(&program_context.db_context(), "db claim")?;
        let key = self
            .eval_program_expr_ref(
                program_context.clone(),
                heap,
                env,
                addr,
                file,
                executable,
                claim.key,
            )
            .await?;
        let key = DbKey::new(runtime_to_wire(&key, heap)?);
        let Some(handle) = store
            .claim_lease(&claim.target.type_name, key, &claim.slot)
            .await?
        else {
            return Ok(RuntimeValue::Bool(false));
        };

        if let Some(binding_slot) = claim.binding_slot {
            let value = runtime_from_wire(handle.value.as_value(), heap)?;
            env.declare_binding("db lease binding", Some(binding_slot as usize), value)?;
        }

        let renew_store = store.clone();
        let renew_hold = handle.hold.clone();
        let request_cancelled = program_context.execution().cancel_flag();
        let renew_period = Duration::from_millis((handle.ttl_ms / 3).max(1));
        let renew_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(renew_period);
            loop {
                interval.tick().await;
                if !handle_lease_renew_result(
                    renew_store.renew_lease(&renew_hold).await,
                    request_cancelled.as_ref(),
                ) {
                    break;
                }
            }
        });

        let flow = self
            .exec_program_block(
                program_context,
                heap,
                env,
                addr,
                file,
                executable,
                &claim.body,
            )
            .await;
        renew_task.abort();
        let lease_lost = store.lease_lost().await;
        let release = store.release_lease(&handle.hold).await;
        if lease_lost {
            return Err(RuntimeError::LeaseLost(
                "db lease was lost while executing claim body".to_string(),
            ));
        }
        release?;
        match flow {
            Ok(Flow::Continue) => Ok(RuntimeValue::Bool(true)),
            Ok(Flow::Return(_)) => Err(RuntimeError::Decode(
                "return is not allowed inside db claim blocks".to_string(),
            )),
            Ok(Flow::Parked | Flow::ContinueConsumer) => Err(RuntimeError::Decode(
                "control flow is not allowed inside db claim blocks".to_string(),
            )),
            Ok(Flow::Break | Flow::LoopContinue) => Err(RuntimeError::Decode(
                "db claim exited with break/continue outside a loop".to_string(),
            )),
            Err(error) => Err(error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn eval_program_db_lease_read(
        &self,
        program_context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        read: &DbLeaseReadIr,
    ) -> Result<RuntimeValue> {
        let store = require_db_store(&program_context.db_context(), "db lease")?;
        let key = self
            .eval_program_expr_ref(program_context, heap, env, addr, file, executable, read.key)
            .await?;
        let key = DbKey::new(runtime_to_wire(&key, heap)?);
        match store
            .read_lease(&read.target.type_name, key, &read.slot)
            .await?
        {
            Some(value) => runtime_from_wire(&value, heap),
            None => Ok(RuntimeValue::Null),
        }
    }
}

fn require_db_store(db_context: &DbCapabilityContext, target: &str) -> Result<DbCapabilityStore> {
    Ok(db_context.require_store(target, SERVICE_DB_UNCONFIGURED_REASON)?)
}

fn handle_lease_renew_result<E>(
    result: std::result::Result<bool, E>,
    request_cancelled: &std::sync::atomic::AtomicBool,
) -> bool {
    match result {
        Ok(true) => true,
        Ok(false) | Err(_) => {
            request_cancelled.store(true, Ordering::SeqCst);
            false
        }
    }
}

#[cfg(all(test, any()))]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::error::RuntimeError;

    use super::handle_lease_renew_result;

    #[test]
    fn lease_renew_success_keeps_request_running() {
        let request_cancelled = AtomicBool::new(false);

        assert!(handle_lease_renew_result(
            Ok::<bool, RuntimeError>(true),
            &request_cancelled
        ));
        assert!(!request_cancelled.load(Ordering::SeqCst));
    }

    #[test]
    fn lease_renew_rejected_cancels_request() {
        let request_cancelled = AtomicBool::new(false);

        assert!(!handle_lease_renew_result(
            Ok::<bool, RuntimeError>(false),
            &request_cancelled
        ));
        assert!(request_cancelled.load(Ordering::SeqCst));
    }

    #[test]
    fn lease_renew_error_cancels_request() {
        let request_cancelled = AtomicBool::new(false);

        assert!(!handle_lease_renew_result(
            Err(RuntimeError::Decode("renew failed".to_string())),
            &request_cancelled,
        ));
        assert!(request_cancelled.load(Ordering::SeqCst));
    }
}

async fn execute_db_command(
    store: &DbCapabilityStore,
    program: EvalProgramProjection<'_>,
    program_context: &ProgramExecutionContext<'_>,
    heap: &mut RequestHeap,
    command: DbCommand,
) -> Result<RuntimeValue> {
    match command {
        DbCommand::FindMany(command) => {
            if let Some(recoverable_runtime) = command.recoverable_runtime {
                let context =
                    db_recoverable_runtime_context(program, program_context, recoverable_runtime)?;
                let values = store
                    .find_many_page_runtime(
                        &command.type_name,
                        command.query,
                        command.options,
                        command.projection,
                        heap,
                        context,
                    )
                    .await?;
                return Ok(RuntimeValue::Heap(heap.alloc_array(values)?));
            }
            let page = store
                .find_many_page(
                    &command.type_name,
                    command.query,
                    command.options,
                    command.projection,
                )
                .await?;
            decode_db_result(
                &serde_json::Value::Array(
                    page.values
                        .into_iter()
                        .map(|value| value.into_value())
                        .collect(),
                ),
                &command.result_plan,
                "db find many result",
                heap,
            )
        }
        DbCommand::FindOne(command) => {
            let type_name = command.type_name;
            let projection = command.projection;
            if let Some(recoverable_runtime) = command.recoverable_runtime {
                let context =
                    db_recoverable_runtime_context(program, program_context, recoverable_runtime)?;
                let found = match command.selector {
                    DbOneCommandSelector::Key { key } => {
                        store
                            .find_one_by_key_runtime(&type_name, key, projection, heap, context)
                            .await?
                    }
                    DbOneCommandSelector::Query { query, order } => {
                        store
                            .find_one_by_query_runtime(
                                &type_name, query, order, projection, heap, context,
                            )
                            .await?
                    }
                };
                return match found {
                    Some(value) => Ok(value),
                    None if command.required => Err(RuntimeError::Decode(format!(
                        "db require could not find {type_name}"
                    ))),
                    None => Ok(RuntimeValue::Null),
                };
            }
            let found = match command.selector {
                DbOneCommandSelector::Key { key } => {
                    store.find_one_by_key(&type_name, key, projection).await?
                }
                DbOneCommandSelector::Query { query, order } => {
                    store
                        .find_one_by_query(&type_name, query, order, projection)
                        .await?
                }
            };
            match found {
                Some(value) => decode_db_result(
                    value.as_value(),
                    &command.result_plan,
                    "db find one result",
                    heap,
                ),
                None if command.required => Err(RuntimeError::Decode(format!(
                    "db require could not find {type_name}"
                ))),
                None => Ok(RuntimeValue::Null),
            }
        }
        DbCommand::InsertOne(command) => match command.value {
            DbCommandValue::Wire(value) => {
                let result = store.create(&command.type_name, value).await?;
                decode_db_result(
                    result.as_value(),
                    &command.result_plan,
                    "db insert one result",
                    heap,
                )
            }
            DbCommandValue::Runtime {
                value,
                recoverable_runtime,
            } => {
                let context =
                    db_recoverable_runtime_context(program, program_context, recoverable_runtime)?;
                Ok(store
                    .create_runtime(&command.type_name, &value, heap, context)
                    .await?)
            }
        },
        DbCommand::InsertMany(command) => {
            let result = store
                .insert_many_result(&command.type_name, command.values)
                .await?;
            decode_db_result(
                result.as_value(),
                &command.result_plan,
                "db insert many result",
                heap,
            )
        }
        DbCommand::UpdateOne(command) => match command.change {
            DbCommandChange::Wire(change) => {
                let result = store
                    .update_one(
                        &command.type_name,
                        service_db_selector(command.selector),
                        change,
                    )
                    .await?;
                result
                    .map(|value| {
                        decode_db_result(
                            value.as_value(),
                            &command.result_plan,
                            "db update one result",
                            heap,
                        )
                    })
                    .transpose()
                    .map(|value| value.unwrap_or(RuntimeValue::Null))
            }
            DbCommandChange::Runtime {
                change,
                recoverable_runtime,
            } => {
                let context =
                    db_recoverable_runtime_context(program, program_context, recoverable_runtime)?;
                Ok(store
                    .update_one_runtime(
                        &command.type_name,
                        service_db_selector(command.selector),
                        change,
                        heap,
                        context,
                    )
                    .await
                    .map(|value| value.unwrap_or(RuntimeValue::Null))?)
            }
        },
        DbCommand::UpdateMany(command) => {
            let result = store
                .update_many(&command.type_name, command.query, command.change)
                .await?;
            decode_db_result(
                result.as_value(),
                &command.result_plan,
                "db update many result",
                heap,
            )
        }
        DbCommand::UpsertKey(command) => {
            let result = store
                .upsert_by_key(
                    &command.type_name,
                    command.key,
                    command.insert,
                    command.change,
                )
                .await?;
            decode_db_result(
                result.as_value(),
                &command.result_plan,
                "db upsert result",
                heap,
            )
        }
        DbCommand::ReplaceOne(command) => match command.value {
            DbCommandValue::Wire(value) => {
                let result = store
                    .replace_one(
                        &command.type_name,
                        service_db_selector(command.selector),
                        value,
                    )
                    .await?;
                result
                    .map(|value| {
                        decode_db_result(
                            value.as_value(),
                            &command.result_plan,
                            "db replace one result",
                            heap,
                        )
                    })
                    .transpose()
                    .map(|value| value.unwrap_or(RuntimeValue::Null))
            }
            DbCommandValue::Runtime {
                value,
                recoverable_runtime,
            } => {
                let context =
                    db_recoverable_runtime_context(program, program_context, recoverable_runtime)?;
                Ok(store
                    .replace_one_runtime(
                        &command.type_name,
                        service_db_selector(command.selector),
                        &value,
                        heap,
                        context,
                    )
                    .await
                    .map(|value| value.unwrap_or(RuntimeValue::Null))?)
            }
        },
        DbCommand::DeleteOne(command) => Ok(RuntimeValue::Bool(
            store
                .delete_one(&command.type_name, service_db_selector(command.selector))
                .await?,
        )),
        DbCommand::DeleteMany(command) => {
            let result = store.delete_many(&command.type_name, command.query).await?;
            runtime_from_wire(result.as_value(), heap)
        }
        DbCommand::Count(command) => Ok(RuntimeValue::Number(
            store.count(&command.type_name, command.query).await? as f64,
        )),
        DbCommand::ExistsKey(command) => Ok(RuntimeValue::Bool(
            store.exists_by_key(&command.type_name, command.key).await?,
        )),
        DbCommand::ExistsQuery(command) => Ok(RuntimeValue::Bool(
            store
                .exists_by_query(&command.type_name, command.query)
                .await?,
        )),
    }
}

fn decode_db_result(
    value: &serde_json::Value,
    plan: &RuntimeTypePlan,
    boundary: &str,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    runtime_from_wire_required_plan_with_use(
        value,
        Some(plan),
        boundary,
        BoundaryUse::DbResultDecode,
        heap,
    )
}

fn db_recoverable_runtime_context(
    program: EvalProgramProjection<'_>,
    program_context: &ProgramExecutionContext<'_>,
    expected_plans: DbRecoverableRuntimeExpectedPlans,
) -> Result<DbRecoverableRuntimeContext> {
    let actor_context = program_context.actor_context();
    let artifact_identity = actor_context
        .request_service_protocol_identity()
        .to_string();
    let build_id = actor_context.request_build_id().to_string();
    Ok(DbRecoverableRuntimeContext {
        behavior_hooks: Arc::new(EvalRecoverableBehaviorHooks::new(
            program,
            &artifact_identity,
            &build_id,
        )?),
        expected_plans,
        artifact_identity,
        build_id: build_id.clone(),
        boundary_context: RuntimeRecoverableBoundaryContext::new(
            RuntimeRecoverableBoundaryKind::DbValue,
            RuntimeRecoverableTrustBoundary::OwnerInternal,
            RuntimeRecoverableStorageLane::RecoverableEnvelope,
        )
        .with_origin_service(RuntimeRecoverableServiceRef {
            service_id: actor_context.service_id().to_string(),
            version: Some(actor_context.service_version().to_string()),
            build_id: Some(build_id.clone()),
        })
        .with_explicit_recoverable_slot(),
        retention_expires_at_epoch_millis: None,
    })
}

fn service_db_selector(selector: DbOneCommandSelector) -> DbOneSelector {
    match selector {
        DbOneCommandSelector::Key { key } => DbOneSelector::Key(key),
        DbOneCommandSelector::Query { query, order } => DbOneSelector::Query { query, order },
    }
}
