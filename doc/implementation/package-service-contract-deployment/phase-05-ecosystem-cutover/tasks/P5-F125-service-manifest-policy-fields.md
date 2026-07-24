# P5-F125 Service manifest policy fields

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §2–§5, §11.
- Entering checkpoint: F105 `eb206aa`.
- Blocker evidence: F121 Codex Relay requires existing service-only `access`; Account requires existing HTTP
  response policy, but the new strict manifest currently allows only id/http/websocket/timeout.

## DAG node

Complete the strict `service.yml` typed schema with the existing service-only routing/access/policy fields
needed by real services, without allowing package-owned data back in.

## Write scope

- Canonical ServiceManifestAuthoring model, parser/validator and focused fixtures/tests.
- Generated deployment/ingress consumers directly affected by these fields.

Do not add version, dependencies, API/type mappings, config values, secrets or Mongo URLs to service.yml;
do not edit real service repos.

## Required outcome

- Preserve and strictly type existing `access` and HTTP response policy semantics used by real service roots.
- Keep id/http/websocket/timeout and reject unknown fields.
- Generated ingress/deployment retains these policies deterministically where they are runtime-relevant.
- Version/dependencies/API mappings remain rejected.

## Acceptance

- Codex Relay and Account manifest fixtures parse with exact policies.
- Missing/unknown/malformed access/response fields fail closed.
- Generated deployment/ingress mutation tests cover the policies.
- Artifact/compiler input/deployment focused tests and `git diff --check` pass.

Risk: high, public service manifest and ingress policy. No merge/push/stable.
