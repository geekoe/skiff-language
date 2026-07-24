# P5-F112 Service API visibility

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §3–§4.
- Entering checkpoint: F108 `fe1ed14`.
- Audit input: P5-D78.

## DAG node

Expose the automatic Available/Unavailable Service API projection as one stable developer-facing result.

## Write scope

- Compiler driver receipt/projection DTO.
- CLI human and JSON output for inspecting a service root.
- Focused driver/Node CLI tests.

Do not implement IDE UI, generated deployment, dev-sync migration, Registry/Internals changes or alter
boundary eligibility.

## Required semantics

- One canonical DTO lists every `api.yml` public function exactly once with Available or structured
  Unavailable reasons.
- Human build/check output summarizes both sets without silently excluding package-only functions.
- Stable JSON output and receipt refer to the same projection; no tool re-analyzes source.
- Ordinary package roots may expose the boundary projection without pretending to be a service; service roots
  additionally expose the generated service operation identity.

## Acceptance

- Human and JSON golden tests cover mixed Available/Unavailable API.
- Reason order and operation order are deterministic.
- Zero API, malformed root and unavailable-only service cases are explicit.
- Driver/CLI focused tests and `git diff --check` pass.

Risk: medium, developer/public tooling surface. No merge/push/stable.
