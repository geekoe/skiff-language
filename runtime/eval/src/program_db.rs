use std::{sync::Arc, time::Duration};

use skiff_runtime_boundary::plan::BoundaryUse;
use skiff_runtime_capability_context::{DbKey, DbOneSelector};
use skiff_runtime_model::{
    recoverable::{
        RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
        RuntimeRecoverableServiceRef, RuntimeRecoverableStorageLane,
        RuntimeRecoverableTrustBoundary,
    },
    request_heap::RequestHeap,
    runtime_value::{RuntimeValue, RuntimeValueCarrier},
    type_plan::RuntimeTypePlan,
};

use super::{
    assembly_execution::RuntimeExecutionProjection,
    capabilities::{
        DbCapabilityContext, DbCapabilityStore, DbRecoverableRuntimeContext,
        DbRecoverableRuntimeExpectedPlans,
    },
    db_command::{DbCommand, DbCommandChange, DbCommandValue, DbOneCommandSelector},
    db_eval::DbIrEvaluator,
    env::{Env, Flow},
    program_execution::ProgramExecutionContext,
    recoverable_behavior::EvalRecoverableBehaviorHooks,
    runtime_ops::{runtime_from_wire, runtime_from_wire_required_plan_with_use, runtime_to_wire},
    Interpreter,
};
use crate::error::{Result, RuntimeError};
use crate::heap_access::HeapAccess;
use skiff_runtime_linked_program::{
    CallIr, DbLeaseClaimIr, DbLeaseReadIr, DbOperationIr, DbProjectionIr, DbQueryIr, DbTargetIr,
    DbTransactionIr, DbTransactionModeIr, ExecutableAddr, LinkedCallTarget, LinkedExecutable,
    LinkedFileUnit,
};
use skiff_runtime_native_contract::native_target_name;

mod lease;
mod rollback;
mod transaction;
mod wait;

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
        heap: &mut HeapAccess<'_>,
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
        heap: &mut HeapAccess<'_>,
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
            RuntimeExecutionProjection::for_context(self, &program_context)?,
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
        heap: &mut HeapAccess<'_>,
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

        let lifecycle =
            transaction::TransactionLifecycle::begin(store, &program_context, heap).await?;
        let checkpoint = rollback::TransactionRollbackCheckpoint::capture(heap.heap_mut(), env);
        let result = self
            .eval_program_expr_ref(
                program_context.clone(),
                heap,
                env,
                addr,
                file,
                executable,
                body,
            )
            .await;
        match result {
            Ok(value) => {
                if let Err(error) = lifecycle.commit(&program_context, heap).await {
                    let error = rollback::rollback_transaction_live_roots(
                        heap.heap_mut(),
                        env,
                        checkpoint,
                        error,
                    )?;
                    return Err(error);
                }
                Ok(value.into_value())
            }
            Err(error) => {
                let error = abort_transaction_and_rollback(
                    lifecycle,
                    &program_context,
                    heap,
                    env,
                    checkpoint,
                    error,
                )
                .await?;
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn eval_program_explicit_db_transaction(
        &self,
        program_context: ProgramExecutionContext<'_>,
        heap: &mut HeapAccess<'_>,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        transaction: &DbTransactionIr,
    ) -> Result<RuntimeValueCarrier> {
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
        heap: &mut HeapAccess<'_>,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        transaction: &DbTransactionIr,
    ) -> Result<RuntimeValueCarrier> {
        let store = require_db_store(db_context, "db.transaction")?;
        let lifecycle =
            transaction::TransactionLifecycle::begin(store, &program_context, heap).await?;
        let checkpoint = rollback::TransactionRollbackCheckpoint::capture(heap.heap_mut(), env);
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
                    DbTransactionModeIr::Effect => Ok(RuntimeValue::Null.into()),
                    DbTransactionModeIr::Value => match transaction.result {
                        Some(result) => {
                            self.eval_program_expr_ref(
                                program_context.clone(),
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
                        let error = abort_transaction_and_rollback(
                            lifecycle,
                            &program_context,
                            heap,
                            env,
                            checkpoint,
                            error,
                        )
                        .await?;
                        return Err(error);
                    }
                };
                if let Err(error) = lifecycle.commit(&program_context, heap).await {
                    let error = rollback::rollback_transaction_live_roots(
                        heap.heap_mut(),
                        env,
                        checkpoint,
                        error,
                    )?;
                    return Err(error);
                }
                Ok(result)
            }
            Ok(Flow::Return(_)) => {
                let error = abort_transaction_and_rollback(
                    lifecycle,
                    &program_context,
                    heap,
                    env,
                    checkpoint,
                    RuntimeError::Decode(
                        "return is not allowed inside db transaction blocks".to_string(),
                    ),
                )
                .await?;
                Err(error)
            }
            Ok(Flow::Parked | Flow::ContinueConsumer) => {
                let error = abort_transaction_and_rollback(
                    lifecycle,
                    &program_context,
                    heap,
                    env,
                    checkpoint,
                    RuntimeError::Decode(
                        "control flow is not allowed inside db transaction blocks".to_string(),
                    ),
                )
                .await?;
                Err(error)
            }
            Ok(Flow::Break | Flow::LoopContinue) => {
                let error = abort_transaction_and_rollback(
                    lifecycle,
                    &program_context,
                    heap,
                    env,
                    checkpoint,
                    RuntimeError::Decode(
                        "db transaction exited with break/continue outside a loop".to_string(),
                    ),
                )
                .await?;
                Err(error)
            }
            Err(error) => {
                let error = abort_transaction_and_rollback(
                    lifecycle,
                    &program_context,
                    heap,
                    env,
                    checkpoint,
                    error,
                )
                .await?;
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn eval_program_db_query_value(
        &self,
        program_context: ProgramExecutionContext<'_>,
        heap: &mut HeapAccess<'_>,
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
        heap: &mut HeapAccess<'_>,
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
        let key = DbKey::new(runtime_to_wire(&key, heap.heap_mut())?);
        let claim_store = store.clone();
        let type_name = super::db_eval::db_capability_target(&claim.target)
            .lookup_key()
            .to_string();
        let slot = claim.slot.clone();
        let Some(handle) = wait::await_operation(&program_context, heap, async move {
            claim_store.claim_lease(&type_name, key, &slot).await
        })
        .await??
        else {
            return Ok(RuntimeValue::Bool(false));
        };

        let renew_store = store.clone();
        let renew_hold = handle.hold.clone();
        let request_cancelled = program_context.execution().cancel_flag();
        let renew_period = Duration::from_millis((handle.ttl_ms / 3).max(1));
        let renew_owner =
            lease::LeaseRenewOwner::start(renew_store, renew_hold, renew_period, request_cancelled);

        let binding = claim.binding_slot.map_or(Ok(()), |binding_slot| {
            let value = runtime_from_wire(handle.value.as_value(), heap.heap_mut())?;
            env.declare_binding("db lease binding", Some(binding_slot as usize), value)
        });
        let flow = match binding {
            Ok(()) => {
                self.exec_program_block(
                    program_context.clone(),
                    heap,
                    env,
                    addr,
                    file,
                    executable,
                    &claim.body,
                )
                .await
            }
            Err(error) => Err(error),
        };
        wait::await_operation(&program_context, heap, async move {
            renew_owner.stop_and_join().await;
        })
        .await?;

        let lost_store = store.clone();
        let lease_lost = wait::await_operation(&program_context, heap, async move {
            lost_store.lease_lost().await
        })
        .await?;
        let release_store = store.clone();
        let release_hold = handle.hold.clone();
        let release = wait::await_operation(&program_context, heap, async move {
            release_store.release_lease(&release_hold).await
        })
        .await?;
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
        heap: &mut HeapAccess<'_>,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        read: &DbLeaseReadIr,
    ) -> Result<RuntimeValue> {
        let store = require_db_store(&program_context.db_context(), "db lease")?;
        let key = self
            .eval_program_expr_ref(
                program_context.clone(),
                heap,
                env,
                addr,
                file,
                executable,
                read.key,
            )
            .await?;
        let key = DbKey::new(runtime_to_wire(&key, heap)?);
        let read_store = store.clone();
        let type_name = super::db_eval::db_capability_target(&read.target)
            .lookup_key()
            .to_string();
        let slot = read.slot.clone();
        match wait::await_operation(&program_context, heap, async move {
            read_store.read_lease(&type_name, key, &slot).await
        })
        .await??
        {
            Some(value) => runtime_from_wire(&value, heap.heap_mut()),
            None => Ok(RuntimeValue::Null),
        }
    }
}

async fn abort_transaction_and_rollback(
    lifecycle: transaction::TransactionLifecycle,
    program_context: &ProgramExecutionContext<'_>,
    heap: &mut HeapAccess<'_>,
    env: &mut Env,
    checkpoint: rollback::TransactionRollbackCheckpoint,
    original_error: RuntimeError,
) -> Result<RuntimeError> {
    let error = match lifecycle.abort(program_context, heap).await {
        Ok(()) => original_error,
        Err(abort_error) => abort_error,
    };
    rollback::rollback_transaction_live_roots(heap.heap_mut(), env, checkpoint, error)
}

fn require_db_store(db_context: &DbCapabilityContext, target: &str) -> Result<DbCapabilityStore> {
    Ok(db_context.require_store(target, SERVICE_DB_UNCONFIGURED_REASON)?)
}

async fn execute_db_command(
    store: &DbCapabilityStore,
    program: RuntimeExecutionProjection<'_>,
    program_context: &ProgramExecutionContext<'_>,
    heap: &mut HeapAccess<'_>,
    command: DbCommand,
) -> Result<RuntimeValue> {
    match command {
        DbCommand::FindMany(command) => {
            if let Some(recoverable_runtime) = command.recoverable_runtime {
                let context =
                    db_recoverable_runtime_context(&program, program_context, recoverable_runtime)?;
                let operation = store.prepare_find_many_page_runtime(
                    command.target.lookup_key(),
                    command.query,
                    command.options,
                    command.projection,
                    heap.heap_mut(),
                    context,
                )?;
                let finalizer =
                    wait::await_operation(program_context, heap, operation.into_wait()).await??;
                let values = finalizer.finalize(heap.heap_mut())?;
                return Ok(RuntimeValue::Heap(heap.heap_mut().alloc_array(values)?));
            }
            let wait_store = store.clone();
            let type_name = command.target.lookup_key().to_string();
            let page = wait::await_operation(program_context, heap, async move {
                wait_store
                    .find_many_page(
                        &type_name,
                        command.query,
                        command.options,
                        command.projection,
                    )
                    .await
            })
            .await??;
            decode_db_result(
                &serde_json::Value::Array(
                    page.values
                        .into_iter()
                        .map(|value| value.into_value())
                        .collect(),
                ),
                &command.result_plan,
                "db find many result",
                heap.heap_mut(),
            )
        }
        DbCommand::FindOne(command) => {
            let display_type_name = command.target.type_name.clone();
            let type_name = command.target.lookup_key().to_string();
            let projection = command.projection;
            if let Some(recoverable_runtime) = command.recoverable_runtime {
                let context =
                    db_recoverable_runtime_context(&program, program_context, recoverable_runtime)?;
                let operation = match command.selector {
                    DbOneCommandSelector::Key { key } => store.prepare_find_one_by_key_runtime(
                        &type_name,
                        key,
                        projection,
                        heap.heap_mut(),
                        context,
                    )?,
                    DbOneCommandSelector::Query { query, order } => store
                        .prepare_find_one_by_query_runtime(
                            &type_name,
                            query,
                            order,
                            projection,
                            heap.heap_mut(),
                            context,
                        )?,
                };
                let finalizer =
                    wait::await_operation(program_context, heap, operation.into_wait()).await??;
                let found = finalizer.finalize(heap.heap_mut())?;
                return match found {
                    Some(value) => Ok(value),
                    None if command.required => Err(RuntimeError::Decode(format!(
                        "db require could not find {display_type_name}"
                    ))),
                    None => Ok(RuntimeValue::Null),
                };
            }
            let found = match command.selector {
                DbOneCommandSelector::Key { key } => {
                    let wait_store = store.clone();
                    let wait_type_name = type_name.clone();
                    wait::await_operation(program_context, heap, async move {
                        wait_store
                            .find_one_by_key(&wait_type_name, key, projection)
                            .await
                    })
                    .await??
                }
                DbOneCommandSelector::Query { query, order } => {
                    let wait_store = store.clone();
                    let wait_type_name = type_name.clone();
                    wait::await_operation(program_context, heap, async move {
                        wait_store
                            .find_one_by_query(&wait_type_name, query, order, projection)
                            .await
                    })
                    .await??
                }
            };
            match found {
                Some(value) => decode_db_result(
                    value.as_value(),
                    &command.result_plan,
                    "db find one result",
                    heap.heap_mut(),
                ),
                None if command.required => Err(RuntimeError::Decode(format!(
                    "db require could not find {display_type_name}"
                ))),
                None => Ok(RuntimeValue::Null),
            }
        }
        DbCommand::InsertOne(command) => match command.value {
            DbCommandValue::Wire(value) => {
                let wait_store = store.clone();
                let type_name = command.target.lookup_key().to_string();
                let result = wait::await_operation(program_context, heap, async move {
                    wait_store.create(&type_name, value).await
                })
                .await??;
                decode_db_result(
                    result.as_value(),
                    &command.result_plan,
                    "db insert one result",
                    heap.heap_mut(),
                )
            }
            DbCommandValue::Runtime {
                value,
                recoverable_runtime,
            } => {
                let context =
                    db_recoverable_runtime_context(&program, program_context, recoverable_runtime)?;
                let operation = store.prepare_create_runtime(
                    command.target.lookup_key(),
                    &value,
                    heap.heap_mut(),
                    context,
                )?;
                let finalizer =
                    wait::await_operation(program_context, heap, operation.into_wait()).await??;
                Ok(finalizer.finalize(heap.heap_mut())?)
            }
        },
        DbCommand::InsertMany(command) => {
            let wait_store = store.clone();
            let type_name = command.target.lookup_key().to_string();
            let result = wait::await_operation(program_context, heap, async move {
                wait_store
                    .insert_many_result(&type_name, command.values)
                    .await
            })
            .await??;
            decode_db_result(
                result.as_value(),
                &command.result_plan,
                "db insert many result",
                heap.heap_mut(),
            )
        }
        DbCommand::UpdateOne(command) => match command.change {
            DbCommandChange::Wire(change) => {
                let wait_store = store.clone();
                let type_name = command.target.lookup_key().to_string();
                let selector = service_db_selector(command.selector);
                let result = wait::await_operation(program_context, heap, async move {
                    wait_store.update_one(&type_name, selector, change).await
                })
                .await??;
                result
                    .map(|value| {
                        decode_db_result(
                            value.as_value(),
                            &command.result_plan,
                            "db update one result",
                            heap.heap_mut(),
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
                    db_recoverable_runtime_context(&program, program_context, recoverable_runtime)?;
                let operation = store.prepare_update_one_runtime(
                    command.target.lookup_key(),
                    service_db_selector(command.selector),
                    change,
                    heap.heap_mut(),
                    context,
                )?;
                let finalizer =
                    wait::await_operation(program_context, heap, operation.into_wait()).await??;
                Ok(finalizer
                    .finalize(heap.heap_mut())?
                    .unwrap_or(RuntimeValue::Null))
            }
        },
        DbCommand::UpdateMany(command) => {
            let wait_store = store.clone();
            let type_name = command.target.lookup_key().to_string();
            let result = wait::await_operation(program_context, heap, async move {
                wait_store
                    .update_many(&type_name, command.query, command.change)
                    .await
            })
            .await??;
            decode_db_result(
                result.as_value(),
                &command.result_plan,
                "db update many result",
                heap.heap_mut(),
            )
        }
        DbCommand::UpsertKey(command) => {
            let wait_store = store.clone();
            let type_name = command.target.lookup_key().to_string();
            let result = wait::await_operation(program_context, heap, async move {
                wait_store
                    .upsert_by_key(&type_name, command.key, command.insert, command.change)
                    .await
            })
            .await??;
            decode_db_result(
                result.as_value(),
                &command.result_plan,
                "db upsert result",
                heap.heap_mut(),
            )
        }
        DbCommand::ReplaceOne(command) => match command.value {
            DbCommandValue::Wire(value) => {
                let wait_store = store.clone();
                let type_name = command.target.lookup_key().to_string();
                let selector = service_db_selector(command.selector);
                let result = wait::await_operation(program_context, heap, async move {
                    wait_store.replace_one(&type_name, selector, value).await
                })
                .await??;
                result
                    .map(|value| {
                        decode_db_result(
                            value.as_value(),
                            &command.result_plan,
                            "db replace one result",
                            heap.heap_mut(),
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
                    db_recoverable_runtime_context(&program, program_context, recoverable_runtime)?;
                let operation = store.prepare_replace_one_runtime(
                    command.target.lookup_key(),
                    service_db_selector(command.selector),
                    &value,
                    heap.heap_mut(),
                    context,
                )?;
                let finalizer =
                    wait::await_operation(program_context, heap, operation.into_wait()).await??;
                Ok(finalizer
                    .finalize(heap.heap_mut())?
                    .unwrap_or(RuntimeValue::Null))
            }
        },
        DbCommand::DeleteOne(command) => {
            let wait_store = store.clone();
            let type_name = command.target.lookup_key().to_string();
            let selector = service_db_selector(command.selector);
            Ok(RuntimeValue::Bool(
                wait::await_operation(program_context, heap, async move {
                    wait_store.delete_one(&type_name, selector).await
                })
                .await??,
            ))
        }
        DbCommand::DeleteMany(command) => {
            let wait_store = store.clone();
            let type_name = command.target.lookup_key().to_string();
            let result = wait::await_operation(program_context, heap, async move {
                wait_store.delete_many(&type_name, command.query).await
            })
            .await??;
            runtime_from_wire(result.as_value(), heap)
        }
        DbCommand::Count(command) => {
            let wait_store = store.clone();
            let type_name = command.target.lookup_key().to_string();
            Ok(RuntimeValue::Number(
                wait::await_operation(program_context, heap, async move {
                    wait_store.count(&type_name, command.query).await
                })
                .await?? as f64,
            ))
        }
        DbCommand::ExistsKey(command) => {
            let wait_store = store.clone();
            let type_name = command.target.lookup_key().to_string();
            Ok(RuntimeValue::Bool(
                wait::await_operation(program_context, heap, async move {
                    wait_store.exists_by_key(&type_name, command.key).await
                })
                .await??,
            ))
        }
        DbCommand::ExistsQuery(command) => {
            let wait_store = store.clone();
            let type_name = command.target.lookup_key().to_string();
            Ok(RuntimeValue::Bool(
                wait::await_operation(program_context, heap, async move {
                    wait_store.exists_by_query(&type_name, command.query).await
                })
                .await??,
            ))
        }
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
    program: &RuntimeExecutionProjection<'_>,
    program_context: &ProgramExecutionContext<'_>,
    expected_plans: DbRecoverableRuntimeExpectedPlans,
) -> Result<DbRecoverableRuntimeContext> {
    let request_context = program_context.request_context();
    let artifact_identity = request_context
        .request_service_protocol_identity()
        .to_string();
    let build_id = request_context.request_build_id().to_string();
    Ok(DbRecoverableRuntimeContext {
        behavior_hooks: Arc::new(EvalRecoverableBehaviorHooks::new_for_execution(program)?),
        expected_plans,
        artifact_identity,
        build_id: build_id.clone(),
        boundary_context: RuntimeRecoverableBoundaryContext::new(
            RuntimeRecoverableBoundaryKind::DbValue,
            RuntimeRecoverableTrustBoundary::OwnerInternal,
            RuntimeRecoverableStorageLane::RecoverableEnvelope,
        )
        .with_origin_service(RuntimeRecoverableServiceRef {
            service_id: request_context.service_id().to_string(),
            version: Some(request_context.service_version().to_string()),
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

#[cfg(test)]
mod tests;
