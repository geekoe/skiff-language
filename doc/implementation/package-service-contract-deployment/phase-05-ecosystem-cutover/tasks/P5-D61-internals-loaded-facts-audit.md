# P5-D61 Internals loaded nominal facts audit

- Authority: `doc/architecture/package-service-contract-deployment.md`, independently compiled
  package facts and canonical dependency linkage.
- Candidate: Internals `7f3f6a4`, Skiff current Phase 5 integration.
- Read-only scope: trace the real workflow failure where `packages/llm-providers` cannot resolve
  `llmApi.LlmModelDescription` and related nominal union/field facts from the already published
  `llm-api` artifact. Determine whether ownership is package API declaration, artifact projection,
  dependency fact loading, workflow receipt wiring, or a combination. Enumerate all failures hidden
  by the first error where bounded.
- Return exact files/identities, minimal shared checkpoint and parallel consumer fixes, plus
  positive/negative probes. No edits, installs, stable access, commits, or full gate.

