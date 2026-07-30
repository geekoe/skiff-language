# P5-F268 Public repository test service migration

## Dependencies

P5-F265 through P5-F267.

## Required implementation

- Migrate Skiff repository fixtures and official skiff-packages tests into
  explicit `kind: test` services.
- Replace overlay `root.*` access to subject internals with exact
  `alias/source.module.name` topLevel dependency calls.
- Move shared configuration to ordinary `config.<profile>.yml`.
- Inline every effect double into its owning test block.
- Delete all `skiff.test-doubles.json` files.
- Split test services only where configurations differ.

## Acceptance

- Skiff canonical suites and all official Package suites pass.
- No old overlay-private dependency access or doubles manifest remains.
- Test services use ordinary artifact/link/runtime paths.
- Separate commits per repository, results, no push/stable operation.
