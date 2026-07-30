# P5-F77 Internals canonical package call addresses

- Authority: `doc/architecture/package-service-contract-deployment.md`, canonical imported package
  callable address syntax.
- Candidate: current Internals integration; I72 proved artifact facts/linkage work and found old
  `llmApi.<module>.<fn>` production calls.
- Parallel repository owners:
  - `providers`: llm-providers production 5 sites plus integration test 2 sites.
  - `agent`: agent `drain_stream_reducer.skiff` 2 sites.
  - `relay`: Codex Relay `completed_responses.skiff` 5 sites.
  - `aihub`: AIHub service/managed provider transport 6 sites.
- Required outcome: mechanically replace exact imported package calls with
  `llmApi/<module>.<fn>` while preserving arguments/results/behavior. Search the owned production
  subtree for the same legacy address shape and close all matches; do not change API or compiler.
- Worktree: shard-specific Internals worktree/branch from current integration.
- Validation: reverse search in owned subtree plus focused compile/type-check with explicit current
  Skiff root where executable. No per-shard full workflow, stable, build/dev/start, merge, push, or
  cross-owner edits. Deliver one commit/evidence.

