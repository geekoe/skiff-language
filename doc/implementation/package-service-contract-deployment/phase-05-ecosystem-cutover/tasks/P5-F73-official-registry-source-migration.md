# P5-F73 Official registry source migration

- Authority: `doc/architecture/package-service-contract-deployment.md`, official packages ownership,
  trusted registry typed API, and canonical package workflow.
- Predecessor: F71 external official authority descriptor in current Skiff integration.
- Repository/worktree: create `skiff-packages-p5-f73-registry` from current skiff-packages Phase 5
  integration.
- Write owner: unique `registry/**` public package source/manifest/API and skiff-packages trusted
  test/build tooling that creates an isolated authority descriptor for the exact canonical worktree
  root, passes it to Skiff tooling, and cleans it.
- Required outcome: public typed DTOs/wrappers expose exactly 21 canonical native operations and
  atomic `activation.activate`; no prepare/commit/abort, paths, JSON, bytes, native signature copies,
  manifest capability/binding strings, or user-authored native declarations. The compiler injects
  declarations. Ordinary/copied roots without the trusted workflow descriptor fail closed.
- Validation: real official registry build through skiff-packages entry, artifact FileIR binding set
  and projected capability/scope parity, negative ordinary root/descriptor cleanup probes.
- Do not edit Skiff repo, stable, merge, push, compatibility, or full gate. Deliver one commit and
  evidence.

