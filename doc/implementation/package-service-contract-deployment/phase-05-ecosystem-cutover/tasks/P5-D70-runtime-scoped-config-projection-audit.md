# P5-D70 Runtime scoped config projection audit

- Authority: `doc/architecture/package-service-contract-deployment.md`, assembly-owned deployment
  config and package runtime execution.
- Candidate: current Skiff/skiff-packages integrations after F86/F88.
- Read-only trace: for http-session test deployment, follow exact config literals from immutable
  ServiceDeployment → RuntimeAssembly resolved deployment → runtime assembly admission/activation →
  RuntimeProgram package slot projection → config intrinsic lookup. Record exact package identity,
  slot and config map at each hop.
- Identify the first dropped/ignored field causing
  `RuntimeProgram package slot 0 is missing scoped config`. Compare test fixture path with production
  assembly path; no legacy RuntimeServiceConfig/file config fallback.
- Return one bounded owner, files, positive own/dependency config probes and missing/wrong-slot/
  retired-activation negatives. No edits, installs, commits, stable, full package run, or full gate.

