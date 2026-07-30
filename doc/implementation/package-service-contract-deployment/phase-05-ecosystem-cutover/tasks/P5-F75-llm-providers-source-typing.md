# P5-F75 llm-providers source typing wave

- Authority: `doc/architecture/package-service-contract-deployment.md`, independently compiled
  package source and explicit nominal typing.
- Candidate: Internals current Phase 5 integration; Skiff current Phase 5 integration after F74.
- Parallel file owners:
  - `auth`: `packages/llm-providers/chatgpt_plan/auth.skiff`, exact object literal near 218.
  - `chatgpt-transport`: `packages/llm-providers/chatgpt_plan/transport.skiff`, unresolved
    `safeResponsesCompletedResultJson` response argument and object literal near 198.
  - `transport`: `packages/llm-providers/transport.skiff`, object literals near 419/421.
- Required outcome: add design-correct explicit nominal/JSON target typing while preserving behavior.
  Do not change compiler, package API, workflow, provider semantics, or other files. If exact error is
  compiler-owned rather than missing source target type, stop with evidence.
- Worktree: shard-specific Internals worktree/branch from current integration.
- Validation: focused production package compile with explicit current Skiff root; no full workflow
  per shard. Deliver one commit/evidence; no stable, merge, push, or cross-file edits.

