# N1 dynamic barrier and excluded-target negative matrix

Status: `implementation`

This leaf closes the dynamic negative matrix assigned to N1. It is test-only:
if a real owner path permits an internal tail transfer, skips its continuation,
or cannot be tested without changing production, this task stops and reports
the production defect instead of repairing it.

## Authority, parent, and exact input

The direct parent is
[`a1-finding-wave-checkpoint.md`](./a1-finding-wave-checkpoint.md). Its
authority trace ends at:

1. [`../../architecture/tail-call-execution.md`](../../architecture/tail-call-execution.md);
2. [`../../reference/runtime.md#tail-call-execution-and-recursive-stack-safety`](../../reference/runtime.md#tail-call-execution-and-recursive-stack-safety).

The exact integrated R3 input is:

- commit: `22c7c8ae28dc59a9f8282f26bef36f9f33e5c0e3`;
- tree: `0745a89bf1923b36a435eea50743a869664d48d3`;
- branch: `codex/tco-n1-dynamic-negatives`;
- worktree: `/Users/geek/workspace/tco-n1-dynamic-negatives`.

The leaf must not add a public control variant, persisted marker, schema,
configuration, API, eligibility rule, checker, compatibility path, or second
trampoline. R3 production files and E1-owned tests are read-only.

## Zero-worktree preflight

The baseline was inspected only through Git objects before this worktree was
created. The following real owners and reusable test seams make the task
executable without production changes or external services:

- `EvalContext::exec_program_return` recognizes only a direct exact
  `LinkedCallTarget::Executable` when the current `TailCallContext` is
  transparent. All owner-created `EvalContext::new` and block/expression
  helpers start barred.
- canonical assembly tests can drive the real linker and ordinary dispatcher
  through `link_package_fixture`, `RuntimeAssemblyEvalTarget`, and
  `execute_runtime_assembly_addr`;
- timeout and concurrent tests already construct real evaluator IR and inspect
  post-body deadline/scheduler ownership;
- DB transaction and lease tests use the in-memory `FakeDbState`, real
  transaction/claim evaluators, actor segments, request heaps, and terminal
  phase traces; MongoDB and a live instance are not required;
- stream tests already drive the real consumer cleanup and supervised deferred
  producer registries.

For an exact local call inside an owner barrier, the proof is paired:

1. from ordinary depth, a structured value or terminal trace proves the real
   owner continuation ran after the callee;
2. from seeded depth `32` at a direct evaluator seam (or `31` before a
   canonical assembly entry), the same call must produce structured
   `programCallDepth`, proving it stayed on the ordinary nested-call path.

For service, Actor, native/builtin, and stream-deferred targets, the tests use
their real dispatch/continuation result and assert no crate-private evaluator
control can escape. A synthetic generic barrier alone is not evidence.

## Exclusive write set

N1 owns only:

- new
  `runtime/eval/src/assembly_execution/ordinary/tests/tail_call_negatives.rs`;
- the one module registration line in
  `runtime/eval/src/assembly_execution/ordinary/tests.rs`;
- `runtime/eval/src/program_execution/execution_scope_tests/evaluator_timeout.rs`;
- `runtime/eval/src/eval_context/concurrent/tests.rs`;
- `runtime/eval/src/program_db/tests/transaction.rs`;
- `runtime/eval/src/program_db/tests/lease.rs`;
- causally required test helpers under
  `runtime/eval/src/program_db/tests/fixture/`;
- `runtime/eval/src/program_stream/current_scope_tests.rs`;
- `runtime/eval/src/program_stream/supervised_executable_tests.rs`;
- this leaf contract.

No production file, E1 file, selector, schema, config, manifest, public API, or
source fixture belongs to this leaf.

## Dynamic evidence matrix

| Matrix item | Real path and minimum assertion |
| --- | --- |
| call argument | Canonical assembly outer call evaluates an exact local call as an argument; structured outer result at ordinary depth, `programCallDepth` at seeded depth. |
| catch | Canonical assembly `Catch` owns the nested exact local call and materializes its catch result; structured result at ordinary depth, depth failure at the seeded boundary. |
| timeout | Real timeout expression/body owns a nested exact local call; successful run preserves the post-body timeout/parent-scope result, seeded run reports depth before owner restoration is lost. |
| concurrent | Real lane/value scheduler owns a nested exact local call; successful run preserves arbitration/parent result, seeded run reports depth through lane arbitration. |
| transaction | Explicit transaction result/body evaluates the exact local call before commit; success traces `Begin -> Commit`, seeded failure traces `Begin -> Abort`, with heap rollback/retention and actor ownership asserted. |
| lease | Claim body exact call remains ordinary; both ordinary illegal-flow and seeded depth error still complete lease-lost/release cleanup, retain the binding policy, and leave no renew/heap escape. |
| stream consumer | Real `for-in` return/callee path performs source stop and local consumer cleanup before exposing the structured result or depth error. |
| deferred producer | Real deferred producer preparation/drive retains registry ownership and terminal cleanup; no internal evaluator control is returned. |
| stream-producing argument | Real supervised producer-argument dispatch retains producer/consumer cleanup and returns the consumer result; seeded caller depth must not turn it into a local frame replacement. |
| service | Canonical linked activation-relative service dispatch returns through the service continuation/fresh activation owner; no internal evaluator control escapes. |
| Actor | Real linked Actor dispatch retains admission/owner continuation and returns its dispatch result; it is never treated as an exact local frame replacement. |
| native/builtin | Real native and builtin dispatch return their native values/errors through `eval_program_call`; no local trampoline control escapes. |

Binary/wrapper and `PackageDirect` retain their already integrated focused
evidence and are not duplicated here.

## Focused verification ownership

Every final Cargo filter must select a non-zero number of tests:

```bash
cargo test -p skiff-runtime-eval assembly_tail_call_negative -- --nocapture
cargo test -p skiff-runtime-eval tail_call_negative_timeout -- --nocapture
cargo test -p skiff-runtime-eval tail_call_negative_concurrent -- --nocapture
cargo test -p skiff-runtime-eval tail_call_negative_db_transaction -- --nocapture
cargo test -p skiff-runtime-eval tail_call_negative_db_lease -- --nocapture
cargo test -p skiff-runtime-eval tail_call_negative_stream -- --nocapture
```

The leaf also runs rustfmt on the touched Rust files, `git diff --check`, and a
reverse diff to prove the change is test-only. It does not run a selector,
workspace suite, full gate, live service, or MongoDB.

## Stop conditions

Stop with `TASK_NOT_EXECUTABLE` or `TASK_SCOPE_EXPANDED` if any required matrix
item:

- demonstrates a production continuation/cleanup/dispatch defect;
- requires a production edit, public seam, schema/config/API change, or E1
  file;
- cannot distinguish the real owner path from a synthetic barrier;
- requires external state or a live service not already authorized.

## Result

Pending implementation, focused verification, commit/tree identity, and
integrator handoff.
