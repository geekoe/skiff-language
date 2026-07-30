# P5-F114 Complete Service API type schema

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §3–§5.
- Blocker evidence: F113 partial commit `c828e71`.

## DAG node

Extend the canonical boundary schema so automatic Service API projection can preserve the package language's
generic/interface semantics instead of dropping them.

## Write scope

- Canonical artifact-model contract/boundary type schema and strict wire tests.
- Identity framing/cross-system fixtures directly affected by these fields.
- Minimum compiler projection adapters/tests for canonical round-trip.

Do not implement the full F113 schema closure, consumer dependency resolution, Registry/Internals migration,
Router or Runtime execution changes.

## Required schema

- `ContractTypeRef::TypeParam { name }`.
- `ContractTypeShape.typeParams` as an ordered validated declaration list.
- `InterfaceMethodSignature.maySuspend` as a required boolean.
- Generic instantiations continue to use canonical declared nominal/container refs; add no duplicate raw type
  text or display-name identity.
- Strict decode rejects missing/duplicate/unknown/invalid type parameters and missing `maySuspend`.
- Canonical identity includes the new semantic fields.

## Acceptance

- Strict positive wire round-trip for generic record/interface and suspending/non-suspending methods.
- Missing/unknown/duplicate/invalid parameter and omitted suspend flag fail closed.
- Identity mutation proves each semantic change is relevant and ordering normalization deterministic.
- Artifact model, identity and affected compiler projection tests plus `git diff --check` pass.

Risk: high, canonical artifact wire. No merge/push/stable.
