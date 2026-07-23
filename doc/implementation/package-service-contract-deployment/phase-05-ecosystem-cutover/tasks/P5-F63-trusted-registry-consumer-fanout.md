# P5-F63 Trusted registry consumer fanout

- Authority: `doc/architecture/package-service-contract-deployment.md`, trusted registry and atomic
  activation clauses.
- Shared checkpoint: Skiff integration commit `e5f0242`; all shards consume its low-level path-free
  DTOs, typed `TrustedRegistryStoreApi`, exact native specs/scopes, and single atomic activation.
- Independent shard ownership:
  - `platform-db`: `deployment` Platform DB/Mongo implementation only. Start from the shared
    checkpoint and reuse/cherry-pick the valid behavior from retained commit
    `d3809c68d1995529dd729d911b541f4b956d49a6`, adapting it to path-free DTOs and the new StoreApi.
    Four immutable records, pointer CAS/history, and activation state/audit must remain durable and
    atomic; no file-store dual write.
  - `router-activation`: Router activation coordinator/endpoint only. Expose the single public
    atomic activate operation; prepare/commit/abort are internal backend coordination and never
    external/native/package API.
  - `official-package`: official trusted-registry package source and its exact canonical projection
    only. Consume canonical binding specs; no invented manifest capability strings, paths, JSON,
    bytes, or public prepare/commit/abort.
- Worktree: shard-specific worktree/branch from the correct integration repository.
- Prohibited: editing another shard, Host/runtime injection (separate owner), compatibility aliases,
  stable services/artifacts, merge, or push.
- Validation: affected formatting/check/tests and focused positive/negative contract probes. Deliver
  one commit with evidence. Report `TASK_NOT_EXECUTABLE` within five minutes if the shared checkpoint
  lacks a required design-defined interface.

