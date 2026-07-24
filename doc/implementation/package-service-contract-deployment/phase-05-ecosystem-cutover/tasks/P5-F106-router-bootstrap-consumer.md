# P5-F106 Router bootstrap consumer and config owner

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §12–§13.
- Shared wire checkpoint: F103 / commit `f5b1a3e`.
- Audit input: P5-D77.

## DAG node

Make Router the only configuration owner for the connection bootstrap and emit it before Runtime
activation/registration.

## Write scope

- Router config/schema/example and runtime-WebSocket session composition.
- Focused Router config/session protocol tests.

Do not implement filesystem snapshot loading, remove the activation backend, modify Runtime Rust consumers,
compiler/tooling or stable instance.

## Required semantics

- Replace plural `artifactRoots` with required singular `artifactsPath`.
- Router config remains the only owner of `serviceDb.mongoUrl`.
- Resolve a relative configured artifact path against Router config directory; emit the normalized absolute
  string in exactly one `router.bootstrap`.
- Bootstrap is the first Router control for a connection and precedes activation/register traffic.
- Do not emit Registry identity/endpoint, legacy `router.control.artifactRoots`, or prepare/commit serviceDb.

## Acceptance

- Positive session test observes one exact bootstrap before other control.
- Missing/empty path or Mongo URL fails startup; relative path normalizes deterministically.
- No second bootstrap is emitted during reload/activation.
- Router type-check, focused tests and `git diff --check` pass.

Risk: high, production connection/config boundary. No merge/push/stable.
