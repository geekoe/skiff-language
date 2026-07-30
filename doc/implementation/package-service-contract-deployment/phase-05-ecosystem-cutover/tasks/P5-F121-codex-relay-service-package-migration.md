# P5-F121 Codex Relay service-package migration

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §2–§5, §10–§11.
- Audit input: P5-D80.
- Entering Skiff checkpoint: integration through F116.

## Repository/worktree

- Repository: `/Users/geek/workspace/internals`
- Worktree: `/Users/geek/workspace/internals-p5-f121-codex-relay-service`
- Branch: `codex/p5-f121-codex-relay-service`

## Write scope

- `codex-relay/service/**` only.

## Required outcome

Add `package.yml`; move exact dependencies from `service.yml` into it; remove service version and checked-in
`contract.yml`; add/normalize the required dev config profile without moving secrets or Mongo URLs into source;
keep service id/access/HTTP/service-only policy; replace contract-file tests with generated Service API receipt
checks. Preserve behavior.

## Acceptance

New Skiff authoring build/check succeeds; all 30 routes resolve to Available operations; local focused tests
pass; no legacy contract/deployment file; `git diff --check`. No merge/push/watch/stable.
