# P5-F79 Artifact exported interface facts

- Authority: `doc/architecture/package-service-contract-deployment.md`, independently compiled
  package public ABI and exact FileIR identity.
- Predecessor: F68/F74; I73 proves `llm-api` publishes but agent sees imported
  `llmApi.LlmClient` as non-interface.
- Worktree: create `skiff-p5-f79-exported-interface-facts` from current Skiff integration.
- Write owner: compiler artifact projection/identity-verified FileIR loading and source
  type-resolution interface ingestion, with focused tests.
- Required outcome: preserve exported interface classification and exact method signatures in
  PackageArtifact public facts, or load only the referenced artifact's identity-verified FileIR units
  where the ABI intentionally carries an external descriptor. Consumer compilation must implement
  `llmApi.LlmClient` without reading dependency source.
- Fail closed: missing/tampered FileIR ref/payload/hash, private/missing interface, mismatched public
  path/module/method signature, duplicate/ambiguous symbol, wrong package coordinate/version.
- Validation: focused public interface import/implements tests with dependency source hidden plus the
  smallest llm-api→agent publish probe. Do not edit Internals, stable, merge, push, or full gate.
- Deliver one commit/evidence and next real diagnostic.

