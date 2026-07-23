# P5-F74 Artifact symbolic type reconstruction

- Authority: `doc/architecture/package-service-contract-deployment.md`, independently compiled
  package artifact type facts.
- Predecessor: F68 artifact-native facts; I71 fails on valid
  `TypeRefIr::ServiceSymbol { module_path: "types", symbol: "LlmContentPart" }`.
- Worktree: create `skiff-p5-f74-artifact-symbolic-types` from current Skiff integration.
- Write owner: compiler source artifact descriptor→source type reconstruction and focused tests.
- Required outcome: resolve identity-validated public `ServiceSymbol` refs within the dependency
  artifact's exported type closure, including same-module/cross-module nested records and arrays,
  without reading dependency source or accepting private/missing/ambiguous symbols. Audit
  `DbObjectSymbol` and implement only if existing artifact semantics require the same public
  reconstruction and can be proven exactly.
- Fail closed: missing/private/mismatched module/symbol, recursive invalid closure, coordinate/version
  mismatch, tampered descriptor/ABI. No LocalType index guessing or ambient fallback.
- Validation: focused positive/negative package import tests and the smallest llm-api consumer probe.
  Do not edit Internals, stable, merge, push, or full gate.
- Deliver one commit/evidence and next real diagnostic.

