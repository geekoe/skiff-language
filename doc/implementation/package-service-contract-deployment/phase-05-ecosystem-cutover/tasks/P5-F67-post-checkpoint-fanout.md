# P5-F67 Post-checkpoint fanout

- Authority: `doc/architecture/package-service-contract-deployment.md`.
- Candidate: current Skiff Phase 5 integration after typed test overlay and Router backend envelope.
- Independent shards:
  - `package-test-projection`: close canonical boundary projection for all six `aliyunoss` and four
    `track` cases using the typed overlay production source graph. Legal public/private owner-local
    calls must receive exact callable facts; genuinely missing targets and caller-owned reference
    escape remain fail closed. No self dependency/artifact linkage.
  - `mongo-router-backend`: implement deployment's internal `RouterActivationBackend` against the
    Platform Mongo registry. Each prepare/commit/abort atomically CASes state and derives/appends
    audit in one transaction; validate exact frozen/connected/prepared replica sets. Remove or
    redirect any direct Mongo public activate path that could bypass Router participant ACK.
  - `router-backend-client`: implement the Router-owned long-lived adapter child/client, strict
    config selection and lifecycle using the F66 envelope. Production must reject filesystem/
    compiler/artifactRoot fallback and missing environments; local/dev/CLI file client remains
    explicitly non-production. Wire the existing `AssemblyActivationCoordinator` state store and
    snapshot loader only; do not implement runtime/native public bridge.
- Worktree: shard-specific worktree/branch from current Skiff integration.
- Validation: focused positive/negative tests and affected compile/type-check. Stop at precise
  out-of-owner blockers. No cross-shard edits, full gate, stable, merge, push, or compatibility.

