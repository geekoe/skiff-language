# P5-F89 Imported interface method slots

- Authority: `doc/architecture/package-service-contract-deployment.md`, exact package interface ABI.
- Predecessor: D69 proved interface identities match; real failure is
  `package_method_slots_for_local_conformance`.
- Worktree: create `skiff-p5-f89-interface-method-slots` from current Skiff integration.
- Write owner: compiler/source artifact-backed interface method-slot signature canonicalization and
  focused real-shape tests.
- Required outcome: compare real imported interface method slots across public path `LlmClient` and
  implementation owner `types.LlmClient`, preserving `Self`, `Stream<T>`, and same-package parameter/
  return types (`LlmRequest`, `LlmStreamEvent`, `WebSearchInput`, `WebSearchResult`) under canonical
  package identity. Multi-file local implementers must satisfy exact slots and box successfully.
- Fail closed: receiver/arity/type-param/stream/package/public-path/method mismatch, structurally
  identical unrelated interface, missing/tampered artifact/FileIR.
- Validation: real-shaped artifact-backed multi-file fixture plus fresh std→llm-api publish→agent
  compile-only regression. No Internals edits, stable, merge, push, or full workflow.
- Deliver one commit/evidence and next diagnostic.

