# P5-F64 Internals package workflow blockers

- Authority: `doc/architecture/package-service-contract-deployment.md`, canonical contract/package/
  deployment/assembly workflow requirements.
- Candidate: Internals integration commit `07e9906`; F59's real linked-worktree transaction now
  fails at `packages/llm-api` contract validation because eight `decode` object literals lack an
  explicit target type.
- Repository/worktree: create `internals-p5-f64-llm-api` from
  `/Users/geek/workspace/internals-phase-05-integration`.
- Write owner: `packages/llm-api` source and its focused tests only.
- Required outcome: add the design-correct explicit nominal target typing, preserve decode behavior,
  then run the real canonical workflow far enough to prove this package passes. If a later blocker
  belongs to another package/owner, stop and report it precisely rather than expanding scope.
- Non-goals: workflow runner, other packages/services/clients, stable build/watch/reload, Skiff
  compiler changes, compatibility paths.
- Validation: focused package checks/tests and linked-worktree canonical script with explicit
  `SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration`.
- Deliver one commit and evidence; do not merge or push.

