# P5-F122 Account service-package migration

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §2–§5, §10–§11.
- Audit input: P5-D80.
- Entering Skiff checkpoint: integration through F116.

## Repository/worktree

- Repository: `/Users/geek/workspace/internals`
- Worktree: `/Users/geek/workspace/internals-p5-f122-account-service`
- Branch: `codex/p5-f122-account-service`

## Write scope

- `skiff-platform/account/**` only.

## Required outcome

Add `package.yml`; move exact dependencies from `service.yml` into it; remove service version; keep service
id/HTTP/response policy and config bindings. Preserve behavior.

## Acceptance

New Skiff authoring build/check succeeds; all 21 routes resolve to Available operations; local focused tests
pass; `git diff --check`. No merge/push/watch/stable.
