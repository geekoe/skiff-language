# P5-D69 Real interface boxing identity audit

- Authority: `doc/architecture/package-service-contract-deployment.md`, exact imported interface ABI.
- Candidate: current Skiff/Internals integrations after F79/F81/F84; focused tests pass but real agent
  still reports 12 boxing failures.
- Read-only trace: for `ChildNoopLlm`, `ChildFlowLlm`, and `ScriptedDrainLlmPort`, record the exact
  identity at each hop: parsed `implements llmApi.LlmClient`, dependency alias→package artifact,
  reconstructed interface declaration, type model implementation set, lowered/FileIR conformance,
  boxing requested interface identity and final equality check. Include multi-file/module ownership.
- Compare the focused F84 fixture with the real agent input and identify the first divergent field or
  skipped phase. Prove whether stale cached facts are impossible using fresh store/build identity.
- Return one bounded fix owner and a real compile-only regression that fails before/fixes after.
  No edits, installs, commits, stable, full workflow, or speculative compatibility.

