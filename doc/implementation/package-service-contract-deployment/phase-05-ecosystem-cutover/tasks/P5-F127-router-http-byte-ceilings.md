# P5-F127 Router HTTP byte ceilings

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §12.
- Entering wire checkpoint: F126 `2ac536b`.

## DAG node

Own the instance-wide HTTP request/response ceilings in Router config, enforcement and Runtime bootstrap.

## Write scope

- Router config/schema/example/CLI overrides.
- Router HTTP request/response gateway accounting and bootstrap producer.
- Focused Router tests.

Do not modify Runtime Rust execution, service manifests, source repos or stable.

## Required semantics

```yaml
http:
  port: 4000
  maxRequestBytes: <required positive integer>
  maxResponseBytes: <required positive integer>
```

- Delete `bodyLimitBytes` and all defaults/aliases.
- Reject missing/zero/fractional/overflow values.
- Enforce request bytes before Runtime dispatch.
- Enforce response bytes cumulatively across one HTTP response, including streaming chunks.
- Emit only `maxResponseBytes` in the one connection bootstrap.
- No per-service override and no WebSocket reuse.

## Acceptance

- Config positive/negative tests, unary request/response and multi-chunk cumulative response probes pass.
- Bootstrap producer supplies the exact configured value.
- Router type-check, focused tests and `git diff --check` pass.

Risk: high, external HTTP boundary. No merge/push/stable.
