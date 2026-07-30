# P5-D79 Registry service source migration audit

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §2–§5 and §13.

## DAG node

Read-only audit of `skiff-packages/registry` against the service-as-package source model.

## Required output

- Exact files/fields that duplicate `api.yml`, independently author a contract or require `deployment.yml`.
- The minimal target source tree using `package.yml`, `api.yml`, `service.yml`, `config.*.yml` and `.skiff`.
- Where package and service dependencies belong.
- Which immutable/pointer storage implementation can be retained unchanged.
- Migration dependencies on Skiff authoring/tooling and the first executable probe after each checkpoint.

Do not work around compiler gaps, edit, commit, push or operate stable.
