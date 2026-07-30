# P5-F269 Internals test service migration

## Dependencies

P5-F265 through P5-F267.

## Required implementation

- Migrate Account, Registry, Relay, AIHub, Agent, Agine and other Internals
  tests to explicit test services.
- Group cases by shared configuration; create separate services/profiles for
  genuinely different config such as Relay Admin scenarios.
- Replace subject-private `root.*` access with topLevel dependency alias paths.
- Inline all HTTP/SSE/service effect plans.
- Delete every Internals `skiff.test-doubles.json`.
- Preserve F260 per-case state isolation.

## Acceptance

- Relay reaches 95/95 without per-case config support.
- Account, Registry, AIHub, Agent and Agine canonical suites pass.
- No old test overlay or doubles manifest remains.
- Internals commits, result, no push/stable operation.
