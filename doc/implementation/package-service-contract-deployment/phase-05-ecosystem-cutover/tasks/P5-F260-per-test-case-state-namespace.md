# P5-F260 Per-test-case state namespace

## Context

Relay test files pass in isolation but fail in the complete suite because
database state survives between test cases. An earlier case creates an enabled
upstream source; later production selection sees it and changes behavior.
Migration cases show the same within-file ordering pollution.

F197 introduced a test-run namespace, but the namespace is currently shared
across multiple cases in one run.

## Required implementation

- Allocate an exact fresh state namespace for every test case execution.
- Bind all declared Package/service state requirements to that case namespace,
  including cross-Package calls.
- Preserve state within one case and isolate retries, files and parallel cases.
- Keep deterministic case identity for diagnostics without deriving a reusable
  database namespace from only file/run identity.
- Clean up or abandon failed-case state without exposing it to another case.

## Acceptance

- Fixtures prove same-case persistence and cross-case isolation in one file,
  across files, sequential and parallel execution.
- Cross-Package state calls remain in the caller's case namespace.
- Relay routes/upstream/migration cases pass in the full suite exactly as they
  do in isolation.
- Runtime/test-runner tests, workspace check, result and commit.
- No push, stable operation or disk cleanup.
