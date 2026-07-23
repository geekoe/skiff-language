# P5-F84 Imported interface implementation identity

- Authority: `doc/architecture/package-service-contract-deployment.md`, exact public interface ABI.
- Predecessor: F79/F81; I76 shows source types explicitly implement `llmApi.LlmClient` and define its
  methods, yet boxing says they do not explicitly implement it.
- Worktree: create `skiff-p5-f84-interface-implementation-identity` from current Skiff integration.
- Write owner: compiler artifact-backed interface implementation identity/lookup and focused tests.
- Required outcome: imported interface declarations reconstructed from PackageArtifact/FileIR must
  use the same canonical package/public-path identity as `implements` entries and boxing checks.
  Preserve method/receiver/type-param validation; no name-only or structural fallback.
- Fail closed: wrong alias/package/version/public path, duplicate interface, method mismatch,
  tampered artifact/FileIR/ABI, structurally identical unrelated interface.
- Validation: dependency source hidden implementation+boxing tests and smallest llm-api→agent probe.
  No Internals edits, stable, merge, push, or full gate. Deliver one commit/evidence.

