# P5-F90 Runtime package config slots

- Authority: `doc/architecture/package-service-contract-deployment.md`, assembly-owned config and
  package slot execution.
- Predecessor: D70 proved literals survive through ActivationContext; Host request adapter drops them
  by constructing empty package config vectors.
- Worktree: create `skiff-p5-f90-package-config-slots` from current Skiff integration.
- Write owner: Runtime Host assembly request eval capability adapter/config view and focused canonical
  assembly execution tests.
- Required outcome: project activation config literals into a vector exactly aligned with the active
  execution image's shared package code slots and exact PackageArtifact config requirements; pass
  that same projection into RuntimeActivation and ConfigCapabilityContext. Preserve service view
  separately where canonical service execution requires it.
- Fail closed before execution on missing required literal, wrong exact package/slot, vector-length/
  slot mismatch, duplicate/unknown config or retired activation/generation. No empty fallback,
  filesystem config, RuntimeServiceConfig, or cross-slot visibility.
- Validation: own slot reads cookieName/maxAgeSeconds; nonzero dependency slot; overlay/production
  parity; missing/wrong/retired negatives; focused Host tests and serial http-session if practical.
- No schema/package edits, stable, merge, push, compatibility, or full gate. Deliver one commit.

