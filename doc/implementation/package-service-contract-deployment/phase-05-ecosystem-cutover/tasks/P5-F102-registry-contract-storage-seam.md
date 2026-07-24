# P5-F102 Registry contract/storage conversion seam

## Authority

- Canonical design:
  `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §4, §5 and §13.
- Entering package checkpoint: `skiff-packages` integration commit `d5fc88de` or its integrated equivalent.
- Blocker evidence: P5-F99 endpoint wiring rejected package-local `root.model.*` nominal types at the
  ServiceContract boundary.

## DAG node

Create the missing typed seam that allows the ordinary `skiff.run/registry` service implementation to
implement its own published contract. This unblocks F99 endpoint wiring.

## Write scope

- `/Users/geek/workspace/skiff-packages-p5-f102-registry-contract-seam/registry/**`
- Focused package/deployment authoring fixtures for this service only.

Do not modify Skiff compiler/runtime/router, the published Registry contract semantics, storage transaction
semantics, or introduce native/compiler privilege.

## Required implementation

- Declare the Registry ServiceContract as a compile requirement of its implementation package.
- Boundary wrappers must use the contract-owned nominal request/result types.
- Convert explicitly between contract nominal values and package-local storage values.
- Keep storage interfaces package-local; do not make structural equality substitute for `ContractTypeId`.
- Use top-level boundary callables. Do not bind interface methods with a `self` receiver.
- Preserve exactly the existing 20 operations and four record families; no activation operation.

## Acceptance

- Registry package publish succeeds from source.
- Registry deployment build binds all 20 operations successfully.
- At least one immutable put/read and one pointer CAS/read path execute through a real boundary wrapper.
- A package-local nominal type used directly at a binding remains rejected.
- `git diff --check` passes.

Risk: high, public typed service boundary. Candidate after completion: implementation checkpoint, not stable.
No push and no stable-instance operation.
