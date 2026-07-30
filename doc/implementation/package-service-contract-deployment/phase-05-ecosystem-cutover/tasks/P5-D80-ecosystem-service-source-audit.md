# P5-D80 Ecosystem service source migration audit

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §2–§5, §10–§11.

## DAG node

Read-only audit of Internals service roots against the service-as-package source model.

## Required output

- Inventory service roots and whether each has `package.yml`, `api.yml`, `service.yml`, `config.*.yml`.
- Locate dependencies currently split across service/config files.
- Locate checked-in `deployment.yml` or independent contract roots that must disappear.
- Group migrations into non-overlapping implementation shards after the Skiff authoring checkpoint.
- Preserve service behavior; identify focused authoring and canonical workflow probes.

Do not edit, commit, push, register watch entries or operate stable.
