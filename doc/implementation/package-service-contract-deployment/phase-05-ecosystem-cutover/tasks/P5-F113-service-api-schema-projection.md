# P5-F113 Service API schema projection correction

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §3–§5.
- Entering checkpoint: F108 `fe1ed14`.
- Blocker evidence: F110 found `project_service_api` hard-codes an empty boundary schema.

## DAG node

Correct the shared Service API projection so the generated ServiceContract contains the complete typed API
schema needed by consumers. This is an upstream repair checkpoint; it unblocks a fresh F110 retry.

## Write scope

- Package/API projection input and ServiceContract projection/schema/identity.
- Focused projection/identity tests and fixtures.

Do not implement consumer name/expression resolution, generated deployment, CLI visibility, source repo
migrations, Router or Runtime.

## Required semantics

- Starting from `api.yml` exports and exact typed package facts, close every Available operation's reachable
  public record/enum/interface/container/generic schema.
- Preserve an explicit deterministic mapping from source package API nominal declarations to service
  API-owned nominal identities so provider and consumer projections agree without user wrappers.
- Private/unexported or unreachable types do not leak; reachable missing/private schema fails closed.
- Interface/type parameters, nested containers, nullable/error/callback/stream schemas retain exact semantics.
- Schema and operation ordering is deterministic.
- Service API identity includes the complete boundary schema and operations, but excludes human version label
  and provider build/ABI.
- Remove the empty-schema success assertion and add non-empty, nested, mutation and missing-schema tests.

## Acceptance

- A service with record/enum/interface types produces a closed non-empty ServiceContract schema.
- Reordering is identity-stable; boundary type/field changes alter identity.
- Missing/private/unresolved reachable type fails closed.
- F108 projection, package boundary and artifact identity focused tests pass.
- `git diff --check` passes.

Risk: high, canonical service API type owner. No merge/push/stable.
