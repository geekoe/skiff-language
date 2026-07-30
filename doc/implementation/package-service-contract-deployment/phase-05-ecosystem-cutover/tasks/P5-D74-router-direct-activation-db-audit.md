# P5-D74 Router direct activation DB audit

- Authority: `doc/architecture/package-service-contract-deployment.md` §13 at commit `335957b`.
- Read-only scope: map removal of `activationBackend.executablePath/args`,
  RouterActivationBackendClient, NDJSON/Rust envelope and child lifecycle. Define the direct Router
  MongoDB implementation using existing `serviceDb.mongoUrl`, coordinator state-store/snapshot
  interfaces and exact activation/audit transaction semantics.
- Verify which existing Router config/protocol already forwards activation-scoped service DB binding
  to Runtime, and distinguish that path from Router's own activation-state collections.
- Return exact files/dependencies/migration order and transaction/recovery/config negative probes.
  No edits, installs, commits, stable, or full gate.

