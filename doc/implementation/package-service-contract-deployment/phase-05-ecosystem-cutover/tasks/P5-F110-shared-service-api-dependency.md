# P5-F110 Shared Service API dependency

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §3–§6 and §10.
- Entering checkpoints: F105 `eb206aa`, F108 `fe1ed14`.
- Audit input: P5-D78.

## DAG node

Make `package.yml.services` materialize the published ServiceContract as the same canonical typed public API
view used by package dependencies, while retaining service-call lowering.

## Write scope

- Compiler input/source dependency indexing, name/type/expression resolution and lowering seam.
- Focused compiler tests.

Do not modify service projection identity/schema, generated deployment, CLI/dev-sync, Registry/Internals,
Router or Runtime.

## Required semantics

- A service alias exposes exported nominal types, constructors, fields, generics, interfaces and operations
  through the ordinary package language mechanisms.
- Nominal identity is the published Service API identity, independent of provider build.
- `alias/path(...)` for a service dependency lowers to `ServiceCallRef`; the same syntax for a package
  dependency lowers to `PackageCallable`.
- Package and service aliases remain disjoint by validated manifest kind and share one namespace.
- Remove contract-only field/constructor/expression frontend paths and JSON/structural bridges.
- Unknown type/member, wrong API identity, alias conflict and unclosed schema fail closed.

## Acceptance

- Positive fixture constructs and reads a service API record and calls a service operation.
- Equivalent package call still uses direct-call lowering.
- Negative fixtures cover identity/type/member/alias failures.
- Existing source/lowering service-call and package-call tests plus `git diff --check` pass.

Risk: high, compiler type and call identity. No merge/push/stable.
