# P5-F95 Router Mongo activation state store

- Authority: `doc/architecture/package-service-contract-deployment.md` §13 at `335957b`.
- Worktree: create `skiff-p5-f95-router-mongo-state` from current integration.
- Write owner: Router in-process Mongo implementation of `AssemblyActivationStateStore`, its
  transaction/recovery tests and `mongodb` dependency. Do not yet change production server backend
  selection or snapshot loader.
- Input owner: exact existing Router `serviceDb.mongoUrl`; no runtime/env/default/extra executable.
- Required semantics: per-environment prepare/commit/abort state CAS and derived audit append in one
  transaction; exact frozen/connected/prepared participant sets; idempotent retries without duplicate
  audit; transient transaction and unknown commit-result handling; independent Router collections
  isolated from Registry service data.
- Validation: temporary replica-set tests for atomicity/concurrency/retry/recovery plus strict config
  unit probes where module-local. Never log URL. No Registry snapshot reading, child process/NDJSON,
  deployment Rust edits, stable, merge, push, or full gate.

