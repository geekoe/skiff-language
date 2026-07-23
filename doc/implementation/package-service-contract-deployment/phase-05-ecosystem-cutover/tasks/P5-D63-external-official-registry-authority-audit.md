# P5-D63 External official registry authority audit

- Authority: `doc/architecture/package-service-contract-deployment.md`, official packages repository,
  compiler-owned trusted registry declarations, and unforgeable authoring provenance.
- Candidate: Skiff current Phase 5 integration and skiff-packages current Phase 5 integration.
- Read-only scope: trace the real skiff-packages package build/test entry and determine the minimal
  explicit configuration/provenance seam by which `CompilerPlatformSources` can authorize the exact
  external canonical `skiff-packages/registry` root. Package id, cwd, manifest strings, or arbitrary
  CLI flags alone must not self-authorize; ordinary or copied roots remain rejected. The Skiff repo
  placeholder registry source must not become a competing second owner.
- Return exact config/CLI/tooling owner, path/identity/provenance validation, source ownership and
  cleanup migration, positive/negative probes, and a minimal implementation DAG.
- No edits, installs, commits, stable access, or full gate.

