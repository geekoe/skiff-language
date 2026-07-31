# V1 legacy tail-call safety and pressure evidence

Status: handoff-ready; focused evidence passes, unique runtime selector records a
baseline stack-overflow failure

Repository: `/Users/geek/workspace/skiff`

Baseline: `bc4fea09685c6e92ed3fb5aa523ebc7cac6ba2df` /
`910bc17d1e752c81ae10a2105a77d89a113a292c`

Branch / worktree:

- `codex/tco-v1-legacy-safety-pressure`
- `/Users/geek/workspace/skiff-tco-v1-legacy-pressure`

Integration: `/root/tco_integrator`

## Authority and trace

The direct parent is
[`parent-checkpoint.md`](parent-checkpoint.md). It traces the execution DAG to:

1. [`../../architecture/tail-call-execution.md`](../../architecture/tail-call-execution.md),
   the unique internal architecture authority;
2. [`../../reference/runtime.md#tail-call-execution-and-recursive-stack-safety`](../../reference/runtime.md#tail-call-execution-and-recursive-stack-safety),
   the user-visible semantics;
3. the parent checkpoint;
4. this leaf contract and its result.

This file only owns evidence execution details. It does not redefine tail
position, runtime behavior, or public error semantics.

## DAG position and current shared state

V1 was blocked by R1. The baseline has R1's shared legacy/assembly evaluator,
single prepared tail frame, lexical barrier policy, and the only trampoline
integrated. V1 supplies the legacy, safety, accounting, diagnostic-space, and
small-worker-stack evidence required before I1.

Successful integration unblocks the combined I1 probe. The current code state is
an implementation checkpoint, not a frozen acceptance candidate.

The canonical legacy entry is:

```text
Interpreter::with_program
  -> execute_test_program_route
  -> call_program_executable[_carriers]
  -> exec_program_executable
  -> EvalContext return recognition
  -> shared program_execution trampoline
```

The old `recursive_executable()` fixture is a direct `Return(Call)` with a
disabled budget. It masks R1 by becoming an unbounded non-yielding tail loop.
V1 must replace that guard fixture with genuine non-tail recursion before the
runtime selector is meaningful.

## Owned write set

V1 owns:

- `runtime/driver/eval/tests/program_execution.rs`;
- `runtime/driver/eval/tests/program_execution/tail_call_execution.rs`, kept
  separate because the existing driver test owner is already several thousand
  lines long;
- this leaf contract/result;
- only if the driver seam cannot express a required assertion, test-only
  mechanical closure in
  `runtime/eval/src/program_execution/execution_scope_tests/tail_call_execution.rs`.

No production file is writable. V1 must not modify V2 assembly tests, the V3
source fixture, S1 `async_stream_cancel`, compiler/lowering/linker files,
artifact shapes, schemas, selectors, or runtime configuration.

## Completion criteria

The legacy projection must prove all of the following on the R1-integrated
baseline:

1. The former guard fixture is genuinely non-tail. Active non-tail depth 32 can
   enter and the next frame fails with ordinary
   `ResourceLimitExceeded(resource = "programCallDepth")`; a subsequent
   invocation on the same Tokio runtime succeeds.
2. Direct and same-file mutual eligible calls execute through the shared
   trampoline and return the correct finite result.
3. A finite tail chain has exact per-hop instruction accounting relative to the
   corresponding ordinary-call accounting seam. The driver proves linear
   finite-hop accounting; the shared evaluator seam proves one eliminated
   transfer consumes the same units as its ordinary-call fallback.
4. Infinite eligible tail recursion is terminated by a small instruction limit
   as `TimeoutError(reason = "instructionLimitExceeded")`, never as
   `programCallDepth`.
5. A one-worker Tokio runtime with an actual 1 MiB worker stack executes a
   100,000-hop tail chain inside `tokio::spawn` and validates the terminal
   result.
6. A failure reached after 100,000 tail hops has a bounded diagnostic stack:
   eliminated tail edges do not accumulate frames, while the final error and
   its current source attribution remain observable.
7. The minimum legacy negative set confirms that a non-direct recursive call
   shape remains ordinary. Existing R1 lexical-barrier tests remain the
   canonical test-only seam; V1 does not duplicate the V2 negative matrix.

The assertions must use existing `ProgramTestInvocation.execution_budget`,
legacy linked IR builders, structured public payloads, and the shared R1
trampoline. Test-only bypasses, a second loop, `tokio::spawn` per hop, and
production seams are forbidden.

## Risk and masking

Risk is high because the old disabled-budget tail fixture can hang the selector
and because root-future polling does not prove worker-stack safety. The 1 MiB
workload therefore runs inside `tokio::spawn` on the configured worker, and the
guard fixture is repaired before the selector is run.

R1 compile/ordinary-return failures mask every V1 dynamic result. Budget
misclassification masks infinite-tail evidence. A failure to enter a spawned
worker masks the pressure proof. Any required production change is
`TASK_NOT_EXECUTABLE` and must be returned to the R1 owner rather than fixed
here.

## Verification ownership

V1 first runs non-overlapping focused Cargo filters for its final test names,
then uniquely runs R1's previously deferred runtime selector after the masking
fixture is repaired:

```bash
cargo test -p runtime runtime_program_non_tail_recursion
cargo test -p runtime runtime_program_legacy_tail_call
cargo test -p skiff-runtime-eval tail_call_
node scripts/verify.mjs --only runtime
```

V1 does not run `pnpm verify`, compiler, skiff-tests, tooling, assembly-only
matrices, live selectors, or the full gate.

Evidence is valid only for the final implementation commit/tree. Changes to R1
production owners, the driver fixture, execution budget implementation, runtime
selector graph, Tokio worker configuration, or relevant dependencies invalidate
the affected evidence.

## Stop conditions and handoff

Stop with `TASK_SCOPE_EXPANDED` for a public-contract change, production write,
second trampoline/lifecycle owner, persisted marker, or sibling write collision.
Stop with `TASK_NOT_EXECUTABLE` if the R1 production implementation cannot meet
the criteria through the existing legacy seam.

On success, commit the contract and tests, report branch/worktree,
implementation commit/tree, actual write set, focused and selector evidence, and
the self-acceptance matrix directly to `/root/tco_integrator`, then notify the
root agent. Do not merge or push.

## Implementation result

V1 replaces the obsolete disabled-budget `Return(Call)` recursion guard with a
counted-down `return 1 + recurse(...)` fixture. It proves that depth 32 enters,
depth 33 fails with the structured `programCallDepth` resource diagnostic, and
the same interpreter and Tokio runtime still serve a subsequent request.

The dedicated legacy driver module covers direct and same-file mutual
transfers, exact linear finite-hop instruction use, instruction-limit
termination of an infinite eligible loop, and a single-frame terminal
diagnostic after 100,000 transfers. The test-only shared-evaluator closure
compares an eligible transfer with the return-plan-mismatch ordinary fallback
and runs a 100,000-hop workload inside `tokio::spawn` on a Tokio
current-thread runtime hosted by an actual 1 MiB stack thread.

No production file, artifact/schema shape, selector, configuration, or sibling
evidence owner changed.

### Verification result

Focused evidence on this worktree:

| Command | Result |
| --- | --- |
| `cargo test -p runtime runtime_program_non_tail_recursion_fails_at_guard_and_stays_healthy -- --nocapture` | pass; 1 test |
| `cargo test -p runtime runtime_program_legacy_tail_call -- --nocapture` | pass; 4 tests, including the 100,000-hop diagnostic |
| `cargo test -p skiff-runtime-eval tail_call_ -- --nocapture` | pass; 4 tests, including ordinary/tail accounting parity and the 1 MiB / 100,000-hop task |
| `cargo fmt --all -- --check` | pass |
| `git diff --check` | pass |

The unique `node scripts/verify.mjs --only runtime` invocation completed with
exit 1. All three runtime boundary self-test/production phases passed. The Rust
phase reported failures for `-p runtime --lib` and
`-p skiff-runtime-eval --lib`; the retained output identifies an aborting stack
overflow in the pre-existing
`runtime_program_executes_bytes_natives_without_json_registry` test. That exact
focused test also stack-overflows from a detached, unmodified baseline
`bc4fea09685c6e92ed3fb5aa523ebc7cac6ba2df`, so the identified selector failure
is not introduced by V1. Per unique-selector ownership, V1 did not rerun the
selector. Every V1-owned focused filter passes after the selector result.

### Self-acceptance matrix

| Criterion | Evidence | Verdict |
| --- | --- | --- |
| Genuine non-tail fuse and recovery | depth 32/33 structured guard test plus same-interpreter health probe | pass |
| Legacy direct and mutual correctness | driver direct 512 and mutual 513 countdowns | pass |
| Finite accounting | driver 0/1/20 linearity plus shared evaluator tail/ordinary parity | pass |
| Infinite-tail stop reason | small instruction limit returns `instructionLimitExceeded`, not depth | pass |
| Native stack bound | 100,000-hop result in a spawned task on an actual 1 MiB thread | pass |
| Diagnostic-space bound | 100,000-hop terminal error retains one current local source frame | pass |
| Ordinary fallback retained | non-tail binary shape and return-plan mismatch use ordinary calls; existing lexical-barrier test remains green | pass |
| Scope discipline | only owned driver/docs and permitted test-only evaluator closure changed | pass |
| Unique selector | invoked once; baseline-reproduced existing stack-overflow failure recorded without rerun | recorded baseline failure |
