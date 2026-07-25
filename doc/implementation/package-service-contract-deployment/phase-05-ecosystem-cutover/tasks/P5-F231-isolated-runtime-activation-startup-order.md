# P5-F231 Isolated Runtime activation startup order

## Context

Canonical package tests intermittently fail before executing fixtures:

```text
NotWritablePrimary
unknown activation environment skiff-test
```

The isolated supervisor currently spawns Mongo, Router, and Runtime in parallel.
Only afterward, during wait-ready, the parent initializes the replica set,
waits for primary, and upserts the `skiff-test` activation environment.

Router connects immediately, runs `ensureIndexes`, then activation coordinator
initialize/read. It can therefore write before primary election or read before
the environment seed exists.

Router-owned collections include:

- `router_assembly_activation_states`
- `router_assembly_activation_audit`

This is test-runner/isolated-instance startup ordering, not Registry behavior.

## Required implementation

1. Start isolated Mongo first.
2. Initialize the replica set and wait for a writable primary.
3. Seed the exact activation environment and required test control records.
4. Only then start Router and Runtime and wait for their readiness.
5. Preserve correct teardown on failure at every stage; no orphan processes,
   ports, temp data, or leases.
6. Preserve concurrency between components whose declared prerequisites are
   already satisfied, but do not use retries to hide wrong dependency order.
7. Fail with stage-specific diagnostics for Mongo spawn, primary election,
   activation seed, Router startup, and Runtime startup.

## Acceptance

- Unit/integration tests assert the dependency order.
- Failure injection at each stage proves cleanup and precise diagnostics.
- Repeated isolated startup does not produce NotWritablePrimary or unknown
  `skiff-test` environment.
- Real Registry package tests enter and execute their fixtures.
- Official package tests remain green.
- Workspace check, diff check, result document, and commit.
- Do not operate the shared stable instance or push.
