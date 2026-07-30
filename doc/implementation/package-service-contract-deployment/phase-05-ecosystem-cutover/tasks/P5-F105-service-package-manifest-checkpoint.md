# P5-F105 Service package manifest checkpoint

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §2–§5 and §10–§11.
- Audit input: P5-D78.

## DAG node

Freeze the typed source-root/input model required by all later service projection work.

## Write scope

- `compiler/input` package/service/config manifest models, IO and validation.
- Minimum shared authoring DTOs in `artifact-model`.
- Focused Rust fixtures/tests for this input boundary.

Do not implement ServiceContract projection, ServiceDeployment generation, CLI/dev-sync commands, Registry
or Internals migrations.

## Required semantics

- A service root is a normal package root containing `package.yml` and `api.yml`, plus `service.yml` and
  optional `config.*.yml`.
- `package.yml` owns `id`, human-readable `version`, exact `packages` and exact `services`.
- Package and service dependency aliases share one namespace; ranges and duplicate aliases fail.
- `service.yml` owns service `id` and existing ingress/service-only policy fields. It rejects `version`,
  dependencies and API/type/function mappings.
- Human version fields are selectors/labels and never identity hash input.
- Config profiles may bind declared requirements but may not add/remove dependencies.
- Remove `contracts` as a package manifest dependency kind; no compatibility path.

## Acceptance

- Positive typed fixtures cover ordinary package and service-as-package roots.
- Missing package/api manifest, service manifest version/dependencies/API mapping, alias conflict and
  non-exact version fail closed.
- Existing package manifest tests remain green after intentional fixture migration.
- `git diff --check` passes.

Risk: high, canonical authoring input. Candidate: shared implementation checkpoint. No merge/push/stable.
