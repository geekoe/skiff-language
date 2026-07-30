# P5-F135 Instance HTTP byte config

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §12.
- Entering Router checkpoint: F127 `6e2d23f`.
- Earlier tooling checkpoint: F117 `83d2267`.

## DAG node

Make every generated Router config explicitly provide required `http.maxRequestBytes` and
`http.maxResponseBytes`.

## Write scope

- Instance/dev/deploy config generation, examples and focused tests.

Do not modify Router/Runtime production code, stable config, service manifests or source repos.

## Required outcome

- Generated Router YAML has port plus both explicit positive byte ceilings.
- No `bodyLimitBytes`, service override or Runtime copy.
- CLI/init inputs are explicit and fail closed; checked-in examples use deliberate values.
- Do not alter the stable `.skiff-instance/config.yml`.

## Acceptance

- Instance/deploy config goldens and local-instance structural checker pass.
- `pnpm --dir scripts type-check` and `git diff --check` pass.

Risk: medium, deployment configuration. No merge/push/stable.
