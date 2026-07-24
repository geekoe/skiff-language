# P5-F111 Generated ServiceDeployment

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §5 and §10–§12.
- Entering checkpoints: F105 `eb206aa`, F108 `fe1ed14`.
- Audit input: P5-D78.

## DAG node

Generate ServiceDeployment from the exact service package compile, automatic ServiceContract, service
manifest and selected config profile; remove the need for developer-authored deployment input at this seam.

## Write scope

- Deployment projection and compiler driver composition for generated deployment.
- Typed service/config authoring DTO consumption and focused tests.

Do not remove public CLI commands/dev-sync yet, migrate source repos, change runtime schemas, Router or Registry.

## Required semantics

- Every projected service operation maps deterministically to its source PackageCallableId.
- service id comes from `service.yml`; version label from `package.yml`.
- Exact PackageArtifact and Service API identities are recorded; version label is not identity input.
- Ingress resolves only Available API operations.
- Config/state/resource/policy bindings come from the selected strict config profile and declared
  requirements.
- No `deployment.yml`, manual operation mapping or display-name runtime lookup is accepted by the new seam.

## Acceptance

- Positive service fixture generates exact operation, ingress and config/state bindings.
- Unavailable ingress, missing/duplicate mapping, unbound requirement and identity mismatch fail closed.
- Rebuild with a compatible implementation can change PackageArtifact identity without changing Service API
  identity.
- Deployment focused tests and `git diff --check` pass.

Risk: high, generated runtime binding artifact. No merge/push/stable.
