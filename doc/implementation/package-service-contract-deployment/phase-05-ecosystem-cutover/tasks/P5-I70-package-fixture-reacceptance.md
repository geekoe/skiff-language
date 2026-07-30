# P5-I70 Package fixture reacceptance

- Authority: `doc/architecture/package-service-contract-deployment.md`, canonical package test
  boundary and official package completion criteria.
- Frozen inputs: current Skiff Phase 5 integration after F60/F66/F70 and current skiff-packages Phase
  5 integration.
- Read-only owner: run the combined callable-semantics compiler/runtime focused probes once, then
  the canonical isolated package tests for `aliyunoss`, `track`, `openai`, and `http-session` using
  explicit Skiff root and isolated temporary stores/runtime. Do not use stable.
- Verify all exported test cases cross boundary, execute, and assert expected results; enumerate all
  remaining independent failures rather than stopping analysis at the first command.
- Verdict PASS only if all four complete. Classify failures as environment, package fixture,
  compiler/runtime semantics, or Host/runtime setup with exact files/commands. No edits, installs,
  commits, merge, push, or full repository gate.

