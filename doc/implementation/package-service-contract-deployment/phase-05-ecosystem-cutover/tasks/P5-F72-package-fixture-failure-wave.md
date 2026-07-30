# P5-F72 Package fixture failure wave

- Authority: `doc/architecture/package-service-contract-deployment.md`, exact callable facts,
  canonical package tests, and terminal legacy cleanup.
- Frozen failure: I70 at Skiff `40fcf735...`, skiff-packages `26ba2a08...`.
- Parallel shards:
  - `callable-transfer`: compiler source callable-effect/provenance transfer only. Make exact registry
    facts actually survive wrapper composition: split must not gain alias/escape/same-heap/unknown/
    suspend; HTTP keeps only suspend/detached fresh; remove remaining shared false facts causing
    aliyunoss RequiresSameHeapIdentity, track full unknown set, and openai unknown/same-heap.
    Preserve missing/dynamic/mutable fail-closed and Array.push W/I.
  - `eval-test-legacy`: runtime-eval test/projection fixture owner only. Replace/delete five test-only
    uses of removed `skiff_runtime_linker::linked_file_unit_from_artifact` with the canonical current
    FileIR/assembly fixture; no production legacy alias or compatibility helper.
  - `http-session-fixture`: skiff-packages `http-session/session.test.skiff` only. Express optional
    refinement/unwrap and exact HttpResponse/string types using current language semantics while
    preserving all 19 test behaviors. If current language is intended to narrow after assert and a
    minimal compiler probe proves a compiler defect, stop `TASK_NOT_EXECUTABLE` instead of changing
    compiler outside this shard.
- Worktree: shard-specific worktree/branch in the relevant integration repository.
- Validation: focused positive/negative tests and exact I70 failing probes; no duplicated four-package
  full run, stable, full gate, merge, push, compatibility, or cross-shard edits.

