# P5-D78 Service-as-package authoring audit

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §2–§5, §10–§11 and §13.

## DAG node

Read-only audit of Skiff compiler/tooling authoring against the confirmed model: a service root is a package
root plus `service.yml` and `config.*.yml`; `api.yml` is the only public API owner; both package and service
dependencies live in `package.yml`; ServiceContract and ServiceDeployment are generated artifacts.

## Required output

- Exact validators/parsers/commands that currently make `package.yml` and `service.yml` exclusive.
- Exact independent contract/deployment authoring paths and contract-only nominal type paths to remove.
- Existing package API machinery reusable for automatic boundary projection.
- Current developer-visible Available/Unavailable output and missing CLI/JSON/receipt/IDE surfaces.
- Minimal implementation DAG split by canonical owner, with focused positive/negative probes.

Do not edit, commit, push or operate stable.
