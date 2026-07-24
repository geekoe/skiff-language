# P5-F107 Runtime bootstrap consumer

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §11–§13.
- Shared wire checkpoint: F103 / commit `f5b1a3e`.
- Audit inputs: P5-D75 and P5-D77.

## DAG node

Make Runtime obtain artifact loading and DB transport configuration only from the connection bootstrap.

## Write scope

- Runtime driver/host Router session state and filesystem assembly resolver composition.
- Runtime config schema/example/docs directly affected by removing artifact-root ownership.
- Focused Runtime host/transport tests.

Do not modify Router TypeScript, compiler/tooling, Registry, artifact schemas or stable instance.

## Required semantics

- Remove Runtime-owned artifact root and Mongo URL config/env/defaults.
- Before bootstrap, activation/register fails closed.
- First valid bootstrap fixes `artifactsPath` and DB transport binding for the connection.
- Duplicate/conflicting bootstrap fails closed.
- Assembly prepare resolves exact content from the fixed path.
- Runtime provisions `std.db` only for activation requirements/bindings; service never sees Mongo URL.
- Remove prepare/commit `serviceDb` consumption introduced by the superseded transient wire.

## Acceptance

- Focused tests cover missing bootstrap, positive load/provision, duplicate/conflict and activation isolation.
- Runtime config rejects or no longer accepts artifact-root/Mongo ownership.
- Relevant host/transport checks and `git diff --check` pass.

Risk: high, production runtime configuration and capability boundary. No merge/push/stable.
