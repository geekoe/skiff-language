# Runtime evaluator checkpoint frequency optimization

日期：2026-08-04
状态：task result

## Goal

Make the tree-walking evaluator's per-node checkpoint path cheap while keeping
bounded stop/deadline/cancel/instruction-limit semantics. The per-node path is
now one atomic instruction-counter `fetch_add`; a full
deadline/cancel/limit/scope-terminal check runs only when the counter crosses
`ExecutionBudgetConfig::poll_interval` (default 1024) nodes, reaches the
instruction limit, or hits one of a few explicit safety points.

## Changes

- `runtime/request/src/execution_control.rs`:
  `ExecutionControl::add_instruction_units` no longer reads the monotonic clock
  on every call. It charges `ExecutionBudget::add_units` (one atomic
  `fetch_add` + interval/limit comparison) and only runs
  `poll_execution_budget()` when the interval is crossed or the limit is
  reached. This is where the old per-node `Instant::now()` and budget poll were
  removed.
- `runtime/eval/src/program_execution/execution_scope.rs`:
  `ProgramExecutionContext::checkpoint` is now counter-only
  (`add_instruction_units`). The old full check (clock read, scope clone,
  terminal check, budget poll) moved into the new
  `poll_execution_scope()` method, used only at explicit safety points.
- `runtime/eval/src/program_execution.rs`:
  `ProgramExecutionContext` caches one long-lived `ExecutionControl` bridge,
  rebuilt only when the owned control is replaced (`derive_timeout_child`).
  `execution()` is now an `Arc` clone instead of a fresh `Arc::new(...)` per
  call. Explicit per-call/return `poll_execution_budget()` calls were removed;
  call and tail-transfer accounting still charges 1 unit and full-checks on
  interval crossing.
- Safety points that must not wait for an interval crossing now call
  `poll_execution_scope()`: actual-pending operation boundaries, provider /
  activation-relative service-call starts, actor create-less admission, and
  stream/invocation waits after a zero-unit checkpoint.
- Derived-scope exit completeness (`runtime/eval/src/eval_context/timeout.rs`):
  a timeout body that completes successfully (fewer than K nodes) now observes
  its child scope terminal on the success path before the scope is dropped,
  converting a child-owned deadline to the same `TimeoutError` semantics as the
  error path. Inherited/ancestor terminals propagate unchanged.
- DB transaction/lease normal exits
  (`runtime/eval/src/program_db.rs`): a full scope check runs before commit /
  before a claim body reports success, so a body shorter than K nodes cannot
  silently escape an expired parent deadline. The lease is still released
  before the terminal is reported.
- Loop accounting is unchanged: `checkpoint_loop_condition(1)` charges each
  iteration (including empty `while true {}` bodies), so loops still hit the
  interval/limit deterministically.

## Semantics preserved

- Instruction units and `instructionCount` diagnostics are unchanged (1 unit
  per statement/expression/function-entry/call/tail transfer).
- `MAX_PROGRAM_CALL_DEPTH = 128` and the native-stack guard are untouched.
- Deadline/cancel/instruction-limit observation latency is bounded by K nodes
  plus scope exits, matching the runtime doc's coarse "not per machine
  instruction" semantics.
- Concurrent IR remains rejected in v1; the unused `LaneStart`/`LaneEnd`
  checkpoint kinds were audited and left untouched.

## Verification

- `cargo check -p skiff-runtime-eval` — ok.
- `cargo test -p skiff-runtime-eval --lib` — 476 passed.
- `cargo test -p skiff-runtime-request --lib` — 43 passed.
- `cargo test -p skiff-runtime-host --lib` — 429 passed (host consumes the
  changed request execution control).
- Tail stress:
  `cargo test -p skiff-runtime-eval --lib program_execution::tests::execution_scope::tail_call_execution::tail_call_completes_100000_hops_on_one_mib_tokio_worker -- --exact`
  — ok.
- New regression tests:
  - request crate: accounting below the interval does not poll; interval
    crossing and instruction-limit/cancel are observed on the full check;
  - evaluator: empty `while true {}` hits the instruction limit exactly and
    observes cancel/deadline within a poll interval;
  - evaluator: pure CPU loops and array chunks complete without reading the
    scripted execution clock (per-node path is counter-only);
  - evaluator: a timeout body with fewer than K nodes still materializes
    `TimeoutError` on its success-path scope exit.

## Perf

Debug-build 100k tail-hop stress test (same machine, same worktree):

| run | test duration |
| --- | --- |
| before (main @ 5edc0962) | 1.55s |
| after | 1.15s |

~26% faster on the tail-hop microbenchmark; the improvement is larger on
non-suspending CPU-bound evaluator loops because the per-node clock reads,
scope clones, and `Arc::new` bridge allocations are gone.

## Remaining risks

- Overshoot is intentionally coarse: a deadline/cancel can be observed up to K
  nodes late inside one scope, plus one scope-exit check. This matches the
  accepted design; tests assert bounded (not exact) latency.
- Zero-unit checkpoints (match arms, array/map literal items, loop backedges)
  are now no-ops. They no longer serve as keep-alive polls; interval crossings
  from positive-unit nodes provide the bound. A single enormous
  pattern/array-literal expression with zero charged units could in principle
  run past a deadline until the surrounding node accounting crosses an
  interval.
- The eval-side scripted-clock unit tests now observe deadlines only at full
  checks (scope exits / safety points); exact per-node crossing positions are
  no longer a stable behavior and the tests were updated accordingly.
