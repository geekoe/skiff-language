# P5-D76 Duplicate Registry service owner audit

- Authority: `doc/architecture/package-service-contract-deployment.md` §13 at `335957b`.
- Read-only repository: current Internals integration.
- Locate the existing `skiff.run/registry` service owner under skiff-platform/package-registry,
  enumerate its contracts/deployments/callers/stable watch references and classify old upload/build/
  catalog semantics versus the new canonical four-record Registry service in skiff-packages.
- Return exact deletion or service-id migration boundary; there must be one production owner and no
  dual-write/compat adapter. Identify any caller functionality that remains required but belongs to a
  differently named build/catalog service.
- No edits, installs, commits, stable, full workflow or gate.

