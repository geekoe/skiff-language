# P5-F82 Agent source typing wave

- Authority: `doc/architecture/package-service-contract-deployment.md`, explicit nominal typing.
- Candidate: current Internals integration; I74 bounded diagnostics.
- Parallel file owners:
  - `tool-mount`: only agent tool-mount source; make two length-derived values exact integers using
    current language semantics without behavior change.
  - `agent-literals`: `child_cleanup`, `drain_checkpoint_store`, `lifecycle_repair`, `thread_config`,
    and `thread_tool_result`; add exact nominal targets to the reported object literals.
  - `drain-execution`: `drain_tool_executor`, drain retry/error sources, and
    `drain_stream_reducer`; type the reported object/nested/return literals and fields. Do not change
    stream/interface diagnostics likely caused by F81.
- Worktree: shard-specific Internals worktree/branch.
- Validation: reverse search/targeted compile where possible with explicit current Skiff root.
  Stop at F81-dependent diagnostics; no compiler/API/behavior changes, cross-owner edits, full
  workflow per shard, stable, build/dev/start, merge, or push.
- Deliver one commit/evidence per shard.

