# P5-F202 Assembly database execution projection

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

F192 restored canonical package-test assembly linking. The real official
`http-session` package now links and activates, and all 13 tests that do not
use database state pass. Its six database tests fail with HTTP 500:

```text
assembly interpreter has no legacy RuntimeProgram projection
```

The assembly has already been prepared and committed, and F197's typed state
binding is present. The first production owner is
`runtime/eval/src/program_db.rs`: `eval_program_db_operation_with_context`
unconditionally asks for `program_projection()` before executing a database
command. `runtime/eval/src/db_eval.rs` has the same legacy-only dependency in
recoverable database planning.

Canonical assembly execution must not synthesize or depend on a legacy
`RuntimeProgram`.

## Required implementation

1. Reproduce the failure with an assembly-backed database operation.
2. Make database command planning and recoverable database planning obtain
   their type view from the current execution context, using the canonical
   assembly execution image for assembly execution and the existing program
   projection only for legacy execution.
3. Reuse the existing `RuntimeExecutionProjection::for_context` abstraction,
   or tighten that abstraction if necessary. Do not add an assembly-to-legacy
   projection adapter.
4. Preserve exact state binding and namespace consumption. Do not introduce a
   default database, global namespace, dynamic type fallback, or filesystem
   lookup.
5. Keep malformed/missing assembly type information fail-closed with a precise
   error.
6. Cover ordinary database commands and the recoverable/transaction path.

## Acceptance

- Focused Runtime eval tests cover both assembly-backed database paths.
- Negative tests prove missing or inconsistent assembly type information is
  rejected.
- Existing legacy execution tests remain green.
- Real `http-session` package tests pass all 19 cases through the isolated
  package-test Runtime.
- `cargo check --workspace` and `git diff --check` pass.
- Add `P5-F202-assembly-database-execution-projection-result.md` with exact
  evidence and any next independent blocker.
- Commit the work on the task branch. Do not push and do not operate the shared
  stable instance.

## Authority

Read this task first. Follow only the directly referenced implementation and
the execution abstractions it calls. If an architectural decision not covered
above is required, stop and ask the primary agent.
