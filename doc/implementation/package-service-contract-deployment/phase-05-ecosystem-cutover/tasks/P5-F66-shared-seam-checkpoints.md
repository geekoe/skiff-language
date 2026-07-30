# P5-F66 Shared seam checkpoints

- Authority: `doc/architecture/package-service-contract-deployment.md`.
- Candidate: current Skiff Phase 5 integration after F60/F61/F63 Platform DB.
- Independent short checkpoint shards:
  - `test-overlay`: keep test-runner as owner of one compilation containing production source graph
    plus transformed private test functions. Add a typed internal overlay input/invariant binding the
    exact production artifact coordinate while proving no self dependency/requirement/link is
    emitted. Add public/private `root.*`, test-local helper, missing target, and explicit self
    dependency probes. Do not add artifact self-linking or public API.
  - `registry-native-source`: move the canonical 21 typed trusted-registry native signatures to the
    low-level trusted-registry contract; runtime specs consume that slice. Add compiler-owned,
    unforgeable platform-package authority/provenance for exact `skiff.run/registry` and inject
    declaration-only synthetic native callables from the canonical slice. User-authored native
    declarations/types remain rejected; package id alone grants nothing; std/prelude behavior is
    unchanged.
  - `router-backend-envelope`: define the deployment-internal strict Router activation backend
    wire/trait and a long-lived trusted Rust adapter process envelope. It owns bounded NDJSON
    requestId framing and typed read/prepare/commit/abort/read-snapshot operations, but not Router TS
    client/config or public native/package API. Audit payload is derived by backend, never supplied
    by Router. Do not expose this trait from `trusted-registry-contract`.
- Worktree: shard-specific Skiff worktree/branch from current integration.
- Each shard runs formatting, affected checks/tests, and stated fail-closed probes; deliver one
  commit. No cross-shard edits, merge, push, stable access, compatibility, or full gate.

