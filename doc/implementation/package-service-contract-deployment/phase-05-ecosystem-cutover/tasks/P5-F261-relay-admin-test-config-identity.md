# P5-F261 Relay admin test config identity

## Context

Two Relay admin tests declare config doubles under
`admin_http::...`, while canonical test callable identity is
`admin_http.__test::...`. The override never matches and production reads
default configuration.

## Required implementation

- Update the affected doubles to the exact canonical test identity.
- Audit all Relay config/effect double keys for the same missing `.__test`
  segment.
- Keep exact matching; do not add fuzzy or legacy-key fallback.
- Improve fixture validation/diagnostics if a declared test key cannot match
  any compiled test callable.

## Acceptance

- Both admin config cases consume their intended overrides.
- Stale/misspelled test identities are rejected or clearly diagnosed.
- Relay admin file and complete suite pass those cases.
- Internals fixture checks, relevant test-runner validation, result and commit.
- No push, stable operation or disk cleanup.
