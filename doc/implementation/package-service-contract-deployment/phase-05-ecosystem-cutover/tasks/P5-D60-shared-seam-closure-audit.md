# P5-D60 Shared seam closure audit

- Authority: `doc/architecture/package-service-contract-deployment.md`.
- Candidate: current Skiff Phase 5 integration after F61/F60.
- Read-only shards:
  - `test-overlay`: determine the architecture-consistent owner and minimal typed representation for
    test cases calling production package public and private callables without emitting a
    self-dependency or weakening unresolved-call fail-closed behavior. Decide from existing language
    test semantics whether private calls are legal; do not invent new public API.
  - `registry-native-source`: determine the minimal compiler-owned source/declaration seam that lets
    an official non-std trusted-registry package consume the canonical typed native binding specs
    without duplicating signatures/capability strings or allowing arbitrary packages to declare
    native functions/types.
  - `router-db-transport`: determine the minimal internal Router-to-Platform-DB activation backend
    contract and process/transport owner. Public API remains one atomic activate; internal durable
    prepare/commit/abort and audit share the Platform DB transaction and are never exposed to package
    or generic native callers.
- Return exact files/types/dependency direction, positive/negative probes, and a parallelizable
  implementation DAG. No edits, installs, commits, stable access, or full gate.

