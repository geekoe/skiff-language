# P5-F249 Alias return followed by catch double execution result

## Result

The duplicate execution was in the Runtime's executable-call argument
preparation, not in alias return or `catch` materialization.

`eval_executable_call_with_stream_producer_arg` used to evaluate every ordinary
argument while looking for an inline stream producer. When no producer existed,
it returned `None`; the normal executable-call path then evaluated the same
arguments again. An effectful nested call used as an ordinary argument therefore
ran twice.

The Runtime now performs a side-effect-free producer scan first:

- no producer returns to the normal path before evaluating any argument;
- exactly one producer enters the special path and evaluates every argument once;
- multiple producers fail before evaluating or preparing arguments.

Alias identity and `catch` projection are unchanged.

## Regression coverage

`test-runner/fixtures/alias-return-catch-once` is a canonical
source-to-package-to-assembly-to-isolated-Runtime fixture. Its six tests cover:

- direct `bytes` alias return and receiver chaining;
- catch success and typed-error paths;
- the nested executable-call argument that reproduced double execution;
- a fresh-copy, non-alias control;
- optional narrowing before the alias receiver and catch.

Before the Runtime fix, the nested success case observed two state mutations.
After the fix, every case observes exactly one.

The fixture is registered in the checked-in canonical source-test registry.

## Validation

- Canonical fixture: 6/6 passed.
- `cargo test -p skiff-runtime-eval --no-fail-fast`: 132/132 passed.
- `node --test scripts/tests/skiff-source-test-suite.test.mjs`: 8/8 passed.
- `cargo check --workspace`: passed.
- `git diff --check`: passed.

## Relay receipt

The requested Relay 23-test receipt could not reach test execution on the
current integration inputs:

- the current Internals F247 Relay source no longer contains its `package.yml`,
  so the canonical runner correctly rejects the single test file as outside a
  package source root;
- restoring the last canonical Relay `package.yml` in a temporary fixture
  reaches compilation, but the current Relay source/package artifacts have
  independent type-check failures in `chatgpt_oauth`, `completed_responses`, and
  `upstream_sources`.

No Relay production source was rewritten and no workaround was added. These
failures happen before Runtime execution and are independent of the
single-evaluation fix.

No stable instance was operated, no changes were pushed, and no disk cleanup was
performed.
