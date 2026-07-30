# P5-D71 Service DB activation projection audit

- Authority: `doc/architecture/package-service-contract-deployment.md`, assembly-owned capability
  projection and package test execution.
- Candidate: current Skiff/skiff-packages integrations after F90; http-session has 13 PASS and six
  DB cases fail `serviceDb is not configured for this service activation`.
- Read-only trace: follow http-session's exact ServiceDb runtime requirement and ephemeral test
  deployment through RuntimeAssembly, activation/linking, Host request context and ServiceDb
  capability adapter. Identify the first missing owner/config value and compare production assembly.
- Determine whether test-runner must author an explicit isolated DB binding or Host failed to project
  an already-present canonical binding. No ambient stable Mongo, runtime default, service-id
  inference, file config or legacy RuntimeServiceConfig.
- Return bounded owner/files and isolated DB positive plus missing/wrong package/retired activation
  negatives. No edits, installs, commits, stable, full package run, or full gate.

