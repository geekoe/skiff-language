# P5-F117 Instance artifact config cutover

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §12–§13.
- Entering checkpoints: Router F106 `cf60242`, Runtime F107 `c0cf1ed`.
- Audit input: P5-D77.

## DAG node

Update local instance/deploy tooling and examples so only Router owns artifact and Mongo configuration and
Runtime receives both through bootstrap.

## Write scope

- `scripts` instance/deploy/runtime-stack config generation and focused tests.
- Router/Runtime example YAML and directly affected operational docs.

Do not modify Router/Runtime production code, compiler authoring, Registry/Internals or stable instance files.

## Required semantics

- Tooling writes singular Router `artifactsPath` and `serviceDb.mongoUrl`.
- Tooling writes neither Runtime artifactRoot/artifactRoots nor Runtime Mongo configuration.
- Router and Runtime may be on different machines; emitted Router absolute path string is the shared path
  Runtime receives unchanged.
- Remove plural/legacy artifact-root flags and examples.
- Do not start services or alter the stable instance.

## Acceptance

- Instance and deploy generation tests assert exact Router/Runtime YAML ownership.
- Missing path/Mongo inputs fail closed.
- Structural probe finds no generated Runtime artifact/Mongo owner.
- Focused Node tests and `git diff --check` pass.

Risk: medium, deployment tooling. No merge/push/stable.
