# P5-F94 Remove registry privilege chain

- Authority: `doc/architecture/package-service-contract-deployment.md` §13 at `335957b`.
- Parallel shards:
  - `compiler`: remove official authority descriptor/CLI, compiler platform-package authority,
    synthetic registry declarations/API, reserved registry id, registry-specific binding metadata
    propagation and trusted capability projection. Preserve generic package/service compilation,
    std platform sources and ordinary runtime requirements.
  - `runtime-deployment`: remove `skiff-trusted-registry-contract`, registry native-contract/context/
    dispatch/spec surface, Rust PlatformDbTrustedRegistry, RouterActivationBackend envelope and
    Skiff-root registry placeholder. Own workspace/Cargo dependency cleanup; preserve four canonical
    artifact types, filesystem local/dev store, assembly/deployment validation and ordinary native
    infrastructure.
- Worktree: shard-specific Skiff worktree/branch from current integration.
- Completion: production/test/Cargo reverse search has no trusted/native/compiler registry privilege
  symbols outside historical task docs; ordinary `skiff.run/registry` is not reserved and receives
  no special runtime requirement.
- Validation: affected crate checks/tests and ordinary package/service authoring regressions. No
  Router TS edits, Registry service implementation, stable, merge, push, compatibility, or full gate.

