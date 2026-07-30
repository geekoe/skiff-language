# P5-I71 Internals workflow progression

- Authority: `doc/architecture/package-service-contract-deployment.md`, canonical ecosystem
  transaction and independently compiled package facts.
- Inputs: current Internals Phase 5 integration and current Skiff Phase 5 integration after F68.
- Read-only: execute the real linked-worktree canonical workflow with explicit Skiff root and a
  fresh temporary ecosystem store. Verify std and llm-api publish, then determine how far package,
  deployment, and assembly stages progress.
- Enumerate all bounded independent remaining failures, distinguishing artifact-native type facts,
  exported interface FileIR facts, Internals source typing, workflow wiring, and environment.
- Do not edit/install, invoke build/dev/start, stable watch/reload/artifacts, commit, merge, push, or
  full repository gates. Cleanup temporary state and return PASS/FAIL with exact next DAG.

