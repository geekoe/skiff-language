# P5-F132 Resolved call-target completeness

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §3 and §6.
- Audit input: P5-D81 C2.

## DAG node

Produce exact call-target facts for statically resolvable same-package/local/helper/root-qualified/interface
dispatch so effect/provenance analysis does not conservatively mark whole real handlers unknown.

## Write scope

- Compiler source call resolution/indexing and effect fixed-point consumers.
- Focused source/compiler tests.

Do not change DB provenance policy, boundary type allowlists, service source, runtime or dynamic-dispatch
fail-closed behavior.

## Required outcome

- `root.module.fn`, same-file helper, exact package dependency and proven interface implementation calls have
  canonical targets.
- Recursive/fixed-point effect summaries recompute from those targets.
- Truly dynamic/ambiguous/unresolved calls remain UnknownCallTarget.
- No display-name runtime lookup.

## Acceptance

- Relay-shaped helper/root call and Registry catch/helper positives.
- Dynamic/ambiguous negatives remain unavailable.
- Relay representative handlers lose the conservative all-effects bundle attributable solely to unknown
  target.
- Source/effect/lowering focused tests and `git diff --check` pass.

Risk: high, compiler semantic facts. No merge/push/stable.
