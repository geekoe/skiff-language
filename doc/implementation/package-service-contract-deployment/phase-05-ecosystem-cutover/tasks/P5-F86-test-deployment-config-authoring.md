# P5-F86 Test deployment config authoring checkpoint

- Authority: `doc/architecture/package-service-contract-deployment.md`, test-owned canonical
  deployment and runtime requirement validation.
- Candidate: current Skiff integration; D68 proved package-test deployments lack required config.
- Worktree: create `skiff-p5-f86-test-config-authoring` from current Skiff integration.
- Write owner: test-runner/input schema and canonical test deployment projection only.
- Required outcome: add explicit test-only config literals input owned by the test fixture/runner,
  keyed by exact package config requirement. Validate exact package id/key/type, required coverage,
  duplicates/unknowns and dependency closure; project only into the ephemeral test-owned
  ServiceDeployment. Preserve optional-unbound semantics and base-assembly exact inheritance.
- Prohibited: production `PackageDependency.config`, runtime defaults/fallback, manifest capability
  strings, stable config, or ordinary package artifact changes.
- Validation: positive own/dependency required values; missing/wrong type/unknown/duplicate/optional
  probes; test deployment record exact literals. This checkpoint need not edit skiff-packages.
- Deliver one commit/evidence; no merge, push, stable, or full gate.

