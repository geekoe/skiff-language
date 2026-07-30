# P5-F124 Retired authoring test-runner cleanup

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §2–§5.
- Entering checkpoint: F116 `86187c0`.
- Blocker evidence: F118 real Registry build cannot compile test-runner because it still references
  `PackageManifest.contracts` and retired `AuthoringObject::{Contract,Deployment}`.

## DAG node

Remove direct mechanical consumers of the retired contract/deployment authoring model from Skiff test-runner
and host fixtures so real service-package builds execute.

## Write scope

- `test-runner` and compiler/test fixtures that fail specifically on the removed manifest field/authoring
  variants.
- Focused tests only.

Do not change production compiler authoring semantics, service API/deployment schemas, Registry/Internals,
Router/Runtime or stable.

## Required outcome

- Canonical package fixtures use `package.yml.services`, not removed contracts.
- Host fixtures build service packages through the single Package authoring object with required environment,
  and consume generated receipts.
- No compatibility enum/field is restored.

## Acceptance

- `cargo check -p skiff-test-runner` and affected compiler fixture targets pass.
- A minimal real service package fixture reaches generated package/contract/deployment output.
- Structural probe finds no retired field/variant references in production test infrastructure.
- `git diff --check` passes.

Risk: medium, direct cutover cleanup. No merge/push/stable.
