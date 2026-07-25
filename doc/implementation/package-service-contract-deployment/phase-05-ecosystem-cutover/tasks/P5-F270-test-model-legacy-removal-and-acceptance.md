# P5-F270 Test model legacy removal and acceptance

## Dependencies

P5-F265 through P5-F269.

## Required implementation

- Remove legacy package test overlay assembly and all-symbol `root.*` subject
  visibility paths.
- Remove obsolete docs, fixtures, errors and CLI branches.
- Reject `skiff.test-doubles.json`, overlay-only tests and topLevel access
  outside `kind: test`.
- Run repository-wide tests, full ecosystem canonical graph and artifact
  reverse searches.
- Confirm production artifact identity is unaffected by test-only source,
  effects and config.

## Acceptance

- Full Skiff workspace and script gates pass.
- All official and Internals test services pass.
- Zero production references to the old model.
- Worktrees/branches are merged and cleaned; no push until requested.
