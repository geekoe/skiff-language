# P5-F129 Remove legacy service access/response metadata

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §3 and §12.
- Confirmed decisions: Skiff has no `organizationRole` model; HTTP byte ceilings are Router instance config,
  not service metadata.

## DAG node

Delete Skiff production parsing/projection/types/tests/docs for legacy service `access` and
`http.response.maxBytes`.

## Write scope

- Router legacy manifest/artifact projection and directly affected tests.
- Artifact/compiler service manifest DTO tests and operational docs mentioning the removed fields.

Do not edit Internals service roots, add compatibility, alter new Router instance limit implementation or
stable.

## Required outcome

- Service manifests reject `access`, `visibility`, `organizationRole` and HTTP response byte policy.
- Router/runtime artifact routing carries none of these fields.
- No authorization behavior is invented or moved elsewhere.
- HTTP routes/handlers remain.

## Acceptance

- Structural absence probes and strict negative manifest tests pass.
- Router/artifact/compiler focused tests and `git diff --check` pass.

Risk: medium, removal of dead/incorrect public metadata. No merge/push/stable.
