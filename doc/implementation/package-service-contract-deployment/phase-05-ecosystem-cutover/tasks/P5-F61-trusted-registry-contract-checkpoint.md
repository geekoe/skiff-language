# P5-F61 Trusted registry contract checkpoint

- Authority: `doc/architecture/package-service-contract-deployment.md`, trusted registry,
  activation, capability, and typed native ABI clauses.
- Predecessor: F56C0 at `811095dd`; D57A/B/C contract-repair audits.
- Repository/worktree: create `skiff-p5-f61-registry-contract` from
  `/Users/geek/workspace/skiff-phase-05-integration`.
- Write owner: new low-level backend-neutral `skiff-trusted-registry-contract` crate; path-free DTO
  and `TrustedRegistryStoreApi`; canonical typed native binding specs/scope mapping; required Cargo
  workspace and lock changes; deployment migration/re-export removal necessary to compile.
- Required outcome: public activation is one atomic `activation.activate` operation; prepare/commit/
  abort remain backend-internal. Public pointer DTOs contain typed identities only and no path/kind/
  JSON/bytes. Exact native binding specs are the sole source for nominal ABI, required context,
  capability id/version, and operation scope. Dependency direction must avoid deployment/runtime
  cycles and there must be no duplicate old trait/signature table.
- Non-goals: Platform DB adapter behavior, Host injection/config/dispatch, Router handler, official
  package implementation, filesystem compatibility.
- Validation: formatting, affected crate checks/tests, dependency/reverse-search probes and negative
  serde/ABI tests. This is an implementation checkpoint, not a complete production candidate.
- Deliver one commit and evidence; do not merge or push.

