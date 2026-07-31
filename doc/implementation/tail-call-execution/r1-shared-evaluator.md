# R1 shared evaluator execution contract

Status: implementation complete; awaiting integration

Direct parent:
[`parent-checkpoint.md`](./parent-checkpoint.md). That checkpoint traces to the
single architecture authority and the runtime reference contract. This leaf
only records implementation facts and does not redefine those semantics.

## Identity and DAG position

- Repository: `/Users/geek/workspace/skiff`
- Baseline commit/tree:
  `c34a954bca3580533c153d5761e8805c423dbb09` /
  `8beb99c62fb2bf2f4fade9f41c855773c2e8a714`
- Branch/worktree:
  `codex/tco-r1-runtime` /
  `/Users/geek/workspace/skiff-tco-r1-runtime`
- Integration owner/branch: `/root/tco_integrator` /
  `codex/tco-integration`
- Dependency: P0 is integrated in the baseline.
- Completion unblocks V1 legacy/safety/pressure, V2 canonical assembly, and V3
  source-path evidence.
- Candidate maturity: implementation checkpoint. R1 does not freeze or accept a
  stable candidate.

## Frozen implementation closure

The existing linked `Return -> ExprRef -> Call -> ExecutableAddr` chain is
sufficient. R1 will add exactly one internal owned prepared tail frame and
exactly one iterative evaluator loop:

1. `EvalContext` recognizes only a direct `LinkedStmtIr::Return` whose referenced
   expression is a `LinkedExprIr::Call` with exact
   `LinkedCallTarget::Executable`.
2. It proves the lexical tail-transfer context is transparent, proves there is
   no deferred or stream-producing argument path, evaluates arguments once in
   existing left-to-right order, resolves the target in the already selected
   legacy/assembly projection, creates the callee env with existing generic and
   self rules, and compares instantiated return plans.
3. An eligible result becomes the single internal `Flow::TailCall` frame.
   Ineligible targets, barriers, stream paths, or return plans fail closed to the
   existing ordinary call using already evaluated carriers where applicable.
4. `Interpreter::exec_program_executable` owns the only trampoline. Each tail
   transfer charges and polls the existing execution budget, replaces the
   current addr/env without pushing program-call depth or local diagnostic
   frames, and executes the next body in the same request context and heap.
5. Implicit-self, explicit-self, legacy, assembly, and ordinary top-level
   invocation entries all reach that same loop. Ordinary nested calls alone
   retain `enter_program_call()`.

The final return is materialized once by the existing caller/entry boundary
against the common proven return plan. Preparation and current-edge accounting
errors use the current tail site; target-body errors retain only the real
non-tail prefix.

## Lexical propagation and fail-closed barriers

Ordinary blocks, `if`, statement `match`, and array/map loop bodies inherit the
transparent lexical context and propagate `Flow::TailCall` after their existing
env pop. A newly entered executable always starts transparent.

The following owners explicitly run nested same-executable evaluation with a
barrier context, so `return exactCall(...)` takes the ordinary path before the
owner completes its continuation:

- timeout statement scopes;
- concurrent statement/serial lanes;
- DB transaction and lease bodies;
- expression value blocks;
- stream-consumer bodies and stream-producing/deferred argument paths.

Catch and other expression wrappers are structurally non-direct return calls.
All exhaustive `Flow` consumers are closed mechanically. A `TailCall` reaching
a terminal/value/entry policy or a barrier owner is an invalid internal state,
not another trampoline or a cleanup bypass.

## Expected write ownership

Production owner:

- `runtime/eval/src/env.rs`
- `runtime/eval/src/eval_context.rs`
- `runtime/eval/src/eval_context/{timeout,concurrent}.rs`
- `runtime/eval/src/program_execution.rs`
- `runtime/eval/src/program_execution/tail_call.rs`
- `runtime/eval/src/flow_completion.rs`
- `runtime/eval/src/program_db.rs`
- `runtime/eval/src/program_stream.rs`
- `runtime/eval/src/program_invocation.rs`
- any other `runtime/eval/src` file required only to close an exhaustive
  `Flow` consumer.

Focused R1 tests may be added under `runtime/eval/src/program_execution/`; they
will cover the internal plan comparison, one shared-loop legacy execution probe,
and fail-closed lexical context without taking V1/V2/V3 evidence ownership.
Because `ProgramExecutionContext` is R1-owned, R1 also provides S1 with the
smallest `#[cfg(test)] pub(crate)` depth seed/inspection seam needed to prove an
independent provider task resets a non-zero caller depth. This is test-only
mechanical support, not a production or public contract.

The expected owner list is not permission to change behavior outside this
closure. Any additional file must be a mechanical consumer/constructor and be
reported in the result.

## Forbidden surfaces and non-goals

- Do not modify S1's
  `runtime/eval/src/assembly_execution/async_stream_cancel.rs` provider task or
  its focused tests.
- Do not modify C1 compiler/lowering/linker tests, V1 runtime driver tests, V2
  assembly ordinary tests, or V3 `.skiff` fixtures.
- Do not change artifact/File IR/schema/config/manifest/keyword surfaces.
- Do not add a second trampoline, projection fallback, heap continuation,
  ordinary tail-call task, or stream terminal behavior.
- Do not extend eligibility to PackageDirect, dynamic interface/const receiver,
  service, Actor, callback, native/builtin, emit, spawn, or stream defer.
- Do not change the depth constant or budget/deadline/internal-stop semantics.

Return `TASK_SCOPE_EXPANDED` before implementation if closure needs a public
contract, persisted marker, second lifecycle/trampoline owner, or a sibling
write surface.

## Completion and evidence

R1 is complete when:

- direct exact executable returns use the sole prepared frame and sole loop in
  both projections;
- argument, generic/self, heap carrier, return-plan, budget, error-site, and
  diagnostic-prefix rules above are preserved;
- tail transfers do not push active non-tail depth, while every ordinary nested
  local executable call still does;
- all listed lexical barriers select ordinary evaluation and all `Flow`
  consumers compile with fail-closed handling;
- reverse search finds no persisted marker, alternate loop, projection
  conversion, or tail-call spawn.

R1 uniquely owns these development checks:

```bash
cargo fmt --all -- --check
cargo check -p skiff-runtime-eval
cargo test -p skiff-runtime-eval tail_call
node scripts/verify.mjs --only runtime
```

The full gate, compiler selector, source selector, driver pressure tests, and
assembly evidence are owned by later DAG nodes. Evidence is valid only for the
reported R1 commit/tree; changes to evaluator production code, runtime model
dependencies, generated artifacts, or relevant build configuration invalidate
it.

## Result and evidence

Implemented the frozen closure with one crate-private `PreparedTailCall`, one
`Flow::TailCall` carrier, and one absorbing loop in
`Interpreter::exec_program_executable`. `EvalContext::new_callable` is the sole
transparent constructor; the general constructor and nested continuation
owners are barriers. The shared preparation path resolves the selected runtime
projection, preserves argument/self/generic/env construction, and requires
canonical instantiated return-plan equivalence before replacement.

Focused tests live in
`runtime/eval/src/program_execution/execution_scope_tests/tail_call_execution.rs`.
They prove a direct exact tail transfer succeeds at the seeded ordinary
call-depth limit and the same return under a lexical barrier falls back to the
ordinary depth-checked call. R1 also exposes the requested test-only
`ProgramExecutionContext` depth seed/inspection seam for S1.

Passing checks:

```text
cargo fmt --all -- --check
cargo check -p skiff-runtime-eval
cargo test -p skiff-runtime-eval tail_call -- --nocapture
  2 passed; 0 failed; 420 filtered out
node scripts/check-runtime-execution-boundaries.mjs --self-test
node scripts/check-runtime-execution-boundaries.mjs
node scripts/check-runtime-eval-error-boundary.mjs --self-test
node scripts/check-runtime-eval-error-boundary.mjs
node scripts/check-runtime-artifact-boundaries.mjs --self-test
node scripts/check-runtime-artifact-boundaries.mjs
git diff --check
```

`node scripts/verify.mjs --only runtime --list` shows that the selector includes
the full `runtime` driver suite. Its baseline
`runtime_program_recursion_fails_before_exhausting_the_worker_stack` fixture is
an unbudgeted exact direct `Return(Call)` loop. It becomes intentionally
iterative under R1 and has no cooperative yield, so running that selector before
V1 replaces the fixture could monopolize the current-thread executor instead of
reaching its timeout. The selector is therefore masked by the declared V1
dependency and was not run on the R1-only branch.
