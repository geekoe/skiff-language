# P5-F98 Service DB provisioning checkpoints

- Authority: `doc/architecture/package-service-contract-deployment.md` §13 at `335957b`.
- Correct owner: Router config `serviceDb.mongoUrl`; URL is transported only in trusted
  Router→Runtime activation control and never stored in artifact/deployment/audit/state/runtime config.
- Parallel checkpoints:
  - `wire`: extend strict TS/Rust assembly prepare/commit/recovery control with optional
    `serviceDb:{mongoUrl}`. Reject empty/unknown/storageNamespace fields; public HTTP activation input
    cannot provide it. No Router emission or Host provisioning yet.
  - `isolated-mongo`: update isolated package-test runtime harness to lease a fourth dynamic port,
    launch/manage its own Mongo replica set, render only Router `serviceDb.mongoUrl`, and finally
    clean DB directory/process/ports. Never use 27017 or stable.
- Worktree: shard-specific Skiff worktree/branch.
- Validation: exact protocol roundtrips/negatives/URL non-persistence; harness ownership/ready/cleanup
  tests. No artifact serviceConfig compatibility, provider record, runtime file/env URL, stable,
  merge, push or full gate.

