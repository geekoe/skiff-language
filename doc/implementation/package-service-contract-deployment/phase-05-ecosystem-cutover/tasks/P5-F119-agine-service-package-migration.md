# P5-F119 Agine service-package migration

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §2–§5, §10–§11.
- Audit input: P5-D80.
- Entering Skiff checkpoint: integration through F116.

## Repository/worktree

- Repository: `/Users/geek/workspace/internals`
- Worktree: `/Users/geek/workspace/internals-p5-f119-agine-service`
- Branch: `codex/p5-f119-agine-service`

## Write scope

- `agine/service/**` only.

## Required outcome

Add `package.yml`; move exact package/service dependencies from `service.yml` into it; remove service version
and checked-in `contract.yml`; keep service id/HTTP/WebSocket/service-only policy and config bindings; replace
contract-file tests with generated Service API receipt checks. Preserve behavior.

## Acceptance

New Skiff authoring build/check succeeds; dependency/API projections are exact; local focused tests pass;
no legacy contract/deployment file; `git diff --check`. No merge/push/watch/stable.
