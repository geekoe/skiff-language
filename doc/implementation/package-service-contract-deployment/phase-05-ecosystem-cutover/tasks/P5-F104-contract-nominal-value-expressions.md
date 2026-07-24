# P5-F104 Contract nominal value expressions

## Authority

- Canonical design:
  `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §3, §4 and §5, especially “ContractTypeId can appear in PackageLocalAbi and is an
  ordinary local value type inside the package”.
- Blocker evidence: P5-F102.

## DAG node

Make validated public-nameable ServiceContract nominal types usable as ordinary typed source values inside a
consumer/provider package. This is the compiler checkpoint required before Registry can implement explicit
contract↔storage wrappers.

## Write scope

- Compiler source name/type resolution and typed expression facts.
- The minimum projection-input handoff needed to preserve exact ContractTypeId.
- Focused compiler fixtures/tests.

Do not modify Registry sources, ServiceContract schemas, boundary eligibility rules, File IR contract-type
erasure, runtime, Router, authoring syntax beyond existing qualified contract type names, or add structural
nominal compatibility.

## Required semantics

- For a validated `ContractRequirement(alias = dep)`, `dep.Type` resolves to the descriptor's exact
  `ContractTypeId` in annotations, constructor expressions and field/member projection.
- Construction and field access use the contract descriptor's closed schema; unknown/missing/extra fields fail.
- Values retain exact ContractTypeId through typed source analysis and PackageLocalAbi/boundary projection.
- A structurally identical package-local nominal type remains distinct.
- Do not use JSON encode/decode, raw DTOs, display names or source reconstruction as a bridge.
- Existing File IR executable projection remains opaque `unknown` at contract nominal leaves as required by §3.

## Acceptance

- Focused positive fixture constructs a contract nominal record, reads its fields, passes/returns it and exposes
  an available top-level boundary wrapper.
- Negative fixtures cover unknown field, wrong/missing constructor fields and package-local nominal mismatch.
- Existing contract import/call/conformance and File IR erasure tests pass.
- `git diff --check` passes.

Risk: high, compiler typed identity. Candidate after completion: shared implementation checkpoint; it unblocks
F102 retry with a new Agent. No push or stable-instance operation.
