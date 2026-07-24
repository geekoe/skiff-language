# P5-F131 Package public-type boundary closure

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §3–§5.
- Audit input: P5-D81 C1.

## DAG node

Normalize exported same-package nominal types into the canonical package public type closure before boundary
eligibility, instead of leaving them as unsupported `ServiceSymbol`.

## Write scope

- Compiler source/projection input and package artifact public type ownership/closure.
- Boundary eligibility adapters and focused tests.

Do not change call-target/effect/provenance, DB semantics, std HTTP types, real services or runtime.

## Required outcome

- api.yml-exported record/union/enum/container/nullable types resolve to package-owned exact nominal refs and
  close recursively.
- No display-name/structural/JSON bridge.
- Private/missing/open/capability/function types remain unavailable.
- Service API projection maps this closure deterministically to service-owned schema identities.

## Acceptance

- Real-shaped nested public DTO positive and private/capability/open negative probes.
- Registry typed DTO `UnsupportedBoundaryType` reason disappears without weakening other reasons.
- Package/service projection tests and `git diff --check` pass.

Risk: high, canonical public type owner. No merge/push/stable.
