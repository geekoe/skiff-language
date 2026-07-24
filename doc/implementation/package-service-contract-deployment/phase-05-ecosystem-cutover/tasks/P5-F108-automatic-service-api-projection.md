# P5-F108 Automatic Service API projection

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §2–§5 and §10.
- Entering checkpoint: F105 integrated as `eb206aa`.
- Audit input: P5-D78.

## DAG node

Generate the canonical code-free ServiceContract directly from one service package compile and its existing
boundary projections.

## Write scope

- Compiler source/projection/driver seams needed to project a service package API.
- Service API identity and operation/schema projection.
- Focused Rust fixtures/tests.

Do not implement service-dependency expression migration, generated ServiceDeployment, public CLI/dev-sync
cleanup, Registry/Internals migration or Runtime/Router changes.

## Required semantics

- `api.yml` remains the only public API list.
- Every `BoundaryCallableProjection::Available` public function automatically becomes one service operation.
- Every Unavailable public function remains package API only and retains stable structured reasons.
- ServiceContract service id comes from `service.yml`; its human version label comes from `package.yml`.
- Version label and provider build/ABI do not participate in Service API identity.
- Contract schemas/types reuse the canonical typed package API representation; no independently authored
  contract definition, type mapping, JSON bridge or manual operation list.
- Reordering source/API entries is identity-stable; changing boundary-observable API changes identity.

## Acceptance

- Positive fixture with Available and Unavailable exports produces exact lists and a closed ServiceContract.
- Identity mutation tests prove version/build changes are irrelevant and API changes are relevant.
- Missing/duplicate operation or unclosed schema fails closed.
- Existing package boundary projection tests and `git diff --check` pass.

Risk: high, canonical service API owner. Candidate: shared implementation checkpoint. No merge/push/stable.
