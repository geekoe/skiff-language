# P5-F91 Interface local type slot canonicalization

- Authority: `doc/architecture/package-service-contract-deployment.md`, exact public interface ABI.
- Predecessor: F89; real agent now fails only because `ToolProvider.tools` expected return contains
  artifact-local `Local type_index 7` while actual is canonical
  `ServiceSymbol tools.ToolDeclaration`.
- Worktree: create `skiff-p5-f91-interface-local-type-slots` from current Skiff integration.
- Write owner: compiler/source imported interface method-slot type canonicalization and tests.
- Required outcome: resolve LocalType/PublicationType references inside nested method parameter/
  return closures through the identity-validated artifact implementation links/public ABI to exact
  package/public symbols, including arrays/unions/nullable/records. Do not compare raw indices across
  artifacts or fall back structurally.
- Fail closed: missing/ambiguous/private index, wrong owner/public path/package/version, tampered
  link/descriptor, structurally identical unrelated type.
- Validation: ToolProvider-shaped artifact fixture plus fresh std→llm-api→agent compile-only. Stop at
  the next independent diagnostic. No Internals edits, stable, merge, push, or full workflow.
- Deliver one commit/evidence.

