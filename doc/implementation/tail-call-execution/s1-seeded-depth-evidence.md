# S1 seeded provider-stream depth evidence

Status: implementation complete; awaiting integration

Direct parent:
[`parent-checkpoint.md`](parent-checkpoint.md). That checkpoint traces through the
runtime reference contract to the single architecture authority. This leaf
records only execution and evidence facts.

## Identity and DAG

- Repository: `/Users/geek/workspace/skiff`
- Baseline commit/tree:
  `bc4fea09685c6e92ed3fb5aa523ebc7cac6ba2df` /
  `910bc17d1e752c81ae10a2105a77d89a113a292c`
- Branch/worktree:
  `codex/tco-s1-seeded-depth-evidence` /
  `/Users/geek/workspace/skiff-tco-s1-seeded-depth-evidence`
- Integration owner/branch: `/root/tco_integrator` /
  `codex/tco-integration`
- Dependencies: the S1 provider-stream scheduler reset and R1 shared evaluator,
  including its test-only depth seed/inspection seam, are integrated at the
  baseline.
- Unblocks: S1 scheduler-depth criterion for I1 combined probe.
- Candidate maturity: focused dynamic evidence checkpoint, not an acceptance
  candidate.

## Frozen entry and precheck facts

The evidence exercises the existing real test fixture and production-shaped
task route:

```text
non-zero receiver ProgramExecutionContext
  -> provider_execution_context
  -> OwnedProgramExecutionContext::capture
  -> spawn_provider_stream
  -> tokio::spawn
  -> run_provider_stream
  -> borrow_for_scheduled_task
  -> call_provider_callable
  -> provider failure
  -> finish_provider_stream ordinary error-export borrow
```

R1 exposes only
`ProgramExecutionContext::{with_program_call_depth_for_test,
program_call_depth_for_test}` under `#[cfg(test)] pub(crate)`. The existing
`provider_stream_failure_task` fixture can seed the receiver before provider
derivation without a new assembly or service fixture. S1 already locks the
provider scheduler entry to `borrow_for_scheduled_task()` and locks error export
and `prepared_unary` to ordinary `borrow()`.

## Write scope and completion

Allowed writes:

- this leaf contract;
- test code and mechanical `#[cfg(test)]` probes/helpers only in
  `runtime/eval/src/assembly_execution/async_stream_cancel.rs`.

Forbidden writes include all production behavior, `program_execution` or its
test-only seam, `prepared_unary.rs`, compiler/linker surfaces, V1/V2/V3
evidence, public contracts, stream terminal semantics, and the trampoline.

Completion requires one focused Tokio test to:

1. seed a non-zero receiver depth before deriving and capturing the provider
   stream context;
2. invoke `spawn_provider_stream`, not `run_provider_stream` directly;
3. dynamically observe depth zero immediately at the provider callable
   boundary in the spawned task;
4. dynamically observe the original non-zero depth at the ordinary
   error-export borrow;
5. combine that runtime evidence with the existing source lock that
   `prepared_unary` remains an ordinary continuation and never calls
   `borrow_for_scheduled_task`;
6. leave production, terminal, cancellation, error-export, ABI, artifact, and
   public semantics unchanged.

If the existing R1 seam or provider failure fixture cannot satisfy this matrix
without a production change, another fixture, or a sibling write surface, stop
with `TASK_NOT_EXECUTABLE` or `TASK_SCOPE_EXPANDED`.

## Risk, verification, and evidence validity

Risk is medium because the criterion concerns a scheduler boundary, but the
write set is test-only and colocated. This node uniquely owns:

```bash
cargo fmt --package skiff-runtime-eval -- --check
cargo test -p skiff-runtime-eval provider_stream_spawn_resets_only_the_callable_depth
git diff --check
```

Do not run a selector, full workspace gate, live instance, V1/V2/V3 filter, or
chat smoke. The integrator owns merged-state probes.

Evidence is valid only for this leaf's reported commit/tree. Changes to
`async_stream_cancel.rs`, the scheduled-task borrow, the R1 depth seam, the
provider failure fixture, or relevant Tokio task behavior invalidate it.

## Result

The existing provider failure fixture now accepts a test-only parent depth.
The focused test seeds depth `17` on the receiver before provider derivation,
confirms the captured provider context still holds `17`, and then invokes
`spawn_provider_stream`. Colocated `#[cfg(test)]` atomics observe depth `0` at
the callable boundary inside the spawned task and depth `17` when the actual
provider failure reaches the ordinary error-export borrow. The same test runs
the existing source lock proving `prepared_unary` uses ordinary `borrow()` and
contains no scheduled-task reset.

Passing evidence:

```text
cargo test -p skiff-runtime-eval provider_stream_spawn_resets_only_the_callable_depth -- --nocapture
  1 passed; 0 failed; 423 filtered out
cargo fmt --package skiff-runtime-eval
git diff --check
```

Reverse search confirms `run_provider_stream` is the provider callable reset,
`finish_provider_stream` and both `prepared_unary` continuations use ordinary
`borrow()`, and the only depth seam consumed by this leaf remains the R1
`#[cfg(test)] pub(crate)` seed/inspection pair.
