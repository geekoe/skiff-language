# P5-F130 Account remove service response policy

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §3 and §12.
- Confirmed decision: HTTP byte ceilings are required Router instance config, never service.yml.

## Repository/worktree

- Repository: `/Users/geek/workspace/internals`
- Worktree: `/Users/geek/workspace/internals-p5-f130-account-response-policy`
- Branch: `codex/p5-f130-account-response-policy`

## Write scope/outcome

Remove `skiff-platform/account/service.yml` HTTP response maxBytes and its stale docs/tests only. Preserve
service id, all 21 routes, handlers and behavior. Do not add another limit or edit shared Internals workflow.

## Acceptance

Strict service manifest check passes; 21/21 route projection remains exact; structural probe finds no
service-level response byte policy; `git diff --check`. No merge/push/watch/stable.
