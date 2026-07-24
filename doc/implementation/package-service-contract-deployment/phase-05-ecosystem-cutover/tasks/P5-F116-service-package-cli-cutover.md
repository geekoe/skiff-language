# P5-F116 Service package CLI/dev-sync cutover

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §2–§5 and §13.
- Entering checkpoints: F105 `eb206aa`, F108/F113/F114 through `9a94e84`, F111 `fa08b02`,
  F112 `47c5b0a`.
- Audit input: P5-D78.

## DAG node

Make one service package root the only authoring entry and generate PackageArtifact, ServiceContract and
ServiceDeployment receipts without independent contract/deployment roots.

## Write scope

- Compiler driver authoring object/CLI composition.
- Node CLI authoring wrappers and dev-sync root classification/workflow.
- Focused CLI/dev-sync tests and fixtures.

Do not migrate Registry/Internals sources, change compiler type semantics, Router/Runtime or operate stable.

## Required semantics

- A root with package.yml+api.yml+service.yml is accepted as a service package and one build/publish workflow
  emits the three exact generated records/receipts.
- Ordinary package root continues to emit only PackageArtifact.
- Remove public `contract build/publish`, `deployment build/publish`, independent `contract.yml` and
  `deployment.yml` authoring objects/roots; canonical artifact storage/runtime types remain.
- Dev-sync discovers service package roots, orders exact dependencies and builds generated records before
  assembly.
- Human/JSON Available/Unavailable output uses F112's canonical projection.
- Checked-in contract/deployment authoring files fail closed rather than silently winning.

## Acceptance

- Positive temporary service root contains only package/api/service/config/source inputs and yields three
  records.
- Ordinary package, missing manifest, legacy contract/deployment roots and mixed invalid roots are covered.
- Dev-sync focused tests prove no independent contract/deployment phase.
- Node/Rust CLI focused tests and `git diff --check` pass.

Risk: high, canonical authoring production entry. No merge/push/stable.
