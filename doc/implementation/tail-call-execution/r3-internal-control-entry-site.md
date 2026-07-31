# R3 internal control and tail-entry handoff

Status: `IMPLEMENTED`

This leaf executes the R3 production repair from
[`a1-finding-wave-checkpoint.md`](./a1-finding-wave-checkpoint.md). It does not
change tail-call design or acceptance scope.

## Authority trace and exact input

The direct parent is
[`a1-finding-wave-checkpoint.md`](./a1-finding-wave-checkpoint.md). Its parent
chain continues through `parent-checkpoint.md` and `f0-ready-to-freeze.md` to
the unique semantic authorities:

1. [`../../architecture/tail-call-execution.md`](../../architecture/tail-call-execution.md);
2. [`../../reference/runtime.md#tail-call-execution-and-recursive-stack-safety`](../../reference/runtime.md#tail-call-execution-and-recursive-stack-safety).

The implementation baseline is commit
`0583e9e097d8883644e5b6e1fb4d21055cbd05d6`, tree
`93691e7fb6193ea15fb8fa4c3bc4cfab8d6f2637`. The worktree is
`/Users/geek/workspace/tco-r3-internal-control` on branch
`agent/tco-r3-internal-control`. The integration owner is
`/root/tco_integrator`.

## DAG position and established code facts

R3 is ready from the finding-wave checkpoint and is the wave's only production
writer. Integrated R3 unblocks the test-only N1 barrier/target lane and E1
carrier/error/entry-site lane. T1 is independent.

Object-only preflight established:

- `runtime/eval/src/lib.rs` publicly exports `env`, while the pre-candidate
  `env::Flow` variants are exactly `Continue`, `Return`, `Break`,
  `LoopContinue`, `Parked`, and `ContinueConsumer`.
- The candidate added `Flow::TailCall(Box<PreparedTailCall>)` and suppressed
  its public/private mismatch. All production consumption remains inside the
  evaluator crate.
- `Interpreter::exec_program_executable` is the sole iterative trampoline.
  `EvalContext::exec_program_return` creates prepared frames, and nested
  block/statement execution only propagates evaluator control to that loop.
- A tail handoff accounts the current edge, destructures
  `PreparedTailCall` with `tail_site: _`, then runs target resolution,
  `EvalContext::new_callable`, two function-entry checkpoints, entry block
  lookup, and the target body. Consequently entry-checkpoint errors cannot be
  promoted at the current edge without also risking promotion of target-body
  throws.

The minimal seam is one crate-private evaluator control enum containing either
an ordinary public `Flow` completion or the boxed `PreparedTailCall`. Public and
barrier-facing methods convert only ordinary completion and fail closed if an
internal tail frame escapes. The existing trampoline is the only tail consumer.

## Write ownership

Production ownership is limited to:

- `runtime/eval/src/env.rs`;
- `runtime/eval/src/eval_context.rs`;
- `runtime/eval/src/eval_context/concurrent.rs`;
- `runtime/eval/src/flow_completion.rs`;
- `runtime/eval/src/program_db.rs`;
- `runtime/eval/src/program_execution.rs`;
- `runtime/eval/src/program_execution/tail_call.rs`;
- `runtime/eval/src/program_invocation.rs`;
- `runtime/eval/src/program_stream.rs`;
- only directly required same-crate `Flow` consumers discovered by compiler or
  reverse search.

The leaf may keep its small internal control layout regression adjacent to the
crate-private type. It must not modify N1 files, E1 files, T1, artifacts,
schemas, configuration, public API authority, dependencies, lockfiles, or
generated output.

## Required implementation

1. Restore the baseline public `Flow` variant set and remove the
   `private_interfaces` suppression. Keep both the tail variant and prepared
   payload crate-private; do not publish a wrapper or payload.
2. Use one crate-private `Complete(Flow) / TailCall(Box<PreparedTailCall>)`
   evaluator seam. Preserve the unique existing trampoline and do not add a
   context side channel, marker, spawn, second loop, or outer trampoline.
3. Split callable entry preparation from entry body execution without changing
   checkpoint order or units. Retain `PreparedTailCall.tail_site` through target
   resolution, callable construction, both function-entry checkpoints, and
   entry block lookup. Promote errors from transfer and that entry preparation
   at the current site exactly once. Do not promote target-body errors.
4. Preserve tail eligibility, argument evaluation order, return-plan
   equivalence, heap carriers, explicit/generic self, call-depth policy,
   barriers, scheduler behavior, and the fixed real non-tail stack prefix.
5. Convert mechanical `Flow` consumers back to exhaustive handling of only the
   baseline variants. Impossible internal escape at a public/barrier seam must
   fail closed rather than iterate.

## Completion and evidence

This is a high-risk implementation checkpoint, not a stable acceptance
candidate. Completion requires:

- the public `Flow` variant set is baseline-identical;
- exactly one production loop consumes `PreparedTailCall`;
- `tail_site` remains live until callable entry preparation succeeds;
- no duplicate promotion path can add the same current edge;
- the internal control layout test is non-zero and proves the prepared frame
  remains boxed;
- formatting, strict private-interface compilation, focused internal-control
  and tail-entry tests, and reverse searches pass.

The leaf owns only these focused commands:

```bash
cargo fmt --check
RUSTFLAGS='-Dprivate_interfaces' cargo check -p skiff-runtime-eval -p runtime
cargo test -p skiff-runtime-eval tail_call_internal_control_layout -- --nocapture
cargo test -p skiff-runtime-eval tail_call_entry_checkpoint -- --nocapture
```

The entry-checkpoint filter is permitted to be zero here because its test file
is exclusively E1-owned; R3 records the absence and leaves the final non-zero
entry-site evidence to E1. R3 does not run selectors or a full gate.

Reverse searches must show:

```text
Flow::TailCall                 absent
allow(private_interfaces)     absent on the evaluator control path
EvaluatorControl::TailCall    created in return lowering and consumed by one loop
PreparedTailCall.tail_site     not discarded before entry preparation
```

Evidence is valid only for the final R3 commit/tree. Any later change to
evaluator control, callable entry/checkpoints, projection/resolution, heap,
exception promotion, return materialization, scheduler/barriers, dependencies,
or toolchain invalidates the affected evidence.

## Stop conditions and handoff

Stop with `TASK_SCOPE_EXPANDED` if closure requires a public contract change,
second production owner, N1/E1/T1 writes, artifact/schema/config changes, or a
new control side channel/trampoline. Otherwise commit the contract, production
repair, and minimal adjacent regression together, then report branch,
worktree, commit/tree, actual write set, reverse-search evidence, and focused
results directly to `/root/tco_integrator` and notify `/root`.

## Implementation result

The implementation restores the six baseline public `Flow` variants and moves
`PreparedTailCall` behind crate-private
`EvaluatorControl::{Complete, TailCall}`. The prepared frame retains both the
tail instruction site and its caller address, so the single trampoline can use
the caller's exact type context after the eliminated evaluator frame is gone.
The trampoline chooses one execution projection, accounts each transfer, and
then promotes target resolution plus both callable-entry checkpoints and entry
block lookup at the retained site. Entry body execution begins only after that
promotion boundary and is returned unchanged.

Actual production write set:

- `runtime/eval/src/env.rs`;
- `runtime/eval/src/eval_context.rs`;
- `runtime/eval/src/eval_context/concurrent.rs`;
- `runtime/eval/src/flow_completion.rs`;
- `runtime/eval/src/program_db.rs`;
- `runtime/eval/src/program_execution.rs`;
- `runtime/eval/src/program_execution/tail_call.rs`;
- `runtime/eval/src/program_invocation.rs`;
- `runtime/eval/src/program_stream.rs`.

Evidence on the uncommitted final source state:

- `cargo fmt --all`, `git diff --check`: pass;
- `RUSTFLAGS='-Dprivate_interfaces' cargo check -p skiff-runtime-eval -p runtime`:
  pass;
- `cargo test -p skiff-runtime-eval tail_call_internal_control_layout -- --nocapture`:
  one test, pass;
- `cargo test -p skiff-runtime-eval tail_call_shared_trampoline -- --nocapture`:
  one test, pass;
- `cargo test -p skiff-runtime-eval tail_call_transfer_accounts -- --nocapture`:
  one test, pass;
- `cargo test -p skiff-runtime-eval assembly_tail_call_direct_branch -- --nocapture`:
  one test, pass;
- `cargo test -p skiff-runtime-eval tail_call_entry_checkpoint -- --nocapture`:
  zero tests, as expected until the exclusively E1-owned entry-site fixture is
  integrated.

Reverse search finds no `Flow::TailCall`, `allow(private_interfaces)`, or
discarded `tail_site: _`. `EvaluatorControl::TailCall` is created only by
return lowering, propagated through transparent evaluator control, and consumed
only by the one loop in `Interpreter::exec_program_executable`.
