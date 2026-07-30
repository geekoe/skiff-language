# P5-F81 Imported interface self reconstruction

- Authority: `doc/architecture/package-service-contract-deployment.md`, exact public interface ABI.
- Predecessor: F79; I74 proves published `LlmClient` methods retain `self: Self`, but consumer
  reconstruction reports the receiver missing.
- Worktree: create `skiff-p5-f81-interface-self` from current Skiff integration.
- Write owner: artifact interface method→source interface reconstruction and focused tests.
- Required outcome: preserve exact receiver role/type (`self: Self`), type parameters and method
  signature when ingesting identity-verified exported interface facts. Do not synthesize receiver
  from naming or source fallback.
- Fail closed: missing/duplicate/wrong receiver, non-Self receiver, method/type-param mismatch,
  tampered artifact/FileIR identity.
- Validation: dependency source hidden interface implementation tests plus smallest
  llm-api→agent compile probe. No Internals edits, stable, merge, push, or full gate.
- Deliver one commit/evidence and next diagnostic.

