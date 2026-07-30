# P5-F123 Internals service workflow cutover

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §2–§5, §10–§13.
- Audit input: P5-D80.
- Entering Skiff checkpoint: integration through F116.

## Repository/worktree

- Repository: `/Users/geek/workspace/internals`
- Worktree: `/Users/geek/workspace/internals-p5-f123-service-workflow`
- Branch: `codex/p5-f123-service-workflow`

## Write scope

- Internals shared `scripts/**`.
- `skiff-platform/client/scripts/generate-services.mjs`.
- `codex-relay/admin/config.mjs`.
- Do not edit any service root owned by F119–F122.

## Required outcome

Replace the hand-authored contract/package/deployment phase workflow with the single service-package authoring
flow and generated receipts; read service id from `service.yml` and human version from `package.yml`; include
Account in canonical source-build coverage; remove consumers of checked-in contract/deployment files.

## Acceptance

Focused workflow/version-reader tests pass; full codex-relay→aihub→agine dependency order remains exact;
Account build is covered; structural probe finds no independent contract/deployment root consumer;
`git diff --check`. No merge/push/watch/stable.
