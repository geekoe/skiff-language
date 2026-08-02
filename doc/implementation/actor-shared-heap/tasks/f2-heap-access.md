# Leaf task f2 (Wave 1): HeapAccess dual mode, EvalContext borrow rework, funnel release/reacquire, provider-stream boundary

## Reference chain

- Authoritative design: `doc/architecture/actor-shared-heap-design.md` v4 (design §4 借用改造,
  §7.3 provider-stream 边界, §12 切片定义).
- Frozen interface contract: `doc/implementation/actor-shared-heap/interfaces.md` (main commit
  `40fac3b68bc076f61aa4c2a094fb1f132058f665`; the file lives on main and reaches the
  integration branch when the batch merges into main). This leaf implements interfaces.md §3–§5
  and §9 F2 column exactly; the contract text is frozen, this file adds no semantics.
- Direct parent: batch "actor-shared-heap" integration agent `/root/integration_actor_shared_heap`.
- Workflow: `/Users/geek/workspace/multi-agent-development.md`.

## Baseline

- Slice 1: `14c06b8cb6c18b6182dfcb3842f82fa7245d2b37` (integration branch, concurrent machinery
  removed; `ActorExecutionFrame` core retained).
- F2 worktree branched at the slice-1 commit, then folded in the current integration HEAD
  `fdfbdbc1` (F1 txn-ban + F5 router evict-race already merged) so the branch is directly
  mergeable into `integration/actor-shared-heap`.
- Worktree: `/Users/geek/workspace/skiff-actor-f2`; branch `feat/actor-heap-access`;
  isolated `CARGO_TARGET_DIR` inside the worktree.

## Contract (frozen; interfaces.md §3–§5)

1. New `runtime/eval/src/heap_access.rs`:
   `HeapAccess::Exclusive(&'a mut RequestHeap)` /
   `HeapAccess::Shared { arena: Arc<tokio::sync::Mutex<RequestHeap>>, guard: Option<OwnedMutexGuard<RequestHeap>> }`
   with `heap_mut()` (Shared guard must be Some; invariant error otherwise), `release()`,
   `async reacquire()`, `is_shared()`, Deref/DerefMut. Exclusive release/reacquire are no-ops.
2. `EvalContext.heap` becomes the `HeapAccess` access; all internal heap access goes through
   `self.heap.heap_mut()`; `heap_mut()` lives on `HeapAccess`, not on `EvalContext`.
3. Funnels: `actual_pending::await_operation`, `program_db::wait::await_operation`,
   `program_stream::current_scope::next_with_actor`, `callback_native::prepared` wait path —
   Shared: poll-once (Ready -> no release; Pending -> release -> await -> reacquire);
   Exclusive: plain await as today.
4. `call_program_executable*` (and the other Pending-capable `Interpreter` entry points) change
   their heap parameter from `&mut RequestHeap` to `&mut HeapAccess`; ordinary request call sites
   pass `Exclusive`; actor call sites pass `Shared`. Sync functions keep `&mut RequestHeap`.
5. `ActorExecutionFrame::await_if_pending` adapts its signature to `&mut HeapAccess` and its heap
   access to `access.heap_mut()`; current snapshot/commit behavior is preserved; the body adds
   release-before-await and reacquire-after-await around the pending cut point so a Shared guard
   never survives Pending. F3 owns the later semantic rewrite of the actor layer.
6. Provider-stream boundary (§7.3): `async_stream_cancel` provider-stream path must not clone the
   caller env verbatim into the spawned task; use the detach-only env construction used by the
   unary path, plus a regression test proving a shared-arena handle cannot reach the task env.

## F2 write scope

`runtime/eval/src/heap_access.rs` (new), `eval_context.rs`, `eval_context/actual_pending.rs`,
`eval_context/timeout.rs`, `program_db/wait.rs`, `program_db.rs`, `program_db/transaction.rs`,
`db_eval.rs`, `program_stream.rs`, `program_stream/current_scope.rs`, `callback_native/prepared.rs`,
`program_execution.rs`, `spawn_ops.rs`, `async_stream_cancel.rs`, `async_stream_cancel/prepared_unary.rs`,
`actor_concurrent_continuation.rs` (await_if_pending only), plus mechanical caller/fixture closure:
`assembly_execution/{mod,ordinary,ingress,activation_relative}.rs`, `actor_dispatch.rs`,
`invocation.rs`, `program_invocation.rs`, `runtime_http_gateway.rs`, `runtime_websocket_connect.rs`,
`runtime_websocket_jsonrpc.rs`, `runtime/request/src/assembly_ingress.rs`, and related tests
(eval, driver, host admission tests). Mechanical test adaptation inside the former F1 test file
`program_db/tests/transaction.rs` is limited to the signature change (F1 already landed).

Do NOT touch: `actor_instance.rs`, `actor_executor.rs` bodies (F3 owns; the actor path cannot
compile until F3 adapts its `call_program_executable_with_self_direct` call sites — F2 verifies
with a temporary local-only adapter and reports the dependency), `program_db/rollback.rs` (F1
landed), router/, compiler, artifact-model/linked-program schema, `.github/workflows`.

## Verification

- `cargo check`/`cargo build` for skiff-runtime-eval (+ dependents), `cargo test` for the affected
  eval tests (funnels, program_stream, program_db wait, eval_context, callback_native,
  async_stream_cancel), `cargo fmt --check` on touched crates.
- Focused new tests: HeapAccess Exclusive/Shared method behavior; Shared funnel release/reacquire
  discipline (guard None while Pending, reacquired after wake); provider-stream env boundary
  regression.
- Report exact write set, evidence, remaining F3 dependency, and rg proof of no Shared guard
  across Pending in converted paths.
