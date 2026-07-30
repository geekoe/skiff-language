# P5-F109 Router filesystem/activation cutover

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §12–§13.
- Entering checkpoints: F106 `cf60242`, Router Mongo state store `f428b7f`.
- Audit input: P5-D77.

## DAG node

Replace Router production compiler/backend subprocesses with direct shared-filesystem routing snapshots and
the in-process Mongo activation state store.

## Write scope

- Router snapshot loader/composition, server production selection, activation state composition.
- Router config/types/tests made obsolete by this production cutover.

Do not modify Runtime Rust, compiler artifact layout, instance/deploy tooling, Registry, legacy test-only
package dispatch or stable instance.

## Required semantics

- Router reads immutable RuntimeAssembly/ServiceContract records from configured `artifactsPath` and projects
  only routing/activation facts; it never links or loads package executable content.
- Router uses its configured Mongo connection through the existing in-process activation state/audit store.
- Delete production `activationBackend` executable/args, child client and compiler
  `__ecosystem-store` snapshot/state subprocess path.
- Router does not know Registry.
- Missing/malformed/identity-mismatched/escaping records and incomplete pointers fail closed.
- Preserve atomic prepare/commit/abort, generation pin/drain and exact runtime registration behavior.

## Acceptance

- Focused filesystem snapshot positive and corruption/escape/missing negatives pass.
- Activation state tests prove in-process Mongo transaction/CAS path is selected.
- Structural probe finds no production child activation backend or ecosystem-store process spawn.
- Router type-check, focused tests and `git diff --check` pass.

Risk: high, production activation owner. No merge/push/stable.
