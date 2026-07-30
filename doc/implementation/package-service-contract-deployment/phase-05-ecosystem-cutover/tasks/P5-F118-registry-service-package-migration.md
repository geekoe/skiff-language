# P5-F118 Registry service-package migration

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §2–§5 and §13.
- Entering Skiff checkpoint: integration through F116/F115/F117.
- Audit input: P5-D79.

## Repository/worktree

- Repository: `/Users/geek/workspace/skiff-packages`
- Worktree: `/Users/geek/workspace/skiff-packages-p5-f118-registry-service`
- Branch: `codex/p5-f118-registry-service`

## Required outcome

- Registry is one ordinary service package with `package.yml`, `api.yml`, `service.yml`,
  `config.dev.yml` and `.skiff` sources.
- `service.yml` contains no version, dependencies or API/type mapping.
- `api.yml` exports all types and the 20 actual top-level boundary functions.
- Delete `contract.yml`, `deployment.yml` and the hand-written contract generator/tests.
- Retain immutable and pointer transaction algorithms; adapt DTOs to the canonical generated API.
- DB requirement/binding is ordinary config/state; no Mongo URL, activation, Registry privilege or Router
  knowledge.

## Acceptance

- Skiff service-package build from this root generates PackageArtifact, closed 20-operation ServiceContract
  and ServiceDeployment without legacy authoring files.
- Available/Unavailable output is explicit; all intended 20 operations are Available.
- Immutable put/read and pointer CAS/read/history focused tests pass.
- Structural probe finds no contract/deployment authoring or native/compiler privilege.
- `git diff --check` passes.

Risk: high, first real service source on new authoring path. Commit in the package worktree only; no
merge/push/stable.
