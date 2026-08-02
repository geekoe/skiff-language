# Leaf task f3 (Wave 2): actor layer + model epoch + counters + compaction + failure semantics

## Reference chain

- Authoritative design: `doc/architecture/actor-shared-heap-design.md` v4 — §3 共享 arena 模型、
  §4.1/4.3 reacquire 纪律与 actor 侧、§7 内存与回收、§9 升级/逐出/断连、§12 F3 定义。
- Frozen interface contract: `doc/implementation/actor-shared-heap/interfaces.md` §6–§7, §9 F3
  column (main commit `40fac3b68bc076f61aa4c2a094fb1f132058f665`).
- Direct parent: batch "actor-shared-heap" integration agent `/root/integration_actor_shared_heap`.
- Workflow: `/Users/geek/workspace/multi-agent-development.md`.

## Baseline

- Branch point: `8e81e6262f0848f81c304b7257744e021859c27a` (`feat/actor-heap-access`, F2). It
  contains Slice 1 (concurrent machinery removed), F1 (actor transaction rollback removed) and
  F2 (HeapAccess dual mode + funnel release/reacquire + provider-stream boundary).
- F2 reported exactly 6 remaining actor-territory compile errors (passing `&mut RequestHeap`
  where `&mut HeapAccess` is now required): `actor_executor.rs` execute/execute_create call
  sites, and `actor_executor/tests.rs` helper/call sites. This leaf closes them.
- Worktree: `/Users/geek/workspace/skiff-actor-f3`; branch `feat/actor-layer`;
  isolated `CARGO_TARGET_DIR` inside the worktree.

## Contract (frozen; interfaces.md §6–§7)

1. `runtime/model`: `RequestHeap` gains `epoch` (default 0; `new_with_epoch(u32)`; `epoch()`);
   `HeapHandle` gains `epoch` (`new_with_epoch`; existing 2-arg `new` keeps epoch 0);
   `slot()`/`slot_mut()` validate `handle.epoch == heap.epoch`; `alloc_*` stamps the current heap
   epoch; rollback-rebase handle construction keeps the source heap epoch; the
   `runtime_values_equal` handle-equality fast path is safe because epoch is part of the handle.
   Ordinary request behavior and `program_db` ordinary rollback stay unchanged.
2. `actor_instance.rs` (§6): `ActorInstanceState { fields: Vec<ActorFieldValue>,
   arena: SharedArena }` with `SharedArena = Arc<tokio::sync::Mutex<RequestHeap>>`; field roots
   are arena handles; `acquire_segment(handle) -> SegmentLease` (holds the arena guard plus
   fence/epoch snapshot); release/commit without fields/heap clone; active/suspended continuation
   counters (create, segments, resume, abandon, commit); upgrade/discard require counters == 0
   (discard marks `pending_discard` when busy and reclaims when the segment ends); per-instance
   arena limits; `compact_if_quiescent()` (counters == 0, no pending upgrade/discard: clone live
   field roots into a fresh arena with epoch+1, atomically swap, bump epoch).
3. `actor_executor.rs` execute/execute_create: build `HeapAccess::Shared` from the instance
   arena; fix the 6 F2-listed call sites; remove snapshot_persistent_fields / resume import /
   write_field wire roundtrip (type + request-scoped validation stay on the write path per §3.5);
   a failed segment leaves already-executed field mutations in place (§3.4).
4. `actor_concurrent_continuation.rs`: `ActorExecutionFrame` suspend/resume/await_if_pending/
   finish rewritten to Shared semantics — poll-once (Ready keeps the guard; Pending →
   `access.release()` → await → `access.reacquire().await` → validate instance fence + arena
   epoch); no field snapshots/imports; `read_field`/`write_field` work directly on the shared
   arena.
5. Tests: actor_instance/tests.rs, actor_executor/tests.rs and directly-caused actor test
   callers updated — zero-copy at real suspension (arena Arc identity / node count observable),
   guard not held across Pending, failure segment leaves partial writes (replacing old
   failed-snapshot atomicity tests), counters gate upgrade/discard/compaction, compaction bumps
   epoch and stale handles fail closed, arena limits error path.

## Write scope

F3-owned: `runtime/model/src/value.rs`, `runtime/model/src/request_heap.rs` (+ tests),
`runtime/eval/src/actor_instance.rs`, `runtime/eval/src/actor_executor.rs`,
`runtime/eval/src/actor_executor/actor_concurrent_continuation.rs`, actor test suites, this leaf.

Directly-caused mechanical callers (actor frame/lease/access call sites in tests that exercise
actor segments; production F2 modules are NOT touched):
`runtime/eval/src/eval_context/actual_pending/activation/tests/activation.rs`,
`runtime/eval/src/program_stream/tests/current_scope.rs`,
`runtime/eval/src/program_db/tests/{fixture.rs, fixture/actor.rs, ordinary.rs, lease.rs}`.

Do NOT touch: F2 production modules (`eval_context.rs`, `eval_context/actual_pending.rs`,
`program_db/wait.rs`, `program_db/transaction.rs`, `program_stream.rs`,
`callback_native/prepared.rs`, `program_execution.rs`, `async_stream_cancel.rs`, `heap_access.rs`
body), router/, compiler, artifact-model/linked-program schema, `.github/workflows`. No public
ABI/wire/artifact schema change; no GC — compaction is whole-arena replacement at quiescence only.

## Verification

- `cargo check --workspace --all-targets` (cross-interface checkpoint; must be green).
- `cargo test -p skiff-runtime-model`, `cargo test -p skiff-runtime-eval` (actor lib +
  integration tests).
- `cargo fmt --check` on touched crates.
- rg proof: `snapshot_persistent_fields`, resume import, write_field wire roundtrip removed;
  Shared guard never constructed across a Pending in the actor frame.

## Handoff

Report repo, branch, worktree path, commit/tree, actual write set, verification evidence and the
自验收矩阵 to `/root/integration_actor_shared_heap` and `/root`. Do not merge or push.
