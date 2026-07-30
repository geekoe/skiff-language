# P5-F133 DB provenance boundary semantics

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §3, §6 and §11.
- Audit input: P5-D81 C3.

## DAG node

Distinguish DB query/read inputs and detached persisted values from actual caller-owned mutable value escape.

## Write scope

- Compiler source effect/provenance transfer for DB operations and focused tests.

Do not change DB runtime behavior, call-target resolution, type eligibility, service source or make all DB
effects boundary-safe.

## Required outcome

- Query keys/filters do not create a database escape lane.
- Persisting a fresh/detached value is allowed.
- Persisting caller-owned mutable/aliased data remains an escape.
- DB-returned values are provider-owned detached values unless an explicit capability/handle type says
  otherwise.

## Acceptance

- Registry-shaped read/history/put/CAS positives and caller-owned mutable persistence negatives.
- Exact provenance/effect reasons are stable and structured.
- Source/projection focused tests and `git diff --check` pass.

Risk: high, alias/escape safety. No merge/push/stable.
