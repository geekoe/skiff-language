# R2 ordinary-expression stack regression repair contract

Status: implementation complete; awaiting integration

Direct parent:
[`f0-gate-preflight.md`](./f0-gate-preflight.md). F0 traces through
[`parent-checkpoint.md`](./parent-checkpoint.md) to the canonical tail-call
architecture and runtime reference contract. This leaf repairs the ordinary
evaluator stack regression classified by F0; it does not redefine tail-call
eligibility, execution, or diagnostics.

## Identity and DAG position

- Repository: `/Users/geek/workspace/skiff`
- Baseline commit/tree:
  `2438c61a38b77133bea1887904cde770b9c1d97b` /
  `76d159389526b80d3e18c3ede44fee9196a2f0a4`
- Branch/worktree:
  `codex/tco-r2-stack-regression` /
  `/Users/geek/workspace/skiff-tco-r2-stack-regression`
- Integration owner/branch: `/root/tco_integrator` /
  `codex/tco-integration`
- Dependency: F0 is integrated in the baseline and has classified the
  task-before PASS / candidate SIGABRT as an R1 evaluator regression.
- Completion unblocks the integrator-owned I2 probe and the minimum dynamic
  evidence rebuild selected from the actual R2 write set.
- Candidate maturity: repair checkpoint. R2 does not freeze or accept a stable
  candidate and does not run a runtime selector or full gate.

## Failure and diagnosis contract

The unchanged
`runtime_program_executes_bytes_natives_without_json_registry` test must retain
its original fixture and assertions. On the exact baseline it aborts with a
libtest-thread stack overflow under F0's command:

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-tco-r2-target \
  cargo test -p runtime \
  runtime_program_executes_bytes_natives_without_json_registry \
  -- --nocapture
```

R1 added `PreparedTailCall` directly as the payload of the common `Flow` enum.
The payload owns an `Env`, optional `RuntimeTypePlan`, target, and source site.
Every ordinary `Result<Flow>` and async statement/block state therefore has the
inline size of a prepared replacement frame even when the path cannot produce a
tail call. R2 must confirm this type/future-state mechanism and repair it with
the smallest indirection at the existing control-result boundary.

The prepared frame may be heap allocated once for the currently live tail
transfer, but it must be moved out and dropped/replaced on every trampoline
iteration. At no point may the fix retain prior prepared frames or build a
linked continuation. Therefore live prepared-frame heap space remains constant
with hop count, and the existing 100,000-hop / 1 MiB TCO evidence remains a
testable property rather than being traded for unbounded heap growth.

## Frozen implementation closure

The expected production write set is only:

- `runtime/eval/src/env.rs`, to make the existing `Flow::TailCall` payload a
  fixed-size owning indirection while preserving the same sole
  `PreparedTailCall` type and control variant;
- causal mechanical consumers in `runtime/eval/src/eval_context.rs`,
  `runtime/eval/src/program_execution.rs`, or
  `runtime/eval/src/program_execution/tail_call.rs` only if Rust ownership
  requires explicit construction/destructuring changes.

A focused evaluator test may measure the control-result size or prove prepared
frame replacement ownership if that evidence can remain robust and
implementation-local. The unchanged runtime bytes-native test is itself the
ordinary-expression regression test and must not be edited, weakened, ignored,
or replaced.

## Forbidden surfaces and stop conditions

- Do not raise thread or worker stack size, alter the program-depth constant,
  reduce fixture expression depth, or change test scheduling.
- Do not add `tokio::spawn`, a second trampoline, a heap continuation or linked
  list of tail frames, a task-local/global side channel, or an alternate
  evaluator.
- Do not change tail-call eligibility, argument/self/generic preparation,
  return-plan equivalence, instruction accounting, error sites, diagnostics,
  heap carrier, deadline, stop, Actor, or stream semantics.
- Do not change artifact/File IR/schema/config/manifest/keyword surfaces.
- Do not take ownership of V1/V2/V3 fixtures except for running affected
  focused filters, and do not run the runtime selector or full gate.

Return `TASK_SCOPE_EXPANDED` if repair requires a public contract, a new
lifecycle owner, a second execution mechanism, or production writes beyond the
R1 evaluator owner and causal mechanical consumers above.

## Completion and evidence

R2 is complete when:

- the exact bytes-native reproduction passes without changing that test;
- `Flow` no longer inlines the full prepared environment/plan and ordinary
  expression evaluation no longer inherits that large control payload;
- the trampoline still owns at most one live prepared frame, consumes it on
  each iteration, and keeps 100,000-hop / 1 MiB tail execution stack-safe;
- affected R1/V1 focused tail-call filters continue to pass, including
  accounting/depth and pressure coverage selected by exact existing names;
- `cargo check -p skiff-runtime-eval -p runtime`, `cargo fmt --all -- --check`,
  and `git diff --check` pass.

R2 records exact commands and results below after implementation. Selector-level
runtime evidence and the merged I2 probe belong to later unique owners.

## Result and evidence

R2 confirmed the regression mechanism and implemented the single minimum
candidate. The inline `PreparedTailCall` made every `Flow` as large as its owned
callee `Env` and optional return plan. Because `Flow` is the result of the
recursive async statement/block evaluator, an ordinary native-call expression
inherited that state shape despite never being eligible for tail transfer.

`Flow::TailCall` now owns `Box<PreparedTailCall>`, and `prepare_tail_call`
allocates that one box only after every eligibility and return-plan check has
succeeded. The sole trampoline consumes the box at the top of the next
iteration, immediately drops the return plan and previous transfer metadata,
and moves only the current callee `Env` into execution. A following transfer
creates a new box rather than linking it to the old one. The maximum live
prepared-frame storage is therefore constant with hop count; there is no heap
continuation or retained tail history.

The actual production write set is:

- `runtime/eval/src/env.rs`
- `runtime/eval/src/program_execution.rs`
- `runtime/eval/src/program_execution/tail_call.rs`

The only new test asserts that the common `Flow` layout is smaller than
`PreparedTailCall`, preventing the full prepared environment from being
inlined into every ordinary control result. No existing runtime, evaluator,
assembly, driver, or source fixture was changed.

### Verification result

All final commands used the R2-owned target
`/Users/geek/workspace/skiff-tco-r2-target`. After an initial focused build
filled that isolated target, R2 removed only its own Cargo cache and reran with
incremental compilation disabled; no integration or sibling cache was touched.

| Command | Result |
| --- | --- |
| F0 exact `cargo test -p runtime runtime_program_executes_bytes_natives_without_json_registry -- --nocapture` | baseline before repair: SIGABRT stack overflow; final repair: pass, 1 test; unchanged fixture |
| `cargo test -p skiff-runtime-eval tail_call_ -- --nocapture` | pass, 14 tests; includes layout regression, R1 depth/accounting, V2 assembly matrix, and 100,000 hops on an actual 1 MiB Tokio worker |
| `cargo test -p runtime runtime_program_non_tail_recursion_fails_at_guard_and_stays_healthy -- --nocapture` | pass, 1 test |
| `cargo test -p runtime runtime_program_legacy_tail_call -- --nocapture` | pass, 4 tests; includes 100,000-hop bounded diagnostic |
| `cargo check -p skiff-runtime-eval -p runtime` | pass; existing warnings only |
| `cargo fmt --all -- --check` | pass |
| `git diff --check` | pass |

R2 did not run the runtime selector, source selector, full gate, live services,
or stable instance. The evaluator production write invalidates prior dynamic
R1/V1/V2/V3 evidence only to the extent selected by the later merged I2 and
evidence-rebuild owners; this leaf's focused results do not claim freeze
readiness.
