# P5-F92 Local interface type slot canonicalization

- Authority: `doc/architecture/package-service-contract-deployment.md`, exact interface ABI.
- Predecessor: F91; imported ToolProvider slots now pass. Real agent next fails local
  `ChildCleanupEntryConsumer.consume`: expected local union/return indices versus actual canonical
  `child_cleanup.ChildCleanupEligibilityScope` / `ChildCleanupConsumeResult`.
- Worktree: create `skiff-p5-f92-local-interface-slots` from current Skiff integration.
- Write owner: compiler/source same-publication interface method-slot canonicalization and tests.
- Required outcome: resolve local method parameter/return LocalType/PublicationType indices through
  the exact owning module/publication type table to canonical service symbols before conformance.
  Preserve nested union/nullable/container closure and exact owner identity.
- Fail closed: wrong module/index/private/ambiguous type, mismatched union member/return, structurally
  identical unrelated symbol. No name/shape fallback.
- Validation: ChildCleanup-shaped multi-file fixture plus fresh std→llm-api→agent compile-only. Stop
  at next independent diagnostic. No Internals edits, stable, merge, push, or full workflow.
- Deliver one commit/evidence.

