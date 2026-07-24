# P5-F128 Runtime HTTP response ceiling

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §12.
- Entering wire checkpoint: F126 `2ac536b`.

## DAG node

Use bootstrap `http.maxResponseBytes` to stop oversized HTTP responses inside Runtime before Router's final
boundary check.

## Write scope

- Runtime host/request/eval response accounting and focused tests.

Do not modify Router TypeScript/config, service manifests, compiler authoring or stable.

## Required semantics

- The connection bootstrap value is the only Runtime owner; no config/env/default.
- Enforce cumulatively for the entire unary or streaming HTTP response lifecycle.
- Exact boundary succeeds; first byte over the boundary terminates with the canonical response error and
  releases request/generation resources.
- Non-HTTP service calls and WebSocket frames do not consume this budget.
- Missing bootstrap already fails closed.

## Acceptance

- Unary exact/over-limit and multi-chunk cumulative exact/over-limit tests pass.
- Cancellation/error/cleanup paths do not leak pins or buffers.
- Relevant Runtime checks/tests and `git diff --check` pass.

Risk: high, runtime response lifecycle. No merge/push/stable.
