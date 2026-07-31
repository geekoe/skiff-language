# E1 carrier, error, and entry-site evidence

Status: `IMPLEMENTED`

This leaf executes the test-only E1 lane from
[`a1-finding-wave-checkpoint.md`](./a1-finding-wave-checkpoint.md). It adds
dynamic evidence for the integrated R3 control seam; it does not change
tail-call design or production behavior.

## Authority trace and exact input

The direct parent is
[`a1-finding-wave-checkpoint.md`](./a1-finding-wave-checkpoint.md). Its parent
chain continues through `parent-checkpoint.md` and `f0-ready-to-freeze.md` to
the unique semantic authorities:

1. [`../../architecture/tail-call-execution.md`](../../architecture/tail-call-execution.md);
2. [`../../reference/runtime.md#tail-call-execution-and-recursive-stack-safety`](../../reference/runtime.md#tail-call-execution-and-recursive-stack-safety).

The implementation baseline is commit
`22c7c8ae28dc59a9f8282f26bef36f9f33e5c0e3`, tree
`0745a89bf1923b36a435eea50743a869664d48d3`. That baseline contains integrated
R3 and therefore unblocks E1. The worktree is
`/Users/geek/workspace/tco-e1-carrier-error` on branch
`agent/tco-e1-carrier-error`. The integration owner is
`/root/tco_integrator`.

## DAG position and preflight facts

E1 is blocked only by integrated R3, runs test-only in parallel with N1, and
unblocks the I3 combined probe. T1 is already integrated. N1 owns all dynamic
barrier and excluded-target negative files and is forbidden here.

Object-only preflight established:

- `Interpreter::exec_program_executable` is the single prepared-frame
  trampoline. A handoff accounts the transfer with the current `tail_site`,
  then `EvalContext::exec_tail_entry_control` promotes target resolution and
  both callable-entry checkpoints at that same site before target-body
  execution begins.
- The execution-scope tests already expose exact source sites, instruction
  accounting, request exceptions, and the legacy evaluator fixture. A local
  test-only execution control can fail one exact budget poll after both tail
  transfers without modifying the shared fixture or production control.
- The canonical assembly fixture already exercises real package identity,
  linking, activation, exact executable addresses, request-heap carriers,
  generic/self plans, and depth-seeded tail calls. The existing file is long;
  the new carrier matrix will be a nested module under the same basename.
- The driver legacy fixture already enters through a real request route with a
  request trace, typed `std.json.DecodeError`, catch/rethrow primitives,
  100,000-hop pressure, request exception stack/correlation access, and the
  non-tail depth guard. It can add one real ordinary prefix before the
  eliminated tail chain without changing production.

The three evidence lanes have disjoint fixtures and no production dependency
beyond the frozen R3 seam. If a focused test demonstrates that the seam cannot
preserve the specified observable behavior, E1 records the exact repro and
stops instead of repairing production.

## Write ownership

E1 exclusively owns:

- `runtime/eval/src/program_execution/execution_scope_tests/tail_call_execution.rs`;
- `runtime/eval/src/assembly_execution/ordinary/tests/tail_call_execution.rs`;
- nested carrier helpers below
  `runtime/eval/src/assembly_execution/ordinary/tests/tail_call_execution/`;
- `runtime/driver/eval/tests/program_execution/tail_call_execution.rs`;
- this leaf contract.

The nested assembly helper is causal mechanical test closure for the explicitly
authorized basename. E1 must not modify production, N1 files, public API,
artifact/schema/config surfaces, dependencies, lockfiles, generated output, or
any other task contract.

## Required dynamic evidence

### Entry checkpoint attribution

Add one non-zero `tail_call_entry_checkpoint` test that:

1. uses distinct source sites for the preceding and current tail edges;
2. proves the transfer instruction was accounted and its budget poll
   succeeded;
3. crosses the scheduled instruction budget on the next target
   function-entry checkpoint;
4. asserts the resulting request exception source is the current tail edge,
   that the current edge occurs exactly once in the diagnostic stack, and that
   the distinct preceding edge is absent.

The fixture must not fail at transfer accounting and must not accept a generic
instruction-limit error whose site is unobservable.

### Assembly carrier parity

Add a non-zero `assembly_tail_call_carrier` matrix in a nested module. For every
case, execute both an eligible tail call and the corresponding ordinary
materialization path, then compare the observable carrier/value result:

- exact nominal identity and catch identity;
- the selected exact union branch;
- representation wrapping/unwrapping;
- a carrier nested in a container element.

The tail case must use the shared canonical assembly/link/eval route and a
depth seed that would reject an accidental ordinary recursive frame. Ordinary
comparators must retain the real caller materialization rather than manually
constructing the expected output after execution.

### Legacy throw/catch/rethrow chain

Add a non-zero `runtime_program_legacy_tail_call_error` test that uses the real
driver request context while retaining the request heap for payload inspection,
crosses a real non-tail prefix, performs a deep tail chain, throws a typed
payload, catches the exact identity, and rethrows the same exception. Assert:

- payload fields and exact catch identity;
- unchanged correlation, `traceId`, and `errorId`;
- the real non-tail prefix remains in the diagnostic stack;
- eliminated tail edges do not accumulate, and the stack remains bounded
  independently of hop count.

The test must observe the final request exception rather than only a helper or
intermediate catch value.

## Focused evidence ownership

E1 uniquely owns these final non-zero commands:

```bash
cargo test -p skiff-runtime-eval tail_call_entry_checkpoint -- --nocapture
cargo test -p skiff-runtime-eval assembly_tail_call_carrier -- --nocapture
cargo test -p runtime runtime_program_legacy_tail_call_error -- --nocapture
```

It also owns `cargo fmt --all --check` and `git diff --check` for its worktree.
It must not run a selector, the runtime suite, `pnpm verify`, or any full gate.

## Completion matrix and risk

This is a high-risk test-only evidence node. Completion requires all three
filters to select at least one test and pass, with no production diff.

| Requirement | Code evidence | Reverse-search evidence | Dynamic proof |
| --- | --- | --- | --- |
| entry checkpoint current-site attribution | exact scheduled-budget fixture and distinct sites | no production/test hook added | `tail_call_entry_checkpoint` |
| nominal/union/representation/container parity | nested canonical assembly carrier matrix | no flat duplicate carrier owner or production materializer | `assembly_tail_call_carrier` |
| throw/catch/rethrow identity and bounded stack | real driver request and typed catch chain | no replacement exception/correlation constructor in the test | `runtime_program_legacy_tail_call_error` |

The current state is a pre-acceptance implementation candidate. Successful E1
does not freeze a candidate; it only unblocks I3 after N1 also integrates.

Evidence is valid only for the final E1 commit/tree. Changes to R3 evaluator
control, callable entry/checkpoints, call-site promotion, heap/carrier
materialization, exception/catch/correlation handling, assembly linking/type
projection, driver request decoding, dependencies, or toolchain invalidate the
affected lane.

## Stop conditions and handoff

Stop with `TASK_NOT_EXECUTABLE` if an existing test seam cannot observe one of
the mandated outcomes without a production/API/schema/config change. Stop with
an exact repro if R3 is behaviorally wrong. Stop with `TASK_SCOPE_EXPANDED` if
closure requires an N1 file, a new production owner, a public contract, or a
second trampoline/control path.

Otherwise commit the contract and test-only evidence together, then report the
branch, worktree, implementation/result commit and tree, actual write set,
non-zero focused results, formatting/diff evidence, and the self-acceptance
matrix directly to `/root/tco_integrator`; notify `/root` of the same state
transition. Do not merge, clean the一级 worktree, or push.

## Implementation result

E1 closes all three dynamic lanes without a production change.

Actual write set:

- `runtime/eval/src/program_execution/execution_scope_tests/tail_call_execution.rs`;
- `runtime/eval/src/assembly_execution/ordinary/tests/tail_call_execution.rs`;
- `runtime/eval/src/assembly_execution/ordinary/tests/tail_call_execution/carrier_materialization.rs`;
- `runtime/driver/eval/tests/program_execution/tail_call_execution.rs`;
- this leaf contract.

Entry-site evidence uses three real linked executables and two distinct source
sites. Its local scheduled-budget control fails poll 11: the current transfer
poll is 10 and succeeds, while the next target's first function-entry
checkpoint accounts unit 11 and fails. The request exception source and
singleton local stack are exactly the current site, excluding the preceding
edge and duplicate promotion.

The nested assembly module runs two tests. It compares an eligible direct tail
call at depth 31 against a `ValueBlock` ordinary materialization at depth zero.
The cases preserve exact local nominal identity, the exact named-union
`"right"` literal branch, a nominal representation over its unchanged string
payload, and the exact named-union carrier written back into an Array element.

The driver evidence enters with the real `ProgramTestInvocation` request trace,
uses an ordinary catch wrapper as the fixed non-tail prefix, executes 100,000
tail hops, throws a typed `std.json.DecodeError`, catches its exact identity,
extracts the same request-local exception node, and rethrows it. The final
exception retains payload fields `target = "tail.pressure"` and
`message = "terminal"`, correlation
`trace-program / trace-program:local-error:1`, and exactly the ordinary prefix
plus terminal throw frames; no eliminated tail edge accumulates.

Final evidence on the formatted uncommitted implementation state:

- `cargo test -p skiff-runtime-eval tail_call_entry_checkpoint -- --nocapture`:
  one test passed;
- `cargo test -p skiff-runtime-eval assembly_tail_call_carrier -- --nocapture`:
  two tests passed;
- `cargo test -p runtime runtime_program_legacy_tail_call_error -- --nocapture`:
  two tests passed, including the pre-existing 100,000-hop bounded terminal
  test and the new catch/rethrow test;
- `cargo fmt --all --check`: passed;
- `git diff --check`: passed;
- write-set reverse search: only this document and the four authorized
  test-only paths changed; no production/API/schema/config/dependency file
  changed.

The first driver build was environment-blocked by `errno=28` before compiling
the new test. Only this worktree's reproducible Cargo target cache was cleaned
with `cargo clean --manifest-path runtime/Cargo.toml`; the exact focused
commands were then rebuilt and passed. No selector or full gate was run.
