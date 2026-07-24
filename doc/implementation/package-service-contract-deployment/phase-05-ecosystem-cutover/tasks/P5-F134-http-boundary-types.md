# P5-F134 Canonical HTTP boundary types

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §3, §6 and §12.
- Audit input: P5-D81 C4.

## DAG node

Define canonical boundary materialization for `std.http.HttpRequest`, `std.http.HttpResponse` and HTTP
response stream events used by service ingress.

## Write scope

- Canonical artifact/boundary type/value-plan schema, compiler projection and Runtime materialization.
- Focused cross-layer tests.

Do not implement generic service `Stream<T>` calls, Router byte ceilings, service source changes or admit
arbitrary native/capability types.

## Required outcome

- HTTP request/response are ordinary detached boundary values with exact closed fields.
- HTTP response stream event has exact event variants/fields and remains distinct from generic Service API
  streaming.
- DB/socket/file/native handles remain unsupported.
- Real ingress preserves headers/body/status and errors without same-heap identity.

## Acceptance

- Minimal `(HttpRequest)->HttpResponse` handler becomes Available and executes through real ingress
  materialization.
- HTTP response stream event positive; malformed/capability-field negatives fail.
- Artifact/compiler/runtime focused tests and `git diff --check` pass.

Risk: high, public ingress ABI and runtime materialization. No merge/push/stable.
