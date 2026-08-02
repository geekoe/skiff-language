use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapCheckpoint},
    runtime_value::{RuntimeValue, RuntimeValueCarrier},
};

use crate::{
    env::{Env, EnvRollbackCheckpoint},
    error::{
        rebind_runtime_error_request_heap_root, runtime_error_request_heap_root, Result,
        RuntimeError,
    },
};

pub(super) struct TransactionRollbackCheckpoint {
    heap: RequestHeapCheckpoint,
    env: EnvRollbackCheckpoint,
}

impl TransactionRollbackCheckpoint {
    pub(super) fn capture(heap: &RequestHeap, env: &Env) -> Self {
        Self {
            heap: heap.checkpoint(),
            env: env.rollback_checkpoint(),
        }
    }
}

/// Rolls back one transaction while rebasing every live request-heap owner in
/// one graph operation. Candidate error and Env state are built before the
/// prepared heap is installed; after that install, publishing those owners
/// consists only of infallible assignments while their mutable access is
/// retained. Actor field rollback was removed in v1: `db transaction` is
/// rejected inside actor methods at compile time.
pub(super) fn rollback_transaction_live_roots(
    heap: &mut RequestHeap,
    env: &mut Env,
    checkpoint: TransactionRollbackCheckpoint,
    error: RuntimeError,
) -> Result<RuntimeError> {
    prepare_and_publish(heap, env, checkpoint, error)
}

fn prepare_and_publish(
    heap: &mut RequestHeap,
    env: &mut Env,
    checkpoint: TransactionRollbackCheckpoint,
    error: RuntimeError,
) -> Result<RuntimeError> {
    let error_root = runtime_error_request_heap_root(&error).cloned();
    let env_roots = env.rollback_root_carriers(&checkpoint.env)?;

    // Root order is part of this coordinator's internal mapping contract:
    // selected error first, then entry-live Env slots.
    let mut roots = Vec::with_capacity(usize::from(error_root.is_some()) + env_roots.len());
    roots.extend(error_root.iter().map(|carrier| carrier.value().clone()));
    roots.extend(env_roots.iter().map(|(_, carrier)| carrier.value().clone()));

    let prepared = match heap.prepare_rollback_rebase(checkpoint.heap, &roots) {
        Ok(prepared) => prepared,
        Err(prepare_error) if prepare_error.is_skippable() => {
            // Rollback compaction is only a GC optimization in this case. Keep
            // the original graph and the already-selected business/abort error.
            return Ok(error);
        }
        Err(error) => return Err(RuntimeError::from(error.into_runtime_error())),
    };
    let rebased = prepared.rebased_roots().to_vec();
    if rebased.len() != roots.len() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "transaction rollback prepared {} roots for {} owners",
            rebased.len(),
            roots.len()
        )));
    }

    let mut cursor = 0;
    let rebased_error = error_root
        .as_ref()
        .map(|_| take_rebased(&rebased, &mut cursor));
    let candidate_error = rebind_runtime_error_request_heap_root(error, rebased_error)?;

    let candidate_env_roots = env_roots
        .iter()
        .map(|(slot, carrier)| {
            (
                *slot,
                carrier
                    .clone()
                    .map_value(|_| take_rebased(&rebased, &mut cursor)),
            )
        })
        .collect::<Vec<(usize, RuntimeValueCarrier)>>();
    let candidate_env = env.rebased_for_rollback(&checkpoint.env, &candidate_env_roots)?;

    if cursor != rebased.len() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "transaction rollback consumed {cursor} of {} prepared roots",
            rebased.len()
        )));
    }

    heap.commit_prepared_rollback_rebase(prepared);
    *env = candidate_env;
    Ok(candidate_error)
}

fn take_rebased(roots: &[RuntimeValue], cursor: &mut usize) -> RuntimeValue {
    let value = roots[*cursor].clone();
    *cursor += 1;
    value
}
