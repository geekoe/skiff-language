# P5-F65 Internals workflow second blocker wave

- Authority: `doc/architecture/package-service-contract-deployment.md`, canonical isolated ecosystem
  workflow and exact language lowering requirements.
- Candidate: Internals `179cb8f`, Skiff current Phase 5 integration. F64 closed `llm-api` decode
  typing and exposed two independent blockers.
- Shards:
  - `std-provisioning` (Internals workflow/fixture owner): make the isolated canonical compile graph
    provision and consume the canonical `skiff.run/std` `PackageArtifact` before production packages.
    It must use the same temporary ecosystem store and cleanup guard; no source symlink, ambient
    artifact root, stable artifact, fallback, or special skip.
  - `union-array-push` (Skiff compiler owner): close the exact lowering defect where
    `Array<"text" | "image" | "video" | "audio">.push(...)` cannot resolve its builtin receiver.
    Preserve exact union element checking and reject nonmembers/wrong receiver types.
- Worktree: shard-specific worktree from the relevant integration repository.
- Validation: focused positive/negative tests plus the cheapest real workflow/package probe. Stop at
  the next out-of-owner blocker with exact evidence. Do not merge, push, or touch stable.

