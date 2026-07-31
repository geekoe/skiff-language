# S1 provider-stream scheduler depth

Status: implementation

Direct parent:
[`parent-checkpoint.md`](parent-checkpoint.md). Its authority chain continues to
the canonical tail-call architecture and runtime reference documents.

## DAG and baseline

- Node: S1 independent scheduler depth.
- Dependency: P0 parent checkpoint at
  `c34a954bca3580533c153d5761e8805c423dbb09` /
  `8beb99c62fb2bf2f4fade9f41c855773c2e8a714`.
- Unblocks: I1 combined probe after integration with the other batch nodes.
- Integration owner: `/root/tco_integrator`, branch `codex/tco-integration`.
- Development branch/worktree: `codex/tco-s1-provider-depth` /
  `/Users/geek/workspace/skiff-tco-s1-provider-depth`.
- Candidate maturity: implementation checkpoint; S1 alone is not an acceptance
  candidate.

## Precheck facts and owners

The bounded production path is
`spawn_provider_stream` -> `tokio::spawn` -> `run_provider_stream` ->
`call_provider_callable`. At the baseline, `run_provider_stream` reconstructs
the captured provider context with `borrow()`, so the first program call in
this independent Tokio task inherits the caller task's active program depth.

`OwnedProgramExecutionContext::borrow_for_scheduled_task()` is the existing
canonical reset and is already used by the independent local stream-producer
task. The provider-stream callable entry and its colocated unit evidence in
`runtime/eval/src/assembly_execution/async_stream_cancel.rs` are S1's only
production/test owners.

Error-export borrows occur after callable execution and remain ordinary.
`prepared_unary` and callback waits are continuations of the original native
call chain and must continue using `borrow()`.

## Write scope and completion

Allowed writes:

- this leaf contract;
- `runtime/eval/src/assembly_execution/async_stream_cancel.rs`, limited to the
  provider callable entry and focused test evidence.

Forbidden writes include the R1 shared evaluator (`env`, `eval_context`,
`program_execution`, `flow_completion` and lexical consumers), compiler/linker
production or evidence, V1/V2/V3 evidence, public contracts, and stream
terminal behavior.

Completion requires:

1. the independent provider task reconstructs its callable context with
   `borrow_for_scheduled_task()`;
2. focused local evidence locks the scheduler entry to the fresh-depth borrow;
3. reverse search shows error-export and `prepared_unary` still use ordinary
   `borrow()`, while only genuine scheduler callable entries reset;
4. no terminal, cancellation, error-export, ABI, artifact, or public semantic
   changes.

The parent owner coordinated the remaining dynamic criterion with R1 because
program-call depth is private to that shared owner. R1 will provide a minimal
test-only seeded-depth seam (or identify an equivalent existing seam); after R1
integration a new narrow evidence node must prove that a non-zero captured
depth reaches this provider callable with the full fresh-task allowance. S1
must not manufacture an assembly/service fixture or touch R1 to close that
evidence early.

If the production change needs another lifecycle/trampoline owner, return
`TASK_SCOPE_EXPANDED` instead of widening this task.

## Verification ownership

S1 uniquely owns these non-overlapping focused checks:

```bash
cargo fmt --package skiff-runtime-eval
cargo test -p skiff-runtime-eval provider_stream_scheduler_entry_uses_fresh_depth_borrow
```

The integrator owns the merged-state combined probe. Runtime selectors, full
workspace gates, live instances, and chat smoke are outside S1.

Evidence remains valid only for the implementation commit/tree. Changes to
`async_stream_cancel.rs`, scheduled-task borrowing, call-depth accounting, or
the focused fixture invalidate it.
